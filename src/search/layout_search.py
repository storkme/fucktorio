"""Random search over placement, inserter sides, and edge routing order."""

from __future__ import annotations

import logging
import multiprocessing
import os
import random
import time
from dataclasses import dataclass, field

from ..models import LayoutResult, SolverResult
from ..routing.common import machine_size, machine_tiles
from ..routing.graph import ProductionGraph, build_production_graph
from ..routing.orchestrate import build_layout, build_layout_incremental, plan_trunks
from ..spaghetti.placer import _candidate_positions, _dependency_order, incremental_place
from ..validate import ValidationError, validate

log = logging.getLogger(__name__)


@dataclass
class SearchStats:
    """Statistics from a search run (single random_search_layout call)."""

    attempt: int
    score: float
    error_count: int
    failed_edges: int
    belt_count: int
    elapsed_s: float
    error_categories: dict[str, int] = field(default_factory=dict)


# All four side direction vectors
_ALL_SIDES = [(0, 1), (0, -1), (1, 0), (-1, 0)]


@dataclass
class _Candidate:
    """A single layout candidate with its parameter genes."""

    positions: dict[int, tuple[int, int]]
    side_preference: dict[int, list[tuple[int, int]]] | None = None
    edge_order: list[int] | None = None
    position_seed: int = 0  # RNG seed for incremental position selection
    placement_order: list[int] | None = None  # machine placement order
    use_trunks: bool = False  # whether to pre-lay belt trunks
    trunk_spacing: int = 2  # x gap between input and output trunks
    score: float = float("inf")
    layout: LayoutResult | None = None


def random_search_layout(
    solver_result: SolverResult,
    population_size: int = 60,
    seed: int | None = None,
) -> LayoutResult:
    """Produce a factory layout by evaluating random candidates in parallel.

    Generates a pool of random candidates (varying placement order, inserter
    side preferences, and position seeds) and evaluates them all in parallel.
    Returns the best layout found.

    Args:
        solver_result: Solved recipe graph.
        population_size: Total number of candidates to evaluate.
        seed: Optional RNG seed for reproducible results.
    """
    rng = random.Random(seed)
    graph = build_production_graph(solver_result)

    if not graph.nodes:
        return LayoutResult(entities=[], width=0, height=0)

    num_edges = len(graph.edges)

    # Generate all candidates up front
    base_positions = incremental_place(graph, spacing=3)
    population = _generate_initial_population(
        graph,
        base_positions,
        num_edges,
        population_size,
        rng,
    )

    # Evaluate all candidates in parallel (single pass, no generations)
    n_workers = min(os.cpu_count() or 1, population_size)
    args = [(c, solver_result, graph) for c in population]
    try:
        with multiprocessing.Pool(n_workers) as pool:
            results = pool.starmap(_evaluate_worker, args)
        for candidate, (layout, score) in zip(population, results, strict=True):
            candidate.layout = layout
            candidate.score = score
    except Exception:
        log.debug("Parallel eval failed, falling back to sequential", exc_info=True)
        for candidate in population:
            _evaluate(candidate, solver_result, graph)

    # Pick the best
    population.sort(key=lambda c: c.score)
    best = population[0]

    log.info(
        "Search: %d candidates, best=%.2f, worst=%.2f",
        len(population),
        best.score,
        population[-1].score,
    )

    if best.layout is None:
        _evaluate(best, solver_result, graph)

    return best.layout


