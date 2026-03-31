"""Bus routing: vertical item lanes with tap-off crossings handled via undergrounds.

Each item that flows between rows gets a dedicated vertical bus lane.
Lanes run SOUTH (top to bottom).  At the consuming row, the lane turns
EAST into the row's input belt (tap-off).  When a tap-off crosses another
lane's vertical segment, that segment goes underground at the crossing point.
"""

from __future__ import annotations

from dataclasses import dataclass, field

from ..models import EntityDirection, PlacedEntity, SolverResult
from .placer import RowSpan


@dataclass
class BusLane:
    """A single vertical lane on the bus."""

    item: str
    x: int  # column in the layout
    source_y: int  # where items enter (0 for external, output_y for intermediate)
    consumer_rows: list[int]  # indices into row_spans
    producer_row: int | None  # index or None for external
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
                        consumer_rows=consumers, producer_row=None)
            )
            seen_items.add(ext.item)

    # Intermediate items
    for idx, rs in enumerate(row_spans):
        for out in rs.spec.outputs:
            if out.is_fluid or out.item in seen_items:
                continue
            consumers = item_to_consumers.get(out.item, [])
            if consumers:
                lanes.append(
                    BusLane(item=out.item, x=0, source_y=rs.output_belt_y,
                            consumer_rows=consumers, producer_row=idx)
                )
                seen_items.add(out.item)

    # Pre-compute tap-off ys before sorting
    for lane in lanes:
        lane.tap_off_ys = _find_tap_off_ys(lane, row_spans)

    # Sort lanes: earliest first tap-off on LEFT (lowest x), so their
    # tap-offs cross fewer active vertical segments to the right.
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

    # Collect all tap-off positions: (y, lane_x) pairs
    all_tap_offs: dict[int, set[int]] = {}
    for lane in lanes:
        for ty in lane.tap_off_ys:
            all_tap_offs.setdefault(ty, set()).add(lane.x)

    for lane in lanes:
        _route_lane(entities, lane, bw, all_tap_offs)

    return entities


def _route_lane(
    entities: list[PlacedEntity],
    lane: BusLane,
    bw: int,
    all_tap_offs: dict[int, set[int]],
) -> None:
    """Route a single bus lane: vertical belts + tap-offs + output return.

    Uses a tile-by-tile approach: for each y in the active range, decide
    whether to place a surface belt, underground pair, or tap-off.
    """
    x = lane.x
    tap_off_set = set(lane.tap_off_ys)

    start_y = lane.source_y
    end_y = max(lane.tap_off_ys) if lane.tap_off_ys else start_y

    # Find y positions where other lanes' tap-offs cross this lane
    crossing_ys: set[int] = set()
    for ty, tap_xs in all_tap_offs.items():
        if ty in tap_off_set:
            continue  # Our own tap-off
        if start_y <= ty <= end_y:
            for tap_x in tap_xs:
                if tap_x < x:
                    crossing_ys.add(ty)
                    break

    # Build the vertical segment tile-by-tile
    # Track which y's get underground pairs so we don't double-place
    underground_tiles: set[int] = set()

    for cy in sorted(crossing_ys):
        # Need to go underground to skip this crossing.
        # Entry at cy-1 (or cy if cy == start_y), exit at cy+1.
        entry = max(start_y, cy - 1)
        exit_ = cy + 1

        # If exit lands on a tap-off or another crossing, extend
        while exit_ in crossing_ys or exit_ in tap_off_set:
            exit_ += 1

        if exit_ > end_y:
            exit_ = end_y  # clamp

        # Only place if entry != exit and distance is reasonable
        if entry < exit_:
            entities.append(
                PlacedEntity(
                    name="underground-belt", x=x, y=entry,
                    direction=EntityDirection.SOUTH, io_type="input",
                    carries=lane.item,
                )
            )
            entities.append(
                PlacedEntity(
                    name="underground-belt", x=x, y=exit_,
                    direction=EntityDirection.SOUTH, io_type="output",
                    carries=lane.item,
                )
            )
            for uy in range(entry, exit_ + 1):
                underground_tiles.add(uy)

    # Place surface belts for remaining positions
    for y in range(start_y, end_y + 1):
        if y in underground_tiles or y in tap_off_set:
            continue
        entities.append(
            PlacedEntity(
                name="transport-belt", x=x, y=y,
                direction=EntityDirection.SOUTH, carries=lane.item,
            )
        )

    # Tap-off horizontal belts (EAST)
    for tap_y in lane.tap_off_ys:
        for hx in range(x, bw):
            entities.append(
                PlacedEntity(
                    name="transport-belt", x=hx, y=tap_y,
                    direction=EntityDirection.EAST, carries=lane.item,
                )
            )

    # Output return: WEST belts from row edge back to bus column
    if lane.producer_row is not None:
        out_y = lane.source_y  # source_y IS the output belt y
        for hx in range(x + 1, bw):
            entities.append(
                PlacedEntity(
                    name="transport-belt", x=hx, y=out_y,
                    direction=EntityDirection.WEST, carries=lane.item,
                )
            )
