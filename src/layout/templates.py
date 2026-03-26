"""Assembly row templates: patterns of belts, inserters, and machines."""

from __future__ import annotations

from ..models import EntityDirection, ItemFlow, PlacedEntity


def single_input_row(
    recipe: str,
    machine_entity: str,
    machine_count: int,
    y_offset: int,
    x_offset: int = 0,
    input_item: str | None = None,
) -> tuple[list[PlacedEntity], int]:
    """Lay out a row for a recipe with 1 belt input.

    Returns (entities, row_height).

    Layout (each machine block is 5 tiles wide: 1 gap + 3 machine + 1 gap):
        Row y+0 : input belt (EAST)
        Row y+1 : input inserter (SOUTH — picks from belt above, drops into machine below)
        Row y+2..y+4 : assembler (3×3, tile_position is top-left)
        Row y+5 : output inserter (SOUTH — picks from machine, drops onto belt)
        Row y+6 : output belt (EAST)
    """
    entities: list[PlacedEntity] = []
    ROW_HEIGHT = 7

    for i in range(machine_count):
        mx = x_offset + i * 4  # 3-wide machine + 1-tile gap

        # Input belt tiles (3 tiles wide to match machine)
        for dx in range(3):
            entities.append(
                PlacedEntity(
                    name="transport-belt",
                    x=mx + dx,
                    y=y_offset,
                    direction=EntityDirection.EAST,
                )
            )

        # Input inserter (centre of machine width)
        entities.append(
            PlacedEntity(
                name="inserter",
                x=mx + 1,
                y=y_offset + 1,
                direction=EntityDirection.SOUTH,
            )
        )

        # Assembling machine (3×3, tile_position = top-left)
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
            )
        )

        # Output belt tiles
        for dx in range(3):
            entities.append(
                PlacedEntity(
                    name="transport-belt",
                    x=mx + dx,
                    y=y_offset + 6,
                    direction=EntityDirection.EAST,
                )
            )

    return entities, ROW_HEIGHT


def dual_input_row(
    recipe: str,
    machine_entity: str,
    machine_count: int,
    y_offset: int,
    x_offset: int = 0,
) -> tuple[list[PlacedEntity], int]:
    """Lay out a row for a recipe with 2 solid belt inputs.

    Layout — inserters directly adjacent to the 3×3 machine:
        y+0 : input belt 1 (EAST)
        y+1 : input belt 2 (EAST)
        y+2 : two inserters (SOUTH — pick belt 2, drop into machine top y+3)
        y+3..y+5 : assembler (3×3)
        y+6 : output inserter (SOUTH — picks machine bottom y+5, drops belt y+7)
        y+7 : output belt (EAST)
    """
    entities: list[PlacedEntity] = []
    ROW_HEIGHT = 8

    for i in range(machine_count):
        mx = x_offset + i * 4

        # Input belt 1
        for dx in range(3):
            entities.append(
                PlacedEntity(
                    name="transport-belt",
                    x=mx + dx,
                    y=y_offset,
                    direction=EntityDirection.EAST,
                )
            )

        # Input belt 2
        for dx in range(3):
            entities.append(
                PlacedEntity(
                    name="transport-belt",
                    x=mx + dx,
                    y=y_offset + 1,
                    direction=EntityDirection.EAST,
                )
            )

        # Inserter 1 at x+0 (picks from belt 2 at y+1, drops into machine at y+3)
        entities.append(
            PlacedEntity(
                name="inserter",
                x=mx,
                y=y_offset + 2,
                direction=EntityDirection.SOUTH,
            )
        )

        # Inserter 2 at x+2
        entities.append(
            PlacedEntity(
                name="inserter",
                x=mx + 2,
                y=y_offset + 2,
                direction=EntityDirection.SOUTH,
            )
        )

        # Assembler (3×3)
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
            )
        )

        # Output belt
        for dx in range(3):
            entities.append(
                PlacedEntity(
                    name="transport-belt",
                    x=mx + dx,
                    y=y_offset + 7,
                    direction=EntityDirection.EAST,
                )
            )

    return entities, ROW_HEIGHT


