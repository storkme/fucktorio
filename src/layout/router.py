"""Main bus belt routing: vertical item lanes + pipe lanes for fluids.

Uses underground belts and pipe-to-ground to pass under assembly rows
without collisions.
"""

from __future__ import annotations

from ..models import EntityDirection, MachineSpec, PlacedEntity, SolverResult

# Max underground distance (tiles between entry and exit, exclusive)
_UG_BELT_REACH = 5  # underground-belt: entry + 4 gap + exit
_UG_PIPE_REACH = 10  # pipe-to-ground: entry + 9 gap + exit


def _find_row_spans(
    machines: list[MachineSpec],
    dependency_order: list[str],
    bus_width: int,
) -> list[tuple[int, int, bool]]:
    """Calculate (y_start, y_end, has_fluid) for each assembly row.

    Mirrors the logic in placer.place_rows to know where rows land.
    """

    spans: list[tuple[int, int, bool]] = []
    y_cursor = 0

    ordered_specs = []
    recipe_to_spec = {m.recipe: m for m in machines}
    for recipe in dependency_order:
        if recipe in recipe_to_spec:
            ordered_specs.append(recipe_to_spec[recipe])
    for m in machines:
        if m not in ordered_specs:
            ordered_specs.append(m)

    for spec in ordered_specs:
        has_fluid = any(f.is_fluid for f in spec.inputs + spec.outputs)
        num_solid = sum(1 for f in spec.inputs if not f.is_fluid)

        is_refinery = spec.entity == "oil-refinery"

        if is_refinery:
            row_h = 8
        elif has_fluid:
            row_h = 7
        elif num_solid <= 1:
            row_h = 7
        else:
            row_h = 8

        spans.append((y_cursor, y_cursor + row_h, has_fluid))
        y_cursor += row_h + 1

    return spans


def route_bus(
    solver_result: SolverResult,
    total_height: int,
    bus_width: int = 6,
    row_spans: list[tuple[int, int, bool]] | None = None,
) -> list[PlacedEntity]:
    """Create vertical main bus on the left side of the layout.

    Solid items use transport-belt on the surface and underground-belt to
    tunnel under assembly rows. Fluids use pipe on the surface and
    pipe-to-ground to tunnel.

    *row_spans* is a list of (y_start, y_end, has_fluid) describing where
    each assembly row sits vertically. The bus goes underground before each
    row and resurfaces after it.
    """
    entities: list[PlacedEntity] = []

    if row_spans is None:
        row_spans = _find_row_spans(
            solver_result.machines,
            solver_result.dependency_order,
            bus_width,
        )

    # Build a set of y-ranges that the bus needs to tunnel under.
    # We tunnel from 1 tile before the row starts to 1 tile after it ends,
    # but we need entry/exit tiles that are NOT inside a row.
    # Strategy: for each row, go underground at y_start and resurface at y_end-1.
    # Between rows (and before/after all rows), use surface transport.

    # Collect all y values that are "inside" a row (where surface bus would collide
    # with row entities at x >= bus_width). The bus is at x < bus_width so it
    # won't actually collide with row entities. But to demonstrate underground
    # capability and make the layout more realistic, we'll tunnel under the
    # occupied zone of each row.
    #
    # Actually, the bus lanes at x=0,2,4,... are already to the LEFT of
    # bus_width, so they don't collide with row entities. Underground belts
    # are valuable when bus lanes need to CROSS row belts — which happens
    # when we add splitter taps later. For now, let's use them to keep the
    # bus clean: go underground through each row's vertical extent.

    for lane_idx, flow in enumerate(solver_result.external_inputs):
        x = lane_idx * 2

        if flow.is_fluid:
            _route_fluid_lane(entities, x, total_height, row_spans)
        else:
            _route_belt_lane(entities, x, total_height, row_spans)

    return entities


