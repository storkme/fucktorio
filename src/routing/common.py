"""Shared constants and utility functions for routing."""

from __future__ import annotations

from ..models import EntityDirection

# Machine footprint sizes (tiles)
_MACHINE_SIZE: dict[str, int] = {
    "assembling-machine-1": 3,
    "assembling-machine-2": 3,
    "assembling-machine-3": 3,
    "chemical-plant": 3,
    "oil-refinery": 5,
}
_DEFAULT_SIZE = 3


def machine_size(entity: str) -> int:
    return _MACHINE_SIZE.get(entity, _DEFAULT_SIZE)


def machine_tiles(x: int, y: int, size: int) -> set[tuple[int, int]]:
    """All tiles occupied by a machine at (x, y) with given size."""
    return {(x + dx, y + dy) for dx in range(size) for dy in range(size)}


# Belt throughput tiers (items per second)
_BELT_TIERS = [
    ("transport-belt", 15.0),
    ("fast-transport-belt", 30.0),
    ("express-transport-belt", 45.0),
]

# Underground belt max reach (tiles between entry and exit, exclusive)
_UG_MAX_REACH = {
    "transport-belt": 4,
    "fast-transport-belt": 6,
    "express-transport-belt": 8,
}
_UG_COST_MULTIPLIER = 3  # underground costs 3x per tile vs surface
_UG_PIPE_REACH = 10  # pipe-to-ground max reach (tiles between entry and exit)


def belt_entity_for_rate(rate: float) -> str:
    """Pick the cheapest belt tier that can handle the given rate."""
    for name, throughput in _BELT_TIERS:
        if rate <= throughput:
            return name
    return _BELT_TIERS[-1][0]  # express if rate exceeds all


# Direction vectors: (dx, dy) for each cardinal direction
DIRECTIONS = [(0, -1), (1, 0), (0, 1), (-1, 0)]  # N, E, S, W

DIR_MAP = {
    (0, -1): EntityDirection.NORTH,
    (1, 0): EntityDirection.EAST,
    (0, 1): EntityDirection.SOUTH,
    (-1, 0): EntityDirection.WEST,
}

# Inverse: EntityDirection -> (dx, dy)
DIR_VEC = {
    EntityDirection.NORTH: (0, -1),
    EntityDirection.EAST: (1, 0),
    EntityDirection.SOUTH: (0, 1),
    EntityDirection.WEST: (-1, 0),
}
