"""Assembly row templates: patterns of belts, inserters, and machines.

Every belt and inserter entity is tagged with ``carries`` so the validator
can trace item flow through the layout.

Machines are packed with zero gap (3-tile pitch for 3x3 machines).
When lane splitting is active, machines are split into two groups with a
sideload bridge in between so both output belt lanes are utilised.
"""

from __future__ import annotations

from ..models import EntityDirection, PlacedEntity

# Horizontal pitch: machine width with no gap
MACHINE_PITCH = 3

# Gap between machine groups when lane-splitting output belts.
# 3 tiles: 1 for sideload target filler, 1 for through-belt filler,
# 1 for the NORTH lift from group 2.
LANE_SPLIT_GAP = 3


def _machine_xs(
    x_offset: int,
    machine_count: int,
    lane_split: bool,
    pitch: int = MACHINE_PITCH,
) -> list[int]:
    """Return x-coordinates for each machine, accounting for lane-split gap."""
    if not lane_split or machine_count < 2:
        return [x_offset + i * pitch for i in range(machine_count)]

    g1 = machine_count // 2
    positions: list[int] = []
    for i in range(g1):
        positions.append(x_offset + i * pitch)
    for j in range(machine_count - g1):
        positions.append(x_offset + g1 * pitch + LANE_SPLIT_GAP + j * pitch)
    return positions


def _sideload_bridge(
    gap_start_x: int,
    y_offset: int,
    output_row_dy: int,
    belt: str,
    item: str,
    output_east: bool = False,
) -> list[PlacedEntity]:
    """Generate the 6-entity sideload bridge between two machine groups.

    ``output_row_dy`` is the output belt's offset from ``y_offset``
    (6 for single_input_row, 7 for dual_input_row).

    When ``output_east`` is True, the bridge is mirrored: group 1 items
    flow EAST across the bridge into group 2 (instead of group 2 → group 1).
    """
    bridge_y = y_offset + output_row_dy - 1  # one row above output belt
    output_y = y_offset + output_row_dy

    if output_east:
        # EAST flow: group 1 → bridge EAST → group 2
        return [
            # Bridge row
            PlacedEntity(
                name=belt,
                x=gap_start_x,
                y=bridge_y,
                direction=EntityDirection.EAST,
                carries=item,
            ),
            PlacedEntity(
                name=belt,
                x=gap_start_x + 1,
                y=bridge_y,
                direction=EntityDirection.EAST,
                carries=item,
            ),
            PlacedEntity(
                name=belt,
                x=gap_start_x + 2,
                y=bridge_y,
                direction=EntityDirection.SOUTH,
                carries=item,
            ),
            # Output belt row — gap tiles
            PlacedEntity(
                name=belt,
                x=gap_start_x,
                y=output_y,
                direction=EntityDirection.NORTH,
                carries=item,
            ),  # lifts group1 items up to bridge
            PlacedEntity(
                name=belt,
                x=gap_start_x + 1,
                y=output_y,
                direction=EntityDirection.EAST,
                carries=item,
            ),  # through-belt filler
            PlacedEntity(
                name=belt,
                x=gap_start_x + 2,
                y=output_y,
                direction=EntityDirection.EAST,
                carries=item,
            ),  # sideload target (through-belt)
        ]

    return [
        # Bridge row (y+5 or y+6 depending on template)
        PlacedEntity(
            name=belt,
            x=gap_start_x,
            y=bridge_y,
            direction=EntityDirection.SOUTH,
            carries=item,
        ),
        PlacedEntity(
            name=belt,
            x=gap_start_x + 1,
            y=bridge_y,
            direction=EntityDirection.WEST,
            carries=item,
        ),
        PlacedEntity(
            name=belt,
            x=gap_start_x + 2,
            y=bridge_y,
            direction=EntityDirection.WEST,
            carries=item,
        ),
        # Output belt row — gap tiles
        PlacedEntity(
            name=belt,
            x=gap_start_x,
            y=output_y,
            direction=EntityDirection.WEST,
            carries=item,
        ),  # sideload target (through-belt)
        PlacedEntity(
            name=belt,
            x=gap_start_x + 1,
            y=output_y,
            direction=EntityDirection.WEST,
            carries=item,
        ),  # through-belt filler
        PlacedEntity(
            name=belt,
            x=gap_start_x + 2,
            y=output_y,
            direction=EntityDirection.NORTH,
            carries=item,
        ),  # lifts group2 items up to bridge
    ]


