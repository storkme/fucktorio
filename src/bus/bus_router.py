"""Bus routing: vertical item lanes with tap-off crossings handled via undergrounds.

Each item that flows between rows gets a dedicated vertical bus lane.
Lanes run SOUTH (top to bottom).  At the consuming row, the lane turns
EAST into the row's input belt (tap-off).  When a tap-off crosses another
lane's vertical segment, the tap-off goes underground (EAST) past it.
"""

from __future__ import annotations

import itertools
import math
from dataclasses import dataclass, field

from ..models import EntityDirection, PlacedEntity, SolverResult
from ..routing.common import _LANE_CAPACITY, _UG_MAX_REACH, belt_entity_for_rate
from .placer import RowSpan


@dataclass
class BusLane:
    """A single vertical lane on the bus."""

    item: str
    x: int  # column in the layout
    source_y: int  # where items enter (0 for external, output_y for intermediate)
    consumer_rows: list[int]  # indices into row_spans
    producer_row: int | None  # index or None for external
    rate: float = 0.0  # total throughput for belt tier selection
    is_fluid: bool = False
    tap_off_ys: list[int] = field(default_factory=list)
    extra_producer_rows: list[int] = field(default_factory=list)  # additional sub-rows
    balancer_y: int | None = None  # y of lane balancer splitter (None = no balancer)
    # For fluid lanes: (row_index, x, y) of pipe-to-ground exit positions
    fluid_port_positions: list[tuple[int, int, int]] = field(default_factory=list)


def plan_bus_lanes(
    solver_result: SolverResult,
    row_spans: list[RowSpan],
    max_belt_tier: str | None = None,
) -> list[BusLane]:
    """Determine which items need bus lanes and assign x-columns.

    Lanes are ordered so that lanes tapping off at earlier (higher) rows
    are placed on the LEFT, reducing tap-off crossings.
    """
    lanes: list[BusLane] = []
    seen_items: set[str] = set()

    item_to_consumers: dict[str, list[int]] = {}
    for idx, rs in enumerate(row_spans):
        for inp in rs.spec.inputs:
            item_to_consumers.setdefault(inp.item, []).append(idx)

    # External inputs (solid AND fluid)
    for ext in solver_result.external_inputs:
        if ext.item in seen_items:
            continue
        consumers = item_to_consumers.get(ext.item, [])
        if consumers:
            lanes.append(
                BusLane(
                    item=ext.item,
                    x=0,
                    source_y=0,
                    consumer_rows=consumers,
                    producer_row=None,
                    rate=ext.rate,
                    is_fluid=ext.is_fluid,
                )
            )
            seen_items.add(ext.item)

    # Intermediate items (solid AND fluid).
    # A recipe split across multiple sub-rows produces the same item from
    # each sub-row. Aggregate rate and track all producer rows.
    item_to_producers: dict[str, list[int]] = {}
    item_to_rate: dict[str, float] = {}
    item_is_fluid: dict[str, bool] = {}
    for idx, rs in enumerate(row_spans):
        for out in rs.spec.outputs:
            item_to_producers.setdefault(out.item, []).append(idx)
            item_to_rate[out.item] = item_to_rate.get(out.item, 0) + out.rate * rs.machine_count
            item_is_fluid[out.item] = out.is_fluid

    for item, producer_rows in item_to_producers.items():
        if item in seen_items:
            continue
        consumers = item_to_consumers.get(item, [])
        if not consumers:
            continue
        first_producer = producer_rows[0]
        lanes.append(
            BusLane(
                item=item,
                x=0,
                source_y=row_spans[first_producer].output_belt_y,
                consumer_rows=consumers,
                producer_row=first_producer,
                rate=item_to_rate[item],
                is_fluid=item_is_fluid[item],
                extra_producer_rows=producer_rows[1:],
            )
        )
        seen_items.add(item)

    # Output collection for final products is skipped for now — each producer
    # row's output belt goes WEST independently.  The user can connect them
    # downstream.  TODO: merge output belts properly in future.

    # Split lanes that exceed max belt tier capacity into parallel trunks
    lanes = _split_overflowing_lanes(lanes, row_spans, max_belt_tier)

    # Pre-compute tap-off ys before sorting
    for lane in lanes:
        lane.tap_off_ys = _find_tap_off_ys(lane, row_spans)
        if lane.is_fluid:
            # Collect fluid port pipe positions for tap-off routing
            for ri in lane.consumer_rows:
                rs = row_spans[ri]
                for px, py in rs.fluid_port_pipes:
                    lane.fluid_port_positions.append((ri, px, py))

    # Compute lane balancer positions for intermediate solid lanes.
    # Balancers go after the last producer output return, before first tap-off.
    # Only for collector lanes (producers but no consumers).  Intermediate lanes
    # use direct routing (no trunk), and external lanes have no producers.
    for lane in lanes:
        if lane.is_fluid or lane.consumer_rows:
            continue  # skip intermediate and external lanes
        all_producers = []
        if lane.producer_row is not None:
            all_producers.append(lane.producer_row)
        all_producers.extend(lane.extra_producer_rows)
        if not all_producers:
            continue
        last_sideload_y = max(row_spans[pri].output_belt_y for pri in all_producers)
        bal_y = last_sideload_y + 1
        tap_set = set(lane.tap_off_ys)
        if bal_y not in tap_set and (bal_y + 1) not in tap_set:
            lane.balancer_y = bal_y

    # Optimize lane left-to-right ordering to minimize underground crossings.
    lanes = _optimize_lane_order(lanes, row_spans)

    # Assign x-columns after ordering.  Start at x=1 so every lane has a
    # gap column at x-1 available for the lane balancer's left splitter tile.
    for i, lane in enumerate(lanes):
        lane.x = i * 2 + 1

    return lanes


