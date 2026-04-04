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
        if lane.is_fluid:
            continue
        # Only collector lanes (no consumers) need balancers.  Intermediate
        # lanes with multiple producers have the same sideload-onto-one-lane
        # issue, but the current balancer design (splitter + sideload loop)
        # doesn't actually rebalance lanes — it just splits total rate between
        # two paths that both feed the same downstream tile.  TODO: proper
        # lane balancing for intermediate lanes.
        if lane.consumer_rows:
            continue
        all_producers = []
        if lane.producer_row is not None:
            all_producers.append(lane.producer_row)
        all_producers.extend(lane.extra_producer_rows)
        if len(all_producers) <= 1:
            continue
        last_sideload_y = max(row_spans[pri].output_belt_y for pri in all_producers)
        bal_y = last_sideload_y + 1
        tap_set = set(lane.tap_off_ys)
        if bal_y not in tap_set and (bal_y + 1) not in tap_set:
            lane.balancer_y = bal_y

    # Optimize lane left-to-right ordering to minimize underground crossings.
    lanes = _optimize_lane_order(lanes, row_spans)

    # Assign x-columns with 1-tile spacing.  Tap-offs go underground to
    # cross other trunks; with tight spacing the underground spans are short.
    for i, lane in enumerate(lanes):
        lane.x = i + 1

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

    ranges = [_active_range(ln) for ln in ordered]

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
    solid = [ln for ln in lanes if not ln.is_fluid]
    fluid = [ln for ln in lanes if ln.is_fluid]

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
        solid.sort(key=lambda ln: -(min(ln.tap_off_ys) if ln.tap_off_ys else 9999))

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
            # source_y from this split's own producers, not the original
            # lane's (which may belong to a different split after redistribution).
            split_source_y = min(row_spans[p].output_belt_y for p in prods) if prods else lane.source_y
            result.append(
                BusLane(
                    item=lane.item,
                    x=0,  # reassigned later
                    source_y=split_source_y,
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
    # +2: one tile before first lane (x=0), one tile after last lane
    # for underground exit clearance.
    return len(lanes) + 2


def route_bus(
    lanes: list[BusLane],
    row_spans: list[RowSpan],
    total_height: int,
    bw: int,
    max_belt_tier: str | None = None,
    row_entities: list[PlacedEntity] | None = None,
    solver_result: SolverResult | None = None,
) -> tuple[list[PlacedEntity], int, int]:
    """Create all bus belt entities.

    Returns (entities, max_y, merge_max_x) where max_y accounts for any
    merger blocks placed below the last row, and merge_max_x is the
    rightmost x used by output merge columns (0 if no mergers).

    Uses lane-first negotiated congestion routing (Rust) to detect
    crossing conflicts between ALL lane segments (including mergers),
    then renders belt entities with underground crossings at conflict points.
    """
    # Route all horizontals via negotiated A* with underground support.
    routed_paths = _negotiate_and_route(
        lanes,
        row_spans,
        total_height,
        bw,
        row_entities,
        solver_result,
    )

    entities: list[PlacedEntity] = []
    max_y = total_height
    merge_max_x = 0
    for lane in lanes:
        _route_lane(entities, lane, lanes, row_spans, bw, max_belt_tier, routed_paths)

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
            group,
            row_spans,
            total_height,
            entities,
            max_belt_tier,
        )
        entities.extend(merger_ents)
        max_y = max(max_y, merger_end_y)

    # Merge output belts for final products at the bottom-right.
    # Final product rows have EAST-flowing output belts; merge columns
    # are placed just past the widest output row.
    if solver_result:
        output_items = {ext.item for ext in solver_result.external_outputs if not ext.is_fluid}
        for item in output_items:
            output_rows = [
                i for i, rs in enumerate(row_spans) if any(o.item == item for o in rs.spec.outputs if not o.is_fluid)
            ]
            if len(output_rows) >= 2:
                merge_ents, merge_end_y, item_merge_x = _merge_output_rows(
                    output_rows,
                    item,
                    row_spans,
                    max_y,
                    max_belt_tier,
                )
                entities.extend(merge_ents)
                max_y = max(max_y, merge_end_y)
                merge_max_x = max(merge_max_x, item_merge_x)

    return entities, max_y, merge_max_x


def _merge_output_rows(
    output_rows: list[int],
    item: str,
    row_spans: list[RowSpan],
    merge_start_y: int,
    max_belt_tier: str | None = None,
) -> tuple[list[PlacedEntity], int, int]:
    """Merge EAST-flowing output belts from multiple rows at the bottom-right.

    Each output row's belt flows EAST and collects items at its rightmost
    tile.  This function extends shorter rows to a common merge column,
    places SOUTH columns, and merges them with a splitter tree.

    Returns (entities, max_y, merge_max_x).
    """
    entities: list[PlacedEntity] = []
    n = len(output_rows)
    if n < 2:
        return entities, merge_start_y, 0

    total_rate = sum(
        sum(o.rate * row_spans[ri].machine_count for o in row_spans[ri].spec.outputs if o.item == item)
        for ri in output_rows
    )
    belt_name = belt_entity_for_rate(total_rate * 2, max_tier=max_belt_tier)
    splitter_name = _SPLITTER_MAP.get(belt_name, "splitter")

    # Merge columns sit just past the widest output row.
    # Earlier rows (lower idx, higher up in the layout) get farther-right
    # columns so their SOUTH columns don't block later rows' EAST extensions.
    merge_x = max(row_spans[ri].row_width for ri in output_rows)

    for idx, ri in enumerate(output_rows):
        out_y = row_spans[ri].output_belt_y
        col_x = merge_x + (n - 1 - idx)  # first row rightmost, last row at merge_x

        # Extend EAST belts from the row's rightmost tile to the merge column.
        # The row's output belt ends at row_width - 1. We need EAST belts from
        # row_width to col_x - 1 (the last belt before the SOUTH turn).
        rw = row_spans[ri].row_width
        for x in range(rw, col_x):
            entities.append(
                PlacedEntity(
                    name=belt_name,
                    x=x,
                    y=out_y,
                    direction=EntityDirection.EAST,
                    carries=item,
                )
            )

        # SOUTH column from out_y to merge_start_y.
        # The EAST belt at (col_x - 1, out_y) feeds into (col_x, out_y) SOUTH
        # via a natural belt turn.
        for y in range(out_y, merge_start_y):
            entities.append(
                PlacedEntity(
                    name=belt_name,
                    x=col_x,
                    y=y,
                    direction=EntityDirection.SOUTH,
                    carries=item,
                )
            )

    # Sequential splitter merge at bottom-right.
    # First output at merge_x, each subsequent merges from merge_x+1
    # via a SOUTH-facing splitter.
    y_cursor = merge_start_y
    for _idx in range(1, n):
        entities.append(
            PlacedEntity(
                name=splitter_name,
                x=merge_x,
                y=y_cursor,
                direction=EntityDirection.SOUTH,
                carries=item,
            )
        )
        y_cursor += 1
        entities.append(
            PlacedEntity(
                name=belt_name,
                x=merge_x,
                y=y_cursor,
                direction=EntityDirection.SOUTH,
                carries=item,
            )
        )
        y_cursor += 1

    return entities, y_cursor, merge_x + n


def _trunk_segments(start_y: int, end_y: int, skip_ys: set[int]) -> list[tuple[int, int]]:
    """Split [start_y, end_y] into contiguous segments excluding skip_ys."""
    segments: list[tuple[int, int]] = []
    seg_start: int | None = None
    for y in range(start_y, end_y + 1):
        if y in skip_ys:
            if seg_start is not None:
                segments.append((seg_start, y - 1))
                seg_start = None
        elif seg_start is None:
            seg_start = y
    if seg_start is not None:
        segments.append((seg_start, end_y))
    return segments


def _compute_trunk_cross_ys(
    lanes: list[BusLane],
    row_spans: list[RowSpan],
) -> dict[int, set[int]]:
    """For each trunk column x, compute y-coordinates crossed by other lanes' tap-offs."""
    cross_ys: dict[int, set[int]] = {}
    for lane in lanes:
        if lane.is_fluid:
            continue
        for tap_y in lane.tap_off_ys:
            for other in lanes:
                if other is lane or other.is_fluid:
                    continue
                other_start = other.source_y
                other_all_ys = list(other.tap_off_ys)
                if other.producer_row is not None:
                    other_all_ys.append(row_spans[other.producer_row].output_belt_y)
                for pri in other.extra_producer_rows:
                    other_all_ys.append(row_spans[pri].output_belt_y)
                other_end = max(other_all_ys) if other_all_ys else other_start
                if other.balancer_y is not None:
                    other_end = max(other_end, other.balancer_y + 1)
                if other_start <= tap_y <= other_end and tap_y not in set(other.tap_off_ys):
                    cross_ys.setdefault(other.x, set()).add(tap_y)
    return cross_ys


def _negotiate_and_route(
    lanes: list[BusLane],
    row_spans: list[RowSpan],
    total_height: int,
    bw: int,
    row_entities: list[PlacedEntity] | None = None,
    solver_result: SolverResult | None = None,
) -> dict[str, list[tuple[int, int]]]:
    """Route all bus segments via negotiated A* with underground support.

    All segments (trunks, tap-offs, returns, mergers) use A* (strategy=2)
    with priority-based hard claims: higher-priority specs claim surface
    tiles that become obstacles for lower-priority specs, forcing them
    underground at crossings.

    Priority order: mergers (8) > tap-offs (6) > trunks (5) >
    returns/balance (4) > output mergers (3).

    Returns a map of string key → routed path tiles.  Keys include:
    - "trunk:{item}:{x}:{start_y}:{end_y}" for trunk segments
    - "tap:{item}:{x}:{y}" for tap-off demands
    - "ret:{item}:{x}:{y}" for output return demands

    Gaps in the path (manhattan distance > 1 between consecutive tiles)
    indicate underground belt jumps.
    """
    try:
        from fucktorio_native import PyLaneSpec, negotiate_lanes
    except ImportError:
        return {}

    # Build item → numeric ID mapping (include output items for merger routing)
    items_set = {lane.item for lane in lanes if not lane.is_fluid}
    if solver_result:
        items_set |= {ext.item for ext in solver_result.external_outputs if not ext.is_fluid}
    items = sorted(items_set)
    item_to_id: dict[str, int] = {item: i for i, item in enumerate(items)}

    # Map demand_id → string key for result lookup
    id_to_key: dict[int, str] = {}

    specs: list[PyLaneSpec] = []
    lane_id = 0

    # --- Collect fixed obstacles ---
    obstacles: list[tuple[int, int]] = []
    if row_entities:
        _MACHINE_ENTITIES = {
            "assembling-machine-1",
            "assembling-machine-2",
            "assembling-machine-3",
            "chemical-plant",
            "electric-furnace",
            "oil-refinery",
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

    # Trunk columns are NOT added as static obstacles — trunks are now A*
    # specs (strategy=2) whose routed paths are claimed in the congestion grid.
    # Priority-based hard claims in Rust promote those claimed tiles to obstacles
    # before routing lower-priority specs, so tap-offs/returns naturally see
    # trunk tiles as blocked and use underground crossings.

    # --- Build demand specs ---

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
            producer_out_ys = [row_spans[p].output_belt_y for p in all_producers]
            start_y = min(producer_out_ys)
            last_tap_y = max(lane.tap_off_ys) if lane.tap_off_ys else start_y

            # Trunk: A* vertical (strategy=2, low priority — routes after tap-offs)
            # Skip producer output ys (return junctions — need surface belts).
            # Cross-lane tap-off positions are NOT skipped here — the A* handles
            # them via underground (promoted obstacles + x_constraint force UG).
            skip_ys = set(producer_out_ys)
            for seg_start, seg_end in _trunk_segments(start_y, last_tap_y - 1, skip_ys):
                trunk_key = f"trunk:{lane.item}:{x}:{seg_start}:{seg_end}"
                id_to_key[lane_id] = trunk_key
                specs.append(
                    PyLaneSpec(
                        id=lane_id,
                        item_id=item_id,
                        waypoints=[(x, seg_start), (x, seg_end)],
                        strategy=2,
                        priority=5,
                        x_constraint=x,
                    )
                )
                lane_id += 1

            # Output returns: A* horizontal WEST (strategy=2)
            for pri in all_producers:
                out_y = row_spans[pri].output_belt_y
                id_to_key[lane_id] = f"ret:{lane.item}:{x}:{out_y}"
                specs.append(
                    PyLaneSpec(
                        id=lane_id,
                        item_id=item_id,
                        waypoints=[(bw - 1, out_y), (x + 1, out_y)],
                        strategy=2,
                        priority=4,
                        y_constraint=out_y,
                    )
                )
                lane_id += 1

            # Splitter balance return: route from splitter's second output
            # to (x-1, out_y) for opposite-side sideloading onto the left lane.
            # No y_constraint — A* finds the Z-shape naturally:
            # WEST (UG past trunks) → SOUTH turn → EAST back to x-1.
            if len(all_producers) >= 2 and x > 1:
                last_out_y = row_spans[all_producers[-1]].output_belt_y
                split_y = last_out_y - 1
                sideload_y = last_out_y  # sideload at the same y as the normal return
                id_to_key[lane_id] = f"bal:{lane.item}:{x}:{split_y}"
                specs.append(
                    PyLaneSpec(
                        id=lane_id,
                        item_id=item_id,
                        waypoints=[(bw - 1, split_y), (x - 1, sideload_y)],
                        strategy=2,
                        priority=4,
                        # No y_constraint — allow vertical movement for the Z-turn
                    )
                )
                lane_id += 1

            # Tap-off: A* horizontal EAST (strategy=2, high priority)
            # Turn belt at (x, tap_y) placed manually; A* starts at x+1.
            tap_y = lane.tap_off_ys[0] if lane.tap_off_ys else last_tap_y
            if x + 1 <= bw - 1:
                id_to_key[lane_id] = f"tap:{lane.item}:{x}:{tap_y}"
                specs.append(
                    PyLaneSpec(
                        id=lane_id,
                        item_id=item_id,
                        waypoints=[(x + 1, tap_y), (bw - 1, tap_y)],
                        strategy=2,
                        priority=6,
                        y_constraint=tap_y,
                    )
                )
                lane_id += 1

        elif lane.consumer_rows:
            # External input: trunk from source to tap-off
            tap_y = lane.tap_off_ys[0] if lane.tap_off_ys else lane.source_y

            # Trunk: A* vertical (strategy=2, low priority)
            # Split into segments excluding tap-off and balancer positions.
            skip_ys = set(lane.tap_off_ys)
            if lane.balancer_y is not None:
                skip_ys.add(lane.balancer_y)
            end_y = tap_y
            if lane.balancer_y is not None:
                end_y = max(end_y, lane.balancer_y + 1)
            for seg_start, seg_end in _trunk_segments(lane.source_y, end_y, skip_ys):
                trunk_key = f"trunk:{lane.item}:{x}:{seg_start}:{seg_end}"
                id_to_key[lane_id] = trunk_key
                specs.append(
                    PyLaneSpec(
                        id=lane_id,
                        item_id=item_id,
                        waypoints=[(x, seg_start), (x, seg_end)],
                        strategy=2,
                        priority=5,
                        x_constraint=x,
                    )
                )
                lane_id += 1

            # Tap-off: A* horizontal EAST (high priority)
            if x + 1 <= bw - 1:
                id_to_key[lane_id] = f"tap:{lane.item}:{x}:{tap_y}"
                specs.append(
                    PyLaneSpec(
                        id=lane_id,
                        item_id=item_id,
                        waypoints=[(x + 1, tap_y), (bw - 1, tap_y)],
                        strategy=2,
                        priority=6,
                        y_constraint=tap_y,
                    )
                )
                lane_id += 1

        else:
            # Collector: trunk + returns
            all_ys = list(lane.tap_off_ys)
            for pri in all_producers:
                all_ys.append(row_spans[pri].output_belt_y)
            end_y = max(all_ys) if all_ys else lane.source_y
            if lane.balancer_y is not None:
                end_y = max(end_y, lane.balancer_y + 1)

            # Trunk: A* vertical (strategy=2, low priority)
            skip_ys = set(lane.tap_off_ys)
            if lane.balancer_y is not None:
                skip_ys.add(lane.balancer_y)
            for seg_start, seg_end in _trunk_segments(lane.source_y, end_y, skip_ys):
                trunk_key = f"trunk:{lane.item}:{x}:{seg_start}:{seg_end}"
                id_to_key[lane_id] = trunk_key
                specs.append(
                    PyLaneSpec(
                        id=lane_id,
                        item_id=item_id,
                        waypoints=[(x, seg_start), (x, seg_end)],
                        strategy=2,
                        priority=5,
                        x_constraint=x,
                    )
                )
                lane_id += 1

            # Output returns: A* horizontal WEST
            for pri in all_producers:
                out_y = row_spans[pri].output_belt_y
                id_to_key[lane_id] = f"ret:{lane.item}:{x}:{out_y}"
                specs.append(
                    PyLaneSpec(
                        id=lane_id,
                        item_id=item_id,
                        waypoints=[(bw - 1, out_y), (x + 1, out_y)],
                        strategy=2,
                        priority=4,
                        y_constraint=out_y,
                    )
                )
                lane_id += 1

    # --- Merger segments (axis-aligned, high priority) ---
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
        merge_y = total_height
        for ln in group:
            specs.append(
                PyLaneSpec(
                    id=lane_id,
                    item_id=item_id,
                    waypoints=[(ln.x, ln.source_y), (ln.x, merge_y + 3)],
                    strategy=0,
                    priority=8,
                )
            )
            lane_id += 1
        i = 0
        while i + 1 < len(trunk_xs):
            left_x = trunk_xs[i]
            right_x = trunk_xs[i + 1]
            specs.append(
                PyLaneSpec(
                    id=lane_id,
                    item_id=item_id,
                    waypoints=[(right_x, merge_y), (left_x + 1, merge_y)],
                    strategy=0,
                    priority=8,
                )
            )
            lane_id += 1
            i += 2

    # Output mergers are no longer routed through the bus trunk zone.
    # Final product rows use EAST-flowing output belts that merge at
    # the bottom-right of the layout (handled by _merge_output_rows).

    if not specs:
        return {}

    routed = negotiate_lanes(
        specs,
        obstacles,
        max_extent=max(bw, total_height) + 50,
        allow_underground=True,
        ug_max_reach=8,  # express belt reach — renderer picks cheapest tier per span
    )

    # Build result map: string key → path (only for A* horizontal demands)
    result: dict[str, list[tuple[int, int]]] = {}
    for r in routed:
        key = id_to_key.get(r.id)
        if key and r.path:
            result[key] = list(r.path)
    return result


def _render_path(
    path: list[tuple[int, int]],
    item: str,
    belt_name: str,
    direction_hint: EntityDirection = EntityDirection.EAST,
) -> list[PlacedEntity]:
    """Convert an A*-routed path into PlacedEntity belt/underground entities.

    Gaps in the path (manhattan distance > 1 between consecutive tiles)
    indicate underground belt jumps — UG entry at the first tile, UG exit
    at the second.  Surface tiles get regular belt entities.

    For single-tile paths, ``direction_hint`` determines the belt direction.
    """
    entities: list[PlacedEntity] = []
    if not path:
        return entities

    if len(path) == 1:
        entities.append(
            PlacedEntity(
                name=belt_name,
                x=path[0][0],
                y=path[0][1],
                direction=direction_hint,
                carries=item,
            )
        )
        return entities

    i = 0
    while i < len(path):
        x, y = path[i]
        if i + 1 < len(path):
            nx, ny = path[i + 1]
            dx = nx - x
            dy = ny - y
            dist = abs(dx) + abs(dy)

            if dist > 1:
                # Underground jump: entry at (x,y), exit at (nx,ny)
                sdx = 1 if dx > 0 else (-1 if dx < 0 else 0)
                sdy = 1 if dy > 0 else (-1 if dy < 0 else 0)
                direction = _vec_to_entity_dir(sdx, sdy)
                ug_name = _ug_for_span(belt_name, dist)
                entities.append(
                    PlacedEntity(
                        name=ug_name,
                        x=x,
                        y=y,
                        direction=direction,
                        io_type="input",
                        carries=item,
                    )
                )
                entities.append(
                    PlacedEntity(
                        name=ug_name,
                        x=nx,
                        y=ny,
                        direction=direction,
                        io_type="output",
                        carries=item,
                    )
                )
                i += 2  # skip both entry and exit
                continue
            else:
                # Surface belt
                direction = _vec_to_entity_dir(dx, dy)
                entities.append(
                    PlacedEntity(
                        name=belt_name,
                        x=x,
                        y=y,
                        direction=direction,
                        carries=item,
                    )
                )
                i += 1
        else:
            # Last tile — use direction from previous tile
            direction = entities[-1].direction if entities else EntityDirection.EAST
            entities.append(
                PlacedEntity(
                    name=belt_name,
                    x=x,
                    y=y,
                    direction=direction,
                    carries=item,
                )
            )
            i += 1

    return entities


def _vec_to_entity_dir(dx: int, dy: int) -> EntityDirection:
    """Convert a direction vector to EntityDirection."""
    if dx > 0:
        return EntityDirection.EAST
    if dx < 0:
        return EntityDirection.WEST
    if dy > 0:
        return EntityDirection.SOUTH
    return EntityDirection.NORTH


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
    routed_paths: dict[str, list[tuple[int, int]]] | None = None,
) -> None:
    """Route a single bus lane: vertical segment + tap-offs + output return."""
    if lane.is_fluid:
        _route_fluid_lane(entities, lane, bw)
    elif _is_intermediate(lane):
        _route_intermediate_lane(entities, lane, all_lanes, row_spans, bw, max_belt_tier, routed_paths)
    else:
        _route_belt_lane(entities, lane, all_lanes, row_spans, bw, max_belt_tier, routed_paths)


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
    routed_paths: dict[str, list[tuple[int, int]]] | None = None,
) -> None:
    """Route an intermediate lane: producer returns, vertical trunk, tap-off.

    Uses pre-routed A* paths for horizontal segments (returns and tap-off).
    Splitter lane balancing is placed manually (fixed geometry).
    """
    x = lane.x
    belt_name = belt_entity_for_rate(lane.rate * 2, max_tier=max_belt_tier)
    horiz_belt = belt_entity_for_rate(lane.rate * 2, max_tier=max_belt_tier)
    paths = routed_paths or {}

    all_producers = []
    if lane.producer_row is not None:
        all_producers.append(lane.producer_row)
    all_producers.extend(lane.extra_producer_rows)

    assert lane.consumer_rows, "Intermediate lane must have a consumer"
    tap_y = lane.tap_off_ys[0] if lane.tap_off_ys else row_spans[lane.consumer_rows[0]].input_belt_y[0]
    producer_out_ys = [row_spans[p].output_belt_y for p in all_producers]
    start_y = min(producer_out_ys)

    # --- Lane balancing via splitter + opposite-side sideload ---
    balance_y: int | None = None
    if len(all_producers) >= 2 and x > 1:
        last_pri = all_producers[-1]
        balance_y = row_spans[last_pri].output_belt_y

    # Output returns
    for pri in all_producers:
        out_y = row_spans[pri].output_belt_y
        if out_y == balance_y:
            # Splitter for lane balancing (manual placement)
            splitter_x = bw
            splitter_name = _SPLITTER_MAP.get(horiz_belt, "splitter")
            entities.append(
                PlacedEntity(
                    name=splitter_name,
                    x=splitter_x,
                    y=out_y - 1,
                    direction=EntityDirection.WEST,
                    carries=lane.item,
                )
            )
            # Normal return: use A*-routed path
            ret_key = f"ret:{lane.item}:{x}:{out_y}"
            ret_path = paths.get(ret_key)
            if ret_path:
                entities.extend(_render_path(ret_path, lane.item, horiz_belt, EntityDirection.WEST))
            # Split return: U-shaped route around the trunk for left-lane sideload.
            # 1. WEST from splitter past all trunks (A*-routed with UG crossings)
            # 2. SOUTH 2 tiles at the left edge
            split_y = out_y - 1

            # Balance route: single A*-routed path (no y_constraint).
            # A* finds the Z-shape: WEST (UG crossings) → SOUTH → EAST.
            split_y = out_y - 1
            bal_key = f"bal:{lane.item}:{x}:{split_y}"
            bal_path = paths.get(bal_key)
            if bal_path:
                bal_entities = _render_path(bal_path, lane.item, horiz_belt, EntityDirection.WEST)
                # Last tile should face EAST to sideload onto the trunk
                if bal_entities:
                    bal_entities[-1].direction = EntityDirection.EAST
                entities.extend(bal_entities)
        else:
            # Normal return: use A*-routed path
            ret_key = f"ret:{lane.item}:{x}:{out_y}"
            ret_path = paths.get(ret_key)
            if ret_path:
                entities.extend(_render_path(ret_path, lane.item, horiz_belt, EntityDirection.WEST))

    # Vertical trunk: use A*-routed segment paths (may include UG crossings).
    # Manual surface belts at producer output ys (return junction points).
    skip_ys = set(producer_out_ys)
    for out_y in producer_out_ys:
        if out_y < tap_y:
            entities.append(
                PlacedEntity(
                    name=belt_name,
                    x=x,
                    y=out_y,
                    direction=EntityDirection.SOUTH,
                    carries=lane.item,
                )
            )
    for seg_start, seg_end in _trunk_segments(start_y, tap_y - 1, skip_ys):
        trunk_key = f"trunk:{lane.item}:{x}:{seg_start}:{seg_end}"
        trunk_path = paths.get(trunk_key)
        if trunk_path:
            entities.extend(_render_path(trunk_path, lane.item, belt_name, EntityDirection.SOUTH))
        else:
            for y in range(seg_start, seg_end + 1):
                entities.append(
                    PlacedEntity(
                        name=belt_name,
                        x=x,
                        y=y,
                        direction=EntityDirection.SOUTH,
                        carries=lane.item,
                    )
                )

    # Tap-off: surface EAST belt at the turn point, then A*-routed path.
    entities.append(
        PlacedEntity(
            name=belt_name,
            x=x,
            y=tap_y,
            direction=EntityDirection.EAST,
            carries=lane.item,
        )
    )
    tap_key = f"tap:{lane.item}:{x}:{tap_y}"
    tap_path = paths.get(tap_key)
    if tap_path:
        entities.extend(_render_path(tap_path, lane.item, belt_name))