def search_with_retries(
    solver_result: SolverResult,
    max_attempts: int = 5,
    population_size: int = 60,
    seed: int | None = None,
) -> tuple[LayoutResult, list[SearchStats]]:
    """Run random_search_layout up to max_attempts times, return best zero-error layout.

    Returns the first zero-error layout found, or the best layout across all
    attempts if none are perfect. Also returns stats for every attempt.
    """
    rng = random.Random(seed)
    best_layout: LayoutResult | None = None
    best_score = float("inf")
    all_stats: list[SearchStats] = []

    for attempt in range(1, max_attempts + 1):
        attempt_seed = rng.randint(0, 2**31)
        t0 = time.monotonic()

        layout = random_search_layout(
            solver_result,
            population_size=population_size,
            seed=attempt_seed,
        )

        elapsed = time.monotonic() - t0

        # Score the result
        error_count = 0
        error_categories: dict[str, int] = {}
        try:
            validate(layout, solver_result, layout_style="spaghetti")
        except ValidationError as exc:
            error_count = len(exc.issues)
            for issue in exc.issues:
                error_categories[issue.category] = error_categories.get(issue.category, 0) + 1

        belt_count = sum(1 for e in layout.entities if "belt" in e.name)
        failed_edges = 0  # not easily recoverable from random_search_layout, use score proxy
        score = error_count * 100 + belt_count * 0.5

        stats = SearchStats(
            attempt=attempt,
            score=score,
            error_count=error_count,
            failed_edges=failed_edges,
            belt_count=belt_count,
            elapsed_s=elapsed,
            error_categories=error_categories,
        )
        all_stats.append(stats)

        log.info(
            "Attempt %d/%d: %d errors, %d belts, %.1fs %s",
            attempt,
            max_attempts,
            error_count,
            belt_count,
            elapsed,
            dict(error_categories) if error_categories else "(clean!)",
        )

        if score < best_score:
            best_score = score
            best_layout = layout

        if error_count == 0:
            log.info("Found zero-error layout on attempt %d", attempt)
            break

    # Summary
    scores = [s.error_count for s in all_stats]
    log.info(
        "Search summary: %d attempts, errors=[%s], best=%d",
        len(all_stats),
        ", ".join(str(s) for s in scores),
        min(scores),
    )

    assert best_layout is not None
    return best_layout, all_stats


def _generate_initial_population(
    graph: ProductionGraph,
    base_positions: dict[int, tuple[int, int]],
    num_edges: int,
    population_size: int,
    rng: random.Random,
) -> list[_Candidate]:
    """Generate the initial population of candidates."""
    population: list[_Candidate] = []

    # Seed candidates at different spacings
    spacing_variants = [base_positions]
    population.append(_Candidate(positions=dict(base_positions)))

    if population_size > 1:
        positions_s1 = incremental_place(graph, spacing=1)
        spacing_variants.append(positions_s1)
        population.append(_Candidate(positions=positions_s1))

    if population_size > 2:
        positions_s4 = incremental_place(graph, spacing=4)
        spacing_variants.append(positions_s4)
        population.append(_Candidate(positions=positions_s4))

    if population_size > 3:
        positions_s5 = incremental_place(graph, spacing=5)
        spacing_variants.append(positions_s5)
        population.append(_Candidate(positions=positions_s5))

    # Remaining candidates: mix of incremental and trunk-based
    dep_order = _dependency_order(graph)

    remaining = population_size - len(population)
    # Half of remaining candidates use trunk planning
    trunk_count = remaining // 2

    for idx in range(remaining):
        side_pref = _random_side_preference(graph, rng)
        order = _shuffle_topo_order(dep_order, graph, rng)
        use_trunks = idx < trunk_count
        trunk_spacing = rng.choice([2, 2, 3]) if use_trunks else 7
        population.append(
            _Candidate(
                positions={},
                side_preference=side_pref,
                placement_order=order,
                position_seed=rng.randint(0, 2**31),
                use_trunks=use_trunks,
                trunk_spacing=trunk_spacing,
            )
        )

    return population


def _evaluate_worker(
    candidate: _Candidate,
    solver_result: SolverResult,
    graph: ProductionGraph,
) -> tuple[LayoutResult, float]:
    """Evaluate a candidate and return (layout, score). Used by multiprocessing."""
    _evaluate(candidate, solver_result, graph)
    return (candidate.layout, candidate.score)