def single_input_row(
    recipe: str,
    machine_entity: str,
    machine_count: int,
    y_offset: int,
    x_offset: int = 0,
    input_item: str = "",
    output_item: str = "",
    input_belt: str = "transport-belt",
    output_belt: str = "transport-belt",
    lane_split: bool = False,
    output_east: bool = False,
) -> tuple[list[PlacedEntity], int]:
    """Row for a recipe with 1 solid input.

    Layout per machine (3-tile horizontal pitch, no gaps):
        y+0 : input belt (EAST)
        y+1 : input inserter (SOUTH)
        y+2..y+4 : machine (3x3)
        y+5 : output inserter (SOUTH)
        y+6 : output belt (WEST -- toward bus)

    When lane_split=True, machines are split into two groups with a
    sideload bridge between them so the output belt uses both lanes.
    """
    entities: list[PlacedEntity] = []
    ROW_HEIGHT = 7

    lane_split = lane_split and machine_count >= 2
    mxs = _machine_xs(x_offset, machine_count, lane_split)
    g1 = machine_count // 2 if lane_split else machine_count

    for mx in mxs:
        # Input belt (3 tiles wide, continuous with adjacent machines)
        for dx in range(3):
            entities.append(
                PlacedEntity(
                    name=input_belt,
                    x=mx + dx,
                    y=y_offset,
                    direction=EntityDirection.EAST,
                    carries=input_item,
                )
            )

        # Input inserter
        entities.append(
            PlacedEntity(
                name="inserter",
                x=mx + 1,
                y=y_offset + 1,
                direction=EntityDirection.SOUTH,
                carries=input_item,
            )
        )

        # Machine (3x3)
        entities.append(
            PlacedEntity(
                name=machine_entity,
                x=mx,
                y=y_offset + 2,
                direction=EntityDirection.NORTH,
                recipe=recipe,
            )
        )

        # Output inserter
        entities.append(
            PlacedEntity(
                name="inserter",
                x=mx + 1,
                y=y_offset + 5,
                direction=EntityDirection.SOUTH,
                carries=output_item,
            )
        )

        # Output belt (3 tiles wide)
        out_dir = EntityDirection.EAST if output_east else EntityDirection.WEST
        for dx in range(3):
            entities.append(
                PlacedEntity(
                    name=output_belt,
                    x=mx + dx,
                    y=y_offset + 6,
                    direction=out_dir,
                    carries=output_item,
                )
            )

    if lane_split:
        gap_start_x = x_offset + g1 * MACHINE_PITCH
        # Input belt tiles through the gap (keep items flowing to group2)
        for dx in range(LANE_SPLIT_GAP):
            entities.append(
                PlacedEntity(
                    name=input_belt,
                    x=gap_start_x + dx,
                    y=y_offset,
                    direction=EntityDirection.EAST,
                    carries=input_item,
                )
            )
        # Sideload bridge
        entities.extend(_sideload_bridge(gap_start_x, y_offset, 6, output_belt, output_item, output_east))

    return entities, ROW_HEIGHT


