"""Bus layout orchestrator: rows + bus lanes + poles -> LayoutResult."""

from __future__ import annotations

from ..models import EntityDirection, LayoutResult, SolverResult
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
    # Final product items get EAST-flowing output belts (merge at right side)
    final_output_items = {ext.item for ext in solver_result.external_outputs if not ext.is_fluid}

    # 1. Pre-plan bus lanes to know bus width before placing rows
    BUS_HEADER = 1

    temp_bw = _estimate_bus_width(solver_result)
    row_entities, row_spans, row_width, total_height = place_rows(
        solver_result.machines,
        solver_result.dependency_order,
        bus_width=temp_bw,
        y_offset=BUS_HEADER,
        max_belt_tier=max_belt_tier,
        final_output_items=final_output_items,
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
            final_output_items=final_output_items,
        )
        lanes = plan_bus_lanes(solver_result, row_spans, max_belt_tier=max_belt_tier)

    # 3. Route bus lanes (with crossing negotiation using row entities as obstacles)
    bus_entities, bus_max_y, merge_max_x = route_bus(
        lanes,
        row_spans,
        total_height,
        actual_bw,
        max_belt_tier=max_belt_tier,
        row_entities=row_entities,
        solver_result=solver_result,
    )
    total_height = max(total_height, bus_max_y)

    # 3b. Remove row entities that overlap with bus splitters.
    # WEST splitters occupy (x, y) and (x, y+1); SOUTH splitters occupy
    # (x, y) and (x+1, y).  Filter row entities at those positions.
    _SPLITTER_NAMES = {"splitter", "fast-splitter", "express-splitter"}
    bus_occupied: set[tuple[int, int]] = set()
    for ent in bus_entities:
        if ent.name in _SPLITTER_NAMES:
            bus_occupied.add((ent.x, ent.y))
            if ent.direction in (EntityDirection.WEST, EntityDirection.EAST):
                bus_occupied.add((ent.x, ent.y + 1))
            else:
                bus_occupied.add((ent.x + 1, ent.y))
    if bus_occupied:
        row_entities = [e for e in row_entities if (e.x, e.y) not in bus_occupied]

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

    width = max(row_width, actual_bw, merge_max_x)

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
