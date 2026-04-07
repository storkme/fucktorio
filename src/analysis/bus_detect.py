"""Bus layout detection and spatial metric extraction from analyzed blueprints."""

from __future__ import annotations

import statistics
from dataclasses import dataclass, field

from .models import AnalyzedMachine, BlueprintGraph, TransportNetwork


@dataclass
class BusStructure:
    """Detected bus layout geometry."""

    orientation: str  # "vertical" (trunks run N-S) or "horizontal" (trunks run E-W)
    trunk_networks: list[TransportNetwork] = field(default_factory=list)
    tapoff_networks: list[TransportNetwork] = field(default_factory=list)
    # X-columns (for vertical) or Y-rows (for horizontal) occupied by trunk lanes
    lane_positions: list[int] = field(default_factory=list)


def detect_bus_structure(graph: BlueprintGraph) -> BusStructure | None:
    """Detect if a blueprint uses a main-bus layout pattern.

    Criteria:
    - At least 3 consistent trunk lanes identified
    - Trunks cover > 40% of bbox in the primary axis
    - Machine rows exist perpendicular to trunk direction

    Returns None if no bus pattern is found.
    """
    if not graph.networks or not graph.machines:
        return None

    belt_nets = [n for n in graph.networks if n.type == "belt"]
    if not belt_nets:
        return None

    # Compute bbox
    all_x = [s.position[0] for n in belt_nets for s in n.segments]
    all_y = [s.position[1] for n in belt_nets for s in n.segments]
    if not all_x:
        return None
    bbox_w = max(all_x) - min(all_x) + 1
    bbox_h = max(all_y) - min(all_y) + 1

    # Trunk candidates: low turn density and long path length
    trunk_candidates = [
        n for n in belt_nets
        if n.path_length >= 10
        and (n.turn_count / n.path_length if n.path_length > 0 else 1.0) < 0.1
    ]

    if not trunk_candidates:
        return None

    # For each candidate, determine its primary axis by comparing x-span vs y-span
    def _network_spans(net: TransportNetwork) -> tuple[int, int]:
        xs = {s.position[0] for s in net.segments}
        ys = {s.position[1] for s in net.segments}
        return max(xs) - min(xs), max(ys) - min(ys)

    vertical_trunks = []  # run N-S (x-span small, y-span large)
    horizontal_trunks = []  # run E-W (y-span small, x-span large)

    for net in trunk_candidates:
        x_span, y_span = _network_spans(net)
        if y_span > x_span and y_span > 5:
            vertical_trunks.append(net)
        elif x_span > y_span and x_span > 5:
            horizontal_trunks.append(net)

    # Pick orientation with more trunks
    if len(vertical_trunks) >= len(horizontal_trunks):
        orientation = "vertical"
        trunks = vertical_trunks
        bbox_primary = bbox_h
    else:
        orientation = "horizontal"
        trunks = horizontal_trunks
        bbox_primary = bbox_w

    if len(trunks) < 2:
        return None

    # Group trunks by their perpendicular-axis column/row
    def _median_perp(net: TransportNetwork, vert: bool) -> int:
        coords = [s.position[0] if vert else s.position[1] for s in net.segments]
        return int(statistics.median(coords))

    is_vert = orientation == "vertical"
    perp_positions = sorted({_median_perp(n, is_vert) for n in trunks})

    if len(perp_positions) < 2:
        return None

    # Check that trunks span enough of the bbox
    primary_coverage = max(
        sum(1 for s in n.segments for _ in [s]) / bbox_primary
        for n in trunks
    ) if bbox_primary > 0 else 0
    if primary_coverage < 0.3:
        return None

    # Check machine rows exist perpendicular to trunk direction
    if is_vert:
        machine_ys = sorted({m.position[1] for m in graph.machines})
    else:
        machine_ys = sorted({m.position[0] for m in graph.machines})

    if len(machine_ys) < 2:
        return None

    # Identify tap-off networks: short, perpendicular to trunks
    tapoffs = [
        n for n in belt_nets
        if n not in trunks and n.path_length >= 2
    ]

    return BusStructure(
        orientation=orientation,
        trunk_networks=trunks,
        tapoff_networks=tapoffs,
        lane_positions=perp_positions,
    )


