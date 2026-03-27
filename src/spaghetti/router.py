"""Belt/pipe routing via A* pathfinding on the tile grid."""

from __future__ import annotations

from dataclasses import dataclass, field

from ..models import EntityDirection, PlacedEntity
from .graph import FlowEdge, ProductionGraph
from .placer import machine_size


@dataclass
class RoutingResult:
    """Result of routing all flow edges."""

    entities: list[PlacedEntity] = field(default_factory=list)
    occupied: set[tuple[int, int]] = field(default_factory=set)
    failed_edges: list[FlowEdge] = field(default_factory=list)


# Belt throughput tiers (items per second)
_BELT_TIERS = [
    ("transport-belt", 15.0),
    ("fast-transport-belt", 30.0),
    ("express-transport-belt", 45.0),
]

# Direction vectors: (dx, dy) for each cardinal direction
_DIRECTIONS = [(0, -1), (1, 0), (0, 1), (-1, 0)]  # N, E, S, W

_DIR_MAP = {
    (0, -1): EntityDirection.NORTH,
    (1, 0): EntityDirection.EAST,
    (0, 1): EntityDirection.SOUTH,
    (-1, 0): EntityDirection.WEST,
}


def _belt_entity_for_rate(rate: float) -> str:
    """Pick the cheapest belt tier that can handle the given rate."""
    for name, throughput in _BELT_TIERS:
        if rate <= throughput:
            return name
    return _BELT_TIERS[-1][0]  # express if rate exceeds all


def _machine_tiles(x: int, y: int, size: int) -> set[tuple[int, int]]:
    """All tiles occupied by a machine at (x, y) with given size."""
    return {(x + dx, y + dy) for dx in range(size) for dy in range(size)}


def _machine_border_tiles(x: int, y: int, size: int) -> list[tuple[int, int, int, int]]:
    """Tiles adjacent to a machine border, with direction toward the machine.

    Returns list of (tile_x, tile_y, dx_toward_machine, dy_toward_machine).
    """
    borders = []
    # Top edge (y - 1)
    for dx in range(size):
        borders.append((x + dx, y - 1, 0, 1))
    # Bottom edge (y + size)
    for dx in range(size):
        borders.append((x + dx, y + size, 0, -1))
    # Left edge (x - 1)
    for dy in range(size):
        borders.append((x - 1, y + dy, 1, 0))
    # Right edge (x + size)
    for dy in range(size):
        borders.append((x + size, y + dy, -1, 0))
    return borders


def _machine_belt_tiles(x: int, y: int, size: int) -> list[tuple[int, int]]:
    """Tiles 2 away from machine — where belts should end (inserter goes between).

    The border tile (1 away) is reserved for the inserter.
    The belt tile (2 away) is where the belt terminates.
    """
    tiles = []
    # Top (y - 2)
    for dx in range(size):
        tiles.append((x + dx, y - 2))
    # Bottom (y + size + 1)
    for dx in range(size):
        tiles.append((x + dx, y + size + 1))
    # Left (x - 2)
    for dy in range(size):
        tiles.append((x - 2, y + dy))
    # Right (x + size + 1)
    for dy in range(size):
        tiles.append((x + size + 1, y + dy))
    return tiles


def _astar_path(
    start: tuple[int, int],
    goals: set[tuple[int, int]],
    obstacles: set[tuple[int, int]],
    max_extent: int = 200,
) -> list[tuple[int, int]] | None:
    """A* pathfinding with Manhattan heuristic.

    Produces shorter, more direct paths than BFS by using A* with
    a Manhattan distance heuristic. Tie-breaking favors straight lines.
    """
    import heapq

    if start in goals:
        return [start]

    if not goals:
        return None

    # Pick a single goal for the heuristic (nearest by Manhattan)
    goal_list = list(goals)
    sx, sy = start

    def _h(x: int, y: int) -> int:
        return min(abs(x - gx) + abs(y - gy) for gx, gy in goal_list)

    counter = 0
    # (f_score, counter, x, y)
    open_set: list[tuple[int, int, int, int]] = []
    heapq.heappush(open_set, (_h(sx, sy), counter, sx, sy))
    counter += 1

    g_score: dict[tuple[int, int], int] = {start: 0}
    parent: dict[tuple[int, int], tuple[int, int]] = {}

    while open_set:
        _, _, cx, cy = heapq.heappop(open_set)

        if (cx, cy) in goals:
            path = [(cx, cy)]
            cur = (cx, cy)
            while cur in parent:
                cur = parent[cur]
                path.append(cur)
            path.reverse()
            return path

        cur_g = g_score.get((cx, cy), 0)

        for dx, dy in _DIRECTIONS:
            nx, ny = cx + dx, cy + dy

            if nx < -10 or ny < -10 or nx > max_extent or ny > max_extent:
                continue
            if (nx, ny) in obstacles:
                continue

            new_g = cur_g + 1
            if (nx, ny) in g_score and g_score[(nx, ny)] <= new_g:
                continue

            g_score[(nx, ny)] = new_g
            parent[(nx, ny)] = (cx, cy)
            f = new_g + _h(nx, ny)
            heapq.heappush(open_set, (f, counter, nx, ny))
            counter += 1

    return None


