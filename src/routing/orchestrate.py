"""Shared layout orchestration: inserters -> routing -> poles."""

from __future__ import annotations

from ..models import EntityDirection, LayoutResult, PlacedEntity, SolverResult
from .common import _MACHINE_SIZE, DIR_MAP, belt_entity_for_rate, inserter_target_lane, machine_size, machine_tiles
from .graph import FlowEdge, ProductionGraph
from .inserters import InserterAssignment, assign_inserter_positions, build_inserter_entities
from .poles import place_poles
from .router import route_connections


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
        a_size = machine_size(
            next(n for n in graph.nodes if n.id == edge.from_node).spec.entity
        )
        b_size = machine_size(
            next(n for n in graph.nodes if n.id == edge.to_node).spec.entity
        )

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