def cluster_machine_rows(machines: list[AnalyzedMachine]) -> list[list[AnalyzedMachine]]:
    """Group machines into rows by Y-coordinate (for vertical buses).

    Uses a gap-scan: sort by Y, split where gap between adjacent Y values
    exceeds machine_size + 2.
    """
    if not machines:
        return []

    sorted_machines = sorted(machines, key=lambda m: m.position[1])
    rows: list[list[AnalyzedMachine]] = [[sorted_machines[0]]]

    for m in sorted_machines[1:]:
        prev = rows[-1][-1]
        gap = m.position[1] - (prev.position[1] + prev.size)
        if gap > 2:
            rows.append([m])
        else:
            rows[-1].append(m)

    return rows


def extract_bus_stats(graph: BlueprintGraph, stats: "BlueprintStats") -> None:  # type: ignore[name-defined]
    """Populate bus-specific fields on BlueprintStats (mutates in place)."""
    bus = detect_bus_structure(graph)

    if bus is None:
        return

    stats.is_bus_layout = True
    stats.bus_orientation = bus.orientation
    stats.bus_lane_count = len(bus.lane_positions)

    # Bus pitch: median gap between adjacent trunk lane positions
    if len(bus.lane_positions) >= 2:
        gaps = [bus.lane_positions[i + 1] - bus.lane_positions[i] for i in range(len(bus.lane_positions) - 1)]
        stats.bus_pitch = statistics.median(gaps)

    # Bus span: total width from first to last lane column
    if bus.lane_positions:
        stats.bus_span_tiles = bus.lane_positions[-1] - bus.lane_positions[0] + 1

    # Row structure
    rows = cluster_machine_rows(graph.machines)
    stats.row_count = len(rows)

    if rows:
        sizes = [len(r) for r in rows]
        stats.machines_per_row = statistics.mean(sizes)
        stats.machines_per_row_list = sizes

    # Row pitch: median gap between consecutive row centers
    if len(rows) >= 2:
        def _row_center(row: list[AnalyzedMachine]) -> float:
            return statistics.mean(m.position[1] + m.size / 2 for m in row)

        centers = [_row_center(r) for r in rows]
        pitches = [centers[i + 1] - centers[i] for i in range(len(centers) - 1)]
        if pitches:
            stats.row_pitch = statistics.median(pitches)

    # Fluid rows: rows that contain machines with fluid recipes (have pipe networks nearby)
    pipe_ys = {
        s.position[1]
        for n in graph.networks
        if n.type == "pipe"
        for s in n.segments
    }
    fluid_rows = 0
    for row in rows:
        row_ys = set()
        for m in row:
            row_ys.update(range(m.position[1], m.position[1] + m.size))
        if row_ys & pipe_ys:
            fluid_rows += 1
    stats.fluid_row_count = fluid_rows

    # Pipe networks near belt trunk x-columns
    trunk_xs = set(bus.lane_positions)
    pipe_near_trunk = sum(
        1 for n in graph.networks
        if n.type == "pipe"
        and any(abs(s.position[0] - x) <= 3 for s in n.segments for x in trunk_xs)
    )
    stats.pipe_net_beside_belt = pipe_near_trunk

    # Recipe groups: clusters of rows with the same dominant recipe
    recipe_groups: list[str | None] = []
    for row in rows:
        recipe_counts: dict[str | None, int] = {}
        for m in row:
            recipe_counts[m.recipe] = recipe_counts.get(m.recipe, 0) + 1
        dominant = max(recipe_counts, key=lambda r: recipe_counts[r])
        if not recipe_groups or recipe_groups[-1] != dominant:
            recipe_groups.append(dominant)

    stats.recipe_groups = len(recipe_groups)
    if stats.recipe_groups > 0 and graph.machines:
        stats.machines_per_recipe_group = len(graph.machines) / stats.recipe_groups