def dual_input_row(
    recipe: str,
    machine_entity: str,
    machine_count: int,
    y_offset: int,
    x_offset: int = 0,
    input_items: tuple[str, str] = ("", ""),
    output_item: str = "",
    input_belts: tuple[str, str] = ("transport-belt", "transport-belt"),
    output_belt: str = "transport-belt",
    lane_split: bool = False,
    output_east: bool = False,
) -> tuple[list[PlacedEntity], int]:
    """Row for a recipe with 2 solid inputs.

    Layout per machine (3-tile horizontal pitch, no gaps):
        y+0 : input belt 1 (EAST) -- far belt
        y+1 : input belt 2 (EAST) -- close belt
        y+2 : long-handed inserter (picks y+0) + inserter (picks y+1)
        y+3..y+5 : machine (3x3)
        y+6 : output inserter (SOUTH)
        y+7 : output belt (WEST -- toward bus)

    When lane_split=True, machines are split into two groups with a
    sideload bridge between them so the output belt uses both lanes.
    """
    entities: list[PlacedEntity] = []
    ROW_HEIGHT = 8
    input1, input2 = input_items
    belt1, belt2 = input_belts

    lane_split = lane_split and machine_count >= 2
    mxs = _machine_xs(x_offset, machine_count, lane_split)
    g1 = machine_count // 2 if lane_split else machine_count

    for mx in mxs:
        # Input belt 1 -- far belt
        for dx in range(3):
            entities.append(
                PlacedEntity(
                    name=belt1,
                    x=mx + dx,
                    y=y_offset,
                    direction=EntityDirection.EAST,
                    carries=input1,
                )
            )

        # Input belt 2 -- close belt
        for dx in range(3):
            entities.append(
                PlacedEntity(
                    name=belt2,
                    x=mx + dx,
                    y=y_offset + 1,
                    direction=EntityDirection.EAST,
                    carries=input2,
                )
            )

        # Long-handed inserter (picks from far belt y+0, drops into machine y+3)
        entities.append(
            PlacedEntity(
                name="long-handed-inserter",
                x=mx,
                y=y_offset + 2,
                direction=EntityDirection.SOUTH,
                carries=input1,
            )
        )

        # Regular inserter (picks from close belt y+1, drops into machine y+3)
        entities.append(
            PlacedEntity(
                name="inserter",
                x=mx + 2,
                y=y_offset + 2,
                direction=EntityDirection.SOUTH,
                carries=input2,
            )
        )

        # Machine (3x3)
        entities.append(
            PlacedEntity(
                name=machine_entity,
                x=mx,
                y=y_offset + 3,
                direction=EntityDirection.NORTH,
                recipe=recipe,
            )
        )

        # Output inserter
        entities.append(
            PlacedEntity(
                name="inserter",
                x=mx + 1,
                y=y_offset + 6,
                direction=EntityDirection.SOUTH,
                carries=output_item,
            )
        )

        # Output belt
        out_dir = EntityDirection.EAST if output_east else EntityDirection.WEST
        for dx in range(3):
            entities.append(
                PlacedEntity(
                    name=output_belt,
                    x=mx + dx,
                    y=y_offset + 7,
                    direction=out_dir,
                    carries=output_item,
                )
            )

    if lane_split:
        gap_start_x = x_offset + g1 * MACHINE_PITCH
        # Input belt tiles through the gap for both input belts
        for dx in range(LANE_SPLIT_GAP):
            entities.append(
                PlacedEntity(
                    name=belt1,
                    x=gap_start_x + dx,
                    y=y_offset,
                    direction=EntityDirection.EAST,
                    carries=input1,
                )
            )
            entities.append(
                PlacedEntity(
                    name=belt2,
                    x=gap_start_x + dx,
                    y=y_offset + 1,
                    direction=EntityDirection.EAST,
                    carries=input2,
                )
            )
        # Sideload bridge (output belt at y+7, bridge at y+6)
        entities.extend(_sideload_bridge(gap_start_x, y_offset, 7, output_belt, output_item, output_east))

    return entities, ROW_HEIGHT


# Fluid port positions relative to machine tile_position.
# chemical-plant input ports: (0,0) north, (2,0) north
# assembling-machine-3 input port: (1,0) north
_FLUID_INPUT_PORT_DX = {
    "chemical-plant": 0,
    "assembling-machine-3": 1,
}