def fluid_row(
    recipe: str,
    machine_entity: str,
    machine_count: int,
    y_offset: int,
    x_offset: int = 0,
    inputs: list[ItemFlow] | None = None,
) -> tuple[list[PlacedEntity], int]:
    """Lay out a row for a recipe that involves fluids.

    Uses pipes for fluid inputs/outputs and inserters+belts for items.
    Chemical plants and assemblers with fluid are both 3×3.

    Pipes are placed directly adjacent to machine fluid ports (from draftsman
    entity data) so that fluids actually connect in-game.

    chemical-plant ports (relative to tile_position):
        input:  (0, 0) north, (2, 0) north  → pipes at y-1
        output: (0, 2) south, (2, 2) south  → pipes at y+3

    assembling-machine-3 ports:
        input:  (1, 0) north  → pipe at y-1
        output: (1, 2) south  → pipe at y+3

    Layout (chemical-plant):
        y+0 : item input belt (EAST)
        y+1 : pipe(mx), inserter(mx+1), pipe(mx+2) — adjacent to machine top
        y+2..y+4 : machine (3×3)
        y+5 : pipe(mx), inserter(mx+1), pipe(mx+2) — adjacent to machine bottom
        y+6 : item output belt (EAST)

    Layout (assembling-machine-3 with fluid):
        y+0 : item input belt (EAST)
        y+1 : inserter(mx), pipe(mx+1) — adjacent to machine top
        y+2..y+4 : machine (3×3)
        y+5 : pipe(mx+1), inserter(mx+2) — adjacent to machine bottom
        y+6 : item output belt (EAST)
    """
    entities: list[PlacedEntity] = []

    if inputs is None:
        inputs = []
    has_solid_input = any(not f.is_fluid for f in inputs)

    ROW_HEIGHT = 7
    is_chem = machine_entity == "chemical-plant"

    for i in range(machine_count):
        mx = x_offset + i * 4

        # Item input belt
        for dx in range(3):
            entities.append(
                PlacedEntity(
                    name="transport-belt",
                    x=mx + dx,
                    y=y_offset,
                    direction=EntityDirection.EAST,
                )
            )

        # y+1: fluid input pipes + item input inserter (adjacent to machine top)
        if is_chem:
            # chemical-plant: input ports at (mx, my) and (mx+2, my) → pipes at mx, mx+2
            entities.append(PlacedEntity(name="pipe", x=mx, y=y_offset + 1))
            entities.append(PlacedEntity(name="pipe", x=mx + 2, y=y_offset + 1))
            if has_solid_input:
                # inserter at mx+1 (between the two pipes)
                entities.append(
                    PlacedEntity(
                        name="inserter",
                        x=mx + 1,
                        y=y_offset + 1,
                        direction=EntityDirection.SOUTH,
                    )
                )
        else:
            # assembling-machine-3: input port at (mx+1, my) → pipe at mx+1
            entities.append(PlacedEntity(name="pipe", x=mx + 1, y=y_offset + 1))
            if has_solid_input:
                # inserter at mx (offset to avoid pipe at mx+1)
                entities.append(
                    PlacedEntity(
                        name="inserter",
                        x=mx,
                        y=y_offset + 1,
                        direction=EntityDirection.SOUTH,
                    )
                )

        # Horizontal connector pipe to next machine
        if i < machine_count - 1:
            entities.append(PlacedEntity(name="pipe", x=mx + 3, y=y_offset + 1))

        # Machine (3×3)
        entities.append(
            PlacedEntity(
                name=machine_entity,
                x=mx,
                y=y_offset + 2,
                direction=EntityDirection.NORTH,
                recipe=recipe,
            )
        )

        # y+5: fluid output pipes + item output inserter (adjacent to machine bottom)
        if is_chem:
            # chemical-plant: output ports at (mx, my+2) and (mx+2, my+2) → pipes at mx, mx+2
            entities.append(PlacedEntity(name="pipe", x=mx, y=y_offset + 5))
            entities.append(PlacedEntity(name="pipe", x=mx + 2, y=y_offset + 5))
            # output inserter at mx+1
            entities.append(
                PlacedEntity(
                    name="inserter",
                    x=mx + 1,
                    y=y_offset + 5,
                    direction=EntityDirection.SOUTH,
                )
            )
        else:
            # assembling-machine-3: output port at (mx+1, my+2) → pipe at mx+1
            entities.append(PlacedEntity(name="pipe", x=mx + 1, y=y_offset + 5))
            # output inserter at mx+2 (offset to avoid pipe)
            entities.append(
                PlacedEntity(
                    name="inserter",
                    x=mx + 2,
                    y=y_offset + 5,
                    direction=EntityDirection.SOUTH,
                )
            )

        # Horizontal connector pipe to next machine (output side)
        if i < machine_count - 1:
            entities.append(PlacedEntity(name="pipe", x=mx + 3, y=y_offset + 5))

        # Item output belt
        for dx in range(3):
            entities.append(
                PlacedEntity(
                    name="transport-belt",
                    x=mx + dx,
                    y=y_offset + 6,
                    direction=EntityDirection.EAST,
                )
            )

    return entities, ROW_HEIGHT


