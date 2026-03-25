"""Power pole grid placement — avoids occupied tiles."""

from __future__ import annotations

from ..models import PlacedEntity

# Medium electric pole: supply area radius ~3.5 tiles from centre,
# so a 7-tile grid ensures full coverage.
POLE_SPACING = 7


def place_poles(
    width: int,
    height: int,
    occupied: set[tuple[int, int]] | None = None,
) -> list[PlacedEntity]:
    """Place medium electric poles in a grid, skipping occupied tiles."""
    if occupied is None:
        occupied = set()

    entities: list[PlacedEntity] = []

    y = -1  # offset to avoid row 0 (usually belts)
    while y < height + POLE_SPACING:
        x = -1
        while x < width + POLE_SPACING:
            if (x, y) not in occupied:
                entities.append(
                    PlacedEntity(
                        name="medium-electric-pole",
                        x=x,
                        y=y,
                    )
                )
            x += POLE_SPACING
        y += POLE_SPACING

    return entities