def fluid_input_row(
    recipe: str,
    machine_entity: str,
    machine_count: int,
    y_offset: int,
    x_offset: int = 0,
    solid_item: str = "",
    fluid_item: str = "",
    output_item: str = "",
    input_belt: str = "transport-belt",
    output_belt: str = "transport-belt",
    output_east: bool = False,
) -> tuple[list[PlacedEntity], int, list[tuple[int, int]]]:
    """Row for a recipe with 1 solid input + 1 fluid input.

    Layout per machine (3-tile pitch, no gaps):
        y+0 : solid input belt (EAST)
        y+1 : inserter (solid) + pipe (fluid port connection)
        y+2..y+4 : machine (3x3)
        y+5 : output inserter (SOUTH)
        y+6 : output belt (WEST -- toward bus)

    Returns (entities, row_height, fluid_port_pipes) where
    fluid_port_pipes is a list of (x, y) for each machine's fluid port pipe.
    """
    entities: list[PlacedEntity] = []
    ROW_HEIGHT = 7
    port_dx = _FLUID_INPUT_PORT_DX.get(machine_entity, 0)

    fluid_port_pipes: list[tuple[int, int]] = []

    for i in range(machine_count):
        mx = x_offset + i * MACHINE_PITCH

        # Solid input belt (3 tiles wide)
        for dx in range(3):
            entities.append(
                PlacedEntity(
                    name=input_belt,
                    x=mx + dx,
                    y=y_offset,
                    direction=EntityDirection.EAST,
                    carries=solid_item,
                )
            )

        # y+1: inserter for solid + fluid port connection
        entities.append(
            PlacedEntity(
                name="inserter",
                x=mx + 1,
                y=y_offset + 1,
                direction=EntityDirection.SOUTH,
                carries=solid_item,
            )
        )

        if machine_entity == "chemical-plant":
            # Chemical-plant: pipe-to-ground bridges port (mx) past inserter
            # to port (mx+2). The ptg_exit at mx+2 connects to next machine's
            # ptg_entry at mx+3 (adjacent), forming a chain across all machines.
            entities.append(
                PlacedEntity(
                    name="pipe-to-ground",
                    x=mx,
                    y=y_offset + 1,
                    direction=EntityDirection.EAST,
                    io_type="input",
                    carries=fluid_item,
                )
            )
            entities.append(
                PlacedEntity(
                    name="pipe-to-ground",
                    x=mx + 2,
                    y=y_offset + 1,
                    direction=EntityDirection.EAST,
                    io_type="output",
                    carries=fluid_item,
                )
            )
            # Output port pipes at y+5 (south face of machine)
            entities.append(PlacedEntity(name="pipe", x=mx, y=y_offset + 5, carries=fluid_item))
            entities.append(PlacedEntity(name="pipe", x=mx + 2, y=y_offset + 5, carries=fluid_item))
        else:
            # Other machines: regular pipe at the port position
            entities.append(
                PlacedEntity(
                    name="pipe",
                    x=mx + port_dx,
                    y=y_offset + 1,
                    carries=fluid_item,
                )
            )

        if i == 0:
            fluid_port_pipes.append((mx, y_offset + 1))

        # Machine (3x3)
        entities.append(
            PlacedEntity(
                name=machine_entity,
                x=mx,
                y=y_offset + 2,
                direction=EntityDirection.NORTH,
                recipe=recipe,
            )
        )

        # Output inserter
        entities.append(
            PlacedEntity(
                name="inserter",
                x=mx + 1,
                y=y_offset + 5,
                direction=EntityDirection.SOUTH,
                carries=output_item,
            )
        )

        # Output belt
        out_dir = EntityDirection.EAST if output_east else EntityDirection.WEST
        for dx in range(3):
            entities.append(
                PlacedEntity(
                    name=output_belt,
                    x=mx + dx,
                    y=y_offset + 6,
                    direction=out_dir,
                    carries=output_item,
                )
            )

    return entities, ROW_HEIGHT, fluid_port_pipes


