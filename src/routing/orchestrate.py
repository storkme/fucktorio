"""Shared layout orchestration: inserters -> routing -> poles."""

from __future__ import annotations

from ..layout.poles import place_poles
from ..models import EntityDirection, LayoutResult, PlacedEntity, SolverResult
from .common import DIR_MAP, _MACHINE_SIZE, belt_entity_for_rate, machine_size, machine_tiles
from .graph import FlowEdge, ProductionGraph
from .inserters import assign_inserter_positions, build_inserter_entities
from .router import route_connections


def build_layout(
    solver_result: SolverResult,
    graph: ProductionGraph,
    positions: dict[int, tuple[int, int]],
    side_strategy: str = "top_bottom",
) -> tuple[LayoutResult, list[FlowEdge]]:
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
        graph, positions, occupied, solver_result=solver_result, side_strategy=side_strategy
    )
    assignments = plan.assignments

    # 3. Build edge->belt_tile mapping and per-edge exclusions for the router
    edge_targets: dict[int, tuple[int, int]] = {}
    edge_starts: dict[int, tuple[int, int]] = {}
    edge_exclusions: dict[int, set[tuple[int, int]]] = {}
    for assignment in assignments:
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

    # 9. Place power poles
    pole_entities = place_poles(width, height, all_occupied)
    entities.extend(pole_entities)

    return (
        LayoutResult(entities=entities, width=width, height=height),
        routing.failed_edges,
    )
