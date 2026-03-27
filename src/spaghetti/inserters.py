"""Inserter pre-assignment and placement between belts and machines."""

from __future__ import annotations

from dataclasses import dataclass

from ..models import EntityDirection, PlacedEntity
from .graph import FlowEdge, ProductionGraph
from .placer import machine_size

# Inserter faces the direction it DROPS toward.
_FACING: dict[tuple[int, int], EntityDirection] = {
    (0, 1): EntityDirection.SOUTH,  # machine is below → drop south
    (0, -1): EntityDirection.NORTH,  # machine is above → drop north
    (1, 0): EntityDirection.EAST,  # machine is right → drop east
    (-1, 0): EntityDirection.WEST,  # machine is left → drop west
}

# The four sides of a machine: (dx_to_machine, dy_to_machine)
_SIDES = [(0, 1), (0, -1), (1, 0), (-1, 0)]  # top, bottom, left, right


@dataclass
class InserterAssignment:
    """Pre-assigned inserter position for a flow edge."""

    edge: FlowEdge
    node_id: int  # which machine this inserter serves
    border_tile: tuple[int, int]  # where the inserter goes (1 tile from machine)
    belt_tile: tuple[int, int]  # where the belt ends (2 tiles from machine)
    direction: EntityDirection  # inserter facing direction


def assign_inserter_positions(
    graph: ProductionGraph,
    positions: dict[int, tuple[int, int]],
    occupied: set[tuple[int, int]],
) -> list[InserterAssignment]:
    """Pre-assign inserter positions for every flow edge.

    For each machine, assigns border tiles to input/output edges round-robin
    across the machine's sides. This ensures every edge gets a guaranteed
    inserter position before routing begins.

    The assigned border tiles are added to the occupied set so the router
    won't route through them.
    """
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

            # Pick the next available side
            border, belt, direction_vec = available_sides.pop(0)

            # For input inserters: picks from belt, drops into machine
            # For output inserters: picks from machine, drops onto belt
            # The facing direction is always toward the machine for inputs,
            # away from machine for outputs
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

    return assignments


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
