"""Main bus routing: vertical item lanes + pipe lanes for fluids.

Belt lanes use underground belts to pass under assembly rows.
Fluid lanes use surface pipes with horizontal tap-offs into consuming rows.
"""

from __future__ import annotations

from ..models import EntityDirection, ItemFlow, MachineSpec, PlacedEntity, SolverResult

# Max underground distance (tiles between entry and exit, exclusive)
_UG_BELT_REACH = 5  # underground-belt: entry + 4 gap + exit
_UG_PIPE_REACH = 10  # pipe-to-ground: entry + 9 gap + exit


def _order_specs(
    machines: list[MachineSpec],
    dependency_order: list[str],
) -> list[MachineSpec]:
    """Return machine specs in dependency order (inputs first)."""
    ordered: list[MachineSpec] = []
    recipe_to_spec = {m.recipe: m for m in machines}
    for recipe in dependency_order:
        if recipe in recipe_to_spec:
            ordered.append(recipe_to_spec[recipe])
    for m in machines:
        if m not in ordered:
            ordered.append(m)
    return ordered


def _row_height(spec: MachineSpec) -> int:
    """Return the row height for a given machine spec."""
    has_fluid = any(f.is_fluid for f in spec.inputs + spec.outputs)
    num_solid = sum(1 for f in spec.inputs if not f.is_fluid)

    if spec.entity == "oil-refinery":
        return 8
    elif has_fluid or num_solid <= 1:
        return 7
    else:
        return 8


def _find_row_spans(
    machines: list[MachineSpec],
    dependency_order: list[str],
    bus_width: int,
) -> list[tuple[int, int, bool, MachineSpec]]:
    """Calculate (y_start, y_end, has_fluid, spec) for each assembly row.

    Mirrors the logic in placer.place_rows to know where rows land.
    """

    spans: list[tuple[int, int, bool, MachineSpec]] = []
    y_cursor = 0

    for spec in _order_specs(machines, dependency_order):
        has_fluid = any(f.is_fluid for f in spec.inputs + spec.outputs)
        row_h = _row_height(spec)
        spans.append((y_cursor, y_cursor + row_h, has_fluid, spec))
        y_cursor += row_h + 1

    return spans


def route_bus(
    solver_result: SolverResult,
    total_height: int,
    bus_width: int = 6,
    row_spans: list[tuple[int, int, bool, MachineSpec]] | None = None,
) -> list[PlacedEntity]:
    """Create vertical main bus on the left side of the layout.

    Solid items use transport-belt on the surface and underground-belt to
    tunnel under assembly rows.  Fluids use surface pipes with horizontal
    tap-offs into rows that consume the fluid.

    *row_spans* is a list of (y_start, y_end, has_fluid, spec) describing
    where each assembly row sits vertically.
    """
    entities: list[PlacedEntity] = []

    if row_spans is None:
        row_spans = _find_row_spans(
            solver_result.machines,
            solver_result.dependency_order,
            bus_width,
        )

    # Belt lanes use underground segments to tunnel through rows.
    # The bus lanes (x < bus_width) don't collide with row entities
    # (x >= bus_width), but underground belts keep the layout clean
    # and are needed when splitter taps cross row belts.
    #
    # Fluid lanes use surface pipes with horizontal tap-off pipes
    # connecting into each row that consumes the fluid.

    for lane_idx, flow in enumerate(solver_result.external_inputs):
        x = lane_idx * 2

        if flow.is_fluid:
            _route_fluid_lane(entities, x, total_height, row_spans, flow, bus_width)
        else:
            _route_belt_lane(entities, x, total_height, row_spans)

    return entities


def _route_belt_lane(
    entities: list[PlacedEntity],
    x: int,
    total_height: int,
    row_spans: list[tuple[int, int, bool, MachineSpec]],
) -> None:
    """Route a single belt bus lane with underground segments through rows."""
    # Merge row spans into underground segments
    simple_spans = [(ys, ye, hf) for ys, ye, hf, _ in row_spans]
    underground_segments = _plan_underground_segments(simple_spans, _UG_BELT_REACH)

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


def _fluid_tap_y(y_start: int, spec: MachineSpec) -> int:
    """Return the y coordinate where a fluid bus lane should tap into a row.

    This is the y of the row's input pipe run, which must match the
    template layout in templates.py.
    """
    if spec.entity == "oil-refinery":
        # refinery_row: input pipes at y+6 (south ports)
        return y_start + 6
    else:
        # fluid_row: input pipes at y+1 (north ports)
        return y_start + 1


def _route_fluid_lane(
    entities: list[PlacedEntity],
    x: int,
    total_height: int,
    row_spans: list[tuple[int, int, bool, MachineSpec]],
    flow: ItemFlow,
    bus_width: int,
) -> None:
    """Route a fluid bus lane with horizontal tap-offs into consuming rows.

    The vertical bus runs as surface pipes (no underground needed since bus
    lanes at x < bus_width don't collide with row entities at x >= bus_width).
    At each row that consumes this fluid, horizontal pipes bridge from the
    bus to the row's pipe run.
    """
    # Vertical surface pipes for the full height
    for y in range(total_height):
        entities.append(PlacedEntity(name="pipe", x=x, y=y))

    # Horizontal tap-offs into consuming rows
    for y_start, _y_end, _has_fluid, spec in row_spans:
        # Check if this row consumes this fluid as an input
        if not any(inp.item == flow.item and inp.is_fluid for inp in spec.inputs):
            continue

        tap_y = _fluid_tap_y(y_start, spec)

        # Determine the x of the leftmost row pipe to connect to.
        # fluid_row: first pipe at mx=bus_width (chemical-plant port at mx+0)
        #            or mx+1 (assembling-machine-3 port at mx+1)
        # refinery_row: first input pipe at mx+1 (port at mx+1)
        if spec.entity == "oil-refinery":
            row_pipe_x = bus_width + 1
        elif spec.entity == "chemical-plant":
            row_pipe_x = bus_width
        else:
            # assembling-machine-3: input port at mx+1
            row_pipe_x = bus_width + 1

        # Fill horizontal pipes from bus lane to row pipe (exclusive of
        # endpoints which already have pipes)
        for hx in range(x + 1, row_pipe_x):
            entities.append(PlacedEntity(name="pipe", x=hx, y=tap_y))


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
