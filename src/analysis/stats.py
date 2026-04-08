"""Per-blueprint statistics extraction for tuning layout engine parameters."""

from __future__ import annotations

import logging
from collections import Counter, defaultdict
from dataclasses import dataclass, field

from ..solver.recipe_db import get_crafting_speed, get_recipe
from .bus_detect import extract_bus_stats
from .models import BlueprintGraph

logger = logging.getLogger(__name__)


@dataclass
class BlueprintStats:
    """Statistics extracted from a single blueprint."""

    # Identity
    final_product: str | None = None
    recipe_count: int = 0
    machine_count: int = 0

    # Entity counts
    belt_tiles: int = 0
    pipe_tiles: int = 0
    inserter_count: int = 0
    beacon_count: int = 0
    pole_count: int = 0

    # Spatial
    bbox_width: int = 0
    bbox_height: int = 0
    bbox_area: int = 0
    density: float = 0.0

    # Machine placement
    machine_gaps: list[float] = field(default_factory=list)

    # Belt metrics
    belt_networks: int = 0
    avg_belt_path_length: float = 0.0
    avg_turn_density: float = 0.0
    avg_underground_ratio: float = 0.0

    # Pipe metrics
    pipe_networks: int = 0
    avg_pipe_path_length: float = 0.0

    # Inserter patterns
    input_inserters_per_machine: float = 0.0
    output_inserters_per_machine: float = 0.0

    # Ratios
    belts_per_machine: float = 0.0
    pipes_per_machine: float = 0.0
    inserters_per_machine: float = 0.0
    beacons_per_machine: float = 0.0
    poles_per_machine: float = 0.0

    # Inference quality
    networks_labeled: int = 0
    networks_unlabeled: int = 0

    # Throughput
    throughput_estimates: dict[str, float] = field(default_factory=dict)

    # Lightweight validation
    machines_without_inserters: int = 0
    orphan_networks: int = 0

    # Bus layout detection
    is_bus_layout: bool = False
    bus_orientation: str | None = None  # "horizontal" | "vertical"
    bus_lane_count: int = 0
    bus_pitch: float = 0.0  # median tile spacing between adjacent trunk columns
    bus_span_tiles: int = 0  # total trunk width (max_x - min_x + 1)

    # Row structure
    row_pitch: float = 0.0  # median gap between machine row Y centers
    row_count: int = 0
    machines_per_row: float = 0.0
    machines_per_row_list: list[float] = field(default_factory=list)

    # Fluid handling
    fluid_row_count: int = 0
    pipe_net_beside_belt: int = 0  # pipe networks within 3 tiles of trunk belt lanes

    # Recipe grouping
    recipe_groups: int = 0
    machines_per_recipe_group: float = 0.0


def extract_stats(graph: BlueprintGraph) -> BlueprintStats:
    """Extract all statistics from an analyzed blueprint."""
    stats = BlueprintStats()

    # --- Identity ---
    stats.final_product = detect_final_product(graph)
    recipes = {m.recipe for m in graph.machines if m.recipe is not None}
    stats.recipe_count = len(recipes)
    stats.machine_count = len(graph.machines)

    # --- Entity counts ---
    belt_nets = [n for n in graph.networks if n.type == "belt"]
    pipe_nets = [n for n in graph.networks if n.type == "pipe"]
    stats.belt_tiles = sum(n.path_length for n in belt_nets)
    stats.pipe_tiles = sum(n.path_length for n in pipe_nets)
    stats.inserter_count = len(graph.inserter_links)
    unhandled = Counter(graph.unhandled)
    stats.beacon_count = unhandled.get("beacon", 0)
    stats.pole_count = sum(v for k, v in unhandled.items() if "pole" in k)

    # --- Spatial ---
    all_positions: list[tuple[int, int]] = []
    for m in graph.machines:
        size = m.size
        for dx in range(size):
            for dy in range(size):
                all_positions.append((m.position[0] + dx, m.position[1] + dy))
    for n in graph.networks:
        for s in n.segments:
            all_positions.append(s.position)

    if all_positions:
        xs = [p[0] for p in all_positions]
        ys = [p[1] for p in all_positions]
        stats.bbox_width = max(xs) - min(xs) + 1
        stats.bbox_height = max(ys) - min(ys) + 1
        stats.bbox_area = stats.bbox_width * stats.bbox_height
        total_entities = (
            stats.machine_count + stats.belt_tiles + stats.pipe_tiles + stats.inserter_count + len(graph.unhandled)
        )
        stats.density = total_entities / stats.bbox_area if stats.bbox_area > 0 else 0.0

    # --- Machine placement gaps ---
    stats.machine_gaps = _compute_machine_gaps(graph)

    # --- Belt metrics ---
    stats.belt_networks = len(belt_nets)
    if belt_nets:
        stats.avg_belt_path_length = sum(n.path_length for n in belt_nets) / len(belt_nets)
        turn_densities = [n.turn_count / n.path_length for n in belt_nets if n.path_length > 0]
        stats.avg_turn_density = sum(turn_densities) / len(turn_densities) if turn_densities else 0.0
        ug_ratios = [n.underground_count / n.path_length for n in belt_nets if n.path_length > 0]
        stats.avg_underground_ratio = sum(ug_ratios) / len(ug_ratios) if ug_ratios else 0.0

    # --- Pipe metrics ---
    stats.pipe_networks = len(pipe_nets)
    if pipe_nets:
        stats.avg_pipe_path_length = sum(n.path_length for n in pipe_nets) / len(pipe_nets)

    # --- Inserter patterns ---
    if stats.machine_count > 0:
        input_count = sum(1 for lk in graph.inserter_links if lk.role == "input")
        output_count = sum(1 for lk in graph.inserter_links if lk.role == "output")
        stats.input_inserters_per_machine = input_count / stats.machine_count
        stats.output_inserters_per_machine = output_count / stats.machine_count

    # --- Ratios ---
    if stats.machine_count > 0:
        stats.belts_per_machine = stats.belt_tiles / stats.machine_count
        stats.pipes_per_machine = stats.pipe_tiles / stats.machine_count
        stats.inserters_per_machine = stats.inserter_count / stats.machine_count
        stats.beacons_per_machine = stats.beacon_count / stats.machine_count
        stats.poles_per_machine = stats.pole_count / stats.machine_count

    # --- Inference quality ---
    stats.networks_labeled = sum(1 for n in graph.networks if n.inferred_item is not None)
    stats.networks_unlabeled = sum(1 for n in graph.networks if n.inferred_item is None)

    # --- Throughput ---
    stats.throughput_estimates = estimate_throughput(graph)

    # --- Lightweight validation ---
    machines_with_inserters = {lk.machine_id for lk in graph.inserter_links}
    machines_with_fluids = {fl.machine_id for fl in graph.fluid_links}
    connected_machines = machines_with_inserters | machines_with_fluids
    stats.machines_without_inserters = sum(1 for m in graph.machines if m.id not in connected_machines)

    linked_networks = {lk.network_id for lk in graph.inserter_links if lk.network_id is not None}
    linked_networks |= {fl.network_id for fl in graph.fluid_links}
    stats.orphan_networks = sum(1 for n in graph.networks if n.id not in linked_networks)

    # --- Bus layout detection ---
    extract_bus_stats(graph, stats)

    return stats


