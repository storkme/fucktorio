"""Stacks assembly rows vertically in dependency order."""

from __future__ import annotations

import math
from dataclasses import dataclass, field

from ..models import MachineSpec, PlacedEntity
from ..routing.common import _LANE_CAPACITY, belt_entity_for_rate
from .templates import LANE_SPLIT_GAP, dual_input_row, fluid_input_row, single_input_row


@dataclass
class RowSpan:
    """Where a row sits in the layout and what it contains."""

    y_start: int
    y_end: int  # exclusive
    spec: MachineSpec
    machine_count: int
    input_belt_y: list[int]  # y-coordinates of input belt rows
    output_belt_y: int  # y-coordinate of output belt row
    fluid_port_ys: list[int] = field(default_factory=list)
    fluid_port_pipes: list[tuple[int, int]] = field(default_factory=list)


def _max_machines_for_belt(spec: MachineSpec, belt_name: str, max_belt_tier: str | None = None) -> int:
    """Max machines in one row before output or input exceeds belt lane capacity.

    Checks output rate against the output belt, and input rate against the
    best available belt tier (constrained or max).
    """
    cap = _LANE_CAPACITY.get(belt_name, 7.5)
    max_m = 999

    for out in spec.outputs:
        if not out.is_fluid and out.rate > 0:
            max_m = min(max_m, int(cap / out.rate))

    max_in_cap = max(_LANE_CAPACITY.values())
    in_cap = _LANE_CAPACITY.get(max_belt_tier, max_in_cap) if max_belt_tier else max_in_cap
    for inp in spec.inputs:
        if not inp.is_fluid and inp.rate > 0:
            per_lane = int(in_cap / inp.rate)
            max_m = min(max_m, per_lane * 2)

    return max(1, max_m)


def _max_machines_for_belt_both_lanes(spec: MachineSpec, belt_name: str, max_belt_tier: str | None = None) -> int:
    """Max machines when using BOTH belt lanes (lane-split output).

    Each lane has its own capacity limit. The total is 2x the per-lane max,
    not int(full_capacity / rate), because integer truncation matters.
    Input throughput is always checked against the best available belt tier.
    """
    lane_cap = _LANE_CAPACITY.get(belt_name, 7.5)
    max_m = 999

    for out in spec.outputs:
        if not out.is_fluid and out.rate > 0:
            per_lane = int(lane_cap / out.rate)
            max_m = min(max_m, per_lane * 2)

    max_in_cap = max(_LANE_CAPACITY.values())
    in_cap = _LANE_CAPACITY.get(max_belt_tier, max_in_cap) if max_belt_tier else max_in_cap
    for inp in spec.inputs:
        if not inp.is_fluid and inp.rate > 0:
            per_lane = int(in_cap / inp.rate)
            max_m = min(max_m, per_lane * 2)

    return max(1, max_m)


def place_rows(
    machines: list[MachineSpec],
    dependency_order: list[str],
    bus_width: int,
    y_offset: int = 0,
    max_belt_tier: str | None = None,
) -> tuple[list[PlacedEntity], list[RowSpan], int, int]:
    """Place assembly rows stacked vertically.

    When a recipe needs more machines than a single belt can handle,
    the row is split into multiple sub-rows.

    Returns (entities, row_spans, total_width, total_height).
    """
    entities: list[PlacedEntity] = []
    row_spans: list[RowSpan] = []
    y_cursor = y_offset

    ordered = _order_specs(machines, dependency_order)
    max_width = 0

    for spec_idx, spec in enumerate(ordered):
        if spec_idx > 0:
            y_cursor += 2  # gap between recipes for lane balancers
        total_count = max(1, math.ceil(spec.count))

        # Determine belt tier and max machines per row.
        # With lane splitting, both lanes are used so full belt capacity applies.
        solid_outputs = [f for f in spec.outputs if not f.is_fluid]
        output_rate = solid_outputs[0].rate * total_count if solid_outputs else 0
        has_fluid = any(f.is_fluid for f in spec.inputs)
        # Fluid rows don't support lane splitting yet — use single-lane math
        if has_fluid:
            out_belt = belt_entity_for_rate(output_rate * 2, max_tier=max_belt_tier)
            max_per_row = _max_machines_for_belt(spec, out_belt, max_belt_tier)
        else:
            out_belt = belt_entity_for_rate(output_rate, max_tier=max_belt_tier)
            max_per_row = _max_machines_for_belt_both_lanes(spec, out_belt, max_belt_tier)

        # Split into evenly-sized chunks.  With 1:1 lane-to-consumer mapping,
        # even rows ensure each bus lane receives the same production rate.
        n_rows = math.ceil(total_count / max_per_row)
        remaining = total_count
        for ri in range(n_rows):
            chunk = math.ceil(remaining / (n_rows - ri))
            ents, span, width = _build_one_row(spec, chunk, bus_width, y_cursor, max_belt_tier)
            entities.extend(ents)
            row_spans.append(span)
            max_width = max(max_width, width)
            y_cursor = span.y_end
            remaining -= chunk

    return entities, row_spans, max_width, y_cursor