def _path_to_entities(
    path: list[tuple[int, int]],
    entity_name: str,
    item: str,
    is_fluid: bool,
) -> list[PlacedEntity]:
    """Convert a tile path to placed belt or pipe entities."""
    entities: list[PlacedEntity] = []

    for i, (x, y) in enumerate(path):
        if is_fluid:
            entities.append(PlacedEntity(name="pipe", x=x, y=y, carries=item))
        else:
            # Determine belt direction from path
            if i + 1 < len(path):
                dx = path[i + 1][0] - x
                dy = path[i + 1][1] - y
            elif i > 0:
                dx = x - path[i - 1][0]
                dy = y - path[i - 1][1]
            else:
                dx, dy = 0, 1  # default south

            direction = _DIR_MAP.get((dx, dy), EntityDirection.SOUTH)
            entities.append(
                PlacedEntity(
                    name=entity_name,
                    x=x,
                    y=y,
                    direction=direction,
                    carries=item,
                )
            )

    return entities


def route_connections(
    graph: ProductionGraph,
    positions: dict[int, tuple[int, int]],
    edge_targets: dict[int, tuple[int, int]] | None = None,
    reserved_tiles: set[tuple[int, int]] | None = None,
    edge_exclusions: dict[int, set[tuple[int, int]]] | None = None,
) -> RoutingResult:
    """Route all flow edges as belts/pipes using BFS pathfinding.

    Args:
        edge_targets: Mapping from edge index to a specific belt tile target.
        reserved_tiles: Pre-occupied tiles the router must avoid.
        edge_exclusions: Per-edge tiles to temporarily unblock from obstacles.
            Maps edge index → set of tiles that only this edge may use.
    """
    if edge_targets is None:
        edge_targets = {}
    if edge_exclusions is None:
        edge_exclusions = {}
    entities: list[PlacedEntity] = []
    failed_edges: list[FlowEdge] = []

    # Build initial obstacle set from machine footprints + reserved tiles
    occupied: set[tuple[int, int]] = set(reserved_tiles) if reserved_tiles else set()
    for node in graph.nodes:
        x, y = positions[node.id]
        size = machine_size(node.spec.entity)
        occupied |= _machine_tiles(x, y, size)

    # Compute max grid extent for BFS bounds
    if positions:
        max_x = max(x for x, y in positions.values()) + 10
        max_y = max(y for x, y in positions.values()) + 10
        max_extent = max(max_x, max_y, 50)
    else:
        max_extent = 50

    # Sort edges: route shorter expected distances first
    def _edge_sort_key(edge: FlowEdge) -> float:
        if edge.from_node is None or edge.to_node is None:
            return 0  # external edges first
        fx, fy = positions[edge.from_node]
        tx, ty = positions[edge.to_node]
        return abs(fx - tx) + abs(fy - ty)

    sorted_edge_indices = sorted(range(len(graph.edges)), key=lambda i: _edge_sort_key(graph.edges[i]))

    for edge_idx in sorted_edge_indices:
        edge = graph.edges[edge_idx]

        # Skip external output edges — no useful destination to route to.
        # The output inserter still drops items onto the belt tile; the player
        # extends the belt from there.
        if edge.to_node is None:
            continue

        # Temporarily unblock tiles reserved for this specific edge
        exclusions = edge_exclusions.get(edge_idx, set())
        if exclusions:
            occupied -= exclusions

        # Use pre-assigned target if available, otherwise compute from endpoints
        if edge_idx in edge_targets:
            target = edge_targets[edge_idx]
            # For assigned targets, start from the other end's belt tiles
            start_tiles, _ = _edge_endpoints(edge, graph, positions, occupied)
            goal_tiles = {target}
        else:
            start_tiles, goal_tiles = _edge_endpoints(edge, graph, positions, occupied)

        if not start_tiles or not goal_tiles:
            if exclusions:
                occupied |= exclusions
            continue

        # Try BFS from each start tile until one works
        best_path = None
        for start in start_tiles:
            if start in occupied:
                continue
            path = _astar_path(start, goal_tiles - occupied, occupied, max_extent)
            if path and (best_path is None or len(path) < len(best_path)):
                best_path = path

        # Re-block the exclusion tiles (they stay unblocked only if this route used them)
        if exclusions:
            occupied |= exclusions

        if best_path is None:
            failed_edges.append(edge)
            continue

        # Choose belt tier based on rate
        belt_name = "pipe" if edge.is_fluid else _belt_entity_for_rate(edge.rate)

        # Place entities along path
        path_entities = _path_to_entities(best_path, belt_name, edge.item, edge.is_fluid)
        entities.extend(path_entities)

        # Mark tiles as occupied
        for x, y in best_path:
            occupied.add((x, y))

    return RoutingResult(entities=entities, occupied=occupied, failed_edges=failed_edges)


