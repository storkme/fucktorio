"""Bus routing: vertical item lanes with tap-off crossings handled via undergrounds.

Each item that flows between rows gets a dedicated vertical bus lane.
Lanes run SOUTH (top to bottom).  At the consuming row, the lane turns
EAST into the row's input belt (tap-off).  When a tap-off crosses another
lane's vertical segment, the tap-off goes underground (EAST) past it.
"""

from __future__ import annotations

from dataclasses import dataclass, field

from ..models import EntityDirection, PlacedEntity, SolverResult
from ..routing.common import belt_entity_for_rate
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
    tap_off_ys: list[int] = field(default_factory=list)


def plan_bus_lanes(
    solver_result: SolverResult,
    row_spans: list[RowSpan],
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
            if not inp.is_fluid:
                item_to_consumers.setdefault(inp.item, []).append(idx)

    # External inputs
    for ext in solver_result.external_inputs:
        if ext.is_fluid or ext.item in seen_items:
            continue
        consumers = item_to_consumers.get(ext.item, [])
        if consumers:
            lanes.append(
                BusLane(item=ext.item, x=0, source_y=0,
                        consumer_rows=consumers, producer_row=None,
                        rate=ext.rate)
            )
            seen_items.add(ext.item)

    # Intermediate items
    for idx, rs in enumerate(row_spans):
        for out in rs.spec.outputs:
            if out.is_fluid or out.item in seen_items:
                continue
            consumers = item_to_consumers.get(out.item, [])
            if consumers:
                total_rate = out.rate * rs.machine_count
                lanes.append(
                    BusLane(item=out.item, x=0, source_y=rs.output_belt_y,
                            consumer_rows=consumers, producer_row=idx,
                            rate=total_rate)
                )
                seen_items.add(out.item)

    # Pre-compute tap-off ys before sorting
    for lane in lanes:
        lane.tap_off_ys = _find_tap_off_ys(lane, row_spans)

    # Sort: earliest first tap-off on LEFT, so their tap-offs cross fewer lanes
    lanes.sort(key=lambda ln: (min(ln.tap_off_ys) if ln.tap_off_ys else 9999, ln.source_y))

    # Assign x-columns after sorting
    for i, lane in enumerate(lanes):
        lane.x = i * 2

    return lanes


def _find_tap_off_ys(lane: BusLane, row_spans: list[RowSpan]) -> list[int]:
    """Find y-coordinates where this lane taps off into consumer rows."""
    tap_ys: list[int] = []
    for ri in lane.consumer_rows:
        rs = row_spans[ri]
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
) -> list[PlacedEntity]:
    """Create all bus belt entities."""
    entities: list[PlacedEntity] = []
    for lane in lanes:
        _route_lane(entities, lane, lanes, bw)
    return entities


def _route_lane(
    entities: list[PlacedEntity],
    lane: BusLane,
    all_lanes: list[BusLane],
    bw: int,
) -> None:
    """Route a single bus lane: vertical belts + tap-offs + output return."""
    x = lane.x
    tap_off_set = set(lane.tap_off_ys)

    start_y = lane.source_y
    end_y = max(lane.tap_off_ys) if lane.tap_off_ys else start_y

    # All inserters drop on one lane, so use 2x rate for per-lane capacity
    belt_name = belt_entity_for_rate(lane.rate * 2)

    # Vertical surface belts (SOUTH), skipping tap-off positions
    for y in range(start_y, end_y + 1):
        if y in tap_off_set:
            continue
        entities.append(
            PlacedEntity(
                name=belt_name, x=x, y=y,
                direction=EntityDirection.SOUTH, carries=lane.item,
            )
        )

    # Tap-off horizontal belts (EAST) from bus column to row start.
    # When crossing another lane's vertical segment, go underground.
    for tap_y in lane.tap_off_ys:
        _route_tap_off(entities, lane, tap_y, all_lanes, bw, belt_name)

    # Output return: WEST belts from row edge back to bus column
    if lane.producer_row is not None:
        out_y = lane.source_y
        for hx in range(x + 1, bw):
            entities.append(
                PlacedEntity(
                    name=belt_name, x=hx, y=out_y,
                    direction=EntityDirection.WEST, carries=lane.item,
                )
            )


_UG_MAP = {
    "transport-belt": "underground-belt",
    "fast-transport-belt": "fast-underground-belt",
    "express-transport-belt": "express-underground-belt",
}


def _underground_for(belt: str) -> str:
    return _UG_MAP.get(belt, "underground-belt")


def _route_tap_off(
    entities: list[PlacedEntity],
    lane: BusLane,
    tap_y: int,
    all_lanes: list[BusLane],
    bw: int,
    belt_name: str = "transport-belt",
) -> None:
    """Route a horizontal tap-off from the bus lane to the row input belt.

    When the tap-off crosses another lane's active vertical segment,
    use underground belts (EAST) to tunnel past it without collision.
    """
    # Find x-columns of other lanes that have active vertical belts at tap_y
    blocked_xs: set[int] = set()
    for other in all_lanes:
        if other is lane:
            continue
        other_start = other.source_y
        other_end = max(other.tap_off_ys) if other.tap_off_ys else other_start
        other_taps = set(other.tap_off_ys)
        # The other lane has a vertical belt at tap_y if it's in its active
        # range and not one of its own tap-off positions
        if other_start <= tap_y <= other_end and tap_y not in other_taps:
            blocked_xs.add(other.x)

    # Route from lane.x to bw-1, going underground past blocked columns
    hx = lane.x
    while hx < bw:
        if hx in blocked_xs:
            # Go underground: entry 1 tile before, exit 1 tile after
            entry_x = hx - 1
            exit_x = hx + 1

            # Extend to cover consecutive blocked columns
            while exit_x in blocked_xs:
                exit_x += 2  # lanes are 2 apart

            if entry_x >= lane.x and exit_x < bw:
                # Replace the surface belt at entry_x with underground entry
                # (remove the belt we just placed there)
                entities[:] = [
                    e for e in entities
                    if not (e.x == entry_x and e.y == tap_y
                            and e.name == "transport-belt"
                            and e.direction == EntityDirection.EAST)
                ]
                ug_name = _underground_for(belt_name)
                entities.append(
                    PlacedEntity(
                        name=ug_name, x=entry_x, y=tap_y,
                        direction=EntityDirection.EAST, io_type="input",
                        carries=lane.item,
                    )
                )
                entities.append(
                    PlacedEntity(
                        name=ug_name, x=exit_x, y=tap_y,
                        direction=EntityDirection.EAST, io_type="output",
                        carries=lane.item,
                    )
                )
                hx = exit_x + 1
                continue
            # Fallback: can't go underground (too close to edge), just place belt
            # This shouldn't happen with proper lane ordering

        # Normal surface belt
        entities.append(
            PlacedEntity(
                name=belt_name, x=hx, y=tap_y,
                direction=EntityDirection.EAST, carries=lane.item,
            )
        )
        hx += 1
