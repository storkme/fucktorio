"""ML layout orchestrator: graph → optimize placement → route → validate → retry."""

from __future__ import annotations

import logging

from ..layout.poles import place_poles
from ..models import EntityDirection, LayoutResult, PlacedEntity, SolverResult
from ..spaghetti.graph import FlowEdge, ProductionGraph, build_production_graph
from ..spaghetti.inserters import assign_inserter_positions, build_inserter_entities
from ..spaghetti.placer import machine_size
from ..spaghetti.router import _DIR_MAP, _belt_entity_for_rate, _machine_tiles, route_connections
from ..validate import ValidationError, validate
from .placer import ml_place_machines

log = logging.getLogger(__name__)

_MAX_RETRIES = 3
_DEFAULT_MIN_GAP = 2.0
_GAP_INCREMENT = 1.0

_MACHINE_ENTITIES = {
    "assembling-machine-1",
    "assembling-machine-2",
    "assembling-machine-3",
    "chemical-plant",
    "oil-refinery",
}


def ml_layout(
    solver_result: SolverResult,
    weights: dict[str, float] | None = None,
) -> LayoutResult:
    """Produce a factory layout using ML-optimized placement + A* routing.

    Retry strategy: on validation failure, increase the min_gap parameter
    in the overlap penalty (giving machines more breathing room) and re-optimize.

    Returns the best layout found (fewest validation errors).
    """
    graph = build_production_graph(solver_result)
    min_gap = _DEFAULT_MIN_GAP
    best_result: LayoutResult | None = None
    best_error_count = float("inf")

    for attempt in range(_MAX_RETRIES + 1):
        layout_result, failed_edges = _attempt_layout(
            solver_result,
            graph,
            weights=weights,
            min_gap=min_gap,
        )

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
                min_gap += _GAP_INCREMENT
                log.warning(
                    "Attempt %d: %d validation error(s), retrying with min_gap=%.1f",
                    attempt + 1,
                    error_count,
                    min_gap,
                )
            else:
                log.warning(
                    "Layout has %d validation error(s) after %d attempts (best-effort)",
                    best_error_count,
                    _MAX_RETRIES + 1,
                )

    return best_result if best_result is not None else layout_result


def _attempt_layout(
    solver_result: SolverResult,
    graph: ProductionGraph,
    weights: dict[str, float] | None = None,
    min_gap: float = 2.0,
    side_strategy: str = "top_bottom",
) -> tuple[LayoutResult, list[FlowEdge]]:
    """Single layout attempt. Returns (result, failed_edges)."""

    # 1. ML-optimized machine placement
    positions = ml_place_machines(graph, weights=weights, min_gap=min_gap)

    # 2. Build occupied set from machine footprints
    occupied: set[tuple[int, int]] = set()
    for node in graph.nodes:
        x, y = positions[node.id]
        size = machine_size(node.spec.entity)
        occupied |= _machine_tiles(x, y, size)

    # 3. Pre-assign inserter positions (lane-aware, reserves border tiles)
    plan = assign_inserter_positions(
        graph, positions, occupied, solver_result=solver_result, side_strategy=side_strategy
    )
    assignments = plan.assignments

    # 4. Build edge→belt_tile mapping and per-edge exclusions for the router
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

    # 6b. Place belt stubs for any external output inserters whose edges
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
                belt_name = _belt_entity_for_rate(edge.rate)
                direction = _DIR_MAP.get((dx, dy), EntityDirection.SOUTH)
                entities.append(
                    PlacedEntity(
                        name=belt_name,
                        x=bx,
                        y=by,
                        direction=direction,
                        carries=edge.item,
                    )
                )

    # 7. Place inserters
    entities.extend(build_inserter_entities(assignments))

    # 8. Collect occupied tiles for pole placement
    all_occupied: set[tuple[int, int]] = set()
    for e in entities:
        size = machine_size(e.name) if e.name in _MACHINE_ENTITIES else 1
        for dx in range(size):
            for dy in range(size):
                all_occupied.add((e.x + dx, e.y + dy))

    # 9. Calculate bounds
    if entities:
        min_x = min(e.x for e in entities)
        min_y = min(e.y for e in entities)
        max_x = max(e.x for e in entities) + 1
        max_y = max(e.y for e in entities) + 1
        width = max_x - min_x + 2
        height = max_y - min_y + 2
    else:
        width = height = 0

    # 10. Place power poles
    pole_entities = place_poles(width, height, all_occupied)
    entities.extend(pole_entities)

    return (
        LayoutResult(entities=entities, width=width, height=height),
        routing.failed_edges,
    )