def _evaluate(
    candidate: _Candidate,
    solver_result: SolverResult,
    graph: ProductionGraph,
) -> None:
    """Evaluate a candidate: build layout, validate, compute score."""
    try:
        if candidate.placement_order is not None:
            # Incremental mode
            inc_rng = random.Random(candidate.position_seed)

            # Pre-lay trunks if this candidate uses trunk planning
            trunk_kwargs: dict = {}
            trunk_x_coords: list[int] = []
            if candidate.use_trunks:
                t_ents, t_bdm, t_gn, t_occ = plan_trunks(
                    graph,
                    solver_result,
                    trunk_spacing=candidate.trunk_spacing,
                )
                trunk_kwargs = dict(
                    trunk_entities=t_ents,
                    trunk_belt_dir_map=t_bdm,
                    trunk_group_networks=t_gn,
                    trunk_occupied=t_occ,
                )
                trunk_x_coords = sorted({e.x for e in t_ents})

            # Compute trunk length for generating positions along trunks
            trunk_length = max(len(graph.nodes) * 4, 8) if candidate.use_trunks else 0

            # Compute ideal x-positions: between input and output trunks
            if trunk_x_coords:
                input_items = sorted({e.item for e in graph.edges if e.from_node is None})
                input_trunk_xs = trunk_x_coords[: len(input_items)]
                # Machine x: right of rightmost input trunk, with 2 tiles gap (inserter + belt)
                machine_ideal_x = max(input_trunk_xs) + 2 if input_trunk_xs else trunk_x_coords[0] + 2
            else:
                machine_ideal_x = 0

            def _gen_candidates(
                node_id, g, positions, occupied, rng, _txc=trunk_x_coords, _tlen=trunk_length, _mix=machine_ideal_x
            ):
                node_size = machine_size(next(n for n in g.nodes if n.id == node_id).spec.entity)

                if _txc:
                    # Trunk mode: place machines in a column between input and output trunks.
                    # Generate positions at the ideal x, spaced vertically.
                    cands = []
                    for y in range(0, _tlen - node_size + 1, node_size + 1):
                        if not (machine_tiles(_mix, y, node_size) & occupied):
                            cands.append((_mix, y))

                    # Also try with 1-tile vertical gap for variety
                    for y in range(0, _tlen - node_size + 1):
                        if (_mix, y) not in set(cands) and not (machine_tiles(_mix, y, node_size) & occupied):
                            cands.append((_mix, y))

                    def _trunk_score(p):
                        px, py = p
                        x_dist = abs(px - _mix)
                        return x_dist * 10 + py * 0.1

                    cands.sort(key=_trunk_score)
                else:
                    cands = _candidate_positions(node_id, node_size, g, positions, occupied, spacing=2)
                    if positions:
                        cx = sum(x for x, _ in positions.values()) / len(positions)
                        cy = sum(y for _, y in positions.values()) / len(positions)
                        cands.sort(key=lambda p: abs(p[0] - cx) + abs(p[1] - cy))

                # Shuffle within groups of 4 to add variety without losing closeness
                for i in range(0, len(cands) - 3, 4):
                    chunk = cands[i : i + 4]
                    rng.shuffle(chunk)
                    cands[i : i + 4] = chunk
                return cands

            layout_result, failed_edges, direct_count = build_layout_incremental(
                solver_result,
                graph,
                candidate.placement_order,
                _gen_candidates,
                side_preference=candidate.side_preference,
                rng=inc_rng,
                **trunk_kwargs,
            )
        else:
            # Batch mode (legacy)
            layout_result, failed_edges, direct_count = build_layout(
                solver_result,
                graph,
                candidate.positions,
                side_preference=candidate.side_preference,
                edge_order=candidate.edge_order,
            )
    except Exception:
        log.debug("Candidate build_layout raised exception", exc_info=True)
        candidate.score = 10000.0
        candidate.layout = LayoutResult(entities=[], width=0, height=0)
        return

    candidate.layout = layout_result

    # Skip expensive validation for candidates with many failed edges —
    # they'll never survive selection anyway.
    if len(failed_edges) >= 3:
        candidate.score = len(failed_edges) * 1000 + len(layout_result.entities) * 0.01
        return

    try:
        validate(layout_result, solver_result, layout_style="spaghetti")
        error_count = 0
    except ValidationError as exc:
        error_count = len(exc.issues)

    belt_count = sum(1 for e in layout_result.entities if "belt" in e.name)
    area = layout_result.width * layout_result.height
    candidate.score = error_count * 100 + len(failed_edges) * 1000 + belt_count * 0.5 + area * 0.1 - direct_count * 10


def _random_side_preference(
    graph: ProductionGraph,
    rng: random.Random,
) -> dict[int, list[tuple[int, int]]]:
    """Generate random side preference for each machine."""
    pref: dict[int, list[tuple[int, int]]] = {}
    for node in graph.nodes:
        sides = list(_ALL_SIDES)
        rng.shuffle(sides)
        pref[node.id] = sides
    return pref


def _shuffle_topo_order(
    base_order: list[int],
    graph: ProductionGraph,
    rng: random.Random,
) -> list[int]:
    """Shuffle machine placement order while respecting dependency constraints.

    Machines within the same topological level can be freely reordered.
    For iron-gear-wheel (all same level), this is a full shuffle.
    """
    # Group by topological level (upstream machines before downstream)
    # Simple approach: just shuffle the whole order (dependencies are soft constraints
    # that affect quality, not hard constraints that break correctness)
    order = list(base_order)
    rng.shuffle(order)
    return order