def fluid_dual_input_row(
    recipe: str,
    machine_entity: str,
    machine_count: int,
    y_offset: int,
    x_offset: int = 0,
    solid_items: tuple[str, str] = ("", ""),
    fluid_item: str = "",
    output_item: str = "",
    output_is_fluid: bool = False,
    input_belts: tuple[str, str] = ("transport-belt", "transport-belt"),
    output_belt: str = "transport-belt",
) -> tuple[list[PlacedEntity], int, list[tuple[int, int]], list[tuple[int, int]]]:
    """Row for a recipe with 2 solid inputs + 1 fluid input.

    Fluid is delivered via a horizontal pipe header ABOVE the machine row,
    with vertical pipe-to-ground tunnels per machine dropping fluid down to
    the machine's fluid input port. This frees y+4 for two inserters.

    Layout per machine (3-tile horizontal pitch, no gaps):
        y+0 : horizontal fluid header (pipes carrying fluid_item)
        y+1 : pipe-to-ground input at mx+port_dx (direction SOUTH)
        y+2 : solid input belt 1 (EAST) -- far belt
        y+3 : solid input belt 2 (EAST) -- close belt
        y+4 : long-handed-inserter at mx+1 + inserter at mx+2 +
              pipe-to-ground output at mx+port_dx (direction SOUTH)
        y+5..y+7 : machine (3x3)
        y+8 : fluid output pipes (if output_is_fluid) OR output inserter
        y+9 : output belt (solid output only)

    Returns (entities, row_height, fluid_input_port_pipes, fluid_output_port_pipes).
    ``fluid_input_port_pipes`` is a single tap point (leftmost header tile).
    ``fluid_output_port_pipes`` lists per-machine output pipe positions
    (only populated for fluid-output recipes).
    """
    entities: list[PlacedEntity] = []
    # Fluid output occupies y+8; add a trailing empty row so sub-row
    # stacking doesn't put output pipes adjacent to the next sub-row's
    # fluid header row (which would trip pipe-isolation).
    ROW_HEIGHT = 10
    input1, input2 = solid_items
    belt1, belt2 = input_belts
    port_dx = _FLUID_INPUT_PORT_DX.get(machine_entity, 0)

    header_y = y_offset
    ptg_in_y = y_offset + 1
    belt1_y = y_offset + 2
    belt2_y = y_offset + 3
    inserter_y = y_offset + 4
    machine_y = y_offset + 5
    output_y = y_offset + 8

    # Horizontal fluid header chain: spans x_offset .. last machine's mx+2
    last_mx = x_offset + (machine_count - 1) * MACHINE_PITCH
    header_end_x = last_mx + 2
    for x in range(x_offset, header_end_x + 1):
        entities.append(PlacedEntity(name="pipe", x=x, y=header_y, carries=fluid_item))

    fluid_output_port_pipes: list[tuple[int, int]] = []

    for i in range(machine_count):
        mx = x_offset + i * MACHINE_PITCH

        # Vertical PTG pair: input at y+1 tunnels SOUTH to output at y+4
        # (span = 2 internal tiles, well within basic PTG reach)
        entities.append(
            PlacedEntity(
                name="pipe-to-ground",
                x=mx + port_dx,
                y=ptg_in_y,
                direction=EntityDirection.SOUTH,
                io_type="input",
                carries=fluid_item,
            )
        )
        entities.append(
            PlacedEntity(
                name="pipe-to-ground",
                x=mx + port_dx,
                y=inserter_y,
                direction=EntityDirection.SOUTH,
                io_type="output",
                carries=fluid_item,
            )
        )

        # Solid input belts (3 tiles wide each)
        for dx in range(3):
            entities.append(
                PlacedEntity(
                    name=belt1,
                    x=mx + dx,
                    y=belt1_y,
                    direction=EntityDirection.EAST,
                    carries=input1,
                )
            )
            entities.append(
                PlacedEntity(
                    name=belt2,
                    x=mx + dx,
                    y=belt2_y,
                    direction=EntityDirection.EAST,
                    carries=input2,
                )
            )

        # Long-handed inserter (far belt -> machine) at mx+1
        entities.append(
            PlacedEntity(
                name="long-handed-inserter",
                x=mx + 1,
                y=inserter_y,
                direction=EntityDirection.SOUTH,
                carries=input1,
            )
        )

        # Regular inserter (close belt -> machine) at mx+2
        entities.append(
            PlacedEntity(
                name="inserter",
                x=mx + 2,
                y=inserter_y,
                direction=EntityDirection.SOUTH,
                carries=input2,
            )
        )

        # Machine (3x3)
        entities.append(
            PlacedEntity(
                name=machine_entity,
                x=mx,
                y=machine_y,
                direction=EntityDirection.NORTH,
                recipe=recipe,
            )
        )

        # Output row
        if output_is_fluid:
            # Chemical-plant fluid output ports at (0,2) and (2,2) south ->
            # pipes one tile south of the machine (y=output_y) at mx+0, mx+2.
            entities.append(PlacedEntity(name="pipe", x=mx, y=output_y, carries=output_item))
            entities.append(PlacedEntity(name="pipe", x=mx + 2, y=output_y, carries=output_item))
            fluid_output_port_pipes.append((mx, output_y))
            fluid_output_port_pipes.append((mx + 2, output_y))
        else:
            # Solid output: inserter at y+8, belt at y+9
            entities.append(
                PlacedEntity(
                    name="inserter",
                    x=mx + 1,
                    y=output_y,
                    direction=EntityDirection.SOUTH,
                    carries=output_item,
                )
            )
            for dx in range(3):
                entities.append(
                    PlacedEntity(
                        name=output_belt,
                        x=mx + dx,
                        y=output_y + 1,
                        direction=EntityDirection.WEST,
                        carries=output_item,
                    )
                )

    fluid_input_port_pipes = [(x_offset, header_y)]

    return entities, ROW_HEIGHT, fluid_input_port_pipes, fluid_output_port_pipes


