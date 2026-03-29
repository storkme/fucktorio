"""Evolutionary search over placement, inserter sides, and edge routing order."""

from __future__ import annotations

import logging
import random
from dataclasses import dataclass

from ..models import LayoutResult, SolverResult
from ..routing.common import machine_size, machine_tiles
from ..routing.graph import ProductionGraph, build_production_graph
from ..routing.orchestrate import build_layout
from ..spaghetti.placer import incremental_place
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
    score: float = float("inf")
    layout: LayoutResult | None = None


def evolutionary_layout(
    solver_result: SolverResult,
    population_size: int = 30,
    survivors: int = 5,
    generations: int = 5,
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
        graph, base_positions, num_edges, population_size, rng,
    )

    best_overall = _Candidate(positions=base_positions)

    for gen in range(generations):
        # Evaluate all unevaluated candidates
        for candidate in population:
            if candidate.layout is not None:
                continue  # already evaluated (survivor from previous gen)
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
    for candidate in population:
        if candidate.layout is None:
            _evaluate(candidate, solver_result, graph)
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
        positions_s4 = incremental_place(graph, spacing=4)
        spacing_variants.append(positions_s4)
        population.append(_Candidate(positions=positions_s4))

    if population_size > 2:
        positions_s5 = incremental_place(graph, spacing=5)
        spacing_variants.append(positions_s5)
        population.append(_Candidate(positions=positions_s5))

    # Remaining candidates: perturb positions, randomize sides/edge order
    for _ in range(population_size - len(population)):
        base = rng.choice(spacing_variants)
        positions = _perturb_positions(base, graph, rng, sigma=2)
        side_pref = _random_side_preference(graph, rng)
        edge_ord = _random_edge_order(num_edges, rng)
        population.append(_Candidate(
            positions=positions,
            side_preference=side_pref,
            edge_order=edge_ord,
        ))

    return population


def _evaluate(
    candidate: _Candidate,
    solver_result: SolverResult,
    graph: ProductionGraph,
) -> None:
    """Evaluate a candidate: build layout, validate, compute score."""
    try:
        layout_result, failed_edges = build_layout(
            solver_result, graph, candidate.positions,
            side_preference=candidate.side_preference,
            edge_order=candidate.edge_order,
        )
    except Exception:
        log.debug("Candidate build_layout raised exception", exc_info=True)
        candidate.score = 10000.0
        candidate.layout = LayoutResult(entities=[], width=0, height=0)
        return

    candidate.layout = layout_result

    try:
        validate(layout_result, solver_result, layout_style="spaghetti")
        error_count = 0
    except ValidationError as exc:
        error_count = len(exc.issues)

    candidate.score = (
        error_count
        + len(failed_edges) * 10
        + len(layout_result.entities) * 0.01
    )


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


def _overlaps(
    x: int, y: int, size: int, occupied: set[tuple[int, int]]
) -> bool:
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
    # Perturb positions with smaller sigma
    positions = _perturb_positions(parent.positions, graph, rng, sigma=1.5)

    # Mutate side preferences: start from parent's or generate fresh
    if parent.side_preference is not None:
        side_pref = dict(parent.side_preference)
    else:
        side_pref = {n.id: list(_ALL_SIDES) for n in graph.nodes}

    # Randomize 1-2 machines' side preferences
    nodes_to_mutate = rng.sample(
        graph.nodes, k=min(rng.randint(1, 2), len(graph.nodes))
    )
    for node in nodes_to_mutate:
        sides = list(_ALL_SIDES)
        rng.shuffle(sides)
        side_pref[node.id] = sides

    # Mutate edge order: partial shuffle of parent's order
    if parent.edge_order is not None:
        edge_ord = list(parent.edge_order)
    else:
        edge_ord = list(range(num_edges))

    # Shuffle a portion (25-50%) of the edge order
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
