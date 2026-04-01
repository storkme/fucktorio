"""Step 1: Parse a blueprint string and classify entities."""

from __future__ import annotations

import logging
from dataclasses import dataclass, field

from draftsman.blueprintable import get_blueprintable_from_string
from draftsman.utils import string_to_JSON

from ..models import EntityDirection
from ..routing.common import _MACHINE_SIZE
from ..solver.recipe_db import get_recipe
from .models import AnalyzedMachine, TransportSegment

logger = logging.getLogger(__name__)

_SURFACE_BELT_ENTITIES = {"transport-belt", "fast-transport-belt", "express-transport-belt"}
_UG_BELT_ENTITIES = {"underground-belt", "fast-underground-belt", "express-underground-belt"}
_SPLITTER_ENTITIES = {"splitter", "fast-splitter", "express-splitter"}
_BELT_ENTITIES = _SURFACE_BELT_ENTITIES | _UG_BELT_ENTITIES | _SPLITTER_ENTITIES
_PIPE_ENTITIES = {"pipe", "pipe-to-ground"}
_INSERTER_ENTITIES = {"inserter", "long-handed-inserter", "fast-inserter", "stack-inserter"}

# Map draftsman direction int to our EntityDirection (modern: 0,4,8,12)
_DIR_MAP = {0: EntityDirection.NORTH, 4: EntityDirection.EAST, 8: EntityDirection.SOUTH, 12: EntityDirection.WEST}

# Old Factorio versions (pre-0.17) use direction values 0-7
_OLD_DIR_MAP = {
    0: EntityDirection.NORTH,
    1: EntityDirection.NORTH,  # NE → approximate
    2: EntityDirection.EAST,
    3: EntityDirection.EAST,  # SE → approximate
    4: EntityDirection.SOUTH,
    5: EntityDirection.SOUTH,  # SW → approximate
    6: EntityDirection.WEST,
    7: EntityDirection.WEST,  # NW → approximate
}


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
    """Parse a blueprint string and classify all entities.

    Tries draftsman first; falls back to raw JSON parsing for old blueprint
    versions that draftsman doesn't support.
    """
    try:
        return _classify_from_draftsman(bp_string)
    except (ValueError, Exception) as exc:
        logger.info("Draftsman parse failed (%s), falling back to raw JSON", exc)
        return _classify_from_json(bp_string)


def _classify_from_draftsman(bp_string: str) -> ClassifiedEntities:
    """Parse via draftsman (modern blueprints)."""
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
            for tile in _belt_tiles(name, pos, direction):
                result.belt_segments.append(
                    TransportSegment(
                        position=tile,
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


def _classify_from_json(bp_string: str) -> ClassifiedEntities:
    """Parse from raw JSON (fallback for old blueprint versions).

    Old Factorio versions use direction values 0-7 and positions as
    {x, y} dicts with center-based coordinates. Entity positions in old
    blueprints are center-based, so we convert to top-left tile_position
    by subtracting half the entity size.
    """
    data = string_to_JSON(bp_string)
    bp_data = data.get("blueprint", data)
    entities = bp_data.get("entities", [])

    result = ClassifiedEntities()
    machine_id = 0

    for e in entities:
        name = e.get("name", "")
        raw_pos = e.get("position", {})
        raw_dir = e.get("direction", 0)
        recipe = e.get("recipe")

        # Old blueprints use center-based positions; convert to tile_position (top-left)
        cx, cy = raw_pos.get("x", 0), raw_pos.get("y", 0)

        # Old format uses direction values 0-7 (not 0,4,8,12)
        direction = _OLD_DIR_MAP.get(raw_dir, EntityDirection.NORTH)

        if name in _MACHINE_SIZE:
            size = _MACHINE_SIZE[name]
            # Center to top-left: subtract floor(size/2)
            half = size // 2
            pos = (int(cx) - half, int(cy) - half)
            inputs, outputs = _recipe_items(recipe)
            result.machines.append(
                AnalyzedMachine(
                    id=machine_id,
                    name=name,
                    recipe=recipe,
                    position=pos,
                    size=size,
                    inputs=inputs,
                    outputs=outputs,
                )
            )
            machine_id += 1

        elif name in _BELT_ENTITIES:
            pos = (int(cx), int(cy))
            is_ug = name in _UG_BELT_ENTITIES
            io_type = e.get("type")  # old format uses "type" for input/output
            for tile in _belt_tiles(name, pos, direction):
                result.belt_segments.append(
                    TransportSegment(
                        position=tile,
                        name=name,
                        direction=direction,
                        is_underground=is_ug,
                        io_type=io_type,
                    )
                )

        elif name in _PIPE_ENTITIES:
            pos = (int(cx), int(cy))
            is_ug = name == "pipe-to-ground"
            io_type = e.get("type")
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
            pos = (int(cx), int(cy))
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


def _belt_tiles(
    name: str, pos: tuple[int, int], direction: EntityDirection
) -> list[tuple[int, int]]:
    """Return all tiles occupied by a belt-type entity.

    Regular belts and underground belts are 1x1.
    Splitters are 1x2 (perpendicular to their direction).
    """
    if name not in _SPLITTER_ENTITIES:
        return [pos]

    x, y = pos
    # Splitters extend perpendicular to their direction
    if direction in (EntityDirection.NORTH, EntityDirection.SOUTH):
        return [(x, y), (x + 1, y)]  # 2 wide
    else:
        return [(x, y), (x, y + 1)]  # 2 tall
