"""Main layout orchestrator: rows + bus + poles → LayoutResult."""

from __future__ import annotations

from ..models import SolverResult, LayoutResult
from .placer import place_rows
from .router import route_bus
from .poles import place_poles


def _bus_width(solver_result: SolverResult) -> int:
    """Calculate how wide the main bus needs to be.

    Each lane uses 2 tiles (entity + gap). Add 1 extra tile of padding.
    Minimum 2 to leave room even with no external inputs.
    """
    n_lanes = len(solver_result.external_inputs)
    return max(2, n_lanes * 2 + 1)


def layout(solver_result: SolverResult) -> LayoutResult:
    """Convert a SolverResult into a positioned LayoutResult."""

    bus_w = _bus_width(solver_result)

    # 1. Place assembly rows
    row_entities, width, height = place_rows(
        solver_result.machines,
        solver_result.dependency_order,
        bus_width=bus_w,
    )

    # 2. Route main bus belts/pipes (with underground segments through rows)
    bus_entities = route_bus(solver_result, height, bus_width=bus_w)

    # 3. Collect occupied tiles
    occupied: set[tuple[int, int]] = set()
    _3x3_ENTITIES = {
        "assembling-machine-1", "assembling-machine-2", "assembling-machine-3",
        "chemical-plant",
    }
    _5x5_ENTITIES = {"oil-refinery"}
    for ent in bus_entities + row_entities:
        if ent.name in _3x3_ENTITIES:
            for dx in range(3):
                for dy in range(3):
                    occupied.add((ent.x + dx, ent.y + dy))
        elif ent.name in _5x5_ENTITIES:
            for dx in range(5):
                for dy in range(5):
                    occupied.add((ent.x + dx, ent.y + dy))
        else:
            occupied.add((ent.x, ent.y))

    # 4. Place power poles avoiding collisions
    pole_entities = place_poles(width, height, occupied)

    all_entities = bus_entities + row_entities + pole_entities

    return LayoutResult(
        entities=all_entities,
        width=width,
        height=height,
    )