def _score_lane_ordering(
    ordered: list[BusLane],
    row_spans: list[RowSpan],
) -> int:
    """Count total underground crossings for a given lane ordering.

    A crossing occurs when:
    - An EAST tap-off at lane position p crosses an active vertical lane to its right
    - A WEST output return at lane position p crosses an active vertical lane to its right

    A lane is "active" at y if source_y <= y and it hasn't turned east yet.
    With 1:1 mapping, a lane's active range is [source_y, consumer_y).
    Collector lanes (no consumers) are active from source_y to their end_y.
    """
    n = len(ordered)
    score = 0

    def _active_range(lane: BusLane) -> tuple[int, int]:
        """Return (start_y, end_y) where the lane occupies its vertical column."""
        all_p = []
        if lane.producer_row is not None:
            all_p.append(lane.producer_row)
        all_p.extend(lane.extra_producer_rows)

        if all_p and lane.consumer_rows:
            # Intermediate: vertical segment from producer output to consumer tap
            start = min(row_spans[p].output_belt_y for p in all_p)
            end = max(lane.tap_off_ys) if lane.tap_off_ys else start
        elif lane.tap_off_ys:
            # External input: trunk from source_y to consumer tap
            start = lane.source_y
            end = max(lane.tap_off_ys)
        else:
            # Collector: active through all producer returns
            start = lane.source_y
            end = max((row_spans[p].output_belt_y for p in all_p), default=start)
        return start, end

    ranges = [_active_range(l) for l in ordered]

    for pos in range(n):
        lane = ordered[pos]
        # EAST tap-off crossings: count active lanes to the RIGHT
        for tap_y in lane.tap_off_ys:
            for rpos in range(pos + 1, n):
                rs, re = ranges[rpos]
                if rs <= tap_y <= re:
                    score += 1

        # WEST output return crossings: count active lanes to the RIGHT
        all_producers = []
        if lane.producer_row is not None:
            all_producers.append(lane.producer_row)
        all_producers.extend(lane.extra_producer_rows)
        for pri in all_producers:
            ret_y = row_spans[pri].output_belt_y
            for rpos in range(pos + 1, n):
                rs, re = ranges[rpos]
                if rs <= ret_y <= re:
                    score += 1

    return score


def _optimize_lane_order(
    lanes: list[BusLane],
    row_spans: list[RowSpan],
) -> list[BusLane]:
    """Find the left-to-right lane ordering that minimizes underground crossings."""
    if len(lanes) <= 1:
        return lanes

    # Separate fluid lanes (placed after solids)
    solid = [l for l in lanes if not l.is_fluid]
    fluid = [l for l in lanes if l.is_fluid]

    if len(solid) <= 10:
        best_order: list[BusLane] | None = None
        best_score = float("inf")
        for perm in itertools.permutations(range(len(solid))):
            ordered = [solid[i] for i in perm]
            score = _score_lane_ordering(ordered, row_spans)
            if score < best_score:
                best_score = score
                best_order = ordered
        if best_order is not None:
            solid = best_order
    else:
        # Heuristic: lanes with later consumers on the left (outside)
        solid.sort(
            key=lambda ln: -(min(ln.tap_off_ys) if ln.tap_off_ys else 9999)
        )

    return solid + fluid