def detect_final_product(graph: BlueprintGraph) -> str | None:
    """Determine the final product of a blueprint.

    Checks for items that leave the factory (edges with to_machine=None).
    Falls back to finding items produced but not consumed internally.
    """
    # Check external output edges
    external_outputs = {e.item for e in graph.edges if e.to_machine is None}
    if len(external_outputs) == 1:
        return next(iter(external_outputs))
    if len(external_outputs) > 1:
        # Multiple outputs — return the most common one
        output_counts: Counter[str] = Counter()
        for e in graph.edges:
            if e.to_machine is None:
                output_counts[e.item] += 1
        return output_counts.most_common(1)[0][0]

    # Fallback: find items produced but not consumed by other machines
    produced_items: set[str] = set()
    consumed_items: set[str] = set()
    for e in graph.edges:
        if e.from_machine is not None:
            produced_items.add(e.item)
        if e.to_machine is not None:
            consumed_items.add(e.item)

    final_only = produced_items - consumed_items
    if len(final_only) == 1:
        return next(iter(final_only))

    # Last resort: look at machine recipes for output items
    all_outputs: set[str] = set()
    all_inputs: set[str] = set()
    for m in graph.machines:
        all_outputs.update(m.outputs)
        all_inputs.update(m.inputs)
    candidates = all_outputs - all_inputs
    if len(candidates) == 1:
        return next(iter(candidates))

    return None


def estimate_throughput(graph: BlueprintGraph) -> dict[str, float]:
    """Estimate theoretical throughput per output item (ignoring beacons/modules).

    Returns {item_name: items_per_second} for each product.
    """
    estimates: dict[str, float] = {}

    # Group machines by recipe
    recipe_machines: dict[str, int] = defaultdict(int)
    recipe_entity: dict[str, str] = {}
    for m in graph.machines:
        if m.recipe is not None:
            recipe_machines[m.recipe] += 1
            recipe_entity[m.recipe] = m.name

    for recipe_name, count in recipe_machines.items():
        try:
            recipe = get_recipe(recipe_name)
        except KeyError:
            continue

        entity = recipe_entity[recipe_name]
        speed = get_crafting_speed(entity)
        energy = recipe.energy if recipe.energy > 0 else 0.5

        for product in recipe.products:
            rate = count * speed * product.amount * product.probability / energy
            item = product.name
            estimates[item] = estimates.get(item, 0.0) + rate

    return estimates


def _compute_machine_gaps(graph: BlueprintGraph) -> list[float]:
    """Compute gaps between pairs of adjacent machines.

    Only considers machines that are close enough to be neighbors
    (manhattan distance <= 2 * max_size + 2).
    """
    gaps: list[float] = []
    machines = graph.machines
    for i, m1 in enumerate(machines):
        for m2 in machines[i + 1 :]:
            dx = abs(m1.position[0] - m2.position[0])
            dy = abs(m1.position[1] - m2.position[1])
            max_size = max(m1.size, m2.size)
            # Only consider nearby pairs
            if dx > 2 * max_size + 2 or dy > 2 * max_size + 2:
                continue
            # Gap = distance between closest edges
            gap_x = max(0, dx - m1.size) if dx >= m1.size else max(0, dx - m2.size)
            gap_y = max(0, dy - m1.size) if dy >= m1.size else max(0, dy - m2.size)
            gap = min(gap_x, gap_y) if gap_x > 0 and gap_y > 0 else max(gap_x, gap_y)
            gaps.append(gap)
    return gaps
