"""Stacks assembly rows vertically in dependency order."""

from __future__ import annotations

import math
from dataclasses import dataclass, field

from ..models import MachineSpec, PlacedEntity
from ..routing.common import _LANE_CAPACITY, belt_entity_for_rate
from .templates import dual_input_row, fluid_input_row, single_input_row


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


def _max_machines_for_belt(spec: MachineSpec, belt_name: str) -> int:
    """Max machines in one row before OUTPUT exceeds belt lane capacity.

    Only checks output rates — input throughput is handled by the bus
    lane which selects its own belt tier independently.
    """
    cap = _LANE_CAPACITY.get(belt_name, 7.5)
    max_m = 999

    for out in spec.outputs:
        if not out.is_fluid and out.rate > 0:
            max_m = min(max_m, int(cap / out.rate))

    return max(1, max_m)


def place_rows(
    machines: list[MachineSpec],
    dependency_order: list[str],
    bus_width: int,
    y_offset: int = 0,
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

    for spec in ordered:
        total_count = max(1, math.ceil(spec.count))

        # Determine belt tier and max machines per row
        solid_outputs = [f for f in spec.outputs if not f.is_fluid]
        output_rate = solid_outputs[0].rate * total_count if solid_outputs else 0
        out_belt = belt_entity_for_rate(output_rate * 2)
        max_per_row = _max_machines_for_belt(spec, out_belt)

        # Split into chunks
        remaining = total_count
        while remaining > 0:
            chunk = min(remaining, max_per_row)
            ents, span, width = _build_one_row(spec, chunk, bus_width, y_cursor)
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
) -> tuple[list[PlacedEntity], RowSpan, int]:
    """Build a single row of machines. Returns (entities, span, width)."""
    solid_inputs = [f for f in spec.inputs if not f.is_fluid]
    solid_outputs = [f for f in spec.outputs if not f.is_fluid]
    fluid_inputs = [f for f in spec.inputs if f.is_fluid]

    output_item = solid_outputs[0].item if solid_outputs else ""

    # Belt tiers based on THIS chunk's throughput (not total)
    output_rate = solid_outputs[0].rate * count if solid_outputs else 0
    out_belt = belt_entity_for_rate(output_rate * 2)

    fluid_port_ys: list[int] = []
    fluid_port_pipes: list[tuple[int, int]] = []

    if fluid_inputs and solid_inputs:
        input_item = solid_inputs[0].item
        fluid_item = fluid_inputs[0].item
        input_rate = solid_inputs[0].rate * count
        in_belt = belt_entity_for_rate(input_rate * 2)
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
        in_belt = belt_entity_for_rate(input_rate * 2)
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
        )
        input_belt_ys = [y_cursor]
        output_belt_y = y_cursor + 6
    else:
        input_items = (solid_inputs[0].item, solid_inputs[1].item)
        in_belt1 = belt_entity_for_rate(solid_inputs[0].rate * count * 2)
        in_belt2 = belt_entity_for_rate(solid_inputs[1].rate * count * 2)
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
        )
        input_belt_ys = [y_cursor, y_cursor + 1]
        output_belt_y = y_cursor + row_h - 1

    machine_pitch = 5 if spec.entity == "oil-refinery" else 3
    row_width = bus_width + count * machine_pitch

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