def _route_belt_lane(
    entities: list[PlacedEntity],
    x: int,
    total_height: int,
    row_spans: list[tuple[int, int, bool]],
) -> None:
    """Route a single belt bus lane with underground segments through rows."""
    # Merge row spans into underground segments
    underground_segments = _plan_underground_segments(row_spans, _UG_BELT_REACH)

    # Track which y positions are handled by underground segments
    underground_ys: set[int] = set()
    for entry_y, exit_y in underground_segments:
        # Entry: underground-belt input facing SOUTH
        entities.append(
            PlacedEntity(
                name="underground-belt",
                x=x,
                y=entry_y,
                direction=EntityDirection.SOUTH,
                io_type="input",
            )
        )
        # Exit: underground-belt output facing SOUTH
        entities.append(
            PlacedEntity(
                name="underground-belt",
                x=x,
                y=exit_y,
                direction=EntityDirection.SOUTH,
                io_type="output",
            )
        )
        for y in range(entry_y, exit_y + 1):
            underground_ys.add(y)

    # Fill remaining positions with surface belts
    for y in range(total_height):
        if y not in underground_ys:
            entities.append(
                PlacedEntity(
                    name="transport-belt",
                    x=x,
                    y=y,
                    direction=EntityDirection.SOUTH,
                )
            )


def _route_fluid_lane(
    entities: list[PlacedEntity],
    x: int,
    total_height: int,
    row_spans: list[tuple[int, int, bool]],
) -> None:
    """Route a single pipe bus lane with pipe-to-ground through rows."""
    underground_segments = _plan_underground_segments(row_spans, _UG_PIPE_REACH)

    underground_ys: set[int] = set()
    for entry_y, exit_y in underground_segments:
        # pipe-to-ground entry (facing SOUTH = fluid enters going south)
        entities.append(
            PlacedEntity(
                name="pipe-to-ground",
                x=x,
                y=entry_y,
                direction=EntityDirection.SOUTH,
            )
        )
        # pipe-to-ground exit (facing NORTH = fluid exits going south)
        entities.append(
            PlacedEntity(
                name="pipe-to-ground",
                x=x,
                y=exit_y,
                direction=EntityDirection.NORTH,
            )
        )
        for y in range(entry_y, exit_y + 1):
            underground_ys.add(y)

    # Fill remaining with surface pipes
    for y in range(total_height):
        if y not in underground_ys:
            entities.append(
                PlacedEntity(
                    name="pipe",
                    x=x,
                    y=y,
                )
            )


def _plan_underground_segments(
    row_spans: list[tuple[int, int, bool]],
    max_reach: int,
) -> list[tuple[int, int]]:
    """Plan underground entry/exit positions to tunnel through rows.

    Returns list of (entry_y, exit_y) pairs.
    Each segment must satisfy: exit_y - entry_y <= max_reach.

    Adjacent rows with small gaps are merged into single underground
    segments to avoid entry/exit overlaps.
    """
    if not row_spans:
        return []

    # First, merge row spans that are close together (gap <= 2 tiles)
    merged_spans: list[tuple[int, int]] = []
    for y_start, y_end, _ in row_spans:
        if merged_spans and y_start - merged_spans[-1][1] <= 2:
            # Merge with previous span
            merged_spans[-1] = (merged_spans[-1][0], y_end)
        else:
            merged_spans.append((y_start, y_end))

    # Now plan underground segments for each merged span
    segments: list[tuple[int, int]] = []
    for y_start, y_end in merged_spans:
        entry_y = max(0, y_start - 1)
        exit_y = y_end  # y_end is exclusive

        span = exit_y - entry_y
        if span <= max_reach:
            segments.append((entry_y, exit_y))
        else:
            # Too long — split into multiple underground segments
            y = entry_y
            while y < exit_y:
                seg_exit = min(y + max_reach, exit_y)
                segments.append((y, seg_exit))
                y = seg_exit + 1

    return segments
