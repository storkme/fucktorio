"""ML layout orchestrator: graph -> optimize placement -> route -> validate -> retry."""

from __future__ import annotations

import logging

from ..models import LayoutResult, SolverResult
from ..routing.graph import build_production_graph
from ..routing.orchestrate import build_layout
from ..validate import ValidationError, validate
from .placer import ml_place_machines

log = logging.getLogger(__name__)

_MAX_RETRIES = 3
_DEFAULT_MIN_GAP = 2.0
_GAP_INCREMENT = 1.0


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
        positions = ml_place_machines(graph, weights=weights, min_gap=min_gap)
        layout_result, failed_edges = build_layout(solver_result, graph, positions)

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