def _split_overflowing_lanes(
    lanes: list[BusLane],
    row_spans: list[RowSpan],
    max_belt_tier: str | None = None,
) -> list[BusLane]:
    """Split lanes whose rate exceeds the available belt's per-lane capacity.

    When a lane carries e.g. 20/s but yellow belt only supports 7.5/s per lane,
    split into ceil(20/7.5) = 3 parallel trunk lanes, distributing consumer rows
    across them.
    """
    # Use full belt capacity (both lanes) as threshold — lane balancers
    # placed on trunks ensure both lanes are utilised.
    if max_belt_tier and max_belt_tier in _BELT_CAPACITY:
        max_lane_cap = _BELT_CAPACITY[max_belt_tier]
    else:
        max_lane_cap = max(_BELT_CAPACITY.values())

    result: list[BusLane] = []
    for lane in lanes:
        if lane.is_fluid:
            result.append(lane)
            continue

        # Split by rate (belt capacity) and by consumer count (1:1 mapping).
        # Each trunk turns EAST into exactly one consumer row — no splitter
        # tap-offs needed.
        n_splits = math.ceil(lane.rate / max_lane_cap) if lane.rate > max_lane_cap else 1
        if lane.consumer_rows:
            n_splits = max(n_splits, len(lane.consumer_rows))

        if n_splits <= 1:
            result.append(lane)
            continue
        # Distribute consumer rows round-robin across splits
        consumers_per_split: list[list[int]] = [[] for _ in range(n_splits)]
        for i, ri in enumerate(lane.consumer_rows):
            consumers_per_split[i % n_splits].append(ri)

        # Distribute producer rows across splits, balancing total production
        # rate.  Producers may have very different machine counts (e.g. 24+8),
        # so round-robin by index would be unbalanced.
        all_producer_rows = []
        if lane.producer_row is not None:
            all_producer_rows.append(lane.producer_row)
        all_producer_rows.extend(lane.extra_producer_rows)
        producers_per_split: list[list[int]] = [[] for _ in range(n_splits)]
        split_prod_rate = [0.0] * n_splits
        for pri in all_producer_rows:
            rs = row_spans[pri]
            prod_rate = sum(o.rate * rs.machine_count for o in rs.spec.outputs if o.item == lane.item)
            # Assign to the split with the least accumulated production rate
            target = min(range(n_splits), key=lambda s: split_prod_rate[s])
            producers_per_split[target].append(pri)
            split_prod_rate[target] += prod_rate

        is_collector = not lane.consumer_rows
        for si in range(n_splits):
            consumers = consumers_per_split[si]
            if not consumers and not is_collector and si > 0:
                continue  # skip empty splits (but keep collector trunks)
            split_rate = lane.rate / n_splits
            # First producer in this split becomes producer_row, rest are extras
            prods = producers_per_split[si]
            first_prod = prods[0] if prods else None
            extra_prods = prods[1:] if len(prods) > 1 else []
            result.append(
                BusLane(
                    item=lane.item,
                    x=0,  # reassigned later
                    source_y=lane.source_y,
                    consumer_rows=consumers,
                    producer_row=first_prod,
                    rate=split_rate,
                    is_fluid=lane.is_fluid,
                    extra_producer_rows=extra_prods,
                )
            )

    return result


def _find_tap_off_ys(lane: BusLane, row_spans: list[RowSpan]) -> list[int]:
    """Find y-coordinates where this lane taps off into consumer rows."""
    tap_ys: list[int] = []
    for ri in lane.consumer_rows:
        rs = row_spans[ri]
        if lane.is_fluid:
            # Fluid lanes tap off at the fluid port y positions
            for port_y in rs.fluid_port_ys:
                tap_ys.append(port_y)
                break  # one tap per consumer row
        else:
            solid_inputs = [f for f in rs.spec.inputs if not f.is_fluid]
            for input_idx, inp in enumerate(solid_inputs):
                if inp.item == lane.item and input_idx < len(rs.input_belt_y):
                    tap_ys.append(rs.input_belt_y[input_idx])
                    break
    return tap_ys


def bus_width_for_lanes(lanes: list[BusLane]) -> int:
    if not lanes:
        return 2
    return len(lanes) * 2 + 1


def route_bus(
    lanes: list[BusLane],
    row_spans: list[RowSpan],
    total_height: int,
    bw: int,
    max_belt_tier: str | None = None,
    row_entities: list[PlacedEntity] | None = None,
) -> tuple[list[PlacedEntity], int]:
    """Create all bus belt entities.

    Returns (entities, max_y) where max_y accounts for any merger blocks
    placed below the last row.

    Uses lane-first negotiated congestion routing (Rust) to detect
    crossing conflicts between ALL lane segments (including mergers),
    then renders belt entities with underground crossings at conflict points.
    """
    # Pre-compute the full crossing map using Rust negotiation.
    # This lets tap-offs/returns know about merger positions BEFORE routing.
    crossing_map = _negotiate_crossings(lanes, row_spans, total_height, bw, row_entities)

    entities: list[PlacedEntity] = []
    max_y = total_height
    for lane in lanes:
        _route_lane(entities, lane, lanes, row_spans, bw, max_belt_tier, crossing_map)

    # Group same-item lanes that were split into parallel trunks.
    # Merge them with splitters if there are more trunks than needed.
    item_lane_groups: dict[str, list[BusLane]] = {}
    for lane in lanes:
        if lane.is_fluid:
            continue
        item_lane_groups.setdefault(lane.item, []).append(lane)

    for _item, group in item_lane_groups.items():
        if len(group) <= 1:
            continue
        # Skip merger for lanes where every split has a consumer (items are
        # fully consumed via belt turns — nothing left to merge).
        if all(ln.consumer_rows for ln in group):
            continue
        merger_ents, merger_end_y = _place_merger_block(
            group, row_spans, total_height, entities, max_belt_tier,
        )
        entities.extend(merger_ents)
        max_y = max(max_y, merger_end_y)

    return entities, max_y


