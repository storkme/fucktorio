"""ML-based machine placement using scipy optimization.

Optimizes machine positions by minimizing a loss function that encodes
overlap avoidance, connectivity distance, compactness, and alignment.
Then snaps to integer grid and resolves any remaining collisions.
"""

from __future__ import annotations

import logging

import numpy as np
from scipy.optimize import minimize

from ..routing.common import machine_size
from ..routing.graph import ProductionGraph
from ..spaghetti.placer import place_machines as grid_place_machines
from .loss import total_loss

log = logging.getLogger(__name__)


def ml_place_machines(
    graph: ProductionGraph,
    weights: dict[str, float] | None = None,
    min_gap: float = 2.0,
    seed: int | None = None,
) -> dict[int, tuple[int, int]]:
    """Place machines using gradient-based optimization.

    1. Start from a simple grid layout (feasible initial positions)
    2. Optimize with L-BFGS-B to minimize the combined loss
    3. Snap to integer grid and resolve collisions

    Args:
        graph: Production graph with machine nodes and flow edges.
        weights: Loss term weights (overlap, edge, compact, align).
        min_gap: Minimum gap between machine edges.
        seed: Random seed for reproducibility.

    Returns:
        dict mapping node_id → (x, y) integer tile position.
    """
    if not graph.nodes:
        return {}

    if seed is not None:
        np.random.seed(seed)

    # Build optimization data structures
    n = len(graph.nodes)
    node_ids = [node.id for node in graph.nodes]
    id_to_idx = {nid: idx for idx, nid in enumerate(node_ids)}

    sizes = np.array([machine_size(node.spec.entity) for node in graph.nodes], dtype=float)

    # Build edge list: (from_idx, to_idx, rate) for internal edges only
    edges: list[tuple[int, int, float]] = []
    for edge in graph.edges:
        if edge.from_node is not None and edge.to_node is not None:
            i = id_to_idx.get(edge.from_node)
            j = id_to_idx.get(edge.to_node)
            if i is not None and j is not None:
                edges.append((i, j, edge.rate))

    # Build same-recipe groups for alignment term
    recipe_groups: dict[str, list[int]] = {}
    for node in graph.nodes:
        recipe_groups.setdefault(node.spec.recipe, []).append(id_to_idx[node.id])
    same_recipe_groups = [idxs for idxs in recipe_groups.values() if len(idxs) > 1]

    # Initial positions from simple grid layout
    initial = grid_place_machines(graph, spacing=4)
    x0 = np.zeros(2 * n, dtype=float)
    for node in graph.nodes:
        idx = id_to_idx[node.id]
        px, py = initial[node.id]
        x0[2 * idx] = float(px)
        x0[2 * idx + 1] = float(py)

    # Bounds: keep everything in a reasonable area
    max_extent = float(np.sum(sizes) + n * min_gap + 20)
    bounds = [(0.0, max_extent)] * (2 * n)

    # Objective function for scipy
    def objective(pos: np.ndarray) -> tuple[float, np.ndarray]:
        return total_loss(
            pos,
            sizes,
            edges,
            same_recipe_groups,
            weights=weights,
            min_gap=min_gap,
        )

    # Optimize
    result = minimize(
        objective,
        x0,
        method="L-BFGS-B",
        jac=True,
        bounds=bounds,
        options={"maxiter": 500, "ftol": 1e-8},
    )

    log.info(
        "L-BFGS-B converged=%s, iterations=%d, loss=%.2f",
        result.success,
        result.nit,
        result.fun,
    )

    # Snap to integer grid
    positions = _snap_to_grid(result.x, sizes, min_gap)

    # Convert back to dict
    return {node_ids[i]: (int(positions[2 * i]), int(positions[2 * i + 1])) for i in range(n)}


def _snap_to_grid(
    positions: np.ndarray,
    sizes: np.ndarray,
    min_gap: float,
) -> np.ndarray:
    """Round positions to integers and resolve overlaps greedily.

    Iteratively shifts colliding machines apart until no overlaps remain
    (or a maximum iteration count is reached).
    """
    n = len(sizes)
    pos = np.round(positions).astype(float)

    # Ensure non-negative
    pos[pos < 0] = 0

    max_iterations = n * n * 4
    for _ in range(max_iterations):
        found_overlap = False
        for i in range(n):
            for j in range(i + 1, n):
                xi, yi = pos[2 * i], pos[2 * i + 1]
                xj, yj = pos[2 * j], pos[2 * j + 1]

                req = (sizes[i] + sizes[j]) / 2.0 + min_gap
                dx = xi - xj
                dy = yi - yj

                overlap_x = req - abs(dx)
                overlap_y = req - abs(dy)

                if overlap_x > 0 and overlap_y > 0:
                    found_overlap = True
                    # Push apart along the axis with less overlap
                    if overlap_x <= overlap_y:
                        shift = max(1.0, overlap_x / 2.0)
                        if dx >= 0:
                            pos[2 * i] += shift
                            pos[2 * j] -= shift
                        else:
                            pos[2 * i] -= shift
                            pos[2 * j] += shift
                    else:
                        shift = max(1.0, overlap_y / 2.0)
                        if dy >= 0:
                            pos[2 * i + 1] += shift
                            pos[2 * j + 1] -= shift
                        else:
                            pos[2 * i + 1] -= shift
                            pos[2 * j + 1] += shift

                    # Re-round after shift
                    pos[2 * i] = round(pos[2 * i])
                    pos[2 * i + 1] = round(pos[2 * i + 1])
                    pos[2 * j] = round(pos[2 * j])
                    pos[2 * j + 1] = round(pos[2 * j + 1])

        if not found_overlap:
            break

    # Final ensure non-negative
    min_x = min(pos[0::2])
    min_y = min(pos[1::2])
    if min_x < 0:
        pos[0::2] -= min_x
    if min_y < 0:
        pos[1::2] -= min_y

    return pos
