"""Spaghetti layout orchestrator: graph → place → route → validate → retry."""

from __future__ import annotations

import logging

from ..layout.poles import place_poles
from ..models import LayoutResult, PlacedEntity, SolverResult
from ..validate import ValidationError, validate
from .graph import FlowEdge, ProductionGraph, build_production_graph
from .inserters import assign_inserter_positions, build_inserter_entities
from .placer import machine_size, place_machines
from .router import _machine_tiles, route_connections

log = logging.getLogger(__name__)

_MAX_RETRIES = 3
_DEFAULT_SPACING = 5
_SPACING_INCREMENT = 2

_MACHINE_ENTITIES = {
    "assembling-machine-1",
    "assembling-machine-2",
    "assembling-machine-3",
    "chemical-plant",
    "oil-refinery",
}


def spaghetti_layout(solver_result: SolverResult) -> LayoutResult:
    """Produce a factory layout using place-and-route with validation.

    Uses an escalating retry strategy:
    - Attempt 1: default spacing
    - Attempt 2+: increase machine spacing and re-place everything

    Returns the best layout found. If validation still has errors after
    all retries, returns the last attempt (best-effort) with warnings logged.
    """
    graph = build_production_graph(solver_result)
    spacing = _DEFAULT_SPACING
    best_result: LayoutResult | None = None
    best_error_count = float("inf")

    for attempt in range(_MAX_RETRIES + 1):
        layout_result, failed_edges = _attempt_layout(solver_result, graph, spacing)

        if failed_edges:
            log.warning(
                "Attempt %d: %d edge(s) failed routing",
                attempt + 1,
                len(failed_edges),
            )

        try:
            issues = validate(layout_result, solver_result, layout_style="spaghetti")
            if issues:
                for issue in issues:
                    log.info("Validation: %s", issue.message)
            return layout_result
        except ValidationError as exc:
            error_count = len(exc.issues)
            if error_count < best_error_count:
                best_result = layout_result
                best_error_count = error_count

            if attempt < _MAX_RETRIES:
                spacing += _SPACING_INCREMENT
                log.warning(
                    "Attempt %d: %d validation error(s), retrying with spacing=%d",
                    attempt + 1,
                    error_count,
                    spacing,
                )
            else:
                log.warning(
                    "Layout has %d validation error(s) after %d attempts (best-effort)",
                    best_error_count,
                    _MAX_RETRIES + 1,
                )

    # Return best-effort layout (the one with fewest errors)
    return best_result if best_result is not None else layout_result


def _attempt_layout(
    solver_result: SolverResult,
    graph: ProductionGraph,
    spacing: int,
) -> tuple[LayoutResult, list[FlowEdge]]:
    """Single layout attempt at a given spacing. Returns (result, failed_edges)."""

    # 1. Place machines
    positions = place_machines(graph, spacing=spacing)

    # 2. Build initial occupied set from machine footprints
    occupied: set[tuple[int, int]] = set()
    for node in graph.nodes:
        x, y = positions[node.id]
        size = machine_size(node.spec.entity)
        occupied |= _machine_tiles(x, y, size)

    # 3. Pre-assign inserter positions (reserves border tiles)
    assignments = assign_inserter_positions(graph, positions, occupied)

    # 4. Build edge→belt_tile mapping for the router
    edge_targets: dict[int, tuple[int, int]] = {}
    for assignment in assignments:
        # Find the edge index in graph.edges
        for i, edge in enumerate(graph.edges):
            if edge is assignment.edge:
                # Map to the belt tile that the route should target
                if assignment.edge.to_node == assignment.node_id:
                    # This is an input inserter — route goal is the belt tile
                    edge_targets[i] = assignment.belt_tile
                # For output inserters, the route starts from this belt tile
                # (handled by _edge_endpoints using from_node's belt tiles)
                break

    # 5. Place machine entities
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

    # 6. Route connections (belts + pipes) to assigned belt tiles
    #    Pass reserved inserter tiles so the router avoids them
    reserved = {a.border_tile for a in assignments}
    routing = route_connections(graph, positions, edge_targets=edge_targets, reserved_tiles=reserved)
    entities.extend(routing.entities)

    # 7. Place inserters from pre-assignments
    entities.extend(build_inserter_entities(assignments))

    # 5. Collect occupied tiles for pole placement
    all_occupied: set[tuple[int, int]] = set()
    for e in entities:
        size = machine_size(e.name) if e.name in _MACHINE_ENTITIES else 1
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

    return (
        LayoutResult(entities=entities, width=width, height=height),
        routing.failed_edges,
    )
