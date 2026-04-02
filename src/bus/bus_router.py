"""Bus routing: vertical item lanes with tap-off crossings handled via undergrounds.

Each item that flows between rows gets a dedicated vertical bus lane.
Lanes run SOUTH (top to bottom).  At the consuming row, the lane turns
EAST into the row's input belt (tap-off).  When a tap-off crosses another
lane's vertical segment, the tap-off goes underground (EAST) past it.
"""

from __future__ import annotations

from dataclasses import dataclass, field

import math

from ..models import EntityDirection, PlacedEntity, SolverResult
from ..routing.common import _LANE_CAPACITY, belt_entity_for_rate
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

    # Sort: earliest first tap-off on LEFT, so their tap-offs cross fewer lanes
    lanes.sort(key=lambda ln: (min(ln.tap_off_ys) if ln.tap_off_ys else 9999, ln.source_y))

    # Assign x-columns after sorting
    for i, lane in enumerate(lanes):
        lane.x = i * 2

    return lanes


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
    if max_belt_tier and max_belt_tier in _LANE_CAPACITY:
        max_lane_cap = _LANE_CAPACITY[max_belt_tier]
    else:
        max_lane_cap = max(_LANE_CAPACITY.values())

    result: list[BusLane] = []
    for lane in lanes:
        if lane.is_fluid or lane.rate <= max_lane_cap:
            result.append(lane)
            continue

        n_splits = math.ceil(lane.rate / max_lane_cap)
        # Distribute consumer rows round-robin across splits
        consumers_per_split: list[list[int]] = [[] for _ in range(n_splits)]
        for i, ri in enumerate(lane.consumer_rows):
            consumers_per_split[i % n_splits].append(ri)

        # Distribute extra producer rows similarly
        producers_per_split: list[list[int]] = [[] for _ in range(n_splits)]
        for i, pri in enumerate(lane.extra_producer_rows):
            producers_per_split[i % n_splits].append(pri)

        for si in range(n_splits):
            consumers = consumers_per_split[si]
            if not consumers and si > 0:
                continue  # skip empty splits
            split_rate = lane.rate / n_splits
            result.append(
                BusLane(
                    item=lane.item,
                    x=0,  # reassigned later
                    source_y=lane.source_y,
                    consumer_rows=consumers,
                    producer_row=lane.producer_row if si == 0 else None,
                    rate=split_rate,
                    is_fluid=lane.is_fluid,
                    extra_producer_rows=producers_per_split[si],
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
) -> list[PlacedEntity]:
    """Create all bus belt entities."""
    entities: list[PlacedEntity] = []
    for lane in lanes:
        _route_lane(entities, lane, lanes, row_spans, bw, max_belt_tier)
    return entities


def _route_lane(
    entities: list[PlacedEntity],
    lane: BusLane,
    all_lanes: list[BusLane],
    row_spans: list[RowSpan],
    bw: int,
    max_belt_tier: str | None = None,
) -> None:
    """Route a single bus lane: vertical segment + tap-offs + output return."""
    if lane.is_fluid:
        _route_fluid_lane(entities, lane, bw)
    else:
        _route_belt_lane(entities, lane, all_lanes, row_spans, bw, max_belt_tier)


def _route_belt_lane(
    entities: list[PlacedEntity],
    lane: BusLane,
    all_lanes: list[BusLane],
    row_spans: list[RowSpan],
    bw: int,
    max_belt_tier: str | None = None,
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

    # Output returns sideload into the bus lane from one direction,
    # so items end up on a single lane. Use 2x rate for per-lane capacity.
    belt_name = belt_entity_for_rate(lane.rate * 2, max_tier=max_belt_tier)

    # Vertical surface belts (SOUTH), skipping tap-off positions
    for y in range(start_y, end_y + 1):
        if y in tap_off_set:
            continue
        entities.append(
            PlacedEntity(
                name=belt_name,
                x=x,
                y=y,
                direction=EntityDirection.SOUTH,
                carries=lane.item,
            )
        )

    # Tap-off horizontal belts (EAST) from bus column to row start.
    # When crossing another lane's vertical segment, go underground.
    for tap_y in lane.tap_off_ys:
        _route_tap_off(entities, lane, tap_y, all_lanes, bw, belt_name)

    # Output return: WEST belts from row edge back to bus column.
    # Must go underground past other lanes' vertical segments (same as tap-offs).
    # Route returns for ALL producer rows (including extra sub-rows).
    all_producers = []
    if lane.producer_row is not None:
        all_producers.append(lane.producer_row)
    all_producers.extend(lane.extra_producer_rows)
    for pri in all_producers:
        out_y = row_spans[pri].output_belt_y
        _route_output_return(entities, lane, out_y, all_lanes, bw, belt_name)


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


def _underground_for(belt: str) -> str:
    return _UG_MAP.get(belt, "underground-belt")


def _blocked_xs_at(lane: BusLane, y: int, all_lanes: list[BusLane]) -> set[int]:
    """Find x-columns of other lanes that have active vertical segments at y."""
    blocked: set[int] = set()
    for other in all_lanes:
        if other is lane:
            continue
        other_start = other.source_y
        other_end = max(other.tap_off_ys) if other.tap_off_ys else other_start
        other_taps = set(other.tap_off_ys)
        if other_start <= y <= other_end and y not in other_taps:
            blocked.add(other.x)
    return blocked


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
    """Route a horizontal belt run, going underground past blocked columns."""
    ug_name = _underground_for(belt_name)

    if direction == EntityDirection.EAST:
        hx = x_from
        while hx <= x_to:
            if hx in blocked_xs:
                entry_x = hx - 1
                exit_x = hx + 1
                while exit_x in blocked_xs:
                    exit_x += 2
                if entry_x >= x_from and exit_x <= x_to:
                    # Remove surface belt we may have just placed at entry_x
                    entities[:] = [
                        e
                        for e in entities
                        if not (e.x == entry_x and e.y == y and e.name == belt_name and e.direction == direction)
                    ]
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
                while ug_output_x in blocked_xs:
                    ug_output_x -= 2
                if ug_output_x >= x_from and ug_input_x <= x_to:
                    # Remove surface belt at ug_input_x (already placed)
                    entities[:] = [
                        e
                        for e in entities
                        if not (e.x == ug_input_x and e.y == y and e.name == belt_name and e.direction == direction)
                    ]
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
    bw: int,
    belt_name: str = "transport-belt",
) -> None:
    """Route a horizontal tap-off (EAST) from bus lane to row input belt."""
    blocked = _blocked_xs_at(lane, tap_y, all_lanes)
    _route_horizontal(entities, lane, tap_y, lane.x, bw - 1, EntityDirection.EAST, blocked, belt_name)


def _route_output_return(
    entities: list[PlacedEntity],
    lane: BusLane,
    out_y: int,
    all_lanes: list[BusLane],
    bw: int,
    belt_name: str = "transport-belt",
) -> None:
    """Route output return (WEST) from row edge back to bus column."""
    blocked = _blocked_xs_at(lane, out_y, all_lanes)
    _route_horizontal(entities, lane, out_y, lane.x + 1, bw - 1, EntityDirection.WEST, blocked, belt_name)
