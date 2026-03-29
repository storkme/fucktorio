"""Inserter pre-assignment and placement between belts and machines."""

from __future__ import annotations

import math
from dataclasses import dataclass, field

from ..models import EntityDirection, PlacedEntity, SolverResult
from .common import machine_size
from .graph import FlowEdge, ProductionGraph

# Inserter faces the direction it DROPS toward.
_FACING: dict[tuple[int, int], EntityDirection] = {
    (0, 1): EntityDirection.SOUTH,  # machine is below -> drop south
    (0, -1): EntityDirection.NORTH,  # machine is above -> drop north
    (1, 0): EntityDirection.EAST,  # machine is right -> drop east
    (-1, 0): EntityDirection.WEST,  # machine is left -> drop west
}

# Belt throughput limits (items per second) — mirrors validate.py
_BELT_CAPACITY = {
    "transport-belt": 15.0,
    "fast-transport-belt": 30.0,
    "express-transport-belt": 45.0,
}
_MAX_BELT_CAPACITY = 45.0

# Side strategy pairs: which two sides to alternate between
_STRATEGY_PAIRS: dict[str, list[tuple[int, int]]] = {
    "top_bottom": [(0, 1), (0, -1)],  # top, bottom
    "left_right": [(1, 0), (-1, 0)],  # left, right
}


@dataclass
class InserterAssignment:
    """Pre-assigned inserter position for a flow edge."""

    edge: FlowEdge
    node_id: int  # which machine this inserter serves
    border_tile: tuple[int, int]  # where the inserter goes (1 tile from machine)
    belt_tile: tuple[int, int]  # where the belt ends (2 tiles from machine)
    direction: EntityDirection  # inserter facing direction


@dataclass
class InsertionPlan:
    """Result of lane-aware inserter assignment."""

    assignments: list[InserterAssignment] = field(default_factory=list)
    # item -> list of sub-groups, each sub-group is a list of edge indices
    edge_subgroups: dict[str, list[list[int]]] = field(default_factory=dict)


def _compute_edge_subgroups(
    graph: ProductionGraph,
    solver_result: SolverResult | None,
) -> dict[str, list[list[int]]]:
    """Partition external edges into sub-groups by belt capacity.

    When total rate for an item exceeds a single belt's capacity,
    machines are split into sub-groups that each fit on one trunk.
    """
    if solver_result is None:
        return {}

    # Map item -> per-machine rate (from solver)
    item_rate: dict[str, float] = {}
    for spec in solver_result.machines:
        for inp in spec.inputs:
            if not inp.is_fluid:
                item_rate[inp.item] = inp.rate
        for out in spec.outputs:
            if not out.is_fluid:
                item_rate[out.item] = out.rate

    subgroups: dict[str, list[list[int]]] = {}

    # Group external edges by item
    ext_groups: dict[str, list[int]] = {}
    for i, edge in enumerate(graph.edges):
        if edge.is_fluid:
            continue
        if edge.from_node is None or edge.to_node is None:
            ext_groups.setdefault(edge.item, []).append(i)

    for item, edge_indices in ext_groups.items():
        per_machine_rate = item_rate.get(item, 0)
        total_rate = per_machine_rate * len(edge_indices)

        if total_rate <= _MAX_BELT_CAPACITY:
            # All fit on one trunk
            subgroups[item] = [edge_indices]
        else:
            # Split into sub-groups
            n_trunks = math.ceil(total_rate / _MAX_BELT_CAPACITY)
            groups: list[list[int]] = [[] for _ in range(n_trunks)]
            for j, idx in enumerate(edge_indices):
                groups[j % n_trunks].append(idx)
            subgroups[item] = groups

    return subgroups