def _route_belt_lane(
    entities: list[PlacedEntity],
    lane: BusLane,
    all_lanes: list[BusLane],
    row_spans: list[RowSpan],
    bw: int,
    max_belt_tier: str | None = None,
    routed_paths: dict[str, list[tuple[int, int]]] | None = None,
) -> None:
    """Route a solid-item bus lane with belts.

    Uses pre-routed A* paths for horizontal segments (tap-offs and returns).
    Trunk and balancer placement is manual.
    """
    x = lane.x
    tap_off_set = set(lane.tap_off_ys)
    paths = routed_paths or {}

    start_y = lane.source_y
    all_ys = list(lane.tap_off_ys)
    for pri in lane.extra_producer_rows:
        all_ys.append(row_spans[pri].output_belt_y)
    end_y = max(all_ys) if all_ys else start_y

    if lane.balancer_y is not None:
        end_y = max(end_y, lane.balancer_y + 1)

    if lane.balancer_y is not None:
        belt_name = belt_entity_for_rate(lane.rate, max_tier=max_belt_tier)
    else:
        belt_name = belt_entity_for_rate(lane.rate * 2, max_tier=max_belt_tier)
    horiz_belt = belt_entity_for_rate(lane.rate * 2, max_tier=max_belt_tier)
    if lane.balancer_y is not None:
        pre_bal_belt = belt_entity_for_rate(lane.rate * 2, max_tier=max_belt_tier)
    else:
        pre_bal_belt = belt_name

    balancer_skip = {lane.balancer_y} if lane.balancer_y is not None else set()

    # Vertical trunk: use A*-routed paths (may include UG crossings)
    bal_y = lane.balancer_y
    skip_ys = tap_off_set | balancer_skip
    for seg_start, seg_end in _trunk_segments(start_y, end_y, skip_ys):
        tier = pre_bal_belt if (bal_y is not None and seg_start < bal_y) else belt_name
        trunk_key = f"trunk:{lane.item}:{x}:{seg_start}:{seg_end}"
        trunk_path = paths.get(trunk_key)
        if trunk_path:
            entities.extend(_render_path(trunk_path, lane.item, tier, EntityDirection.SOUTH))
        else:
            # Fallback: manual SOUTH belts
            for y in range(seg_start, seg_end + 1):
                entities.append(
                    PlacedEntity(
                        name=tier,
                        x=x,
                        y=y,
                        direction=EntityDirection.SOUTH,
                        carries=lane.item,
                    )
                )

    # Lane balancer (manual placement)
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
        entities.append(
            PlacedEntity(
                name=belt_name,
                x=x - 1,
                y=by + 1,
                direction=EntityDirection.EAST,
                carries=lane.item,
            )
        )

    # Tap-offs: surface turn belt + A*-routed path
    for tap_y in lane.tap_off_ys:
        entities.append(
            PlacedEntity(
                name=horiz_belt,
                x=x,
                y=tap_y,
                direction=EntityDirection.EAST,
                carries=lane.item,
            )
        )
        tap_key = f"tap:{lane.item}:{x}:{tap_y}"
        tap_path = paths.get(tap_key)
        if tap_path:
            entities.extend(_render_path(tap_path, lane.item, horiz_belt))

    # Output returns: A*-routed paths
    all_producers = []
    if lane.producer_row is not None:
        all_producers.append(lane.producer_row)
    all_producers.extend(lane.extra_producer_rows)
    for pri in all_producers:
        out_y = row_spans[pri].output_belt_y
        ret_key = f"ret:{lane.item}:{x}:{out_y}"
        ret_path = paths.get(ret_key)
        if ret_path:
            entities.extend(_render_path(ret_path, lane.item, horiz_belt))


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
