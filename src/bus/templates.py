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
) -> list[PlacedEntity]:
    """Generate the 6-entity sideload bridge between two machine groups.

    ``output_row_dy`` is the output belt's offset from ``y_offset``
    (6 for single_input_row, 7 for dual_input_row).
    """
    bridge_y = y_offset + output_row_dy - 1  # one row above output belt
    output_y = y_offset + output_row_dy

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

        # Output belt (3 tiles wide, flowing WEST toward bus)
        for dx in range(3):
            entities.append(
                PlacedEntity(
                    name=output_belt,
                    x=mx + dx,
                    y=y_offset + 6,
                    direction=EntityDirection.WEST,
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
        entities.extend(_sideload_bridge(gap_start_x, y_offset, 6, output_belt, output_item))

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

        # Output belt (WEST toward bus)
        for dx in range(3):
            entities.append(
                PlacedEntity(
                    name=output_belt,
                    x=mx + dx,
                    y=y_offset + 7,
                    direction=EntityDirection.WEST,
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
        entities.extend(_sideload_bridge(gap_start_x, y_offset, 7, output_belt, output_item))

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

        # Output belt (WEST toward bus)
        for dx in range(3):
            entities.append(
                PlacedEntity(
                    name=output_belt,
                    x=mx + dx,
                    y=y_offset + 6,
                    direction=EntityDirection.WEST,
                    carries=output_item,
                )
            )

    return entities, ROW_HEIGHT, fluid_port_pipes
