"""Inserter placement between belts and machines."""

from __future__ import annotations

from ..models import EntityDirection, PlacedEntity
from .graph import ProductionGraph
from .placer import machine_size

# Inserter faces the direction it DROPS toward.
# A SOUTH-facing inserter picks from the tile to its north and drops to the tile to its south.
_FACING: dict[tuple[int, int], EntityDirection] = {
    (0, 1): EntityDirection.SOUTH,  # machine is below → drop south
    (0, -1): EntityDirection.NORTH,  # machine is above → drop north
    (1, 0): EntityDirection.EAST,  # machine is right → drop east
    (-1, 0): EntityDirection.WEST,  # machine is left → drop west
}


def place_inserters(
    graph: ProductionGraph,
    positions: dict[int, tuple[int, int]],
    routed_tiles: set[tuple[int, int]],
) -> list[PlacedEntity]:
    """Place inserters between routed belts/pipes and machines.

    The router places belts 2 tiles from the machine. The inserter goes on
    the border tile (1 tile from machine), picking from the belt behind it
    and dropping into the machine in front.

    Layout: ... [belt @ 2 tiles] [inserter @ 1 tile] [machine]
    """
    entities: list[PlacedEntity] = []
    used_tiles: set[tuple[int, int]] = set()

    for node in graph.nodes:
        mx, my = positions[node.id]
        size = machine_size(node.spec.entity)

        # Check each border tile around the machine
        borders = [
            *((mx + dx, my - 1, 0, 1) for dx in range(size)),  # top border
            *((mx + dx, my + size, 0, -1) for dx in range(size)),  # bottom border
            *((mx - 1, my + dy, 1, 0) for dy in range(size)),  # left border
            *((mx + size, my + dy, -1, 0) for dy in range(size)),  # right border
        ]

        for bx, by, dx, dy in borders:
            if (bx, by) in used_tiles:
                continue
            if (bx, by) in routed_tiles:
                # Border tile is occupied by a belt — can't place inserter here
                continue

            # Check if there's a belt one tile further from the machine
            belt_x, belt_y = bx - dx, by - dy
            if (belt_x, belt_y) not in routed_tiles:
                continue

            # Belt is at (belt_x, belt_y), inserter goes at (bx, by)
            # Inserter faces toward the machine (drops into machine)
            facing = _FACING.get((dx, dy))
            if facing is None:
                continue

            entities.append(
                PlacedEntity(
                    name="inserter",
                    x=bx,
                    y=by,
                    direction=facing,
                )
            )
            used_tiles.add((bx, by))

    return entities