def _edge_endpoints(
    edge: FlowEdge,
    graph: ProductionGraph,
    positions: dict[int, tuple[int, int]],
    occupied: set[tuple[int, int]],
) -> tuple[set[tuple[int, int]], set[tuple[int, int]]]:
    """Determine start and goal tile sets for routing an edge.

    For internal edges: start/goal = belt tiles (2 tiles from machine),
    leaving the border tile free for an inserter.
    For external inputs: start = edge of grid, goal = belt tiles of dest.
    For external outputs: start = belt tiles of source, goal = edge of grid.
    """
    start_tiles: set[tuple[int, int]] = set()
    goal_tiles: set[tuple[int, int]] = set()

    if edge.from_node is not None:
        fx, fy = positions[edge.from_node]
        src_node = next(n for n in graph.nodes if n.id == edge.from_node)
        size = machine_size(src_node.spec.entity)
        for bx, by in _machine_belt_tiles(fx, fy, size):
            start_tiles.add((bx, by))
    else:
        # External input — start from the nearest grid edge to the target machine
        if edge.to_node is not None and edge.to_node in {n.id for n in graph.nodes}:
            tx, ty = positions[edge.to_node]
            dst_size = machine_size(next(n for n in graph.nodes if n.id == edge.to_node).spec.entity)
            # Find grid bounds
            all_x = [x for x, _ in positions.values()]
            all_y = [y for _, y in positions.values()]
            min_gx, max_gx = min(all_x) - 3, max(all_x) + dst_size + 3
            min_gy, max_gy = min(all_y) - 3, max(all_y) + dst_size + 3
            # Center of target machine
            cx, cy = tx + dst_size // 2, ty + dst_size // 2
            # Distance to each edge
            edges_dist = [
                (cx - min_gx, "left"),
                (max_gx - cx, "right"),
                (cy - min_gy, "top"),
                (max_gy - cy, "bottom"),
            ]
            edges_dist.sort(key=lambda d: d[0])
            _, nearest = edges_dist[0]
            # Place start tiles along that edge
            if nearest == "left":
                for y in range(min_gy, max_gy + 1):
                    start_tiles.add((min_gx, y))
            elif nearest == "right":
                for y in range(min_gy, max_gy + 1):
                    start_tiles.add((max_gx, y))
            elif nearest == "top":
                for x in range(min_gx, max_gx + 1):
                    start_tiles.add((x, min_gy))
            else:  # bottom
                for x in range(min_gx, max_gx + 1):
                    start_tiles.add((x, max_gy))
        else:
            # Fallback: left edge
            min_y = min(y for _, y in positions.values()) if positions else 0
            max_y = max(y for _, y in positions.values()) + 5 if positions else 10
            for y in range(min_y - 3, max_y + 4):
                start_tiles.add((min(x for x, _ in positions.values()) - 3, y))

    if edge.to_node is not None:
        tx, ty = positions[edge.to_node]
        dst_node = next(n for n in graph.nodes if n.id == edge.to_node)
        size = machine_size(dst_node.spec.entity)
        for bx, by in _machine_belt_tiles(tx, ty, size):
            goal_tiles.add((bx, by))
    else:
        # External output — route to right edge
        max_x = max(x for x, _ in positions.values()) + 10 if positions else 20
        min_y = min(y for _, y in positions.values()) if positions else 0
        max_y = max(y for _, y in positions.values()) + 5 if positions else 10
        for y in range(min_y - 2, max_y + 3):
            goal_tiles.add((max_x, y))

    return start_tiles, goal_tiles
