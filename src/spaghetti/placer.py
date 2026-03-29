"""Machine placement on the 2D tile grid."""

from __future__ import annotations

import math
from collections import defaultdict, deque

from ..routing.common import machine_size as machine_size
from .graph import ProductionGraph


def place_machines(graph: ProductionGraph, spacing: int = 4) -> dict[int, tuple[int, int]]:
    """Place machine nodes on the grid using simple grid layout.

    Arranges machines in rows with *spacing* tiles between them
    (measured from edge of one machine to edge of next). This leaves
    room for inserters, belts, and routing.

    Returns a dict mapping node_id -> (x, y) tile position (top-left corner).
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


def _machine_footprint(x: int, y: int, size: int) -> set[tuple[int, int]]:
    """All tiles occupied by a machine at (x, y) with given size."""
    return {(x + dx, y + dy) for dx in range(size) for dy in range(size)}


def _dependency_order(graph: ProductionGraph) -> list[int]:
    """Return node IDs in placement order (upstream-first topological sort).

    Machines with only external inputs (leaf producers) come first,
    then their downstream consumers, etc. Within each topological level,
    nodes with more connections are placed first (they are the "core"
    and benefit most from central positioning).
    """
    if not graph.nodes:
        return []

    node_ids = {n.id for n in graph.nodes}

    # Build adjacency: upstream → downstream (from_node → to_node)
    # in_degree counts how many internal upstream edges each node has
    in_degree: dict[int, int] = {nid: 0 for nid in node_ids}
    downstream: dict[int, list[int]] = {nid: [] for nid in node_ids}

    for edge in graph.edges:
        if edge.from_node is not None and edge.to_node is not None:
            if edge.from_node in node_ids and edge.to_node in node_ids:
                in_degree[edge.to_node] += 1
                downstream[edge.from_node].append(edge.to_node)

    # Count total connections per node (for tie-breaking)
    connection_count: dict[int, int] = {nid: 0 for nid in node_ids}
    for edge in graph.edges:
        if edge.from_node is not None and edge.from_node in node_ids:
            connection_count[edge.from_node] += 1
        if edge.to_node is not None and edge.to_node in node_ids:
            connection_count[edge.to_node] += 1

    # Kahn's algorithm with tie-breaking by connection count (most first)
    queue: list[int] = sorted(
        [nid for nid, deg in in_degree.items() if deg == 0],
        key=lambda nid: -connection_count[nid],
    )
    order: list[int] = []

    while queue:
        # Sort by connection count descending for stable tie-breaking
        queue.sort(key=lambda nid: -connection_count[nid])
        nid = queue.pop(0)
        order.append(nid)
        for child in downstream[nid]:
            in_degree[child] -= 1
            if in_degree[child] == 0:
                queue.append(child)

    # Any nodes not reached (cycles or disconnected) get appended at the end
    remaining = [nid for nid in node_ids if nid not in set(order)]
    remaining.sort(key=lambda nid: -connection_count[nid])
    order.extend(remaining)

    return order


def _connected_placed(
    node_id: int, graph: ProductionGraph, placed: set[int]
) -> list[int]:
    """Return IDs of already-placed machines connected to node_id.

    Includes both direct graph edges AND machines that share external
    items (same recipe siblings), since they need to be routed together.
    """
    connected: list[int] = []

    # Direct graph edges
    for edge in graph.edges:
        if edge.from_node == node_id and edge.to_node in placed:
            connected.append(edge.to_node)
        elif edge.to_node == node_id and edge.from_node in placed:
            connected.append(edge.from_node)

    # Same-recipe siblings: machines making the same recipe share external
    # input/output trunks and should be placed near each other.
    # Only add the most recently placed sibling to avoid over-clustering.
    if not connected:
        node = next(n for n in graph.nodes if n.id == node_id)
        best_sibling = None
        for other in graph.nodes:
            if other.id != node_id and other.id in placed and other.spec.recipe == node.spec.recipe:
                if best_sibling is None or other.id > best_sibling:
                    best_sibling = other.id
        if best_sibling is not None:
            connected.append(best_sibling)

    # Deduplicate while preserving order
    seen: set[int] = set()
    result: list[int] = []
    for c in connected:
        if c not in seen:
            seen.add(c)
            result.append(c)
    return result


def _overlaps(
    x: int, y: int, size: int, occupied: set[tuple[int, int]]
) -> bool:
    """Check if placing a machine at (x,y) would overlap occupied tiles."""
    for dx in range(size):
        for dy in range(size):
            if (x + dx, y + dy) in occupied:
                return True
    return False


def _candidate_positions(
    node_id: int,
    node_size: int,
    graph: ProductionGraph,
    positions: dict[int, tuple[int, int]],
    occupied: set[tuple[int, int]],
    spacing: int,
) -> list[tuple[int, int]]:
    """Generate candidate positions for a machine near its connected placed machines."""
    connected = _connected_placed(node_id, graph, set(positions.keys()))

    candidates: list[tuple[int, int]] = []

    for cid in connected:
        cx, cy = positions[cid]
        csize = machine_size(
            next(n for n in graph.nodes if n.id == cid).spec.entity
        )

        # Generate candidates in cardinal directions at various distances
        for dist in range(spacing, spacing + 5):
            # Right of connected machine (preferred — keeps layout left-to-right)
            candidates.append((cx + csize + dist, cy))
            # Below connected machine
            candidates.append((cx, cy + csize + dist))
            # Left of connected machine
            candidates.append((cx - node_size - dist, cy))
            # Above connected machine
            candidates.append((cx, cy - node_size - dist))

            # Diagonal positions (offset by half)
            half = dist // 2
            candidates.append((cx + csize + dist, cy + half))
            candidates.append((cx + csize + dist, cy - half))
            candidates.append((cx - node_size - dist, cy + half))
            candidates.append((cx - node_size - dist, cy - half))
            candidates.append((cx + half, cy + csize + dist))
            candidates.append((cx - half, cy + csize + dist))
            candidates.append((cx + half, cy - node_size - dist))
            candidates.append((cx - half, cy - node_size - dist))

    # Filter out positions that would overlap
    valid = [
        (x, y) for x, y in candidates if not _overlaps(x, y, node_size, occupied)
    ]

    # Deduplicate
    seen: set[tuple[int, int]] = set()
    result: list[tuple[int, int]] = []
    for pos in valid:
        if pos not in seen:
            seen.add(pos)
            result.append(pos)

    return result


def _score_position(
    x: int,
    y: int,
    node_size: int,
    node_id: int,
    graph: ProductionGraph,
    positions: dict[int, tuple[int, int]],
    occupied: set[tuple[int, int]],
) -> float:
    """Score a candidate position (lower is better).

    Considers:
    - Manhattan distance to connected placed machines
    - Alignment bonus (same row or column as connected machines)
    - Corridor penalty (tight spaces between machines)
    """
    connected = _connected_placed(node_id, graph, set(positions.keys()))
    if not connected:
        return 0.0

    score = 0.0
    center_x = x + node_size / 2
    center_y = y + node_size / 2

    for cid in connected:
        cx, cy = positions[cid]
        csize = machine_size(
            next(n for n in graph.nodes if n.id == cid).spec.entity
        )
        c_center_x = cx + csize / 2
        c_center_y = cy + csize / 2

        # Manhattan distance between centers
        dist = abs(center_x - c_center_x) + abs(center_y - c_center_y)
        score += dist

        # Alignment bonus: if on same row or column, inserters can face
        # each other directly, making routing easier
        if abs(center_y - c_center_y) < 1.0:
            score -= 2.0  # horizontal alignment bonus
        elif abs(center_x - c_center_x) < 1.0:
            score -= 2.0  # vertical alignment bonus

    # Corridor penalty: check for tight spaces (< 2 tiles) between
    # this machine and any placed machine
    for pid, (px, py) in positions.items():
        psize = machine_size(
            next(n for n in graph.nodes if n.id == pid).spec.entity
        )
        # Horizontal gap
        if y < py + psize and y + node_size > py:
            # Vertically overlapping — check horizontal gap
            gap_right = px - (x + node_size)
            gap_left = x - (px + psize)
            gap = max(gap_right, gap_left)
            if 0 < gap < 2:
                score += 10.0  # tight corridor penalty
        # Vertical gap
        if x < px + psize and x + node_size > px:
            gap_bottom = py - (y + node_size)
            gap_top = y - (py + psize)
            gap = max(gap_bottom, gap_top)
            if 0 < gap < 2:
                score += 10.0

    # Penalize positions that go too far negative (A* bounds at -10)
    if x < -5 or y < -5:
        score += 50.0
    # Small compactness bonus — prefer positions closer to origin
    score += (abs(x) + abs(y)) * 0.1

    return score


def incremental_place(
    graph: ProductionGraph,
    spacing: int = 3,
) -> dict[int, tuple[int, int]]:
    """Place machines incrementally in dependency order.

    Places machines one at a time, choosing positions that minimize
    estimated routing cost to already-placed connected machines.

    Algorithm:
    1. Build dependency order (topological sort, leaves first)
    2. Place first machine at origin
    3. For each subsequent machine:
       a. Find all already-placed machines it connects to
       b. Generate candidate positions around those machines
       c. Score each candidate by estimated routing distance
       d. Pick the best valid (non-overlapping) position
    """
    if not graph.nodes:
        return {}

    order = _dependency_order(graph)
    node_map = {n.id: n for n in graph.nodes}

    positions: dict[int, tuple[int, int]] = {}
    occupied: set[tuple[int, int]] = set()

    for i, nid in enumerate(order):
        node = node_map[nid]
        size = machine_size(node.spec.entity)

        if i == 0:
            # Place first machine at origin
            pos = (0, 0)
        else:
            candidates = _candidate_positions(
                nid, size, graph, positions, occupied, spacing
            )

            if not candidates:
                # Fallback: generate positions in a spiral around existing machines
                candidates = _spiral_fallback(
                    size, positions, occupied, spacing
                )

            if candidates:
                # Score and pick best
                best_pos = None
                best_score = float("inf")
                for cx, cy in candidates:
                    s = _score_position(
                        cx, cy, size, nid, graph, positions, occupied
                    )
                    if s < best_score:
                        best_score = s
                        best_pos = (cx, cy)
                pos = best_pos if best_pos is not None else candidates[0]
            else:
                # Last resort: place far away
                max_x = max(x + machine_size(node_map[pid].spec.entity)
                           for pid, (x, _) in positions.items()) if positions else 0
                pos = (max_x + spacing + size, 0)

        positions[nid] = pos
        # Mark occupied tiles (machine footprint + 1-tile border for inserters)
        occupied |= _machine_footprint(pos[0], pos[1], size)

    return positions


def _spiral_fallback(
    size: int,
    positions: dict[int, tuple[int, int]],
    occupied: set[tuple[int, int]],
    spacing: int,
) -> list[tuple[int, int]]:
    """Generate fallback positions in a spiral pattern around placed machines."""
    if not positions:
        return [(0, 0)]

    # Center of mass of placed machines
    avg_x = sum(x for x, _ in positions.values()) / len(positions)
    avg_y = sum(y for _, y in positions.values()) / len(positions)
    cx, cy = int(avg_x), int(avg_y)

    candidates = []
    for radius in range(spacing + size, spacing + size + 20, 2):
        for dx in range(-radius, radius + 1, max(1, spacing)):
            for dy in [-radius, radius]:
                x, y = cx + dx, cy + dy
                if not _overlaps(x, y, size, occupied):
                    candidates.append((x, y))
            for dy in range(-radius + 1, radius, max(1, spacing)):
                for dx_val in [-radius, radius]:
                    x, y = cx + dx_val, cy + dy
                    if not _overlaps(x, y, size, occupied):
                        candidates.append((x, y))
        if candidates:
            return candidates

    return candidates