def assign_inserter_positions(
    graph: ProductionGraph,
    positions: dict[int, tuple[int, int]],
    occupied: set[tuple[int, int]],
    solver_result: SolverResult | None = None,
    side_strategy: str = "top_bottom",
    side_preference: dict[int, list[tuple[int, int]]] | None = None,
) -> InsertionPlan:
    """Pre-assign inserter positions for every flow edge.

    Lane-aware: for edges sharing an external item, alternates inserter
    sides based on the side_strategy so items spread across both belt lanes.

    Args:
        side_strategy: "top_bottom", "left_right", or "round_robin".
            Controls which axis inserters alternate on for shared-item edges.
        side_preference: Per-machine side order override. Maps node_id to
            an ordered list of (dx, dy) direction vectors to try. When
            provided, overrides side_strategy for the given machines.

    Returns an InsertionPlan with assignments and sub-group info.
    """
    # Compute sub-groups for capacity splitting
    edge_subgroups = _compute_edge_subgroups(graph, solver_result)

    # Build preferred side mapping for each edge
    # edge index -> preferred direction_vec (toward machine)
    edge_preferred: dict[int, tuple[int, int]] = {}
    strategy_pair = _STRATEGY_PAIRS.get(side_strategy, [(0, 1), (0, -1)])

    for _item, groups in edge_subgroups.items():
        for g_idx, group in enumerate(groups):
            # All edges in the same sub-group go on the same side so the
            # router can connect them into a single trunk network.
            # Inputs use side 0, outputs use side 1 (so they don't conflict).
            # Multiple sub-groups for the same direction alternate further.
            sample_edge = graph.edges[group[0]]
            is_output = sample_edge.to_node is None
            base = 1 if is_output else 0
            side_idx = (base + g_idx) % 2
            side = strategy_pair[side_idx]
            for edge_idx in group:
                edge_preferred[edge_idx] = side

    # Now do per-machine assignment with preferred sides
    assignments: list[InserterAssignment] = []
    used_borders: set[tuple[int, int]] = set()

    for node in graph.nodes:
        mx, my = positions[node.id]
        size = machine_size(node.spec.entity)

        # Gather all edges for this machine
        input_edges = graph.inputs_for(node.id)
        output_edges = graph.outputs_for(node.id)

        # Get available border positions per side
        sides = _get_sides(mx, my, size)

        # If side_preference is provided for this node, sort sides by
        # the preference order (try preferred directions first)
        if side_preference is not None and node.id in side_preference:
            pref_order = side_preference[node.id]
            sides = sorted(sides, key=lambda s, po=pref_order: (
                po.index(s[2]) if s[2] in po else len(po)
            ))

        # Filter out sides where border tiles are already occupied
        available_sides: list[tuple[tuple[int, int], tuple[int, int], tuple[int, int]]] = []
        for border, belt, direction_vec in sides:
            if border not in occupied and border not in used_borders and belt not in occupied:
                available_sides.append((border, belt, direction_vec))

        # Assign input edges first, then output edges
        all_edges = [(e, True) for e in input_edges] + [(e, False) for e in output_edges]

        for edge, is_input in all_edges:
            if not available_sides:
                break

            # Find this edge's index in graph.edges for preferred side lookup
            preferred = None
            for i, ge in enumerate(graph.edges):
                if ge is edge and i in edge_preferred:
                    preferred = edge_preferred[i]
                    break

            # Sort available sides: preferred first, then the rest
            # (only when side_preference is not overriding for this node)
            if side_preference is None or node.id not in side_preference:
                if preferred is not None:
                    available_sides.sort(
                        key=lambda s: 0 if s[2] == preferred else 1,
                    )

            # Pick the best available side
            border, belt, direction_vec = available_sides.pop(0)

            # For input inserters: picks from belt, drops into machine
            # For output inserters: picks from machine, drops onto belt
            if is_input:
                facing = _FACING.get(direction_vec)
            else:
                # Reverse direction for output inserter
                reverse = (-direction_vec[0], -direction_vec[1])
                facing = _FACING.get(reverse)

            if facing is None:
                continue

            assignment = InserterAssignment(
                edge=edge,
                node_id=node.id,
                border_tile=border,
                belt_tile=belt,
                direction=facing,
            )
            assignments.append(assignment)
            used_borders.add(border)
            occupied.add(border)  # Reserve so router avoids this tile

    return InsertionPlan(assignments=assignments, edge_subgroups=edge_subgroups)


def _get_sides(mx: int, my: int, size: int) -> list[tuple[tuple[int, int], tuple[int, int], tuple[int, int]]]:
    """Get (border_tile, belt_tile, direction_toward_machine) for each side.

    Returns one position per side, centered on the machine.
    """
    center = size // 2
    return [
        # Top side
        ((mx + center, my - 1), (mx + center, my - 2), (0, 1)),
        # Bottom side
        ((mx + center, my + size), (mx + center, my + size + 1), (0, -1)),
        # Left side
        ((mx - 1, my + center), (mx - 2, my + center), (1, 0)),
        # Right side
        ((mx + size, my + center), (mx + size + 1, my + center), (-1, 0)),
    ]


def build_inserter_entities(assignments: list[InserterAssignment]) -> list[PlacedEntity]:
    """Create inserter PlacedEntity objects from pre-assignments."""
    return [
        PlacedEntity(
            name="inserter",
            x=a.border_tile[0],
            y=a.border_tile[1],
            direction=a.direction,
        )
        for a in assignments
    ]
