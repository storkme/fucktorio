"""Differentiable loss function terms for ML-based machine placement.

Each term returns (value, gradient) where gradient has shape (2N,)
matching the flat position vector [x0, y0, x1, y1, ...].
"""

from __future__ import annotations

import numpy as np

# Smoothing epsilon for differentiable |x| approximation
_EPS = 0.01

# Default loss weights
DEFAULT_WEIGHTS: dict[str, float] = {
    "overlap": 100.0,
    "edge": 1.0,
    "compact": 0.1,
    "align": 0.05,
}


def _smooth_abs(x: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    """Smooth approximation of |x| and its gradient."""
    val = np.sqrt(x * x + _EPS)
    grad = x / val
    return val, grad


def overlap_penalty(
    positions: np.ndarray,
    sizes: np.ndarray,
    min_gap: float = 2.0,
) -> tuple[float, np.ndarray]:
    """Penalize overlapping machine footprints.

    Args:
        positions: flat (2N,) array [x0, y0, x1, y1, ...]
        sizes: (N,) array of machine footprint sizes (square)
        min_gap: minimum gap between machine edges (for inserter + belt)

    Returns:
        (loss_value, gradient) where gradient has shape (2N,).
    """
    n = len(sizes)
    grad = np.zeros_like(positions)
    total = 0.0

    if n < 2:
        return total, grad

    xs = positions[0::2]
    ys = positions[1::2]

    for i in range(n):
        for j in range(i + 1, n):
            # Required separation = half-sizes + min_gap
            req_x = (sizes[i] + sizes[j]) / 2.0 + min_gap
            req_y = (sizes[i] + sizes[j]) / 2.0 + min_gap

            # Center-to-center distance
            dx = xs[i] - xs[j]
            dy = ys[i] - ys[j]

            # Overlap along each axis (positive = overlapping)
            overlap_x = req_x - np.abs(dx)
            overlap_y = req_y - np.abs(dy)

            if overlap_x > 0 and overlap_y > 0:
                # Loss = overlap_x * overlap_y (area of intersection)
                total += overlap_x * overlap_y

                # Gradient: d/d(xi) [overlap_x * overlap_y]
                # overlap_x = req_x - |dx|, d(overlap_x)/d(xi) = -sign(dx)
                sign_dx = 1.0 if dx >= 0 else -1.0
                sign_dy = 1.0 if dy >= 0 else -1.0

                # d(loss)/d(xi) = d(overlap_x)/d(xi) * overlap_y
                #               = -sign(dx) * overlap_y
                grad_xi = -sign_dx * overlap_y
                grad_yi = -sign_dy * overlap_x

                grad[2 * i] += grad_xi
                grad[2 * i + 1] += grad_yi
                grad[2 * j] -= grad_xi
                grad[2 * j + 1] -= grad_yi

    return total, grad


def edge_distance(
    positions: np.ndarray,
    edges: list[tuple[int, int, float]],
) -> tuple[float, np.ndarray]:
    """Penalize distance between connected machines, weighted by flow rate.

    Args:
        positions: flat (2N,) array
        edges: list of (from_idx, to_idx, rate) tuples (node indices, not position indices)

    Returns:
        (loss_value, gradient)
    """
    grad = np.zeros_like(positions)
    total = 0.0

    xs = positions[0::2]
    ys = positions[1::2]

    for i, j, rate in edges:
        dx = xs[i] - xs[j]
        dy = ys[i] - ys[j]

        # Smooth Manhattan distance: sqrt(dx^2 + eps) + sqrt(dy^2 + eps)
        dist_x, grad_x = _smooth_abs(dx)
        dist_y, grad_y = _smooth_abs(dy)

        total += rate * (dist_x + dist_y)

        grad[2 * i] += rate * grad_x
        grad[2 * i + 1] += rate * grad_y
        grad[2 * j] -= rate * grad_x
        grad[2 * j + 1] -= rate * grad_y

    return total, grad


def compactness(positions: np.ndarray, sizes: np.ndarray) -> tuple[float, np.ndarray]:
    """Penalize bounding box area.

    Uses log-sum-exp for smooth differentiable min/max.

    Args:
        positions: flat (2N,) array
        sizes: (N,) array of machine sizes

    Returns:
        (loss_value, gradient)
    """
    n = len(sizes)
    grad = np.zeros_like(positions)

    if n < 2:
        return 0.0, grad

    xs = positions[0::2]
    ys = positions[1::2]

    # Account for machine size: right/bottom edges
    x_rights = xs + sizes
    y_bottoms = ys + sizes

    # Smooth max/min via log-sum-exp
    # smooth_max(v) ≈ (1/alpha) * log(sum(exp(alpha * v)))
    alpha = 5.0

    def _lse_max(v: np.ndarray) -> tuple[float, np.ndarray]:
        shifted = alpha * (v - np.max(v))  # numerical stability
        exp_v = np.exp(shifted)
        sum_exp = np.sum(exp_v)
        val = np.max(v) + np.log(sum_exp) / alpha
        g = exp_v / sum_exp
        return val, g

    def _lse_min(v: np.ndarray) -> tuple[float, np.ndarray]:
        val, g = _lse_max(-v)
        return -val, g

    max_x, g_max_x = _lse_max(x_rights)
    min_x, g_min_x = _lse_min(xs)
    max_y, g_max_y = _lse_max(y_bottoms)
    min_y, g_min_y = _lse_min(ys)

    width = max_x - min_x
    height = max_y - min_y
    total = width * height

    # d(width * height)/d(xi) = height * d(width)/d(xi) + width * d(height)/d(xi)
    # d(width)/d(xi) = d(max_x)/d(x_right_i) * d(x_right_i)/d(xi) - d(min_x)/d(xi)
    # x_right_i = xi + size_i, so d(x_right_i)/d(xi) = 1
    for k in range(n):
        grad[2 * k] += height * (g_max_x[k] - g_min_x[k])
        grad[2 * k + 1] += width * (g_max_y[k] - g_min_y[k])

    return total, grad


def alignment(
    positions: np.ndarray,
    same_recipe_groups: list[list[int]],
) -> tuple[float, np.ndarray]:
    """Encourage machines of the same recipe to align on rows or columns.

    For each pair of same-recipe machines, penalize min(|dy|, |dx|) —
    zero when perfectly aligned on either axis.

    Args:
        positions: flat (2N,) array
        same_recipe_groups: list of groups, each group is a list of node indices

    Returns:
        (loss_value, gradient)
    """
    grad = np.zeros_like(positions)
    total = 0.0

    xs = positions[0::2]
    ys = positions[1::2]

    for group in same_recipe_groups:
        for a_idx in range(len(group)):
            for b_idx in range(a_idx + 1, len(group)):
                i, j = group[a_idx], group[b_idx]
                dx = xs[i] - xs[j]
                dy = ys[i] - ys[j]

                abs_dx, grad_dx = _smooth_abs(dx)
                abs_dy, grad_dy = _smooth_abs(dy)

                # Smooth min via negative log-sum-exp of negatives
                # smooth_min(a, b) = -log(exp(-alpha*a) + exp(-alpha*b)) / alpha
                alpha = 5.0
                shifted_dx = -alpha * abs_dx
                shifted_dy = -alpha * abs_dy
                max_shift = max(shifted_dx, shifted_dy)
                exp_dx = np.exp(shifted_dx - max_shift)
                exp_dy = np.exp(shifted_dy - max_shift)
                sum_exp = exp_dx + exp_dy
                min_val = -max_shift / alpha - np.log(sum_exp) / alpha

                total += min_val

                # Gradients: d(smooth_min)/d(abs_dx) = exp(-alpha*abs_dx) / sum_exp (softmax weight)
                w_dx = exp_dx / sum_exp
                w_dy = exp_dy / sum_exp

                grad[2 * i] += w_dx * grad_dx
                grad[2 * i + 1] += w_dy * grad_dy
                grad[2 * j] -= w_dx * grad_dx
                grad[2 * j + 1] -= w_dy * grad_dy

    return total, grad


def total_loss(
    positions: np.ndarray,
    sizes: np.ndarray,
    edges: list[tuple[int, int, float]],
    same_recipe_groups: list[list[int]],
    weights: dict[str, float] | None = None,
    min_gap: float = 2.0,
) -> tuple[float, np.ndarray]:
    """Compute weighted sum of all loss terms.

    Returns:
        (total_loss, gradient) suitable for scipy.optimize.minimize with jac=True.
    """
    w = {**DEFAULT_WEIGHTS, **(weights or {})}

    loss = 0.0
    grad = np.zeros_like(positions)

    v, g = overlap_penalty(positions, sizes, min_gap=min_gap)
    loss += w["overlap"] * v
    grad += w["overlap"] * g

    v, g = edge_distance(positions, edges)
    loss += w["edge"] * v
    grad += w["edge"] * g

    v, g = compactness(positions, sizes)
    loss += w["compact"] * v
    grad += w["compact"] * g

    if same_recipe_groups:
        v, g = alignment(positions, same_recipe_groups)
        loss += w["align"] * v
        grad += w["align"] * g

    return loss, grad
