"""Power pole placement — greedy near-machine or grid fallback."""

from __future__ import annotations

import logging

from ..models import PlacedEntity

logger = logging.getLogger(__name__)

# Medium electric pole: supply area radius ~3.5 tiles from centre,
# so a 7-tile grid ensures full coverage.
POLE_SPACING = 7
POLE_RANGE = 3  # 7x7 area = 3 tiles each direction from center


def place_poles(
    width: int,
    height: int,
    occupied: set[tuple[int, int]] | None = None,
    machine_centers: list[tuple[int, int]] | None = None,
) -> list[PlacedEntity]:
    """Place medium electric poles.

    When *machine_centers* is provided, uses a greedy algorithm that places
    poles near machines that need them.  Otherwise falls back to a simple
    7x7 grid pattern.
    """
    if occupied is None:
        occupied = set()

    if machine_centers is not None:
        return _place_poles_greedy(occupied, machine_centers)
    return _place_poles_grid(width, height, occupied)


def _place_poles_grid(
    width: int,
    height: int,
    occupied: set[tuple[int, int]],
) -> list[PlacedEntity]:
    """Legacy grid-based pole placement."""
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


def _in_pole_range(ax: int, ay: int, bx: int, by: int) -> bool:
    """True if (ax,ay) is within Chebyshev distance POLE_RANGE of (bx,by)."""
    return abs(ax - bx) <= POLE_RANGE and abs(ay - by) <= POLE_RANGE


def _place_poles_greedy(
    occupied: set[tuple[int, int]],
    machine_centers: list[tuple[int, int]],
) -> list[PlacedEntity]:
    """Greedy pole placement: each pole covers as many unpowered machines as possible."""
    if not machine_centers:
        return []

    occupied = set(occupied)  # copy so we can mutate
    unpowered = set(range(len(machine_centers)))
    entities: list[PlacedEntity] = []

    while unpowered:
        best_pos: tuple[int, int] | None = None
        best_score = 0
        best_dist = float("inf")

        centroid_x = sum(machine_centers[i][0] for i in unpowered) / len(unpowered)
        centroid_y = sum(machine_centers[i][1] for i in unpowered) / len(unpowered)

        candidates: set[tuple[int, int]] = set()
        for i in unpowered:
            mx, my = machine_centers[i]
            for dx in range(-POLE_RANGE, POLE_RANGE + 1):
                for dy in range(-POLE_RANGE, POLE_RANGE + 1):
                    pos = (mx + dx, my + dy)
                    if pos not in occupied:
                        candidates.add(pos)

        if not candidates:
            for i in unpowered:
                mx, my = machine_centers[i]
                logger.warning(
                    "No valid pole position for machine center (%d, %d)",
                    mx,
                    my,
                )
            break

        for px, py in candidates:
            score = sum(1 for i in unpowered if _in_pole_range(machine_centers[i][0], machine_centers[i][1], px, py))
            dist = abs(px - centroid_x) + abs(py - centroid_y)
            if score > best_score or (score == best_score and dist < best_dist):
                best_score = score
                best_dist = dist
                best_pos = (px, py)

        if best_pos is None:
            break

        px, py = best_pos
        entities.append(PlacedEntity(name="medium-electric-pole", x=px, y=py))
        occupied.add(best_pos)

        unpowered -= {i for i in unpowered if _in_pole_range(machine_centers[i][0], machine_centers[i][1], px, py)}

    return entities
