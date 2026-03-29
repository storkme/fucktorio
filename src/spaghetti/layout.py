"""Spaghetti layout orchestrator: graph -> place -> route -> validate -> retry."""

from __future__ import annotations

import logging

from ..models import LayoutResult, SolverResult
from ..routing.graph import build_production_graph
from ..routing.orchestrate import build_layout
from ..validate import ValidationError, validate
from .placer import incremental_place

log = logging.getLogger(__name__)

_DEFAULT_SPACING = 3
_SPACING_INCREMENT = 1

# Retry strategies: vary spacing and side strategy
_RETRY_STRATEGIES = [
    ("top_bottom", 3),
    ("left_right", 3),
    ("top_bottom", 4),
    ("left_right", 4),
    ("top_bottom", 5),
    ("top_bottom", 6),
]


def spaghetti_layout(solver_result: SolverResult) -> LayoutResult:
    """Produce a factory layout using place-and-route with validation.

    Uses an escalating retry strategy that varies both machine spacing
    and inserter side strategy (top/bottom vs left/right) to explore
    different layout configurations.

    Returns the best layout found. If validation still has errors after
    all retries, returns the last attempt (best-effort) with warnings logged.
    """
    graph = build_production_graph(solver_result)
    best_result: LayoutResult | None = None
    best_error_count = float("inf")

    for attempt, (strategy, spacing) in enumerate(_RETRY_STRATEGIES):
        positions = incremental_place(graph, spacing=spacing)
        layout_result, failed_edges = build_layout(
            solver_result, graph, positions, side_strategy=strategy
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
            # Score includes both validation errors and failed routing edges
            # (failed edges are worse — they mean completely missing connections)
            score = len(exc.issues) + len(failed_edges) * 10
            if score < best_error_count:
                best_result = layout_result
                best_error_count = score

            if attempt < len(_RETRY_STRATEGIES) - 1:
                next_strategy, next_spacing = _RETRY_STRATEGIES[attempt + 1]
                log.warning(
                    "Attempt %d: %d validation error(s) + %d failed edges, retrying with strategy=%s spacing=%d",
                    attempt + 1,
                    len(exc.issues),
                    len(failed_edges),
                    next_strategy,
                    next_spacing,
                )
            else:
                log.warning(
                    "Layout has score %d (errors + failed edges) after %d attempts (best-effort)",
                    best_error_count,
                    len(_RETRY_STRATEGIES),
                )

    # Return best-effort layout (the one with fewest errors)
    return best_result if best_result is not None else layout_result
