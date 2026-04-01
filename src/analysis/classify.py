"""Step 1: Parse a blueprint string and classify entities."""

from __future__ import annotations

import logging
from dataclasses import dataclass, field

from draftsman.blueprintable import get_blueprintable_from_string

from ..models import EntityDirection
from ..routing.common import _MACHINE_SIZE
from ..solver.recipe_db import get_recipe
from .models import AnalyzedMachine, TransportSegment

logger = logging.getLogger(__name__)

_SURFACE_BELT_ENTITIES = {"transport-belt", "fast-transport-belt", "express-transport-belt"}
_UG_BELT_ENTITIES = {"underground-belt", "fast-underground-belt", "express-underground-belt"}
_BELT_ENTITIES = _SURFACE_BELT_ENTITIES | _UG_BELT_ENTITIES
_PIPE_ENTITIES = {"pipe", "pipe-to-ground"}
_INSERTER_ENTITIES = {"inserter", "long-handed-inserter", "fast-inserter", "stack-inserter"}

# Map draftsman direction int to our EntityDirection
_DIR_MAP = {0: EntityDirection.NORTH, 4: EntityDirection.EAST, 8: EntityDirection.SOUTH, 12: EntityDirection.WEST}


@dataclass
class ClassifiedEntities:
    """Entities sorted by type after parsing."""

    machines: list[AnalyzedMachine] = field(default_factory=list)
    belt_segments: list[TransportSegment] = field(default_factory=list)
    pipe_segments: list[TransportSegment] = field(default_factory=list)
    inserters: list[_RawInserter] = field(default_factory=list)
    unhandled: list[str] = field(default_factory=list)


@dataclass
class _RawInserter:
    """Raw inserter data before resolution."""

    position: tuple[int, int]
    name: str
    direction: EntityDirection


def classify_entities(bp_string: str) -> ClassifiedEntities:
    """Parse a blueprint string and classify all entities."""
    bp = get_blueprintable_from_string(bp_string)
    result = ClassifiedEntities()

    machine_id = 0
    for e in bp.entities:
        name = e.name
        pos = (int(e.tile_position.x), int(e.tile_position.y))
        direction = _DIR_MAP.get(getattr(e, "direction", 0), EntityDirection.NORTH)

        if name in _MACHINE_SIZE:
            inputs, outputs = _recipe_items(getattr(e, "recipe", None))
            result.machines.append(
                AnalyzedMachine(
                    id=machine_id,
                    name=name,
                    recipe=getattr(e, "recipe", None),
                    position=pos,
                    size=_MACHINE_SIZE[name],
                    inputs=inputs,
                    outputs=outputs,
                )
            )
            machine_id += 1

        elif name in _BELT_ENTITIES:
            is_ug = name in _UG_BELT_ENTITIES
            io_type = getattr(e, "io_type", None)
            result.belt_segments.append(
                TransportSegment(
                    position=pos,
                    name=name,
                    direction=direction,
                    is_underground=is_ug,
                    io_type=io_type,
                )
            )

        elif name in _PIPE_ENTITIES:
            is_ug = name == "pipe-to-ground"
            io_type = getattr(e, "io_type", None)
            result.pipe_segments.append(
                TransportSegment(
                    position=pos,
                    name=name,
                    direction=direction,
                    is_underground=is_ug,
                    io_type=io_type,
                )
            )

        elif name in _INSERTER_ENTITIES:
            result.inserters.append(_RawInserter(position=pos, name=name, direction=direction))

        else:
            result.unhandled.append(name)

    return result


def _recipe_items(recipe_name: str | None) -> tuple[list[str], list[str]]:
    """Get input and output item names from a recipe."""
    if recipe_name is None:
        return [], []
    try:
        recipe = get_recipe(recipe_name)
    except KeyError:
        logger.warning("Unknown recipe: %s", recipe_name)
        return [], []
    inputs = [ing.name for ing in recipe.ingredients]
    outputs = [prod.name for prod in recipe.products]
    return inputs, outputs
