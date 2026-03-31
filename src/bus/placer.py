"""Stacks assembly rows vertically in dependency order."""

from __future__ import annotations

import math
from dataclasses import dataclass

from ..models import MachineSpec, PlacedEntity
from ..routing.common import belt_entity_for_rate
from .templates import dual_input_row, single_input_row


@dataclass
class RowSpan:
    """Where a row sits in the layout and what it contains."""

    y_start: int
    y_end: int  # exclusive
    spec: MachineSpec
    machine_count: int
    input_belt_y: list[int]  # y-coordinates of input belt rows
    output_belt_y: int  # y-coordinate of output belt row


def place_rows(
    machines: list[MachineSpec],
    dependency_order: list[str],
    bus_width: int,
    y_offset: int = 0,
) -> tuple[list[PlacedEntity], list[RowSpan], int, int]:
    """Place assembly rows stacked vertically.

    Returns (entities, row_spans, total_width, total_height).
    """
    entities: list[PlacedEntity] = []
    row_spans: list[RowSpan] = []
    y_cursor = y_offset

    ordered = _order_specs(machines, dependency_order)
    max_width = 0

    for spec in ordered:
        count = max(1, math.ceil(spec.count))
        solid_inputs = [f for f in spec.inputs if not f.is_fluid]
        solid_outputs = [f for f in spec.outputs if not f.is_fluid]

        output_item = solid_outputs[0].item if solid_outputs else ""

        # Pick belt tiers based on total throughput across all machines.
        # All inserters drop on the same lane, so we need 2x the total rate
        # to ensure per-lane capacity is sufficient.
        output_rate = solid_outputs[0].rate * count if solid_outputs else 0
        out_belt = belt_entity_for_rate(output_rate * 2)

        if len(solid_inputs) <= 1:
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
            input_belt_ys = [y_cursor]  # y+0
            output_belt_y = y_cursor + 6  # y+6
        else:
            input_items = (
                solid_inputs[0].item,
                solid_inputs[1].item,
            )
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
            input_belt_ys = [y_cursor, y_cursor + 1]  # y+0, y+1
            output_belt_y = y_cursor + 7  # y+7

        entities.extend(row_ents)

        row_spans.append(
            RowSpan(
                y_start=y_cursor,
                y_end=y_cursor + row_h,
                spec=spec,
                machine_count=count,
                input_belt_y=input_belt_ys,
                output_belt_y=output_belt_y,
            )
        )

        machine_pitch = 6 if spec.entity == "oil-refinery" else 4
        row_width = bus_width + count * machine_pitch
        max_width = max(max_width, row_width)
        y_cursor += row_h + 1  # 1-tile gap between rows

    return entities, row_spans, max_width, y_cursor


def _order_specs(
    machines: list[MachineSpec],
    dependency_order: list[str],
) -> list[MachineSpec]:
    """Return machine specs with upstream (producing) recipes first.

    The solver's dependency_order lists the final product first and raw
    inputs last.  For a bus layout we reverse this so that upstream rows
    sit at the top and items flow SOUTH naturally to consuming rows below.
    """
    ordered: list[MachineSpec] = []
    recipe_to_spec = {m.recipe: m for m in machines}
    for recipe in reversed(dependency_order):
        if recipe in recipe_to_spec:
            ordered.append(recipe_to_spec[recipe])
    for m in machines:
        if m not in ordered:
            ordered.append(m)
    return ordered
