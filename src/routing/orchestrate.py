"""Shared layout orchestration: inserters -> routing -> poles."""

from __future__ import annotations

from ..models import EntityDirection, LayoutResult, PlacedEntity, SolverResult
from .common import (
    _MACHINE_SIZE,
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
    _fix_belt_directions,
    _path_to_entities,
    route_connections,
)


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


def build_layout_incremental(
    solver_result: SolverResult,
    graph: ProductionGraph,
    placement_order: list[int],
    candidate_positions_fn,
    side_preference: dict[int, list[tuple[int, int]]] | None = None,
    max_positions_per_machine: int = 5,
    rng=None,
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
    """
    import random as _random

    if rng is None:
        rng = _random.Random()

    # Pre-compute edge subgroups and io_y_slots from the full graph
    # (these need all positions, so we compute from the candidate_positions_fn's
    # expected final positions — or just use a placeholder and accept some
    # imprecision in y-slot assignment for incremental builds)
    edge_subgroups = _compute_edge_subgroups(graph, solver_result)

    # Accumulated state
    occupied: set[tuple[int, int]] = set()
    entities: list[PlacedEntity] = []
    all_assignments: list[InserterAssignment] = []
    all_failed_edges: list[FlowEdge] = []
    direct_edge_indices: set[int] = set()
    positions: dict[int, tuple[int, int]] = {}
    placed_node_ids: set[int] = set()

    # Routing state carried across incremental calls
    belt_dir_map: dict[tuple[int, int], EntityDirection] = {}
    group_networks: dict[tuple[str, int], set[tuple[int, int]]] = {}

    node_map = {n.id: n for n in graph.nodes}

    for node_id in placement_order:
        node = node_map[node_id]
        size = machine_size(node.spec.entity)

        # Generate candidate positions for this machine
        candidates = candidate_positions_fn(node_id, graph, positions, occupied, rng)
        if not candidates:
            # Fallback: place at origin offset
            candidates = [(len(positions) * 6, 0)]

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

            # Assign belt-based inserters for remaining routable edges
            for i, edge in routable_edges:
                if i in trial_direct:
                    continue
                # Determine which node this inserter serves
                if edge.to_node == node_id:
                    # Input inserter on this machine
                    mx, my = cx, cy
                elif edge.from_node == node_id:
                    # Output inserter on this machine
                    mx, my = cx, cy
                else:
                    continue

                sides = _get_sides(mx, my, size)
                if side_preference is not None and node_id in side_preference:
                    pref_order = side_preference[node_id]
                    sides = sorted(
                        sides,
                        key=lambda s, po=pref_order: po.index(s[2]) if s[2] in po else len(po),
                    )
                else:
                    rng.shuffle(sides)

                # Find first available side
                for border, belt, direction_vec in sides:
                    if border in trial_occupied or belt in trial_occupied:
                        continue
                    is_input = edge.to_node == node_id
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
                        node_id=node_id,
                        border_tile=border,
                        belt_tile=belt,
                        direction=facing,
                        approach_vec=direction_vec,
                        target_lane=target_lane,
                    )
                    trial_assignments.append(assignment)
                    trial_occupied.add(border)

                    # Build routing targets
                    if is_input:
                        edge_targets[i] = belt
                        ins_side = (border[0] - belt[0], border[1] - belt[1])
                        edge_lane_info[i] = (None, ins_side)
                    else:
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

                if existing_network:
                    # Continuation: A* pathfind from belt tile to existing network
                    obstacles = trial_occupied - {(bx, by)} - existing_network
                    # Goal: any tile adjacent to the existing network that's free
                    goals = set()
                    for nx, ny in existing_network:
                        for ddx, ddy in [(1, 0), (-1, 0), (0, 1), (0, -1)]:
                            adj = (nx + ddx, ny + ddy)
                            if adj not in obstacles and adj not in existing_network:
                                goals.add(adj)
                    # Also allow routing directly into existing network tiles
                    goals |= existing_network

                    path = None
                    if goals:
                        # Precompute foreign item tiles for contamination avoidance
                        _oit: set[tuple[int, int]] | None = None
                        if not edge.is_fluid:
                            _oit = set()
                            for (oi, _sg), tiles in trial_group_networks.items():
                                if oi != edge.item:
                                    _oit |= tiles
                            if not _oit:
                                _oit = None
                        path = _astar_path(
                            (bx, by),
                            goals,
                            obstacles,
                            allow_underground=True,
                            ug_max_reach=6,
                            belt_dir_map=trial_belt_dir_map,
                            other_item_tiles=_oit,
                        )

                    if path:
                        belt_name = belt_entity_for_rate(edge.rate) if not edge.is_fluid else "pipe"
                        path_ents = _path_to_entities(path, belt_name, edge.item, edge.is_fluid)
                        trial_entities.extend(path_ents)
                        for pe in path_ents:
                            trial_belt_dir_map[(pe.x, pe.y)] = pe.direction
                            trial_occupied.add((pe.x, pe.y))
                        trial_group_networks.setdefault(group_key, set()).update(set(path))
                    else:
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
                    # First edge for this item: place a belt stub
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

    # Extend trunks with entry/exit tails so the validator sees boundary connections
    from .router import _network_downstream_ends, _network_upstream_ends

    for (item, _sg_idx), network in group_networks.items():
        if not network:
            continue
        # Determine if this is an input or output item
        is_input = any(e.from_node is None and e.item == item for e in graph.edges)
        is_output = any(e.to_node is None and e.item == item for e in graph.edges)

        if is_input:
            # Extend upstream end backward by a few tiles
            upstream = _network_upstream_ends(network, belt_dir_map)
            for tile in upstream:
                d = belt_dir_map.get(tile)
                if d is None:
                    continue
                dx, dy = DIR_VEC[d]
                for ext in range(1, 4):
                    nx, ny = tile[0] - dx * ext, tile[1] - dy * ext
                    if (nx, ny) in occupied:
                        break
                    belt_name = belt_entity_for_rate(
                        next((e.rate for e in graph.edges if e.item == item and e.from_node is None), 15)
                    )
                    entities.append(PlacedEntity(name=belt_name, x=nx, y=ny, direction=d, carries=item))
                    belt_dir_map[(nx, ny)] = d
                    occupied.add((nx, ny))
                break  # only extend one upstream end

        if is_output:
            # Extend downstream end forward by a few tiles
            downstream = _network_downstream_ends(network, belt_dir_map)
            for tile in downstream:
                d = belt_dir_map.get(tile)
                if d is None:
                    continue
                dx, dy = DIR_VEC[d]
                for ext in range(1, 4):
                    nx, ny = tile[0] + dx * ext, tile[1] + dy * ext
                    if (nx, ny) in occupied:
                        break
                    belt_name = belt_entity_for_rate(
                        next((e.rate for e in graph.edges if e.item == item and e.to_node is None), 15)
                    )
                    entities.append(PlacedEntity(name=belt_name, x=nx, y=ny, direction=d, carries=item))
                    belt_dir_map[(nx, ny)] = d
                    occupied.add((nx, ny))
                break  # only extend one downstream end

    # Place inserter entities
    entities.extend(build_inserter_entities(all_assignments))

    # Post-process belt directions
    _fix_belt_directions(entities, belt_dir_map)

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
