"""Spaghetti layout orchestrator: graph → place → route → poles → LayoutResult."""

from __future__ import annotations

from ..layout.poles import place_poles
from ..models import LayoutResult, PlacedEntity, SolverResult
from .graph import build_production_graph
from .inserters import place_inserters
from .placer import machine_size, place_machines
from .router import route_connections


def spaghetti_layout(solver_result: SolverResult) -> LayoutResult:
    """Produce a factory layout using place-and-route (no predefined pattern).

    1. Build production graph from solver output
    2. Place machines on the grid
    3. Route belts/pipes between machines via BFS pathfinding
    4. Place inserters at machine borders
    5. Place power poles
    """
    # 1. Build production graph
    graph = build_production_graph(solver_result)

    # 2. Place machines
    positions = place_machines(graph)

    # 3. Place machine entities
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

    # 4. Route connections (belts + pipes)
    route_entities, occupied = route_connections(graph, positions)
    entities.extend(route_entities)

    # 5. Place inserters
    routed_tiles = {(e.x, e.y) for e in route_entities}
    inserter_entities = place_inserters(graph, positions, routed_tiles)
    entities.extend(inserter_entities)

    # Update occupied set with all entity tiles
    all_occupied: set[tuple[int, int]] = set()
    for e in entities:
        size = (
            machine_size(e.name)
            if e.name
            in (
                "assembling-machine-1",
                "assembling-machine-2",
                "assembling-machine-3",
                "chemical-plant",
                "oil-refinery",
            )
            else 1
        )
        for dx in range(size):
            for dy in range(size):
                all_occupied.add((e.x + dx, e.y + dy))

    # 6. Calculate bounds
    if entities:
        min_x = min(e.x for e in entities)
        min_y = min(e.y for e in entities)
        max_x = max(e.x for e in entities) + 1
        max_y = max(e.y for e in entities) + 1
        width = max_x - min_x + 2
        height = max_y - min_y + 2
    else:
        width = height = 0

    # 7. Place power poles
    pole_entities = place_poles(width, height, all_occupied)
    entities.extend(pole_entities)

    return LayoutResult(
        entities=entities,
        width=width,
        height=height,
    )