# Oil refinery: 5x5 machine, fluid-in + fluid-out, placed with mirror=True
# so fluid ports flip N<->S. Layout:
#   y+0 : crude-oil input pipe (at mx+1, one per refinery)
#   y+1..y+5 : oil-refinery entity (5x5)
#   y+6 : petroleum-gas output pipe (at mx+0, one per refinery)
OIL_REFINERY_PITCH = 5


def oil_refinery_row(
    recipe: str,
    machine_count: int,
    y_offset: int,
    x_offset: int = 0,
    fluid_input_item: str = "crude-oil",
    fluid_output_item: str = "petroleum-gas",
) -> tuple[list[PlacedEntity], int, list[tuple[int, int]], list[tuple[int, int]]]:
    """Row for basic-oil-processing (1 fluid in, 1 fluid out, 5x5 refinery).

    Refineries are placed at ``direction=NORTH`` with ``mirror=True`` so
    crude-oil inputs sit at the NORTH edge (matching the bus trunk-above
    pattern) and petroleum-gas outputs sit at the SOUTH edge.

    With mirror+NORTH, fluid-box[0] (input, center-rel (-1, 2)) flips to
    center-rel (-1, -2); its external pipe lands at world (mx+1, y+0).
    Fluid-box[2] (first output, center-rel (-2, -2)) flips to (-2, 2);
    its external pipe lands at world (mx+0, y+6).

    Returns (entities, row_height, fluid_input_port_pipes, fluid_output_port_pipes).
    """
    entities: list[PlacedEntity] = []
    ROW_HEIGHT = 7
    fluid_input_port_pipes: list[tuple[int, int]] = []
    fluid_output_port_pipes: list[tuple[int, int]] = []

    for i in range(machine_count):
        mx = x_offset + i * OIL_REFINERY_PITCH

        # Input port pipe (crude-oil), 1 tile north of the refinery footprint
        input_x = mx + 1
        input_y = y_offset
        entities.append(
            PlacedEntity(
                name="pipe",
                x=input_x,
                y=input_y,
                carries=fluid_input_item,
            )
        )
        fluid_input_port_pipes.append((input_x, input_y))

        # Refinery (5x5), mirrored so inputs face north, outputs face south
        entities.append(
            PlacedEntity(
                name="oil-refinery",
                x=mx,
                y=y_offset + 1,
                direction=EntityDirection.NORTH,
                recipe=recipe,
                mirror=True,
            )
        )

        # Output port pipe (petroleum-gas), 1 tile south of the refinery footprint
        output_x = mx + 0
        output_y = y_offset + 6
        entities.append(
            PlacedEntity(
                name="pipe",
                x=output_x,
                y=output_y,
                carries=fluid_output_item,
            )
        )
        fluid_output_port_pipes.append((output_x, output_y))

    return entities, ROW_HEIGHT, fluid_input_port_pipes, fluid_output_port_pipes