def _negotiate_crossings(
    lanes: list[BusLane],
    row_spans: list[RowSpan],
    total_height: int,
    bw: int,
    row_entities: list[PlacedEntity] | None = None,
) -> dict[tuple[int, int], set[str]]:
    """Use Rust negotiation to find all crossing points between lane segments.

    Returns a map of (x, y) → set of item names that occupy that tile.
    Tiles with multiple different items are crossing points where
    underground belts should be used.
    """
    try:
        from fucktorio_native import PyLaneSpec, negotiate_lanes
    except ImportError:
        return {}  # graceful fallback: no crossing info, use legacy routing

    # Build item → numeric ID mapping
    items = sorted({lane.item for lane in lanes if not lane.is_fluid})
    item_to_id: dict[str, int] = {item: i for i, item in enumerate(items)}

    specs: list[PyLaneSpec] = []
    lane_id = 0

    for lane in lanes:
        if lane.is_fluid:
            continue
        item_id = item_to_id.get(lane.item, 0)
        x = lane.x

        all_producers = []
        if lane.producer_row is not None:
            all_producers.append(lane.producer_row)
        all_producers.extend(lane.extra_producer_rows)

        if _is_intermediate(lane):
            # Intermediate: return paths (WEST) + trunk (SOUTH) + tap-off (EAST)
            producer_out_ys = [row_spans[p].output_belt_y for p in all_producers]
            start_y = min(producer_out_ys)
            tap_y = lane.tap_off_ys[0] if lane.tap_off_ys else start_y

            # Return paths: horizontal WEST
            for pri in all_producers:
                out_y = row_spans[pri].output_belt_y
                specs.append(PyLaneSpec(
                    id=lane_id, item_id=item_id,
                    waypoints=[(bw - 1, out_y), (x + 1, out_y)],
                    strategy=0, priority=3,
                ))
                lane_id += 1

            # Trunk + tap-off: vertical then horizontal
            specs.append(PyLaneSpec(
                id=lane_id, item_id=item_id,
                waypoints=[(x, start_y), (x, tap_y), (bw - 1, tap_y)],
                strategy=0, priority=5,
            ))
            lane_id += 1

        elif lane.consumer_rows:
            # External input: trunk from source to tap-off
            tap_y = lane.tap_off_ys[0] if lane.tap_off_ys else lane.source_y
            specs.append(PyLaneSpec(
                id=lane_id, item_id=item_id,
                waypoints=[(x, lane.source_y), (x, tap_y), (bw - 1, tap_y)],
                strategy=0, priority=5,
            ))
            lane_id += 1

        else:
            # Collector: trunk + returns
            all_ys = list(lane.tap_off_ys)
            for pri in all_producers:
                all_ys.append(row_spans[pri].output_belt_y)
            end_y = max(all_ys) if all_ys else lane.source_y
            if lane.balancer_y is not None:
                end_y = max(end_y, lane.balancer_y + 1)

            # Trunk
            specs.append(PyLaneSpec(
                id=lane_id, item_id=item_id,
                waypoints=[(x, lane.source_y), (x, end_y)],
                strategy=0, priority=5,
            ))
            lane_id += 1

            # Returns
            for pri in all_producers:
                out_y = row_spans[pri].output_belt_y
                specs.append(PyLaneSpec(
                    id=lane_id, item_id=item_id,
                    waypoints=[(bw - 1, out_y), (x + 1, out_y)],
                    strategy=0, priority=3,
                ))
                lane_id += 1

    # Add merger segments — these are the routes that were previously invisible
    # to tap-off routing.
    item_lane_groups: dict[str, list[BusLane]] = {}
    for lane in lanes:
        if lane.is_fluid:
            continue
        item_lane_groups.setdefault(lane.item, []).append(lane)

    for _item, group in item_lane_groups.items():
        if len(group) <= 1 or all(ln.consumer_rows for ln in group):
            continue
        item_id = item_to_id.get(_item, 0)
        trunk_xs = sorted(ln.x for ln in group)
        # Merger block starts at total_height, merges pairs with WEST routes
        merge_y = total_height
        # Trunk extensions to merge_y
        for ln in group:
            specs.append(PyLaneSpec(
                id=lane_id, item_id=item_id,
                waypoints=[(ln.x, ln.source_y), (ln.x, merge_y + 3)],
                strategy=0, priority=8,  # high priority — merger is fixed
            ))
            lane_id += 1
        # Horizontal merger routes between pairs
        i = 0
        while i + 1 < len(trunk_xs):
            left_x = trunk_xs[i]
            right_x = trunk_xs[i + 1]
            specs.append(PyLaneSpec(
                id=lane_id, item_id=item_id,
                waypoints=[(right_x, merge_y), (left_x + 1, merge_y)],
                strategy=0, priority=8,
            ))
            lane_id += 1
            i += 2

    # Collect fixed obstacles from row entities
    obstacles: list[tuple[int, int]] = []
    if row_entities:
        _MACHINE_ENTITIES = {
            "assembling-machine-1", "assembling-machine-2", "assembling-machine-3",
            "chemical-plant", "electric-furnace", "oil-refinery",
        }
        for e in row_entities:
            if e.name in _MACHINE_ENTITIES:
                from ..routing.common import machine_size
                sz = machine_size(e.name)
                for dx in range(sz):
                    for dy in range(sz):
                        obstacles.append((e.x + dx, e.y + dy))
            else:
                obstacles.append((e.x, e.y))

    if not specs:
        return {}

    # Run negotiation
    routed = negotiate_lanes(specs, obstacles, max_extent=max(bw, total_height) + 50)

    # Build crossing map: (x, y) → set of items on that tile
    tile_items: dict[tuple[int, int], set[str]] = {}
    id_to_item = {v: k for k, v in item_to_id.items()}
    for r in routed:
        item_name = id_to_item.get(r.item_id, "")
        for pos in r.path:
            tile_items.setdefault(pos, set()).add(item_name)

    return tile_items


