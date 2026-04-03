"""Bus layout orchestrator: rows + bus lanes + poles -> LayoutResult."""

from __future__ import annotations

from ..models import LayoutResult, SolverResult
from ..routing.common import machine_size
from ..routing.poles import place_poles
from .bus_router import bus_width_for_lanes, plan_bus_lanes, route_bus
from .placer import place_rows


def bus_layout(
    solver_result: SolverResult,
    max_belt_tier: str | None = None,
) -> LayoutResult:
    """Convert a SolverResult into a bus-style LayoutResult.

    Args:
        solver_result: Solved recipe graph.
        max_belt_tier: Constrain belt tier (e.g. "transport-belt" for
            early game). Rows auto-split to stay within capacity.
    """
    # 1. Pre-plan bus lanes to know bus width before placing rows
    # We need row spans first to know where rows land, but we need bus width
    # to place rows.  Solve with two passes: first compute row spans without
    # bus routing, then route.
    #
    # Actually, plan_bus_lanes only needs row_spans (not bus width), and
    # place_rows needs bus_width.  So: do a preliminary row placement with
    # a guess, plan lanes, compute real bus width, then re-place if needed.

    # First pass: plan lanes with a temporary placement.
    # Add a 1-tile header row so underground entries can start above
    # the first row when tap-offs cross other lanes.
    BUS_HEADER = 1

    temp_bw = _estimate_bus_width(solver_result)
    row_entities, row_spans, row_width, total_height = place_rows(
        solver_result.machines,
        solver_result.dependency_order,
        bus_width=temp_bw,
        y_offset=BUS_HEADER,
        max_belt_tier=max_belt_tier,
    )

    lanes = plan_bus_lanes(solver_result, row_spans, max_belt_tier=max_belt_tier)
    actual_bw = bus_width_for_lanes(lanes)

    # Re-place rows if bus width changed
    if actual_bw != temp_bw:
        row_entities, row_spans, row_width, total_height = place_rows(
            solver_result.machines,
            solver_result.dependency_order,
            bus_width=actual_bw,
            y_offset=BUS_HEADER,
            max_belt_tier=max_belt_tier,
        )
        lanes = plan_bus_lanes(solver_result, row_spans, max_belt_tier=max_belt_tier)

    # 3. Route bus lanes (with crossing negotiation using row entities as obstacles)
    bus_entities, bus_max_y = route_bus(
        lanes,
        row_spans,
        total_height,
        actual_bw,
        max_belt_tier=max_belt_tier,
        row_entities=row_entities,
        solver_result=solver_result,
    )
    total_height = max(total_height, bus_max_y)

    # 4. Collect occupied tiles for pole placement
    occupied: set[tuple[int, int]] = set()
    machine_centers: list[tuple[int, int]] = []
    all_row_and_bus = row_entities + bus_entities

    for ent in all_row_and_bus:
        sz = machine_size(ent.name) if ent.name in _MACHINE_ENTITIES else 0
        if sz > 1:
            for dx in range(sz):
                for dy in range(sz):
                    occupied.add((ent.x + dx, ent.y + dy))
            center = (ent.x + sz // 2, ent.y + sz // 2)
            machine_centers.append(center)
        else:
            occupied.add((ent.x, ent.y))

    width = max(row_width, actual_bw)

    # 5. Place power poles
    pole_entities = place_poles(width, total_height, occupied, machine_centers)

    all_entities = row_entities + bus_entities + pole_entities

    return LayoutResult(
        entities=all_entities,
        width=width,
        height=total_height,
    )


_MACHINE_ENTITIES = {
    "assembling-machine-1",
    "assembling-machine-2",
    "assembling-machine-3",
    "chemical-plant",
    "electric-furnace",
    "oil-refinery",
}


def _estimate_bus_width(solver_result: SolverResult) -> int:
    """Estimate bus width before full lane planning."""
    # Count external solid inputs + intermediate items
    n_external = sum(1 for f in solver_result.external_inputs if not f.is_fluid)

    # Count intermediate items (items produced by one recipe, consumed by another)
    produced = set()
    consumed = set()
    for m in solver_result.machines:
        for out in m.outputs:
            if not out.is_fluid:
                produced.add(out.item)
        for inp in m.inputs:
            if not inp.is_fluid:
                consumed.add(inp.item)
    n_intermediate = len(produced & consumed)

    n_lanes = n_external + n_intermediate
    return max(2, n_lanes * 2 + 1)