def _build_one_row(
    spec: MachineSpec,
    count: int,
    bus_width: int,
    y_cursor: int,
    max_belt_tier: str | None = None,
) -> tuple[list[PlacedEntity], RowSpan, int]:
    """Build a single row of machines. Returns (entities, span, width)."""
    solid_inputs = [f for f in spec.inputs if not f.is_fluid]
    solid_outputs = [f for f in spec.outputs if not f.is_fluid]
    fluid_inputs = [f for f in spec.inputs if f.is_fluid]

    output_item = solid_outputs[0].item if solid_outputs else ""

    # Lane splitting: use both belt lanes when count >= 2 (not for fluid rows)
    has_fluid = bool(fluid_inputs and solid_inputs)
    lane_split = count >= 2 and not has_fluid

    # Belt tiers based on THIS chunk's throughput
    output_rate = solid_outputs[0].rate * count if solid_outputs else 0
    out_belt = belt_entity_for_rate(output_rate * (1 if lane_split else 2), max_tier=max_belt_tier)

    fluid_port_ys: list[int] = []
    fluid_port_pipes: list[tuple[int, int]] = []

    if has_fluid:
        input_item = solid_inputs[0].item
        fluid_item = fluid_inputs[0].item
        input_rate = solid_inputs[0].rate * count
        in_belt = belt_entity_for_rate(input_rate * 2, max_tier=max_belt_tier)
        row_ents, row_h, port_pipes = fluid_input_row(
            recipe=spec.recipe,
            machine_entity=spec.entity,
            machine_count=count,
            y_offset=y_cursor,
            x_offset=bus_width,
            solid_item=input_item,
            fluid_item=fluid_item,
            output_item=output_item,
            input_belt=in_belt,
            output_belt=out_belt,
        )
        input_belt_ys = [y_cursor]
        output_belt_y = y_cursor + 6
        fluid_port_ys = [port_pipes[0][1]] if port_pipes else []
        fluid_port_pipes = port_pipes
    elif len(solid_inputs) <= 1:
        input_item = solid_inputs[0].item if solid_inputs else ""
        input_rate = solid_inputs[0].rate * count if solid_inputs else 0
        in_belt = belt_entity_for_rate(input_rate * 2, max_tier=max_belt_tier)
        row_ents, row_h = single_input_row(
            recipe=spec.recipe,
            machine_entity=spec.entity,
            machine_count=count,
            y_offset=y_cursor,
            x_offset=bus_width,
            input_item=input_item,
            output_item=output_item,
            input_belt=in_belt,
            output_belt=out_belt,
            lane_split=lane_split,
        )
        input_belt_ys = [y_cursor]
        output_belt_y = y_cursor + 6
    else:
        input_items = (solid_inputs[0].item, solid_inputs[1].item)
        in_belt1 = belt_entity_for_rate(solid_inputs[0].rate * count * 2, max_tier=max_belt_tier)
        in_belt2 = belt_entity_for_rate(solid_inputs[1].rate * count * 2, max_tier=max_belt_tier)
        row_ents, row_h = dual_input_row(
            recipe=spec.recipe,
            machine_entity=spec.entity,
            machine_count=count,
            y_offset=y_cursor,
            x_offset=bus_width,
            input_items=input_items,
            output_item=output_item,
            input_belts=(in_belt1, in_belt2),
            output_belt=out_belt,
            lane_split=lane_split,
        )
        input_belt_ys = [y_cursor, y_cursor + 1]
        output_belt_y = y_cursor + row_h - 1

    machine_pitch = 5 if spec.entity == "oil-refinery" else 3
    gap = LANE_SPLIT_GAP if lane_split else 0
    row_width = bus_width + count * machine_pitch + gap

    span = RowSpan(
        y_start=y_cursor,
        y_end=y_cursor + row_h,
        spec=spec,
        machine_count=count,
        input_belt_y=input_belt_ys,
        output_belt_y=output_belt_y,
        fluid_port_ys=fluid_port_ys,
        fluid_port_pipes=fluid_port_pipes,
    )

    return row_ents, span, row_width


def _order_specs(
    machines: list[MachineSpec],
    dependency_order: list[str],
) -> list[MachineSpec]:
    """Return machine specs with upstream (producing) recipes first."""
    ordered: list[MachineSpec] = []
    recipe_to_spec = {m.recipe: m for m in machines}
    for recipe in reversed(dependency_order):
        if recipe in recipe_to_spec:
            ordered.append(recipe_to_spec[recipe])
    for m in machines:
        if m not in ordered:
            ordered.append(m)
    return ordered