def _place_merger_block(
    trunk_lanes: list[BusLane],
    row_spans: list[RowSpan],
    merge_start_y: int,
    existing_entities: list[PlacedEntity],
    max_belt_tier: str | None = None,
) -> tuple[list[PlacedEntity], int]:
    """Merge N parallel trunk lanes into M output belts using splitters.

    M = ceil(total_rate / full_belt_capacity).  The merger block is placed
    below the last row at merge_start_y.  Extends each trunk downward from
    its end_y to merge_start_y so items can flow into the merger.

    Returns (entities, end_y).
    """
    entities: list[PlacedEntity] = []
    total_rate = sum(ln.rate for ln in trunk_lanes)

    # Determine belt tier and capacity
    belt_name = belt_entity_for_rate(total_rate * 2, max_tier=max_belt_tier)
    full_cap = _BELT_CAPACITY.get(belt_name, 15.0)
    target_m = max(1, math.ceil(total_rate / full_cap))

    trunk_xs = sorted(ln.x for ln in trunk_lanes)
    n = len(trunk_xs)

    if n <= target_m:
        return entities, merge_start_y

    splitter_name = _SPLITTER_MAP.get(belt_name, "splitter")
    item = trunk_lanes[0].item

    # Build set of already-occupied positions to avoid overlaps
    occupied: set[tuple[int, int]] = {(e.x, e.y) for e in existing_entities}

    # Extend each trunk from its current end_y to merge_start_y
    for lane in trunk_lanes:
        all_ys = list(lane.tap_off_ys)
        for pri in lane.extra_producer_rows:
            all_ys.append(row_spans[pri].output_belt_y)
        end_y = max(all_ys) if all_ys else lane.source_y
        for y in range(end_y + 1, merge_start_y):
            if (lane.x, y) in occupied:
                continue  # skip tiles occupied by tap-offs etc.
            entities.append(
                PlacedEntity(
                    name=belt_name,
                    x=lane.x,
                    y=y,
                    direction=EntityDirection.SOUTH,
                    carries=item,
                )
            )

    y_cursor = merge_start_y
    current_xs = list(trunk_xs)

    while len(current_xs) > target_m:
        # How many pairs to merge this stage (at most half, enough to reach target)
        pairs_needed = min(len(current_xs) - target_m, len(current_xs) // 2)
        next_xs: list[int] = []
        i = 0
        pairs_done = 0

        while i < len(current_xs):
            if pairs_done < pairs_needed and i + 1 < len(current_xs):
                left_x = current_xs[i]
                right_x = current_xs[i + 1]
                # Route right trunk to left_x + 1 using horizontal WEST belts
                for rx in range(right_x, left_x, -1):
                    entities.append(
                        PlacedEntity(
                            name=belt_name,
                            x=rx,
                            y=y_cursor,
                            direction=EntityDirection.WEST,
                            carries=item,
                        )
                    )
                # Continue left trunk straight down
                entities.append(
                    PlacedEntity(
                        name=belt_name,
                        x=left_x,
                        y=y_cursor,
                        direction=EntityDirection.SOUTH,
                        carries=item,
                    )
                )
                # Splitter (SOUTH-facing, occupies left_x and left_x+1)
                entities.append(
                    PlacedEntity(
                        name=splitter_name,
                        x=left_x,
                        y=y_cursor + 1,
                        direction=EntityDirection.SOUTH,
                        carries=item,
                    )
                )
                # Output belt on the left side only (right side empty → all items go left)
                entities.append(
                    PlacedEntity(
                        name=belt_name,
                        x=left_x,
                        y=y_cursor + 2,
                        direction=EntityDirection.SOUTH,
                        carries=item,
                    )
                )
                next_xs.append(left_x)
                pairs_done += 1
                i += 2
            else:
                # Passthrough — extend this trunk down through the merge stage
                px = current_xs[i]
                for dy in range(3):
                    entities.append(
                        PlacedEntity(
                            name=belt_name,
                            x=px,
                            y=y_cursor + dy,
                            direction=EntityDirection.SOUTH,
                            carries=item,
                        )
                    )
                next_xs.append(px)
                i += 1

        y_cursor += 3  # each stage is 3 rows: route + splitter + output
        current_xs = next_xs

    return entities, y_cursor


def _route_lane(
    entities: list[PlacedEntity],
    lane: BusLane,
    all_lanes: list[BusLane],
    row_spans: list[RowSpan],
    bw: int,
    max_belt_tier: str | None = None,
    crossing_map: dict[tuple[int, int], set[str]] | None = None,
) -> None:
    """Route a single bus lane: vertical segment + tap-offs + output return."""
    if lane.is_fluid:
        _route_fluid_lane(entities, lane, bw)
    elif _is_intermediate(lane):
        _route_intermediate_lane(entities, lane, all_lanes, row_spans, bw, max_belt_tier, crossing_map)
    else:
        _route_belt_lane(entities, lane, all_lanes, row_spans, bw, max_belt_tier, crossing_map)


def _is_intermediate(lane: BusLane) -> bool:
    """True if lane has both producers and consumers (intermediate item)."""
    has_producers = lane.producer_row is not None or lane.extra_producer_rows
    has_consumers = bool(lane.consumer_rows)
    return has_producers and has_consumers


def _route_intermediate_lane(
    entities: list[PlacedEntity],
    lane: BusLane,
    all_lanes: list[BusLane],
    row_spans: list[RowSpan],
    bw: int,
    max_belt_tier: str | None = None,
    crossing_map: dict[tuple[int, int], set[str]] | None = None,
) -> None:
    """Route an intermediate lane: direct path from producer output to consumer input.

    Instead of trunk + tap-off + return, the producer output belt goes WEST to
    lane.x, then SOUTH to the consumer input y, then EAST to the consumer.
    One continuous belt path, no separate infrastructure.
    """
    x = lane.x
    belt_name = belt_entity_for_rate(lane.rate * 2, max_tier=max_belt_tier)

    # All producer output ys
    all_producers = []
    if lane.producer_row is not None:
        all_producers.append(lane.producer_row)
    all_producers.extend(lane.extra_producer_rows)

    # Output returns: WEST from row edge to lane.x, sideloading onto vertical
    for pri in all_producers:
        out_y = row_spans[pri].output_belt_y
        ret_blocked = _blocked_xs_at(lane, out_y, all_lanes, row_spans, crossing_map)
        _route_horizontal(entities, lane, out_y, x + 1, bw - 1, EntityDirection.WEST, ret_blocked, belt_name)

    # Vertical segment: SOUTH from first producer output to consumer tap-off
    assert lane.consumer_rows, "Intermediate lane must have a consumer"
    tap_y = lane.tap_off_ys[0] if lane.tap_off_ys else row_spans[lane.consumer_rows[0]].input_belt_y[0]

    # Start y: the earliest producer output y (items enter via sideload)
    producer_out_ys = [row_spans[p].output_belt_y for p in all_producers]
    start_y = min(producer_out_ys)

    for y in range(start_y, tap_y):
        entities.append(
            PlacedEntity(
                name=belt_name,
                x=x,
                y=y,
                direction=EntityDirection.SOUTH,
                carries=lane.item,
            )
        )

    # Tap-off: EAST from lane.x to row input belt
    tap_blocked = _blocked_xs_at(lane, tap_y, all_lanes, row_spans, crossing_map)
    _route_horizontal(entities, lane, tap_y, x, bw - 1, EntityDirection.EAST, tap_blocked, belt_name)


def _route_belt_lane(
    entities: list[PlacedEntity],
    lane: BusLane,
    all_lanes: list[BusLane],
    row_spans: list[RowSpan],
    bw: int,
    max_belt_tier: str | None = None,
    crossing_map: dict[tuple[int, int], set[str]] | None = None,
) -> None:
    """Route a solid-item bus lane with belts."""
    x = lane.x
    tap_off_set = set(lane.tap_off_ys)

    start_y = lane.source_y
    # End y must cover all tap-offs AND all extra producer output belts
    all_ys = list(lane.tap_off_ys)
    for pri in lane.extra_producer_rows:
        all_ys.append(row_spans[pri].output_belt_y)
    end_y = max(all_ys) if all_ys else start_y

    # Extend end_y to include lane balancer if present
    if lane.balancer_y is not None:
        end_y = max(end_y, lane.balancer_y + 1)

    # Trunk belt: with a balancer both lanes are used, so select from full rate.
    # Horizontal belts (tap-offs, output returns) always sideload onto one lane.
    if lane.balancer_y is not None:
        belt_name = belt_entity_for_rate(lane.rate, max_tier=max_belt_tier)
    else:
        belt_name = belt_entity_for_rate(lane.rate * 2, max_tier=max_belt_tier)
    # Tap-off/return belts carry full rate on one lane — always use per-lane sizing
    horiz_belt = belt_entity_for_rate(lane.rate * 2, max_tier=max_belt_tier)
    # Pre-balancer trunk segment carries full rate on one lane.  Respect the
    # belt tier constraint — items buffer briefly before the balancer, which
    # is fine in Factorio.
    pre_bal_belt = belt_entity_for_rate(lane.rate * 2, max_tier=max_belt_tier) if lane.balancer_y is not None else belt_name

    # Positions occupied by the lane balancer splitter (skip from vertical belt loop)
    balancer_skip = {lane.balancer_y} if lane.balancer_y is not None else set()

    # Vertical surface belts (SOUTH), skipping tap-off and balancer positions.
    # Before the balancer, items are on one lane — use the higher-tier horiz_belt.
    # After the balancer, items are balanced — use the cheaper belt_name.
    bal_y = lane.balancer_y
    for y in range(start_y, end_y + 1):
        if y in tap_off_set or y in balancer_skip:
            continue
        tier = pre_bal_belt if (bal_y is not None and y < bal_y) else belt_name
        entities.append(
            PlacedEntity(
                name=tier,
                x=x,
                y=y,
                direction=EntityDirection.SOUTH,
                carries=lane.item,
            )
        )

    # Place lane balancer: splitter (left tile at x-1, right tile at x)
    # + EAST sideload belt at x-1 to feed left output onto trunk's left lane
    if lane.balancer_y is not None:
        by = lane.balancer_y
        splitter_name = _SPLITTER_MAP.get(belt_name, "splitter")
        entities.append(
            PlacedEntity(
                name=splitter_name,
                x=x - 1,
                y=by,
                direction=EntityDirection.SOUTH,
                carries=lane.item,
            )
        )
        # Left output sideloads EAST from x-1 onto trunk at x (hits left lane)
        entities.append(
            PlacedEntity(
                name=belt_name,
                x=x - 1,
                y=by + 1,
                direction=EntityDirection.EAST,
                carries=lane.item,
            )
        )

    # Tap-off horizontal belts (EAST) from bus column to row start.
    # When crossing another lane's vertical segment, go underground.
    for tap_y in lane.tap_off_ys:
        _route_tap_off(entities, lane, tap_y, all_lanes, row_spans, bw, horiz_belt, crossing_map)

    # Output return: WEST belts from row edge back to bus column.
    # Must go underground past other lanes' vertical segments (same as tap-offs).
    # Route returns for ALL producer rows (including extra sub-rows).
    all_producers = []
    if lane.producer_row is not None:
        all_producers.append(lane.producer_row)
    all_producers.extend(lane.extra_producer_rows)
    for pri in all_producers:
        out_y = row_spans[pri].output_belt_y
        _route_output_return(entities, lane, out_y, all_lanes, row_spans, bw, horiz_belt, crossing_map)


def _route_fluid_lane(
    entities: list[PlacedEntity],
    lane: BusLane,
    bw: int,
) -> None:
    """Route a fluid bus lane with pipes + pipe-to-ground tap-offs."""
    x = lane.x

    start_y = lane.source_y
    end_y = max(lane.tap_off_ys) if lane.tap_off_ys else start_y

    # Vertical pipe run on the bus
    for y in range(start_y, end_y + 1):
        entities.append(PlacedEntity(name="pipe", x=x, y=y, carries=lane.item))

    # Pipe-to-ground tap-offs: tunnel EAST from bus+1 to the machine port
    for _ri, port_x, port_y in lane.fluid_port_positions:
        # Entry: one tile right of the bus pipe (x+1), at the port's y
        entry_x = x + 1
        # Exit: one tile left of the port pipe position
        exit_x = port_x - 1

        if exit_x > entry_x:
            entities.append(
                PlacedEntity(
                    name="pipe-to-ground",
                    x=entry_x,
                    y=port_y,
                    direction=EntityDirection.EAST,
                    io_type="input",
                    carries=lane.item,
                )
            )
            entities.append(
                PlacedEntity(
                    name="pipe-to-ground",
                    x=exit_x,
                    y=port_y,
                    direction=EntityDirection.EAST,
                    io_type="output",
                    carries=lane.item,
                )
            )
        elif exit_x == entry_x:
            # Adjacent — just a surface pipe
            entities.append(PlacedEntity(name="pipe", x=entry_x, y=port_y, carries=lane.item))
        # The port pipe itself is placed by the template


_UG_MAP = {
    "transport-belt": "underground-belt",
    "fast-transport-belt": "fast-underground-belt",
    "express-transport-belt": "express-underground-belt",
}

_SPLITTER_MAP = {
    "transport-belt": "splitter",
    "fast-transport-belt": "fast-splitter",
    "express-transport-belt": "express-splitter",
}

# Full belt capacity (both lanes)
_BELT_CAPACITY = {k: v * 2 for k, v in _LANE_CAPACITY.items()}


def _underground_for(belt: str) -> str:
    return _UG_MAP.get(belt, "underground-belt")


def _blocked_xs_at(
    lane: BusLane,
    y: int,
    all_lanes: list[BusLane],
    row_spans: list[RowSpan] | None = None,
    crossing_map: dict[tuple[int, int], set[str]] | None = None,
) -> set[int]:
    """Find x-columns that should be crossed underground at y.

    Uses two sources:
    1. Legacy: other lanes' active vertical segments (same as before)
    2. Crossing map from Rust negotiation: tiles where a different item
       occupies the same position (includes merger blocks, balancers, etc.)
    """
    blocked: set[int] = set()
    for other in all_lanes:
        if other is lane:
            continue
        other_start = other.source_y
        all_ys = list(other.tap_off_ys)
        if row_spans:
            for pri in other.extra_producer_rows:
                all_ys.append(row_spans[pri].output_belt_y)
        other_end = max(all_ys) if all_ys else other_start
        other_taps = set(other.tap_off_ys)
        if other_start <= y <= other_end + 1 and y not in other_taps:
            blocked.add(other.x)
        # Block gap column at lane balancer positions (splitter left tile + sideload belt)
        if other.balancer_y is not None and y in (other.balancer_y, other.balancer_y + 1):
            blocked.add(other.x - 1)

    # Augment with crossing map from Rust negotiation — blocks columns
    # where a different item's lane path exists (e.g. merger splitters).
    # Also blocks adjacent tiles for splitter-width entities (2 tiles wide).
    if crossing_map:
        max_x = max((ln.x for ln in all_lanes), default=0) + 2
        for x in range(0, max_x):
            items_at = crossing_map.get((x, y))
            if items_at and lane.item not in items_at:
                blocked.add(x)
                # Splitters are 2 tiles wide — also block adjacent tile
                if x > 0:
                    blocked.add(x - 1)

    return blocked


def _ug_for_span(belt_name: str, span: int) -> str:
    """Pick the cheapest underground belt tier that can cover *span* tiles.

    Prefers the same tier as *belt_name* but upgrades if the span exceeds
    that tier's max reach.  Tap-off crossings don't need to match the trunk
    tier — they just need to clear the bus columns.
    """
    # Try tiers from cheapest to most expensive
    tiers = [
        ("underground-belt", _UG_MAX_REACH.get("transport-belt", 4)),
        ("fast-underground-belt", _UG_MAX_REACH.get("fast-transport-belt", 6)),
        ("express-underground-belt", _UG_MAX_REACH.get("express-transport-belt", 8)),
    ]
    preferred = _underground_for(belt_name)
    # Start from the preferred tier, upgrade if needed
    started = False
    for ug_name, reach in tiers:
        if ug_name == preferred:
            started = True
        if started and reach >= span:
            return ug_name
    # Fallback: express (longest reach)
    return "express-underground-belt"


def _route_horizontal(
    entities: list[PlacedEntity],
    lane: BusLane,
    y: int,
    x_from: int,
    x_to: int,
    direction: EntityDirection,
    blocked_xs: set[int],
    belt_name: str,
) -> None:
    """Route a horizontal belt run, going underground past blocked columns.

    Underground belt tier is auto-upgraded when the span exceeds the base
    tier's reach — tap-off crossings don't need to match the trunk tier.
    """

    if direction == EntityDirection.EAST:
        hx = x_from
        while hx <= x_to:
            if hx in blocked_xs:
                entry_x = hx - 1
                exit_x = hx + 1
                # Extend past all consecutive blocked columns, including
                # cases where the tile after the exit is also blocked.
                while exit_x in blocked_xs or (exit_x + 1) in blocked_xs:
                    exit_x += 2
                # Clamp exit to x_to if it overshoots (blocked zone at edge)
                if exit_x > x_to:
                    exit_x = x_to
                if entry_x >= x_from and exit_x > entry_x:
                    # Remove surface belt we may have just placed at entry_x
                    entities[:] = [
                        e
                        for e in entities
                        if not (e.x == entry_x and e.y == y and e.name == belt_name and e.direction == direction)
                    ]
                    span = exit_x - entry_x
                    ug_name = _ug_for_span(belt_name, span)
                    entities.append(
                        PlacedEntity(
                            name=ug_name,
                            x=entry_x,
                            y=y,
                            direction=EntityDirection.EAST,
                            io_type="input",
                            carries=lane.item,
                        )
                    )
                    entities.append(
                        PlacedEntity(
                            name=ug_name,
                            x=exit_x,
                            y=y,
                            direction=EntityDirection.EAST,
                            io_type="output",
                            carries=lane.item,
                        )
                    )
                    hx = exit_x + 1
                    continue
            entities.append(
                PlacedEntity(
                    name=belt_name,
                    x=hx,
                    y=y,
                    direction=direction,
                    carries=lane.item,
                )
            )
            hx += 1
    else:  # WEST — items flow right-to-left
        hx = x_to
        while hx >= x_from:
            if hx in blocked_xs:
                # For WEST: entry (input) is on the RIGHT (high x),
                # exit (output) is on the LEFT (low x)
                ug_input_x = hx + 1  # items enter underground here
                ug_output_x = hx - 1  # items emerge here
                while ug_output_x in blocked_xs or (ug_output_x - 1) in blocked_xs:
                    ug_output_x -= 2
                if ug_output_x >= x_from and ug_input_x <= x_to:
                    # Remove surface belt at ug_input_x (already placed)
                    entities[:] = [
                        e
                        for e in entities
                        if not (e.x == ug_input_x and e.y == y and e.name == belt_name and e.direction == direction)
                    ]
                    span = ug_input_x - ug_output_x
                    ug_name = _ug_for_span(belt_name, span)
                    entities.append(
                        PlacedEntity(
                            name=ug_name,
                            x=ug_input_x,
                            y=y,
                            direction=EntityDirection.WEST,
                            io_type="input",
                            carries=lane.item,
                        )
                    )
                    entities.append(
                        PlacedEntity(
                            name=ug_name,
                            x=ug_output_x,
                            y=y,
                            direction=EntityDirection.WEST,
                            io_type="output",
                            carries=lane.item,
                        )
                    )
                    hx = ug_output_x - 1
                    continue
            entities.append(
                PlacedEntity(
                    name=belt_name,
                    x=hx,
                    y=y,
                    direction=direction,
                    carries=lane.item,
                )
            )
            hx -= 1


def _route_tap_off(
    entities: list[PlacedEntity],
    lane: BusLane,
    tap_y: int,
    all_lanes: list[BusLane],
    row_spans: list[RowSpan],
    bw: int,
    belt_name: str = "transport-belt",
    crossing_map: dict[tuple[int, int], set[str]] | None = None,
) -> None:
    """Route a horizontal tap-off (EAST) from bus lane to row input belt."""
    blocked = _blocked_xs_at(lane, tap_y, all_lanes, row_spans, crossing_map)
    _route_horizontal(entities, lane, tap_y, lane.x, bw - 1, EntityDirection.EAST, blocked, belt_name)


def _route_output_return(
    entities: list[PlacedEntity],
    lane: BusLane,
    out_y: int,
    all_lanes: list[BusLane],
    row_spans: list[RowSpan],
    bw: int,
    belt_name: str = "transport-belt",
    crossing_map: dict[tuple[int, int], set[str]] | None = None,
) -> None:
    """Route output return (WEST) from row edge back to bus column."""
    blocked = _blocked_xs_at(lane, out_y, all_lanes, row_spans, crossing_map)
    _route_horizontal(entities, lane, out_y, lane.x + 1, bw - 1, EntityDirection.WEST, blocked, belt_name)
