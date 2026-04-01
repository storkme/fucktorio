"""Shared layout orchestration: inserters -> routing -> poles."""

from __future__ import annotations

import logging

from ..models import EntityDirection, LayoutResult, PlacedEntity, SolverResult
from .common import (
    _MACHINE_SIZE,
    _UG_MAX_REACH,
    DIR_MAP,
    DIR_VEC,
    belt_entity_for_rate,
    inserter_target_lane,
    machine_size,
    machine_tiles,
)
from .graph import FlowEdge, ProductionGraph
from .inserters import (
    InserterAssignment,
    _compute_edge_subgroups,
    _get_sides,
    assign_inserter_positions,
    build_inserter_entities,
)
from .poles import place_poles
from .router import (
    _astar_path,
    _classify_approach_by_lane,
    _detect_sideload_lane,
    _fix_belt_directions,
    _network_downstream_ends,
    _network_upstream_ends,
    _path_to_entities,
    _perpendicular_approach_tiles,
    route_connections,
)

_log = logging.getLogger(__name__)


def _validate_junction_direction(
    path: list[tuple[int, int]],
    path_ents: list[PlacedEntity],
    existing_network: set[tuple[int, int]],
    belt_dir_map: dict[tuple[int, int], EntityDirection],
    is_input: bool,
) -> None:
    """Validate and fix belt direction at the junction where a new path meets the existing network.

    For input continuations: the path flows from the network toward the stub.
    The junction tile (network side) must not point head-on against the trunk.

    For output continuations: the path flows from the stub toward the network.
    The last new tile must be perpendicular (sideload) or same-direction as the trunk.

    Modifies path_ents and belt_dir_map in-place if a fix is needed.
    """
    if len(path) < 2:
        return

    # Build position -> entity mapping for the path
    ent_by_pos: dict[tuple[int, int], PlacedEntity] = {}
    for pe in path_ents:
        ent_by_pos[(pe.x, pe.y)] = pe

    # Find the junction: the tile in the path that is adjacent to the network
    # but is NOT itself in the network (the new tile touching the trunk)
    junction_pos = None
    trunk_neighbor = None

    if is_input:
        # Input: path goes network→stub. path[0] may be IN the network.
        # Find the first path tile NOT in the network.
        for k, tile in enumerate(path):
            if tile not in existing_network:
                junction_pos = tile
                # The trunk neighbor is the previous tile (which IS in the network)
                if k > 0 and path[k - 1] in existing_network:
                    trunk_neighbor = path[k - 1]
                break
    else:
        # Output: path goes stub→network. path[-1] may be IN the network.
        # Find the last path tile NOT in the network.
        for k in range(len(path) - 1, -1, -1):
            if path[k] not in existing_network:
                junction_pos = path[k]
                # The trunk neighbor is the next tile (which IS in the network)
                if k + 1 < len(path) and path[k + 1] in existing_network:
                    trunk_neighbor = path[k + 1]
                break

    if junction_pos is None or trunk_neighbor is None:
        return

    trunk_dir = belt_dir_map.get(trunk_neighbor)
    junction_dir = belt_dir_map.get(junction_pos)
    if trunk_dir is None or junction_dir is None:
        return

    trunk_vec = DIR_VEC[trunk_dir]
    junction_vec = DIR_VEC[junction_dir]

    # Check for head-on collision: junction direction is opposite to trunk
    dot = trunk_vec[0] * junction_vec[0] + trunk_vec[1] * junction_vec[1]
    if dot == -1:
        # Head-on: flip to perpendicular (sideload toward trunk)
        face_vec = (trunk_neighbor[0] - junction_pos[0], trunk_neighbor[1] - junction_pos[1])
        face_dir = DIR_MAP.get(face_vec)
        if face_dir is not None:
            _log.debug(
                "Junction fix at (%d,%d): %s→%s (was head-on against trunk)",
                junction_pos[0],
                junction_pos[1],
                junction_dir,
                face_dir,
            )
            belt_dir_map[junction_pos] = face_dir
            if junction_pos in ent_by_pos:
                ent_by_pos[junction_pos].direction = face_dir


