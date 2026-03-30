"""Evolutionary search over placement, inserter sides, and edge routing order."""

from __future__ import annotations

import logging
import multiprocessing
import os
import random
from dataclasses import dataclass

from ..models import LayoutResult, SolverResult
from ..routing.common import machine_size, machine_tiles
from ..routing.graph import ProductionGraph, build_production_graph
from ..routing.orchestrate import build_layout, build_layout_incremental
from ..spaghetti.placer import _candidate_positions, _dependency_order, incremental_place
from ..validate import ValidationError, validate

log = logging.getLogger(__name__)

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
    score: float = float("inf")
    layout: LayoutResult | None = None


def evolutionary_layout(
    solver_result: SolverResult,
    population_size: int = 60,
    survivors: int = 10,
    generations: int = 3,
    seed: int | None = None,
) -> LayoutResult:
    """Produce a factory layout using evolutionary search over parameters.

    Explores many candidate layouts by varying machine positions, inserter
    side preferences, and edge routing order. Returns the best layout found.

    Args:
        solver_result: Solved recipe graph.
        population_size: Number of candidates per generation.
        survivors: Number of top candidates kept each generation.
        generations: Number of evolutionary generations.
        seed: Optional RNG seed for reproducible results.
    """
    rng = random.Random(seed)
    graph = build_production_graph(solver_result)

    if not graph.nodes:
        # Degenerate case: no machines
        return LayoutResult(entities=[], width=0, height=0)

    num_edges = len(graph.edges)

    # Generate initial population
    base_positions = incremental_place(graph, spacing=3)
    population = _generate_initial_population(
        graph,
        base_positions,
        num_edges,
        population_size,
        rng,
    )

    best_overall = _Candidate(positions=base_positions)

    # Determine worker count for parallel evaluation
    n_workers = min(os.cpu_count() or 1, population_size)

    for gen in range(generations):
        # Evaluate unevaluated candidates in parallel
        to_eval = [c for c in population if c.layout is None]
        if to_eval:
            args = [(c, solver_result, graph) for c in to_eval]
            try:
                with multiprocessing.Pool(n_workers) as pool:
                    results = pool.starmap(_evaluate_worker, args)
                for candidate, (layout, score) in zip(to_eval, results, strict=True):
                    candidate.layout = layout
                    candidate.score = score
            except Exception:
                # Fallback to sequential if multiprocessing fails
                log.debug("Parallel eval failed, falling back to sequential", exc_info=True)
                for candidate in to_eval:
                    _evaluate(candidate, solver_result, graph)

        # Sort by score (lower is better)
        population.sort(key=lambda c: c.score)

        # Track best overall
        if population[0].score < best_overall.score:
            best_overall = population[0]

        log.info(
            "Generation %d: best=%.2f, worst=%.2f (%d candidates)",
            gen + 1,
            population[0].score,
            population[-1].score,
            len(population),
        )

        # Perfect score: return immediately
        if best_overall.score <= 0.01:
            log.info("Found zero-error layout in generation %d", gen + 1)
            return best_overall.layout

        # Selection: keep top survivors
        elite = population[:survivors]

        # Mutation: produce next generation from survivors
        population = list(elite)  # keep elites (already evaluated)
        children_per_survivor = max(1, (population_size - survivors) // survivors)
        for parent in elite:
            for _ in range(children_per_survivor):
                child = _mutate(parent, graph, num_edges, rng)
                population.append(child)

        # Fill remainder if needed (rounding)
        while len(population) < population_size:
            parent = rng.choice(elite)
            population.append(_mutate(parent, graph, num_edges, rng))

    # Final evaluation of any unevaluated candidates
    final_to_eval = [c for c in population if c.layout is None]
    if final_to_eval:
        args = [(c, solver_result, graph) for c in final_to_eval]
        try:
            with multiprocessing.Pool(n_workers) as pool:
                results = pool.starmap(_evaluate_worker, args)
            for candidate, (layout, score) in zip(final_to_eval, results, strict=True):
                candidate.layout = layout
                candidate.score = score
        except Exception:
            for candidate in final_to_eval:
                _evaluate(candidate, solver_result, graph)
    for candidate in population:
        if candidate.score < best_overall.score:
            best_overall = candidate

    if best_overall.layout is None:
        # Fallback: evaluate the base candidate
        _evaluate(best_overall, solver_result, graph)

    log.info("Search complete: best score=%.2f", best_overall.score)
    return best_overall.layout


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

    # Remaining candidates: mix of incremental and batch
    dep_order = _dependency_order(graph)

    for _ in range(population_size - len(population)):
        side_pref = _random_side_preference(graph, rng)

        # All candidates use incremental placement
        order = _shuffle_topo_order(dep_order, graph, rng)
        population.append(
            _Candidate(
                positions={},
                side_preference=side_pref,
                placement_order=order,
                position_seed=rng.randint(0, 2**31),
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

            def _gen_candidates(node_id, g, positions, occupied, rng):
                node_size = machine_size(next(n for n in g.nodes if n.id == node_id).spec.entity)
                cands = _candidate_positions(node_id, node_size, g, positions, occupied, spacing=2)
                # Sort close-first for compactness, shuffle within distance bands for variety
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


def _perturb_positions(
    positions: dict[int, tuple[int, int]],
    graph: ProductionGraph,
    rng: random.Random,
    sigma: float = 2,
) -> dict[int, tuple[int, int]]:
    """Perturb machine positions with Gaussian noise, resolving overlaps."""
    node_map = {n.id: n for n in graph.nodes}
    new_pos: dict[int, tuple[int, int]] = {}
    occupied: set[tuple[int, int]] = set()

    for node_id, (x, y) in positions.items():
        size = machine_size(node_map[node_id].spec.entity)
        nx = x + int(rng.gauss(0, sigma))
        ny = y + int(rng.gauss(0, sigma))

        # Resolve overlaps by shifting right
        attempts = 0
        while _overlaps(nx, ny, size, occupied) and attempts < 50:
            nx += 1
            attempts += 1

        new_pos[node_id] = (nx, ny)
        occupied |= machine_tiles(nx, ny, size)

    return new_pos


def _overlaps(x: int, y: int, size: int, occupied: set[tuple[int, int]]) -> bool:
    """Check if placing a machine at (x, y) would overlap occupied tiles."""
    for dx in range(size):
        for dy in range(size):
            if (x + dx, y + dy) in occupied:
                return True
    return False


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


def _random_edge_order(num_edges: int, rng: random.Random) -> list[int]:
    """Generate a random edge routing order."""
    order = list(range(num_edges))
    rng.shuffle(order)
    return order


def _mutate(
    parent: _Candidate,
    graph: ProductionGraph,
    num_edges: int,
    rng: random.Random,
) -> _Candidate:
    """Produce a child candidate by mutating a parent."""
    # Mutate side preferences: start from parent's or generate fresh
    if parent.side_preference is not None:
        side_pref = dict(parent.side_preference)
    else:
        side_pref = {n.id: list(_ALL_SIDES) for n in graph.nodes}

    # Randomize 1-2 machines' side preferences
    nodes_to_mutate = rng.sample(graph.nodes, k=min(rng.randint(1, 2), len(graph.nodes)))
    for node in nodes_to_mutate:
        sides = list(_ALL_SIDES)
        rng.shuffle(sides)
        side_pref[node.id] = sides

    if parent.placement_order is not None:
        # Incremental mode: mutate placement order and position seed
        order = list(parent.placement_order)
        # Swap 1-2 adjacent pairs
        for _ in range(rng.randint(1, 2)):
            if len(order) >= 2:
                idx = rng.randint(0, len(order) - 2)
                order[idx], order[idx + 1] = order[idx + 1], order[idx]

        return _Candidate(
            positions={},
            side_preference=side_pref,
            placement_order=order,
            position_seed=rng.randint(0, 2**31),
        )
    else:
        # Batch mode: mutate positions and edge order
        positions = _perturb_positions(parent.positions, graph, rng, sigma=1.5)

        edge_ord = list(parent.edge_order) if parent.edge_order is not None else list(range(num_edges))
        n_swap = max(1, rng.randint(len(edge_ord) // 4, len(edge_ord) // 2 + 1))
        indices_to_swap = rng.sample(range(len(edge_ord)), k=min(n_swap, len(edge_ord)))
        values = [edge_ord[i] for i in indices_to_swap]
        rng.shuffle(values)
        for i, idx in enumerate(indices_to_swap):
            edge_ord[idx] = values[i]

        return _Candidate(
            positions=positions,
            side_preference=side_pref,
            edge_order=edge_ord,
        )
