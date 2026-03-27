"""Machine placement on the 2D tile grid."""

from __future__ import annotations

import math

from .graph import ProductionGraph

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


def place_machines(graph: ProductionGraph, spacing: int = 5) -> dict[int, tuple[int, int]]:
    """Place machine nodes on the grid using simple grid layout.

    Arranges machines in rows with *spacing* tiles between them
    (measured from edge of one machine to edge of next). This leaves
    room for inserters, belts, and routing.

    Returns a dict mapping node_id → (x, y) tile position (top-left corner).
    """
    if not graph.nodes:
        return {}

    positions: dict[int, tuple[int, int]] = {}

    # Determine grid dimensions
    n = len(graph.nodes)
    cols = math.ceil(math.sqrt(n))

    x = 0
    y = 0
    col = 0
    row_height = 0

    for node in graph.nodes:
        size = machine_size(node.spec.entity)
        positions[node.id] = (x, y)

        row_height = max(row_height, size)
        col += 1

        if col >= cols:
            # Next row
            col = 0
            x = 0
            y += row_height + spacing
            row_height = 0
        else:
            x += size + spacing

    return positions