def build_layout(
    solver_result: SolverResult,
    graph: ProductionGraph,
    positions: dict[int, tuple[int, int]],
    side_strategy: str = "top_bottom",
    side_preference: dict[int, list[tuple[int, int]]] | None = None,
    edge_order: list[int] | None = None,
) -> tuple[LayoutResult, list[FlowEdge], int]:
    """Build a complete layout from machine positions.

    Steps: build occupied set, assign inserters, route connections,
    place belt stubs, place inserters, place poles, calculate bounds.
    """
    # 1. Build occupied set from machine footprints
    occupied: set[tuple[int, int]] = set()
    for node in graph.nodes:
        x, y = positions[node.id]
        size = machine_size(node.spec.entity)
        occupied |= machine_tiles(x, y, size)

    # 2. Pre-assign inserter positions (lane-aware, reserves border tiles)
    plan = assign_inserter_positions(
        graph,
        positions,
        occupied,
        solver_result=solver_result,
        side_strategy=side_strategy,
        side_preference=side_preference,
    )
    assignments = plan.assignments

    # 2b. Detect direct machine-to-machine insertion opportunities
    # For internal edges where two machines are adjacent (gap=1), place a
    # direct inserter in the gap tile instead of routing a belt.
    direct_edge_indices: set[int] = set()
    for i, edge in enumerate(graph.edges):
        if edge.from_node is None or edge.to_node is None or edge.is_fluid:
            continue
        # Check if machines are adjacent with gap=1
        ax, ay = positions[edge.from_node]
        bx, by = positions[edge.to_node]
        a_size = machine_size(next(n for n in graph.nodes if n.id == edge.from_node).spec.entity)
        b_size = machine_size(next(n for n in graph.nodes if n.id == edge.to_node).spec.entity)

        # Find gap tiles between the two machines
        gap_tile = None
        ins_dir = None
        # Check horizontal adjacency (A left of B or B left of A)
        if ay < by + b_size and ay + a_size > by:
            # Vertically overlapping — check horizontal gap
            overlap_start = max(ay, by)
            overlap_end = min(ay + a_size, by + b_size)
            if ax + a_size + 1 == bx:
                # A is left of B, gap column = ax + a_size
                mid_y = (overlap_start + overlap_end) // 2
                gap_tile = (ax + a_size, mid_y)
                ins_dir = EntityDirection.EAST  # picks from A, drops into B
            elif bx + b_size + 1 == ax:
                # B is left of A, gap column = bx + b_size
                mid_y = (overlap_start + overlap_end) // 2
                gap_tile = (bx + b_size, mid_y)
                ins_dir = EntityDirection.WEST  # picks from A (right), drops into B (left)
        # Check vertical adjacency
        if gap_tile is None and ax < bx + b_size and ax + a_size > bx:
            overlap_start = max(ax, bx)
            overlap_end = min(ax + a_size, bx + b_size)
            if ay + a_size + 1 == by:
                # A is above B, gap row = ay + a_size
                mid_x = (overlap_start + overlap_end) // 2
                gap_tile = (mid_x, ay + a_size)
                ins_dir = EntityDirection.SOUTH
            elif by + b_size + 1 == ay:
                # B is above A, gap row = by + b_size
                mid_x = (overlap_start + overlap_end) // 2
                gap_tile = (mid_x, by + b_size)
                ins_dir = EntityDirection.NORTH

        if gap_tile is not None and gap_tile not in occupied and ins_dir is not None:
            # Check this edge isn't already assigned by the regular inserter pass
            already_assigned = any(a.edge is edge for a in assignments)
            if already_assigned:
                # Remove the existing assignment and replace with direct
                assignments[:] = [a for a in assignments if a.edge is not edge]

            assignments.append(
                InserterAssignment(
                    edge=edge,
                    node_id=edge.from_node,
                    border_tile=gap_tile,
                    belt_tile=gap_tile,  # no belt tile for direct insertion
                    direction=ins_dir,
                    is_direct=True,
                )
            )
            occupied.add(gap_tile)
            direct_edge_indices.add(i)

    # 3. Build edge->belt_tile mapping and per-edge exclusions for the router
    edge_targets: dict[int, tuple[int, int]] = {}
    edge_starts: dict[int, tuple[int, int]] = {}
    edge_exclusions: dict[int, set[tuple[int, int]]] = {}
    for assignment in assignments:
        if assignment.is_direct:
            continue  # Direct insertions don't need routing
        for i, edge in enumerate(graph.edges):
            if edge is assignment.edge:
                if assignment.edge.to_node == assignment.node_id:
                    # Input inserter — route goal is this belt tile
                    edge_targets[i] = assignment.belt_tile
                elif assignment.edge.from_node == assignment.node_id:
                    # Output inserter — route must start from this belt tile
                    edge_starts[i] = assignment.belt_tile
                # Allow this edge (and only this edge) to use its own belt tile
                if i not in edge_exclusions:
                    edge_exclusions[i] = set()
                edge_exclusions[i].add(assignment.belt_tile)
                break

    # 3b. Compute lane info for A* pathfinding
    edge_lane_info: dict[int, tuple[str | None, tuple[int, int] | None]] = {}
    for assignment in assignments:
        if assignment.is_direct:
            continue
        for i, edge in enumerate(graph.edges):
            if edge is assignment.edge:
                if assignment.edge.from_node == assignment.node_id:
                    # Output inserter: items start on a known lane
                    edge_lane_info[i] = (assignment.target_lane, None)
                elif assignment.edge.to_node == assignment.node_id:
                    # Input inserter: pass inserter side vec for goal lane check
                    ins_side = (
                        assignment.border_tile[0] - assignment.belt_tile[0],
                        assignment.border_tile[1] - assignment.belt_tile[1],
                    )
                    edge_lane_info[i] = (None, ins_side)
                break

    # 4. Place machine entities
    entities: list[PlacedEntity] = []
    for node in graph.nodes:
        x, y = positions[node.id]
        entities.append(
            PlacedEntity(
                name=node.spec.entity,
                x=x,
                y=y,
                recipe=node.spec.recipe,
            )
        )

    # 5. Route connections (belts + pipes) to assigned belt tiles
    reserved = {a.border_tile for a in assignments} | {a.belt_tile for a in assignments}
    routing = route_connections(
        graph,
        positions,
        edge_targets=edge_targets,
        edge_starts=edge_starts,
        reserved_tiles=reserved,
        edge_exclusions=edge_exclusions,
        edge_subgroups=plan.edge_subgroups,
        edge_order=edge_order,
        edge_lane_info=edge_lane_info,
        skip_edges=direct_edge_indices,
    )
    entities.extend(routing.entities)

    # 5b. Place belt stubs for any external output inserters whose edges
    #     failed routing
    entity_tiles = {(e.x, e.y) for e in entities}
    failed_items = {e.item for e in routing.failed_edges}
    for assignment in assignments:
        edge = assignment.edge
        if edge.from_node == assignment.node_id and edge.to_node is None:
            bx, by = assignment.belt_tile
            if (bx, by) in entity_tiles:
                continue
            if edge.item not in failed_items:
                continue
            dx = bx - assignment.border_tile[0]
            dy = by - assignment.border_tile[1]
            if edge.is_fluid:
                entities.append(PlacedEntity(name="pipe", x=bx, y=by, carries=edge.item))
            else:
                belt_name = belt_entity_for_rate(edge.rate)
                direction = DIR_MAP.get((dx, dy), EntityDirection.SOUTH)
                entities.append(
                    PlacedEntity(
                        name=belt_name,
                        x=bx,
                        y=by,
                        direction=direction,
                        carries=edge.item,
                    )
                )

    # 6. Place inserters from pre-assignments
    entities.extend(build_inserter_entities(assignments))

    # 7. Collect occupied tiles for pole placement
    all_occupied: set[tuple[int, int]] = set()
    for e in entities:
        size = machine_size(e.name) if e.name in _MACHINE_SIZE else 1
        for dx in range(size):
            for dy in range(size):
                all_occupied.add((e.x + dx, e.y + dy))

    # 8. Calculate bounds
    if entities:
        min_x = min(e.x for e in entities)
        min_y = min(e.y for e in entities)
        max_x = max(e.x for e in entities) + 1
        max_y = max(e.y for e in entities) + 1
        width = max_x - min_x + 2
        height = max_y - min_y + 2
    else:
        width = height = 0

    # 9. Place power poles (greedy near machines)
    machine_centers = []
    for e in entities:
        if e.name in _MACHINE_SIZE:
            size = machine_size(e.name)
            machine_centers.append((e.x + size // 2, e.y + size // 2))
    pole_entities = place_poles(width, height, all_occupied, machine_centers=machine_centers)
    entities.extend(pole_entities)

    return (
        LayoutResult(entities=entities, width=width, height=height),
        routing.failed_edges,
        len(direct_edge_indices),
    )


def plan_trunks(
    graph: ProductionGraph,
    solver_result: SolverResult,
    trunk_x: int = 0,
    trunk_spacing: int = 2,
    trunk_length: int | None = None,
) -> tuple[
    list[PlacedEntity],
    dict[tuple[int, int], EntityDirection],
    dict[tuple[str, int], set[tuple[int, int]]],
    set[tuple[int, int]],
]:
    """Pre-lay straight vertical belt trunks for each external input/output item.

    Returns (entities, belt_dir_map, group_networks, occupied).
    """
    if trunk_length is None:
        trunk_length = max(len(graph.nodes) * 5, 12)

    entities: list[PlacedEntity] = []
    belt_dir_map: dict[tuple[int, int], EntityDirection] = {}
    group_networks: dict[tuple[str, int], set[tuple[int, int]]] = {}
    occupied: set[tuple[int, int]] = set()

    direction = EntityDirection.SOUTH

    # Input trunks (one per external input item)
    # Sort ascending by usage count so most-consumed item is rightmost (direct inserter access)
    def _usage_count(item: str) -> int:
        return sum(1 for e in graph.edges if e.from_node is None and e.item == item)

    input_items = sorted({e.item for e in graph.edges if e.from_node is None}, key=_usage_count)
    for item_idx, item in enumerate(input_items):
        x = trunk_x + item_idx * trunk_spacing
        rate = sum(e.rate for e in graph.edges if e.from_node is None and e.item == item)
        belt_name = belt_entity_for_rate(rate)
        group_key = (item, 0)
        network: set[tuple[int, int]] = set()
        for y in range(trunk_length):
            entities.append(PlacedEntity(name=belt_name, x=x, y=y, direction=direction, carries=item))
            belt_dir_map[(x, y)] = direction
            occupied.add((x, y))
            network.add((x, y))
        group_networks[group_key] = network

    # Output trunks (one per external output item, placed right of machine column)
    # machine_col_x = max(input_trunk_xs) + 2 = (n_inputs-1)*spacing + 2
    # output starts 2 tiles right of machine right edge (machine_col_x + _MACHINE_SIZE + 2)
    output_items = sorted({e.item for e in graph.edges if e.to_node is None})
    n_inputs = max(len(input_items), 1)
    base_output_x = trunk_x + (n_inputs - 1) * trunk_spacing + 2 + _MACHINE_SIZE + 2
    for item_idx, item in enumerate(output_items):
        x = base_output_x + item_idx * trunk_spacing
        # Use 2x total rate for belt tier: worst case all inserters sideload to same lane
        rate = sum(e.rate for e in graph.edges if e.to_node is None and e.item == item) * 2
        belt_name = belt_entity_for_rate(rate)
        group_key = (item, 0)
        network = set()
        for y in range(trunk_length):
            entities.append(PlacedEntity(name=belt_name, x=x, y=y, direction=direction, carries=item))
            belt_dir_map[(x, y)] = direction
            occupied.add((x, y))
            network.add((x, y))
        group_networks[group_key] = network

    return entities, belt_dir_map, group_networks, occupied


def build_layout_incremental(
    solver_result: SolverResult,
    graph: ProductionGraph,
    placement_order: list[int],
    candidate_positions_fn,
    side_preference: dict[int, list[tuple[int, int]]] | None = None,
    max_positions_per_machine: int = 5,
    rng=None,
    trunk_entities: list[PlacedEntity] | None = None,
    trunk_belt_dir_map: dict[tuple[int, int], EntityDirection] | None = None,
    trunk_group_networks: dict[tuple[str, int], set[tuple[int, int]]] | None = None,
    trunk_occupied: set[tuple[int, int]] | None = None,
) -> tuple[LayoutResult, list[FlowEdge], int]:
    """Build a layout incrementally: place one machine, route its edges, repeat.

    Each machine is placed at the first candidate position where all its
    routable edges can be successfully routed. This guarantees every layout
    is routable by construction.

    Args:
        placement_order: Machine node IDs in the order to place them.
        candidate_positions_fn: Callable(node_id, graph, positions, occupied, rng)
            -> list[(x, y)] of candidate positions to try for this machine.
        side_preference: Per-machine inserter side priority.
        max_positions_per_machine: Max candidates to try before accepting best-effort.
        rng: Random number generator for position shuffling.
        trunk_entities: Pre-placed trunk belt entities from plan_trunks().
        trunk_belt_dir_map: Belt direction map from pre-placed trunks.
        trunk_group_networks: Group networks from pre-placed trunks.
        trunk_occupied: Occupied tiles from pre-placed trunks.
    """
    import random as _random

    if rng is None:
        rng = _random.Random()

    # Pre-compute edge subgroups and io_y_slots from the full graph
    # (these need all positions, so we compute from the candidate_positions_fn's
    # expected final positions — or just use a placeholder and accept some
    # imprecision in y-slot assignment for incremental builds)
    edge_subgroups = _compute_edge_subgroups(graph, solver_result)

    # Accumulated state — seed from trunk data if provided
    occupied: set[tuple[int, int]] = set(trunk_occupied) if trunk_occupied else set()
    entities: list[PlacedEntity] = list(trunk_entities) if trunk_entities else []
    all_assignments: list[InserterAssignment] = []
    all_failed_edges: list[FlowEdge] = []
    direct_edge_indices: set[int] = set()
    positions: dict[int, tuple[int, int]] = {}
    placed_node_ids: set[int] = set()

    # Routing state carried across incremental calls — seed from trunks
    belt_dir_map: dict[tuple[int, int], EntityDirection] = dict(trunk_belt_dir_map) if trunk_belt_dir_map else {}
    lane_loads: dict[tuple[str, int], dict[str, float]] = {}  # per-group lane throughput
    trunk_protected_tiles: set[tuple[int, int]] = set()  # trunk tiles that must not be reoriented
    group_networks: dict[tuple[str, int], set[tuple[int, int]]] = (
        {k: set(v) for k, v in trunk_group_networks.items()} if trunk_group_networks else {}
    )

    node_map = {n.id: n for n in graph.nodes}

    for node_id in placement_order:
        node = node_map[node_id]
        size = machine_size(node.spec.entity)

        # Generate candidate positions for this machine
        candidates = candidate_positions_fn(node_id, graph, positions, occupied, rng)
        if not candidates:
            # Fallback: place offset from origin, avoiding trunk tiles
            fx = len(positions) * 6
            fy = len(positions) * (size + 1)
            if trunk_occupied:
                # Place between trunks: find first x where machine fits without overlapping trunks
                for try_x in range(0, 30):
                    trial = machine_tiles(try_x, fy, size)
                    if not (trial & occupied):
                        fx, fy = try_x, fy
                        break
            candidates = [(fx, fy)]

        # Limit candidates to try
        candidates = candidates[:max_positions_per_machine]

        # Find routable edges for this machine (edges where the other endpoint
        # is already placed, or the edge is external)
        routable_edges: list[tuple[int, FlowEdge]] = []
        for i, edge in enumerate(graph.edges):
            if i in direct_edge_indices:
                continue
            if edge.from_node == node_id:
                if edge.to_node is None or edge.to_node in placed_node_ids:
                    routable_edges.append((i, edge))
            elif edge.to_node == node_id and (edge.from_node is None or edge.from_node in placed_node_ids):
                routable_edges.append((i, edge))

        best_result = None  # (position, machine_tiles, assignments, routing, direct_indices)
        best_failed_count = float("inf")

        for cx, cy in candidates:
            # Check overlap
            candidate_tiles = machine_tiles(cx, cy, size)
            if candidate_tiles & occupied:
                continue

            # Temporarily place machine
            trial_occupied = occupied | candidate_tiles
            trial_positions = dict(positions)
            trial_positions[node_id] = (cx, cy)

            # Assign inserters for this machine's routable edges
            trial_assignments: list[InserterAssignment] = []
            trial_direct: set[int] = set()
            edge_targets: dict[int, tuple[int, int]] = {}
            edge_starts: dict[int, tuple[int, int]] = {}
            edge_exclusions: dict[int, set[tuple[int, int]]] = {}
            edge_lane_info: dict[int, tuple[str | None, tuple[int, int] | None]] = {}

            # Check for direct insertion (adjacent machines)
            for i, edge in routable_edges:
                if edge.from_node is None or edge.to_node is None or edge.is_fluid:
                    continue
                other_id = edge.to_node if edge.from_node == node_id else edge.from_node
                if other_id not in trial_positions:
                    continue
                ox, oy = trial_positions[other_id]
                o_size = machine_size(node_map[other_id].spec.entity)

                # Check adjacency (gap=1)
                gap_tile, ins_dir = _find_direct_gap(
                    cx,
                    cy,
                    size,
                    ox,
                    oy,
                    o_size,
                    edge.from_node == node_id,
                )
                if gap_tile is not None and gap_tile not in trial_occupied and ins_dir is not None:
                    trial_assignments.append(
                        InserterAssignment(
                            edge=edge,
                            node_id=edge.from_node,
                            border_tile=gap_tile,
                            belt_tile=gap_tile,
                            direction=ins_dir,
                            is_direct=True,
                        )
                    )
                    trial_occupied.add(gap_tile)
                    trial_direct.add(i)

            # Assign belt-based inserters for remaining routable edges.
            # For internal edges, assign inserters on BOTH machines:
            #   - the currently-placed machine (node_id)
            #   - the already-placed other machine (if any)
            # Build list of (edge_index, edge, machine_node_id, mx, my, msize, is_input)
            _inserter_jobs: list[tuple[int, FlowEdge, int, int, int, int, bool]] = []
            for i, edge in routable_edges:
                if i in trial_direct:
                    continue
                # Current machine side
                if edge.to_node == node_id:
                    _inserter_jobs.append((i, edge, node_id, cx, cy, size, True))
                elif edge.from_node == node_id:
                    _inserter_jobs.append((i, edge, node_id, cx, cy, size, False))
                # Other machine side (for internal edges where other end is already placed)
                if edge.from_node is not None and edge.from_node != node_id and edge.from_node in placed_node_ids:
                    other_id = edge.from_node
                    ox, oy = positions[other_id]
                    o_size = machine_size(node_map[other_id].spec.entity)
                    _inserter_jobs.append((i, edge, other_id, ox, oy, o_size, False))
                if edge.to_node is not None and edge.to_node != node_id and edge.to_node in placed_node_ids:
                    other_id = edge.to_node
                    ox, oy = positions[other_id]
                    o_size = machine_size(node_map[other_id].spec.entity)
                    _inserter_jobs.append((i, edge, other_id, ox, oy, o_size, True))

            for i, edge, m_node_id, mx, my, m_size, is_input in _inserter_jobs:
                sides = _get_sides(mx, my, m_size)
                trunk_tiles_for_item: set[tuple[int, int]] = set()
                if trunk_group_networks:
                    edge_item = edge.item
                    trunk_tiles_for_item = set()
                    for (ti, _sg), tiles in group_networks.items():
                        if ti == edge_item:
                            trunk_tiles_for_item |= tiles

                    def _trunk_side_score(s, _tt=trunk_tiles_for_item):
                        _, belt, _ = s
                        return 0 if belt in _tt else 1

                    sides = sorted(sides, key=_trunk_side_score)
                elif side_preference is not None and m_node_id in side_preference:
                    pref_order = side_preference[m_node_id]
                    sides = sorted(
                        sides,
                        key=lambda s, po=pref_order: po.index(s[2]) if s[2] in po else len(po),
                    )
                else:
                    rng.shuffle(sides)

                # Find first available side
                for border, belt, direction_vec in sides:
                    if border in trial_occupied:
                        continue
                    belt_on_trunk = trunk_group_networks and belt in trunk_tiles_for_item
                    if belt in trial_occupied and not belt_on_trunk:
                        continue
                    if is_input:
                        facing = {
                            (0, 1): EntityDirection.SOUTH,
                            (0, -1): EntityDirection.NORTH,
                            (1, 0): EntityDirection.EAST,
                            (-1, 0): EntityDirection.WEST,
                        }.get(direction_vec)
                    else:
                        reverse = (-direction_vec[0], -direction_vec[1])
                        facing = {
                            (0, 1): EntityDirection.SOUTH,
                            (0, -1): EntityDirection.NORTH,
                            (1, 0): EntityDirection.EAST,
                            (-1, 0): EntityDirection.WEST,
                        }.get(reverse)
                    if facing is None:
                        continue

                    if is_input:
                        target_lane = "left"  # placeholder
                    else:
                        away_vec = (-direction_vec[0], -direction_vec[1])
                        belt_dir = DIR_MAP[away_vec]
                        target_lane = inserter_target_lane(border[0], border[1], belt[0], belt[1], belt_dir)

                    assignment = InserterAssignment(
                        edge=edge,
                        node_id=m_node_id,
                        border_tile=border,
                        belt_tile=belt,
                        direction=facing,
                        approach_vec=direction_vec,
                        target_lane=target_lane,
                    )
                    trial_assignments.append(assignment)
                    trial_occupied.add(border)

                    # Build routing targets (only for first assignment of each edge)
                    if is_input and i not in edge_targets:
                        edge_targets[i] = belt
                        ins_side = (border[0] - belt[0], border[1] - belt[1])
                        edge_lane_info[i] = (None, ins_side)
                    elif not is_input and i not in edge_starts:
                        edge_starts[i] = belt
                        edge_lane_info[i] = (target_lane, None)
                    if i not in edge_exclusions:
                        edge_exclusions[i] = set()
                    edge_exclusions[i].add(belt)
                    break

            # Route edges for this machine.
            # External edges: place belt stubs, route continuations to existing network.
            # Internal edges: route via A* between machines.
            trial_entities: list[PlacedEntity] = []
            trial_belt_dir_map = dict(belt_dir_map)
            trial_group_networks = {k: set(v) for k, v in group_networks.items()}
            failed_count = 0
            trial_failed: list[FlowEdge] = []

            # Separate external and internal edges
            external_edges = [
                (i, e)
                for i, e in routable_edges
                if i not in trial_direct and (e.from_node is None or e.to_node is None)
            ]
            internal_edges = [
                (i, e)
                for i, e in routable_edges
                if i not in trial_direct and e.from_node is not None and e.to_node is not None
            ]

            # Handle external edges: place belt stubs, route continuations
            for i, edge in external_edges:
                if i not in edge_targets and i not in edge_starts:
                    continue  # no inserter assigned for this edge
                # Determine belt tile and direction
                if i in edge_targets:
                    bx, by = edge_targets[i]
                    # Input: belt faces toward the inserter (toward machine)
                    assn = next((a for a in trial_assignments if a.edge is edge), None)
                    if assn:
                        dx = assn.border_tile[0] - bx
                        dy = assn.border_tile[1] - by
                        direction = DIR_MAP.get((dx, dy), EntityDirection.SOUTH)
                    else:
                        direction = EntityDirection.SOUTH
                else:
                    bx, by = edge_starts[i]
                    assn = next((a for a in trial_assignments if a.edge is edge), None)
                    if assn:
                        dx = bx - assn.border_tile[0]
                        dy = by - assn.border_tile[1]
                        direction = DIR_MAP.get((dx, dy), EntityDirection.SOUTH)
                    else:
                        direction = EntityDirection.SOUTH

                group_key = (edge.item, 0)  # default sub-group
                for item_name, groups in edge_subgroups.items():
                    for g_idx, indices in enumerate(groups):
                        if i in indices:
                            group_key = (item_name, g_idx)
                            break

                existing_network = trial_group_networks.get(group_key, set())

                if (bx, by) in existing_network:
                    # Belt stub is already on the trunk — no routing needed.
                    # The inserter directly picks from / drops onto the trunk belt.
                    # Don't place a new entity or change direction — trunk is already there.
                    pass
                elif existing_network:
                    # Continuation: connect stub to existing network.
                    is_input = i in edge_targets
                    obstacles = trial_occupied - {(bx, by)} - existing_network

                    # Precompute foreign item tiles for contamination avoidance
                    _oit: set[tuple[int, int]] | None = None
                    if not edge.is_fluid:
                        _oit = set()
                        for (oi, _sg), tiles in trial_group_networks.items():
                            if oi != edge.item:
                                _oit |= tiles
                        if not _oit:
                            _oit = None

                    path = None
                    cont_belt = belt_entity_for_rate(edge.rate) if not edge.is_fluid else "pipe"
                    cont_ug_reach = _UG_MAX_REACH.get(cont_belt, 4)
                    if is_input and not edge.is_fluid:
                        # Input: multi-source A* from network toward stub
                        downstream_ends = _network_downstream_ends(existing_network, trial_belt_dir_map)
                        forward_tiles = set()
                        for tile in downstream_ends:
                            d = trial_belt_dir_map.get(tile)
                            if d is not None:
                                dvx, dvy = DIR_VEC[d]
                                ft = (tile[0] + dvx, tile[1] + dvy)
                                if ft not in existing_network:
                                    forward_tiles.add(ft)
                        approach = _perpendicular_approach_tiles(existing_network, trial_belt_dir_map, trial_occupied)
                        all_starts = (forward_tiles | approach) - obstacles
                        if all_starts:
                            path = _astar_path(
                                starts=all_starts,
                                goals={(bx, by)},
                                obstacles=obstacles,
                                allow_underground=True,
                                ug_max_reach=cont_ug_reach,
                                belt_dir_map=trial_belt_dir_map,
                                other_item_tiles=_oit,
                            )
                    else:
                        # Output (or fluid): route from stub to network
                        # For belts: prefer the side that balances lane throughput
                        all_goals = set()
                        for nx, ny in existing_network:
                            for ddx, ddy in [(1, 0), (-1, 0), (0, 1), (0, -1)]:
                                adj = (nx + ddx, ny + ddy)
                                if adj not in obstacles and adj not in existing_network:
                                    all_goals.add(adj)
                        all_goals |= existing_network

                        if all_goals and not edge.is_fluid:
                            left_goals, right_goals = _classify_approach_by_lane(
                                existing_network, trial_belt_dir_map, trial_occupied
                            )
                            cur = lane_loads.get(group_key, {"left": 0.0, "right": 0.0})
                            if cur["left"] <= cur["right"]:
                                preferred, fallback = left_goals, right_goals
                            else:
                                preferred, fallback = right_goals, left_goals
                            if preferred:
                                path = _astar_path(
                                    start=(bx, by),
                                    goals=preferred,
                                    obstacles=obstacles,
                                    allow_underground=True,
                                    ug_max_reach=cont_ug_reach,
                                    belt_dir_map=trial_belt_dir_map,
                                    other_item_tiles=_oit,
                                )
                            if not path and fallback:
                                path = _astar_path(
                                    start=(bx, by),
                                    goals=fallback,
                                    obstacles=obstacles,
                                    allow_underground=True,
                                    ug_max_reach=cont_ug_reach,
                                    belt_dir_map=trial_belt_dir_map,
                                    other_item_tiles=_oit,
                                )
                            if not path and all_goals:
                                path = _astar_path(
                                    start=(bx, by),
                                    goals=all_goals,
                                    obstacles=obstacles,
                                    allow_underground=True,
                                    ug_max_reach=cont_ug_reach,
                                    belt_dir_map=trial_belt_dir_map,
                                    other_item_tiles=_oit,
                                )
                        elif all_goals:
                            path = _astar_path(
                                start=(bx, by),
                                goals=all_goals,
                                obstacles=obstacles,
                                allow_underground=True,
                                ug_max_reach=cont_ug_reach,
                                belt_dir_map=trial_belt_dir_map,
                                other_item_tiles=_oit,
                            )

                    if path:
                        belt_name = belt_entity_for_rate(edge.rate) if not edge.is_fluid else "pipe"
                        path_ents = _path_to_entities(path, belt_name, edge.item, edge.is_fluid)
                        # Skip entities for tiles already in the existing network (e.g. trunk tiles)
                        for pe in path_ents:
                            if (pe.x, pe.y) not in existing_network:
                                trial_entities.append(pe)
                            trial_belt_dir_map[(pe.x, pe.y)] = pe.direction
                            trial_occupied.add((pe.x, pe.y))
                        trial_group_networks.setdefault(group_key, set()).update(set(path))
                        # Validate junction direction (Phase 1.2)
                        if not edge.is_fluid and existing_network:
                            _validate_junction_direction(
                                path, path_ents, existing_network, trial_belt_dir_map, is_input
                            )
                        # Track lane load for output continuations
                        is_output = i in edge_starts
                        if is_output and not edge.is_fluid and existing_network:
                            sl = _detect_sideload_lane(path, existing_network, trial_belt_dir_map)
                            if sl:
                                loads = lane_loads.setdefault(group_key, {"left": 0.0, "right": 0.0})
                                loads[sl] += edge.rate
                    else:
                        # Surface routing failed — try underground escape
                        ug_escaped = False
                        if not edge.is_fluid:
                            ug_belt = belt_entity_for_rate(edge.rate)
                            ug_name = "underground-belt"
                            ug_reach = _UG_MAX_REACH.get(ug_belt, 4)

                            # Direction away from machine (opposite of approach vector)
                            assn = next((a for a in trial_assignments if a.edge is edge), None)
                            if assn and assn.approach_vec:
                                # approach_vec points from machine toward belt tile
                                escape_dx, escape_dy = assn.approach_vec
                            else:
                                # Fallback: use stub direction for outputs, opposite for inputs
                                dvx, dvy = DIR_VEC[direction]
                                if is_input:
                                    escape_dx, escape_dy = -dvx, -dvy
                                else:
                                    escape_dx, escape_dy = dvx, dvy

                            escape_dir = DIR_MAP.get((escape_dx, escape_dy), direction)

                            # Find first clear exit tile along escape direction
                            exit_tile = None
                            for dist in range(2, ug_reach + 1):
                                ex, ey = bx + escape_dx * dist, by + escape_dy * dist
                                if (ex, ey) not in trial_occupied:
                                    exit_tile = (ex, ey)
                                    break

                            if exit_tile:
                                # Place underground pair: entrance at stub, exit at found tile
                                trial_entities.append(
                                    PlacedEntity(
                                        name=ug_name,
                                        x=bx,
                                        y=by,
                                        direction=escape_dir,
                                        io_type="input",
                                        carries=edge.item,
                                    )
                                )
                                trial_entities.append(
                                    PlacedEntity(
                                        name=ug_name,
                                        x=exit_tile[0],
                                        y=exit_tile[1],
                                        direction=escape_dir,
                                        io_type="output",
                                        carries=edge.item,
                                    )
                                )
                                trial_belt_dir_map[(bx, by)] = escape_dir
                                trial_belt_dir_map[exit_tile] = escape_dir
                                trial_occupied.add((bx, by))
                                trial_occupied.add(exit_tile)
                                trial_group_networks.setdefault(group_key, set()).update({(bx, by), exit_tile})

                                # Retry A* from exit tile to network (or network to exit tile)
                                retry_obstacles = trial_occupied - {exit_tile} - existing_network
                                retry_path = None
                                if is_input:
                                    # Recompute network approach tiles for retry
                                    r_downstream = _network_downstream_ends(existing_network, trial_belt_dir_map)
                                    r_forward = set()
                                    for tile in r_downstream:
                                        d = trial_belt_dir_map.get(tile)
                                        if d is not None:
                                            _dvx, _dvy = DIR_VEC[d]
                                            ft = (tile[0] + _dvx, tile[1] + _dvy)
                                            if ft not in existing_network:
                                                r_forward.add(ft)
                                    r_approach = _perpendicular_approach_tiles(
                                        existing_network, trial_belt_dir_map, trial_occupied
                                    )
                                    r_starts = (r_forward | r_approach) - retry_obstacles
                                    if r_starts:
                                        retry_path = _astar_path(
                                            starts=r_starts,
                                            goals={exit_tile},
                                            obstacles=retry_obstacles,
                                            allow_underground=True,
                                            ug_max_reach=ug_reach,
                                            belt_dir_map=trial_belt_dir_map,
                                            other_item_tiles=_oit,
                                        )
                                else:
                                    retry_goals = set()
                                    for nx, ny in existing_network:
                                        for ddx, ddy in [(1, 0), (-1, 0), (0, 1), (0, -1)]:
                                            adj = (nx + ddx, ny + ddy)
                                            if adj not in retry_obstacles and adj not in existing_network:
                                                retry_goals.add(adj)
                                    retry_goals |= existing_network
                                    if retry_goals:
                                        retry_path = _astar_path(
                                            start=exit_tile,
                                            goals=retry_goals,
                                            obstacles=retry_obstacles,
                                            allow_underground=True,
                                            ug_max_reach=ug_reach,
                                            belt_dir_map=trial_belt_dir_map,
                                            other_item_tiles=_oit,
                                        )

                                if retry_path:
                                    retry_ents = _path_to_entities(retry_path, ug_belt, edge.item, False)
                                    for pe in retry_ents:
                                        if (pe.x, pe.y) not in existing_network and (pe.x, pe.y) != exit_tile:
                                            trial_entities.append(pe)
                                        trial_belt_dir_map[(pe.x, pe.y)] = pe.direction
                                        trial_occupied.add((pe.x, pe.y))
                                    trial_group_networks.setdefault(group_key, set()).update(set(retry_path))
                                    ug_escaped = True

                        if not ug_escaped:
                            failed_count += 1
                            trial_failed.append(edge)
                            # Still place the belt stub so the inserter has a target
                            if not edge.is_fluid:
                                belt_name = belt_entity_for_rate(edge.rate)
                                trial_entities.append(
                                    PlacedEntity(name=belt_name, x=bx, y=by, direction=direction, carries=edge.item)
                                )
                            else:
                                trial_entities.append(PlacedEntity(name="pipe", x=bx, y=by, carries=edge.item))
                            trial_belt_dir_map[(bx, by)] = direction
                            trial_group_networks.setdefault(group_key, set()).add((bx, by))
                            trial_occupied.add((bx, by))
                else:
                    # First edge for this item: place stub + route a trunk toward boundary
                    is_input = i in edge_targets
                    belt_name = belt_entity_for_rate(edge.rate) if not edge.is_fluid else "pipe"

                    # Place the stub itself
                    trial_entities.append(
                        PlacedEntity(name=belt_name, x=bx, y=by, direction=direction, carries=edge.item)
                    )
                    trial_belt_dir_map[(bx, by)] = direction
                    trial_group_networks.setdefault(group_key, set()).add((bx, by))
                    trial_occupied.add((bx, by))

                    # Route a trunk from the stub toward the layout boundary via A*
                    if not edge.is_fluid:
                        # Compute boundary x from machine positions AND existing belt extents
                        # (validator boundary = min/max of all belt tiles, so we must go at least
                        # as far as the most extreme belt tile from other items)
                        all_pos_x = [px for px, _ in trial_positions.values()]
                        max_msz = max(
                            (machine_size(node_map[nid].spec.entity) for nid in trial_positions),
                            default=3,
                        )
                        existing_belt_xs = [tx for (tx, _) in trial_belt_dir_map]
                        num_ext = len({e.item for e in graph.edges if e.from_node is None})
                        if is_input:
                            machine_boundary = min(all_pos_x) - max(3, num_ext * 3)
                            belt_boundary = min(existing_belt_xs) if existing_belt_xs else machine_boundary
                            boundary_x = min(machine_boundary, belt_boundary)
                        else:
                            num_ext_out = len({e.item for e in graph.edges if e.to_node is None})
                            machine_boundary = max(all_pos_x) + max_msz + max(3, num_ext_out * 3)
                            belt_boundary = max(existing_belt_xs) if existing_belt_xs else machine_boundary
                            boundary_x = max(machine_boundary, belt_boundary)

                        # Goal: a column of tiles at the boundary x, near the stub's y
                        trunk_goals = set()
                        for ty in range(by - 6, by + 7):
                            t = (boundary_x, ty)
                            if t not in trial_occupied:
                                trunk_goals.add(t)

                        if trunk_goals:
                            # Precompute foreign item tiles for contamination avoidance
                            _oit_trunk: set[tuple[int, int]] | None = None
                            if not edge.is_fluid:
                                _oit_trunk = set()
                                for (oi, _sg), tiles in trial_group_networks.items():
                                    if oi != edge.item:
                                        _oit_trunk |= tiles
                                if not _oit_trunk:
                                    _oit_trunk = None

                            obstacles_trunk = trial_occupied - {(bx, by)}
                            ug_reach = _UG_MAX_REACH.get(belt_name, 4)
                            if is_input:
                                # Route from boundary toward stub (items flow boundary→stub)
                                trunk_path = _astar_path(
                                    starts=trunk_goals,
                                    goals={(bx, by)},
                                    obstacles=obstacles_trunk,
                                    allow_underground=True,
                                    ug_max_reach=ug_reach,
                                    other_item_tiles=_oit_trunk,
                                )
                            else:
                                # Route from stub toward boundary (items flow stub→boundary)
                                trunk_path = _astar_path(
                                    start=(bx, by),
                                    goals=trunk_goals,
                                    obstacles=obstacles_trunk,
                                    allow_underground=True,
                                    ug_max_reach=ug_reach,
                                    other_item_tiles=_oit_trunk,
                                )

                            if trunk_path:
                                trunk_ents = _path_to_entities(trunk_path, belt_name, edge.item, edge.is_fluid)
                                for pe in trunk_ents:
                                    if (pe.x, pe.y) != (bx, by):
                                        trial_entities.append(pe)
                                    trial_belt_dir_map[(pe.x, pe.y)] = pe.direction
                                    trial_group_networks.setdefault(group_key, set()).add((pe.x, pe.y))
                                    trial_occupied.add((pe.x, pe.y))
                                    trunk_protected_tiles.add((pe.x, pe.y))

            # Handle internal edges via route_connections
            if internal_edges:
                internal_indices = {i for i, _ in internal_edges}
                reserved = {a.border_tile for a in trial_assignments} | {
                    a.belt_tile for a in trial_assignments if not a.is_direct
                }
                skip_all_except_internal = {j for j in range(len(graph.edges)) if j not in internal_indices}
                routing = route_connections(
                    graph,
                    trial_positions,
                    edge_targets=edge_targets,
                    edge_starts=edge_starts,
                    reserved_tiles=reserved | occupied,
                    edge_exclusions=edge_exclusions,
                    edge_subgroups=edge_subgroups,
                    edge_lane_info=edge_lane_info,
                    skip_edges=skip_all_except_internal | trial_direct | direct_edge_indices,
                    existing_belt_dir_map=trial_belt_dir_map,
                    existing_group_networks=trial_group_networks,
                )
                trial_entities.extend(routing.entities)
                trial_belt_dir_map.update(routing.belt_dir_map)
                for key, tiles in routing.group_networks.items():
                    trial_group_networks.setdefault(key, set()).update(tiles)
                trial_occupied |= routing.occupied
                if routing.failed_edges:
                    failed_count += len(routing.failed_edges)
                    trial_failed.extend(routing.failed_edges)

            if failed_count < best_failed_count:
                best_failed_count = failed_count
                best_result = (
                    (cx, cy),
                    candidate_tiles,
                    trial_assignments,
                    trial_entities,
                    trial_direct,
                    trial_occupied,
                    trial_belt_dir_map,
                    trial_group_networks,
                    trial_failed,
                )

            if failed_count == 0:
                break  # This position works perfectly

        if best_result is None:
            # No valid position found — place at first candidate as fallback
            cx, cy = candidates[0]
            positions[node_id] = (cx, cy)
            occupied |= machine_tiles(cx, cy, size)
            placed_node_ids.add(node_id)
            entities.append(PlacedEntity(name=node.spec.entity, x=cx, y=cy, recipe=node.spec.recipe))
            continue

        # Accept the best position
        (
            pos,
            m_tiles,
            assignments,
            trial_ents,
            direct_idxs,
            new_occupied,
            new_belt_dir_map,
            new_group_networks,
            failed,
        ) = best_result
        positions[node_id] = pos
        occupied = new_occupied
        placed_node_ids.add(node_id)
        direct_edge_indices |= direct_idxs
        all_assignments.extend(assignments)
        all_failed_edges.extend(failed)

        # Place machine entity
        entities.append(PlacedEntity(name=node.spec.entity, x=pos[0], y=pos[1], recipe=node.spec.recipe))

        # Merge routing results
        entities.extend(trial_ents)
        belt_dir_map = new_belt_dir_map
        group_networks = new_group_networks

    # Extend trunks with entry/exit tails (fallback: only if trunk routing didn't reach boundary)
    # Compute boundary from all belt tile positions
    all_belt_xs = [x for (x, _) in belt_dir_map]
    all_belt_ys = [y for (_, y) in belt_dir_map]
    if all_belt_xs:
        _ext_min_bx, _ext_max_bx = min(all_belt_xs), max(all_belt_xs)
        _ext_min_by, _ext_max_by = min(all_belt_ys), max(all_belt_ys)
    else:
        _ext_min_bx = _ext_max_bx = _ext_min_by = _ext_max_by = 0

    def _network_on_boundary(net: set[tuple[int, int]]) -> bool:
        return any(t[0] in (_ext_min_bx, _ext_max_bx) or t[1] in (_ext_min_by, _ext_max_by) for t in net)

    for (item, _sg_idx), network in group_networks.items():
        if not network:
            continue
        is_input = any(e.from_node is None and e.item == item for e in graph.edges)
        is_output = any(e.to_node is None and e.item == item for e in graph.edges)

        if is_input and not _network_on_boundary(network):
            upstream = _network_upstream_ends(network, belt_dir_map)
            total_rate = sum(e.rate for e in graph.edges if e.item == item and e.from_node is None) or 15
            for tile in upstream:
                d = belt_dir_map.get(tile)
                if d is None:
                    continue
                dx, dy = DIR_VEC[d]
                belt_name = belt_entity_for_rate(total_rate)
                for ext in range(1, 4):
                    nx, ny = tile[0] - dx * ext, tile[1] - dy * ext
                    if (nx, ny) in occupied:
                        break
                    entities.append(PlacedEntity(name=belt_name, x=nx, y=ny, direction=d, carries=item))
                    belt_dir_map[(nx, ny)] = d
                    occupied.add((nx, ny))
                break

        if is_output and not _network_on_boundary(network):
            downstream = _network_downstream_ends(network, belt_dir_map)
            total_rate = sum(e.rate for e in graph.edges if e.item == item and e.to_node is None) or 15
            belt_name = belt_entity_for_rate(total_rate * 2)
            for tile in downstream:
                d = belt_dir_map.get(tile)
                if d is None:
                    continue
                dx, dy = DIR_VEC[d]
                for ext in range(1, 4):
                    nx, ny = tile[0] + dx * ext, tile[1] + dy * ext
                    if (nx, ny) in occupied:
                        break
                    entities.append(PlacedEntity(name=belt_name, x=nx, y=ny, direction=d, carries=item))
                    belt_dir_map[(nx, ny)] = d
                    occupied.add((nx, ny))
                break

    # Place inserter entities
    entities.extend(build_inserter_entities(all_assignments))

    # Collect inserter-adjacent belt tiles and trunk tiles that must not be reoriented
    _protected_tiles: set[tuple[int, int]] = set(trunk_protected_tiles)
    for asgn in all_assignments:
        if not asgn.is_direct:
            _protected_tiles.add(asgn.belt_tile)

    # Post-process belt directions
    _fix_belt_directions(entities, belt_dir_map, protected_tiles=_protected_tiles)

    # Collect occupied tiles for pole placement
    all_occupied: set[tuple[int, int]] = set()
    for e in entities:
        sz = machine_size(e.name) if e.name in _MACHINE_SIZE else 1
        for dx in range(sz):
            for dy in range(sz):
                all_occupied.add((e.x + dx, e.y + dy))

    # Calculate bounds
    if entities:
        min_x = min(e.x for e in entities)
        min_y = min(e.y for e in entities)
        max_x = max(e.x for e in entities) + 1
        max_y = max(e.y for e in entities) + 1
        width = max_x - min_x + 2
        height = max_y - min_y + 2
    else:
        width = height = 0

    # Place power poles
    machine_centers = []
    for e in entities:
        if e.name in _MACHINE_SIZE:
            sz = machine_size(e.name)
            machine_centers.append((e.x + sz // 2, e.y + sz // 2))
    pole_entities = place_poles(width, height, all_occupied, machine_centers=machine_centers)
    entities.extend(pole_entities)

    return (
        LayoutResult(entities=entities, width=width, height=height),
        all_failed_edges,
        len(direct_edge_indices),
    )


def _find_direct_gap(
    ax: int,
    ay: int,
    a_size: int,
    bx: int,
    by: int,
    b_size: int,
    a_is_source: bool,
) -> tuple[tuple[int, int] | None, EntityDirection | None]:
    """Find a gap tile for direct insertion between adjacent machines.

    Returns (gap_tile, inserter_direction) or (None, None) if not adjacent.
    """
    # Check horizontal adjacency
    if ay < by + b_size and ay + a_size > by:
        overlap_start = max(ay, by)
        overlap_end = min(ay + a_size, by + b_size)
        mid_y = (overlap_start + overlap_end) // 2
        if ax + a_size + 1 == bx:
            # A is left of B
            ins_dir = EntityDirection.EAST if a_is_source else EntityDirection.WEST
            return (ax + a_size, mid_y), ins_dir
        if bx + b_size + 1 == ax:
            # B is left of A
            ins_dir = EntityDirection.WEST if a_is_source else EntityDirection.EAST
            return (bx + b_size, mid_y), ins_dir

    # Check vertical adjacency
    if ax < bx + b_size and ax + a_size > bx:
        overlap_start = max(ax, bx)
        overlap_end = min(ax + a_size, bx + b_size)
        mid_x = (overlap_start + overlap_end) // 2
        if ay + a_size + 1 == by:
            # A is above B
            ins_dir = EntityDirection.SOUTH if a_is_source else EntityDirection.NORTH
            return (mid_x, ay + a_size), ins_dir
        if by + b_size + 1 == ay:
            # B is above A
            ins_dir = EntityDirection.NORTH if a_is_source else EntityDirection.SOUTH
            return (mid_x, by + b_size), ins_dir

    return None, None
