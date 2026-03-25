"""Assembly row templates: patterns of belts, inserters, and machines."""

from __future__ import annotations

import math

from ..models import EntityDirection, PlacedEntity, ItemFlow


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
            entities.append(PlacedEntity(
                name="transport-belt",
                x=mx + dx, y=y_offset,
                direction=EntityDirection.EAST,
            ))

        # Input inserter (centre of machine width)
        entities.append(PlacedEntity(
            name="inserter",
            x=mx + 1, y=y_offset + 1,
            direction=EntityDirection.SOUTH,
        ))

        # Assembling machine (3×3, tile_position = top-left)
        entities.append(PlacedEntity(
            name=machine_entity,
            x=mx, y=y_offset + 2,
            direction=EntityDirection.NORTH,
            recipe=recipe,
        ))

        # Output inserter
        entities.append(PlacedEntity(
            name="inserter",
            x=mx + 1, y=y_offset + 5,
            direction=EntityDirection.SOUTH,
        ))

        # Output belt tiles
        for dx in range(3):
            entities.append(PlacedEntity(
                name="transport-belt",
                x=mx + dx, y=y_offset + 6,
                direction=EntityDirection.EAST,
            ))

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
            entities.append(PlacedEntity(
                name="transport-belt",
                x=mx + dx, y=y_offset,
                direction=EntityDirection.EAST,
            ))

        # Input belt 2
        for dx in range(3):
            entities.append(PlacedEntity(
                name="transport-belt",
                x=mx + dx, y=y_offset + 1,
                direction=EntityDirection.EAST,
            ))

        # Inserter 1 at x+0 (picks from belt 2 at y+1, drops into machine at y+3)
        entities.append(PlacedEntity(
            name="inserter",
            x=mx, y=y_offset + 2,
            direction=EntityDirection.SOUTH,
        ))

        # Inserter 2 at x+2
        entities.append(PlacedEntity(
            name="inserter",
            x=mx + 2, y=y_offset + 2,
            direction=EntityDirection.SOUTH,
        ))

        # Assembler (3×3)
        entities.append(PlacedEntity(
            name=machine_entity,
            x=mx, y=y_offset + 3,
            direction=EntityDirection.NORTH,
            recipe=recipe,
        ))

        # Output inserter
        entities.append(PlacedEntity(
            name="inserter",
            x=mx + 1, y=y_offset + 6,
            direction=EntityDirection.SOUTH,
        ))

        # Output belt
        for dx in range(3):
            entities.append(PlacedEntity(
                name="transport-belt",
                x=mx + dx, y=y_offset + 7,
                direction=EntityDirection.EAST,
            ))

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

    Layout:
        y+0 : fluid input pipes (connecting machines horizontally)
        y+1 : item input belt (EAST) — for solid ingredients
        y+2 : item input inserter (SOUTH)
        y+3..y+5 : machine (3×3)
        y+6 : item output inserter (SOUTH)
        y+7 : item output belt (EAST) — for solid products
        y+8 : fluid output pipes (connecting machines horizontally)
    """
    entities: list[PlacedEntity] = []

    if inputs is None:
        inputs = []
    has_solid_input = any(not f.is_fluid for f in inputs)

    ROW_HEIGHT = 9

    for i in range(machine_count):
        mx = x_offset + i * 4

        # Fluid input pipe row — pipes across machine width + gap pipe to next
        for dx in range(3):
            entities.append(PlacedEntity(name="pipe", x=mx + dx, y=y_offset))
        if i < machine_count - 1:
            entities.append(PlacedEntity(name="pipe", x=mx + 3, y=y_offset))

        # Item input belt
        for dx in range(3):
            entities.append(PlacedEntity(
                name="transport-belt",
                x=mx + dx, y=y_offset + 1,
                direction=EntityDirection.EAST,
            ))

        # Item input inserter (only if solid ingredients exist)
        if has_solid_input:
            entities.append(PlacedEntity(
                name="inserter",
                x=mx + 1, y=y_offset + 2,
                direction=EntityDirection.SOUTH,
            ))

        # Machine (3×3)
        entities.append(PlacedEntity(
            name=machine_entity,
            x=mx, y=y_offset + 3,
            direction=EntityDirection.NORTH,
            recipe=recipe,
        ))

        # Item output inserter
        entities.append(PlacedEntity(
            name="inserter",
            x=mx + 1, y=y_offset + 6,
            direction=EntityDirection.SOUTH,
        ))

        # Item output belt
        for dx in range(3):
            entities.append(PlacedEntity(
                name="transport-belt",
                x=mx + dx, y=y_offset + 7,
                direction=EntityDirection.EAST,
            ))

        # Fluid output pipe row
        for dx in range(3):
            entities.append(PlacedEntity(name="pipe", x=mx + dx, y=y_offset + 8))
        if i < machine_count - 1:
            entities.append(PlacedEntity(name="pipe", x=mx + 3, y=y_offset + 8))

    return entities, ROW_HEIGHT
