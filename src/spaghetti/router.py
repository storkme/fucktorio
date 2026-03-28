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

# Inverse: EntityDirection → (dx, dy)
_DIR_VEC = {
    EntityDirection.NORTH: (0, -1),
    EntityDirection.EAST: (1, 0),
    EntityDirection.SOUTH: (0, 1),
    EntityDirection.WEST: (-1, 0),
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


def _network_downstream_ends(
    network: set[tuple[int, int]],
    belt_dir_map: dict[tuple[int, int], EntityDirection],
) -> set[tuple[int, int]]:
    """Find tiles at the downstream end of a belt network.

    A downstream end is a network tile whose belt direction points to a tile
    NOT in the network — the tip where items would exit.
    """
    ends = set()
    for tile in network:
        d = belt_dir_map.get(tile)
        if d is None:
            continue
        dx, dy = _DIR_VEC[d]
        forward = (tile[0] + dx, tile[1] + dy)
        if forward not in network:
            ends.add(tile)
    return ends


def _perpendicular_approach_tiles(
    network: set[tuple[int, int]],
    belt_dir_map: dict[tuple[int, int], EntityDirection],
    occupied: set[tuple[int, int]],
) -> set[tuple[int, int]]:
    """Find unoccupied tiles perpendicular to existing network belt directions.

    These are valid sideload connection points — approaching a belt from
    the side rather than head-on or from behind.
    """
    approach = set()
    for nx, ny in network:
        d = belt_dir_map.get((nx, ny))
        if d is None:
            continue
        dx, dy = _DIR_VEC[d]
        # Perpendicular directions: rotate 90° both ways
        for pdx, pdy in [(-dy, dx), (dy, -dx)]:
            tile = (nx + pdx, ny + pdy)
            if tile not in occupied and tile not in network:
                approach.add(tile)
    return approach


def route_connections(
    graph: ProductionGraph,
    positions: dict[int, tuple[int, int]],
    edge_targets: dict[int, tuple[int, int]] | None = None,
    edge_starts: dict[int, tuple[int, int]] | None = None,
    reserved_tiles: set[tuple[int, int]] | None = None,
    edge_exclusions: dict[int, set[tuple[int, int]]] | None = None,
    edge_subgroups: dict[str, list[list[int]]] | None = None,
) -> RoutingResult:
    """Route all flow edges as belts/pipes using A* pathfinding.

    External edges sharing the same item are grouped and routed consecutively.
    After the first edge in a group is routed, subsequent edges branch from
    the existing network rather than routing independently from the boundary.

    Args:
        edge_targets: Mapping from edge index to a specific belt tile target.
        edge_starts: Mapping from edge index to a specific belt tile start.
        reserved_tiles: Pre-occupied tiles the router must avoid.
        edge_exclusions: Per-edge tiles to temporarily unblock from obstacles.
            Maps edge index → set of tiles that only this edge may use.
        edge_subgroups: Per-item sub-groups for capacity splitting.
            Maps item → list of sub-groups (each a list of edge indices).
            Sub-groups route independently with separate trunk networks.
    """
    if edge_targets is None:
        edge_targets = {}
    if edge_starts is None:
        edge_starts = {}
    if edge_exclusions is None:
        edge_exclusions = {}
    if edge_subgroups is None:
        edge_subgroups = {}
    entities: list[PlacedEntity] = []
    failed_edges: list[FlowEdge] = []

    # Build initial obstacle set from machine footprints + reserved tiles
    occupied: set[tuple[int, int]] = set(reserved_tiles) if reserved_tiles else set()
    for node in graph.nodes:
        x, y = positions[node.id]
        size = machine_size(node.spec.entity)
        occupied |= _machine_tiles(x, y, size)

    # Compute max grid extent for A* bounds
    if positions:
        max_x = max(x for x, y in positions.values()) + 10
        max_y = max(y for x, y in positions.values()) + 10
        max_extent = max(max_x, max_y, 50)
    else:
        max_extent = 50

    # Track belt networks per sub-group for network-aware routing
    # Key: (item, subgroup_idx) — each sub-group routes independently
    group_networks: dict[tuple[str, int], set[tuple[int, int]]] = {}
    # Track belt directions for junction-aware routing
    belt_dir_map: dict[tuple[int, int], EntityDirection] = {}

    # --- Map each edge index to its sub-group key ---
    edge_group_key: dict[int, tuple[str, int]] = {}
    for item, groups in edge_subgroups.items():
        for g_idx, edge_indices in enumerate(groups):
            for ei in edge_indices:
                edge_group_key[ei] = (item, g_idx)

    # --- Group edges by type ---
    internal_indices = []
    # Routing groups: each is (group_key, [edge_indices])
    # Input and output sub-groups route as separate groups
    input_routing_groups: list[tuple[tuple[str, int], list[int]]] = []
    output_routing_groups: list[tuple[tuple[str, int], list[int]]] = []
    # Track edges not in any sub-group (fallback to item-based grouping)
    ungrouped_inputs: dict[str, list[int]] = {}
    ungrouped_outputs: dict[str, list[int]] = {}

    for i, edge in enumerate(graph.edges):
        if edge.from_node is not None and edge.to_node is not None:
            internal_indices.append(i)
        elif edge.from_node is None:
            if i in edge_group_key:
                # Will be added via sub-groups below
                pass
            else:
                ungrouped_inputs.setdefault(edge.item, []).append(i)
        elif i in edge_group_key:
            pass  # handled via sub-groups
        else:
            ungrouped_outputs.setdefault(edge.item, []).append(i)

    # Build sub-group routing groups from edge_subgroups
    for item, groups in edge_subgroups.items():
        for g_idx, edge_indices in enumerate(groups):
            key = (item, g_idx)
            inputs = [i for i in edge_indices if graph.edges[i].from_node is None]
            outputs = [i for i in edge_indices if graph.edges[i].to_node is None]
            if inputs:
                input_routing_groups.append((key, inputs))
            if outputs:
                output_routing_groups.append((key, outputs))

    # Add ungrouped edges as their own groups
    for item, indices in ungrouped_inputs.items():
        input_routing_groups.append(((item, 0), indices))
    for item, indices in ungrouped_outputs.items():
        output_routing_groups.append(((item, 0), indices))

    # Sort internal edges by distance (shorter first)
    def _distance_key(idx: int) -> float:
        e = graph.edges[idx]
        if e.from_node is None or e.to_node is None:
            return 0
        fx, fy = positions[e.from_node]
        tx, ty = positions[e.to_node]
        return abs(fx - tx) + abs(fy - ty)

    internal_indices.sort(key=_distance_key)

    # Sort edges within each input group by spatial proximity
    for _key, indices in input_routing_groups:
        if len(indices) <= 1:
            continue

        def _input_sort_key(idx: int) -> float:
            e = graph.edges[idx]
            if e.to_node is None:
                return 0
            tx, ty = positions[e.to_node]
            return tx + ty

        indices.sort(key=_input_sort_key)

    # Build the routing order: internal edges, then input groups, then output groups
    routing_order: list[tuple[int, bool, tuple[str, int]]] = []
    for idx in internal_indices:
        edge = graph.edges[idx]
        key = edge_group_key.get(idx, (edge.item, 0))
        routing_order.append((idx, False, key))
    for key, indices in input_routing_groups:
        for rank, idx in enumerate(indices):
            routing_order.append((idx, rank > 0, key))
    for key, indices in output_routing_groups:
        for rank, idx in enumerate(indices):
            routing_order.append((idx, rank > 0, key))

    # Compute total rate per routing group (for belt tier selection)
    group_total_rate: dict[tuple[str, int], float] = {}
    for key, indices in input_routing_groups:
        group_total_rate[key] = sum(graph.edges[i].rate for i in indices)
    for key, indices in output_routing_groups:
        group_total_rate.setdefault(key, 0)
        group_total_rate[key] += sum(graph.edges[i].rate for i in indices)

    # --- Route each edge ---
    for edge_idx, is_continuation, group_key in routing_order:
        edge = graph.edges[edge_idx]

        # Temporarily unblock tiles reserved for this specific edge
        exclusions = edge_exclusions.get(edge_idx, set())
        if exclusions:
            occupied -= exclusions

        # Determine start and goal tiles
        has_start = edge_idx in edge_starts
        has_target = edge_idx in edge_targets
        network = group_networks.get(group_key, set())

        if is_continuation and network:
            # Network-aware routing for continuations.
            # Input vs output use different strategies:
            # - Inputs: extend trunk from downstream ends (items flow forward)
            # - Outputs: sideload into trunk via perpendicular approach tiles
            approach = _perpendicular_approach_tiles(network, belt_dir_map, occupied)
            junction_tiles = approach if approach else set(network)

            if edge.from_node is None and not edge.is_fluid:
                # External input continuation (belts): extend the trunk from
                # its downstream end toward this machine's input belt tile.
                # Items flow forward through the trunk extension.
                downstream_ends = _network_downstream_ends(network, belt_dir_map)
                forward_tiles = set()
                for tile in downstream_ends:
                    d = belt_dir_map.get(tile)
                    if d is not None:
                        dx, dy = _DIR_VEC[d]
                        forward_tiles.add((tile[0] + dx, tile[1] + dy))
                start_tiles = forward_tiles if forward_tiles else junction_tiles
                if has_target:
                    goal_tiles = {edge_targets[edge_idx]}
                else:
                    _, goal_tiles = _edge_endpoints(edge, graph, positions, occupied)
                # Forward tiles are outside the network — no obstacle changes needed
            elif edge.from_node is None:
                # External input continuation (fluids): pipes connect
                # omnidirectionally, perpendicular approach works fine
                start_tiles = junction_tiles
                if has_target:
                    goal_tiles = {edge_targets[edge_idx]}
                else:
                    _, goal_tiles = _edge_endpoints(edge, graph, positions, occupied)
            else:
                # External output continuation: sideload into trunk via
                # perpendicular approach tiles (items merge onto trunk)
                if has_start:
                    start_tiles = {edge_starts[edge_idx]}
                else:
                    start_tiles, _ = _edge_endpoints(edge, graph, positions, occupied)
                goal_tiles = junction_tiles

            # Never remove network from obstacles — routing through existing
            # network tiles creates belt loops. If no approach/forward tiles
            # are available, the edge will fail routing (better than a loop).
        elif has_start or has_target:
            if has_start and has_target:
                start_tiles = {edge_starts[edge_idx]}
                goal_tiles = {edge_targets[edge_idx]}
            elif has_target:
                start_tiles, _ = _edge_endpoints(edge, graph, positions, occupied)
                goal_tiles = {edge_targets[edge_idx]}
            else:  # has_start only
                _, goal_tiles = _edge_endpoints(edge, graph, positions, occupied)
                start_tiles = {edge_starts[edge_idx]}
        else:
            start_tiles, goal_tiles = _edge_endpoints(edge, graph, positions, occupied)

        if not start_tiles or not goal_tiles:
            if exclusions:
                occupied |= exclusions
            continue

        # Try A* from each start tile until one works
        best_path = None
        for start in start_tiles:
            if start in occupied:
                continue
            path = _astar_path(start, goal_tiles - occupied, occupied, max_extent)
            if path and (best_path is None or len(path) < len(best_path)):
                best_path = path

        # Re-block the exclusion tiles
        if exclusions:
            occupied |= exclusions

        if best_path is None:
            failed_edges.append(edge)
            continue

        # Choose belt tier: use total group rate for external edges
        if edge.is_fluid:
            belt_name = "pipe"
        elif group_key in group_total_rate:
            belt_name = _belt_entity_for_rate(group_total_rate[group_key])
        else:
            belt_name = _belt_entity_for_rate(edge.rate)

        # Place entities along path, skipping tiles already on the network
        new_tiles = [t for t in best_path if t not in network]
        if new_tiles:
            path_entities = _path_to_entities(new_tiles, belt_name, edge.item, edge.is_fluid)
            entities.extend(path_entities)
            # Track belt directions for junction-aware routing
            for pe in path_entities:
                belt_dir_map[(pe.x, pe.y)] = pe.direction

        # Update network and occupied tiles
        path_set = set(best_path)
        group_networks.setdefault(group_key, set()).update(path_set)
        occupied |= path_set

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