def refinery_row(
    recipe: str,
    machine_entity: str,
    machine_count: int,
    y_offset: int,
    x_offset: int = 0,
    inputs: list[ItemFlow] | None = None,
) -> tuple[list[PlacedEntity], int]:
    """Lay out a row for oil-refinery (5x5) recipes.

    Uses pipes for fluid inputs/outputs and inserters+belts for solid items.
    Pipes are placed directly adjacent to machine fluid ports.

    oil-refinery ports (relative to tile_position):
        input:  (1, 4) south, (3, 4) south  → pipes at y+5 below machine
        output: (0, 0) north, (2, 0) north, (4, 0) north → pipes at y-1 above

    Note: refinery inputs are on the SOUTH side, outputs on the NORTH side.

    Layout:
        y+0  : fluid output pipes at mx, mx+2, mx+4 + inserter at mx+1 or mx+3
        y+1..y+5 : machine (5×5)
        y+6  : fluid input pipes at mx+1, mx+3 + inserter at mx+2
        y+7  : item input belt (EAST) — for any solid ingredients
    """
    entities: list[PlacedEntity] = []

    if inputs is None:
        inputs = []
    has_solid_input = any(not f.is_fluid for f in inputs)

    ROW_HEIGHT = 8
    MACHINE_PITCH = 6  # 5-wide machine + 1-tile gap

    for i in range(machine_count):
        mx = x_offset + i * MACHINE_PITCH

        # y+0: fluid output pipes (adjacent to machine top, connecting to north ports)
        # Output ports at (mx, my), (mx+2, my), (mx+4, my) → pipes one tile above
        entities.append(PlacedEntity(name="pipe", x=mx, y=y_offset))
        entities.append(PlacedEntity(name="pipe", x=mx + 2, y=y_offset))
        entities.append(PlacedEntity(name="pipe", x=mx + 4, y=y_offset))
        # Output inserter between pipes (for any solid outputs)
        entities.append(
            PlacedEntity(
                name="inserter",
                x=mx + 3,
                y=y_offset,
                direction=EntityDirection.NORTH,
            )
        )
        # Horizontal connector pipes between output pipes
        entities.append(PlacedEntity(name="pipe", x=mx + 1, y=y_offset))
        if i < machine_count - 1:
            entities.append(PlacedEntity(name="pipe", x=mx + 5, y=y_offset))

        # y+1..y+5: Machine (5×5)
        entities.append(
            PlacedEntity(
                name=machine_entity,
                x=mx,
                y=y_offset + 1,
                direction=EntityDirection.NORTH,
                recipe=recipe,
            )
        )

        # y+6: fluid input pipes (adjacent to machine bottom, connecting to south ports)
        # Input ports at (mx+1, my+4) and (mx+3, my+4) → pipes one tile below
        entities.append(PlacedEntity(name="pipe", x=mx + 1, y=y_offset + 6))
        entities.append(PlacedEntity(name="pipe", x=mx + 3, y=y_offset + 6))
        # Input inserter at mx+2 (between the two input pipes)
        if has_solid_input:
            entities.append(
                PlacedEntity(
                    name="inserter",
                    x=mx + 2,
                    y=y_offset + 6,
                    direction=EntityDirection.NORTH,
                )
            )
        # Horizontal connector pipes for input side
        if i < machine_count - 1:
            entities.append(PlacedEntity(name="pipe", x=mx + 5, y=y_offset + 6))

        # y+7: item input belt (for any solid ingredients)
        if has_solid_input:
            for dx in range(5):
                entities.append(
                    PlacedEntity(
                        name="transport-belt",
                        x=mx + dx,
                        y=y_offset + 7,
                        direction=EntityDirection.EAST,
                    )
                )

    return entities, ROW_HEIGHT
