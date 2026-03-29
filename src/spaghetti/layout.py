"""Spaghetti layout orchestrator: evolutionary search over placement parameters."""

from __future__ import annotations

import logging

from ..models import LayoutResult, SolverResult

log = logging.getLogger(__name__)


def spaghetti_layout(solver_result: SolverResult) -> LayoutResult:
    """Produce a factory layout using evolutionary search.

    Explores many candidate layouts by varying machine positions, inserter
    side preferences, and edge routing order. Returns the best layout found.
    """
    from ..search.layout_search import evolutionary_layout

    return evolutionary_layout(solver_result)


# --- Legacy retry loop (kept for reference/comparison) ---
#
# from ..routing.graph import build_production_graph
# from ..routing.orchestrate import build_layout
# from ..validate import ValidationError, validate
# from .placer import incremental_place
#
# _RETRY_STRATEGIES = [
#     ("top_bottom", 3),
#     ("left_right", 3),
#     ("top_bottom", 4),
#     ("left_right", 4),
#     ("top_bottom", 5),
#     ("top_bottom", 6),
# ]
#
# def spaghetti_layout_legacy(solver_result: SolverResult) -> LayoutResult:
#     graph = build_production_graph(solver_result)
#     best_result = None
#     best_error_count = float("inf")
#     for attempt, (strategy, spacing) in enumerate(_RETRY_STRATEGIES):
#         positions = incremental_place(graph, spacing=spacing)
#         layout_result, failed_edges = build_layout(
#             solver_result, graph, positions, side_strategy=strategy
#         )
#         try:
#             issues = validate(layout_result, solver_result, layout_style="spaghetti")
#             return layout_result
#         except ValidationError as exc:
#             score = len(exc.issues) + len(failed_edges) * 10
#             if score < best_error_count:
#                 best_result = layout_result
#                 best_error_count = score
#     return best_result if best_result is not None else layout_result
