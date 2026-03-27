"""Inserter placement between belts and machines."""

from __future__ import annotations

from ..models import EntityDirection, PlacedEntity
from .graph import ProductionGraph
from .placer import machine_size

# Direction from inserter to machine → inserter facing direction
# Inserter faces the direction it DROPS toward
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

    For each machine, finds tiles that are:
    - Adjacent to the machine border
    - Occupied by a routed belt (in routed_tiles)
    - Have a free tile for the inserter between belt and machine

    Places an inserter at the border tile between belt and machine.
    """
    entities: list[PlacedEntity] = []
    used_tiles: set[tuple[int, int]] = set()

    for node in graph.nodes:
        mx, my = positions[node.id]
        size = machine_size(node.spec.entity)
        machine_tiles = {(mx + dx, my + dy) for dx in range(size) for dy in range(size)}

        # Check each border tile
        _try_place_inserters_for_machine(mx, my, size, machine_tiles, routed_tiles, used_tiles, entities)

    return entities


def _try_place_inserters_for_machine(
    mx: int,
    my: int,
    size: int,
    machine_tiles: set[tuple[int, int]],
    routed_tiles: set[tuple[int, int]],
    used_tiles: set[tuple[int, int]],
    entities: list[PlacedEntity],
) -> None:
    """Try to place inserters around a single machine."""
    # Border tiles with direction toward machine
    borders = [
        # (border_x, border_y, dx_to_machine, dy_to_machine)
        *((mx + dx, my - 1, 0, 1) for dx in range(size)),  # top
        *((mx + dx, my + size, 0, -1) for dx in range(size)),  # bottom
        *((mx - 1, my + dy, 1, 0) for dy in range(size)),  # left
        *((mx + size, my + dy, -1, 0) for dy in range(size)),  # right
    ]

    for bx, by, dx, dy in borders:
        if (bx, by) in used_tiles:
            continue

        # Check if this border tile has a routed belt/pipe
        if (bx, by) in routed_tiles:
            # The belt IS on the border tile. Place inserter here —
            # it picks from the belt at (bx, by) and drops into machine.
            # But we can't place an inserter on top of a belt.
            # Instead, check the tile one step further from the machine.
            # If THAT tile has the belt, inserter goes on the border tile.
            pass

        # Check tile one step away from machine (belt position)
        belt_x, belt_y = bx - dx, by - dy  # one tile further from machine
        if (belt_x, belt_y) in routed_tiles and (bx, by) not in routed_tiles:
            # Belt at (belt_x, belt_y), inserter at (bx, by), machine adjacent
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
            continue

        # Direct adjacency: belt on the border tile itself
        if (bx, by) in routed_tiles:
            # Belt right next to machine — inserter picks directly from adjacent belt
            # In Factorio, inserter at (bx, by) facing (dx, dy) picks from behind
            # and drops forward. So inserter should be between belt and machine.
            # But belt IS at the border... the inserter needs its own tile.
            # Try: inserter at border, picks from belt behind, drops into machine
            facing = _FACING.get((dx, dy))
            if facing is not None and (bx, by) not in used_tiles:
                # This means the inserter and belt share a tile, which isn't valid.
                # Skip — the router should have left a gap.
                pass
