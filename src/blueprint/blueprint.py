"""Thin draftsman wrapper: LayoutResult → Factorio blueprint string."""

from __future__ import annotations

from draftsman.blueprintable import Blueprint
from draftsman.constants import Direction
from draftsman.entity import new_entity

from ..models import LayoutResult, EntityDirection


_DIR_MAP = {
    EntityDirection.NORTH: Direction.NORTH,
    EntityDirection.EAST: Direction.EAST,
    EntityDirection.SOUTH: Direction.SOUTH,
    EntityDirection.WEST: Direction.WEST,
}


def build_blueprint(
    layout_result: LayoutResult,
    label: str = "Generated Factory",
) -> str:
    """Convert a LayoutResult into an importable Factorio blueprint string."""
    bp = Blueprint()
    bp.label = label
    bp.version = (2, 0)

    for ent in layout_result.entities:
        kwargs: dict = {"tile_position": (ent.x, ent.y)}

        # Only pass direction for entities that support it
        test = new_entity(ent.name)
        if hasattr(test, "direction"):
            kwargs["direction"] = _DIR_MAP.get(ent.direction, Direction.NORTH)

        # io_type for underground belts
        if ent.io_type is not None and hasattr(test, "io_type"):
            kwargs["io_type"] = ent.io_type

        added = bp.entities.append(ent.name, **kwargs)

        if ent.recipe is not None:
            added.recipe = ent.recipe

    return bp.to_string()
