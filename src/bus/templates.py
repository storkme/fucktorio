"""Assembly row templates: patterns of belts, inserters, and machines.

Every belt and inserter entity is tagged with ``carries`` so the validator
can trace item flow through the layout.
"""

from __future__ import annotations

from ..models import EntityDirection, PlacedEntity


def single_input_row(
    recipe: str,
    machine_entity: str,
    machine_count: int,
    y_offset: int,
    x_offset: int = 0,
    input_item: str = "",
    output_item: str = "",
) -> tuple[list[PlacedEntity], int]:
    """Row for a recipe with 1 solid input.

    Layout per machine (4-tile horizontal pitch):
        y+0 : input belt (EAST)
        y+1 : input inserter (SOUTH)
        y+2..y+4 : machine (3x3)
        y+5 : output inserter (SOUTH)
        y+6 : output belt (WEST — toward bus)
    """
    entities: list[PlacedEntity] = []
    ROW_HEIGHT = 7

    for i in range(machine_count):
        mx = x_offset + i * 4

        # Input belt (3 tiles wide, flowing EAST along the row)
        for dx in range(3):
            entities.append(
                PlacedEntity(
                    name="transport-belt",
                    x=mx + dx,
                    y=y_offset,
                    direction=EntityDirection.EAST,
                    carries=input_item,
                )
            )

        # Input inserter (picks from belt at y+0, drops into machine at y+2)
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

        # Output inserter (picks from machine at y+4, drops onto belt at y+6)
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
                    name="transport-belt",
                    x=mx + dx,
                    y=y_offset + 6,
                    direction=EntityDirection.WEST,
                    carries=output_item,
                )
            )

    return entities, ROW_HEIGHT


def dual_input_row(
    recipe: str,
    machine_entity: str,
    machine_count: int,
    y_offset: int,
    x_offset: int = 0,
    input_items: tuple[str, str] = ("", ""),
    output_item: str = "",
) -> tuple[list[PlacedEntity], int]:
    """Row for a recipe with 2 solid inputs.

    Layout per machine (4-tile horizontal pitch):
        y+0 : input belt 1 (EAST) — far belt
        y+1 : input belt 2 (EAST) — close belt
        y+2 : long-handed inserter (picks y+0) + inserter (picks y+1)
        y+3..y+5 : machine (3x3)
        y+6 : output inserter (SOUTH)
        y+7 : output belt (WEST — toward bus)
    """
    entities: list[PlacedEntity] = []
    ROW_HEIGHT = 8
    input1, input2 = input_items

    for i in range(machine_count):
        mx = x_offset + i * 4

        # Input belt 1 — far belt (3 tiles wide)
        for dx in range(3):
            entities.append(
                PlacedEntity(
                    name="transport-belt",
                    x=mx + dx,
                    y=y_offset,
                    direction=EntityDirection.EAST,
                    carries=input1,
                )
            )

        # Input belt 2 — close belt (3 tiles wide)
        for dx in range(3):
            entities.append(
                PlacedEntity(
                    name="transport-belt",
                    x=mx + dx,
                    y=y_offset + 1,
                    direction=EntityDirection.EAST,
                    carries=input2,
                )
            )

        # Long-handed inserter at x+0 (picks from far belt y+0, drops into machine y+3)
        entities.append(
            PlacedEntity(
                name="long-handed-inserter",
                x=mx,
                y=y_offset + 2,
                direction=EntityDirection.SOUTH,
                carries=input1,
            )
        )

        # Regular inserter at x+2 (picks from close belt y+1, drops into machine y+3)
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

        # Output belt (3 tiles wide, flowing WEST toward bus)
        for dx in range(3):
            entities.append(
                PlacedEntity(
                    name="transport-belt",
                    x=mx + dx,
                    y=y_offset + 7,
                    direction=EntityDirection.WEST,
                    carries=output_item,
                )
            )

    return entities, ROW_HEIGHT
