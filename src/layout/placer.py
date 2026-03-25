"""Stacks assembly rows vertically, manages the overall grid."""

from __future__ import annotations

import math

from ..models import MachineSpec, PlacedEntity
from .templates import single_input_row, dual_input_row, fluid_row


def _has_fluid(spec: MachineSpec) -> bool:
    """True if any input or output is a fluid."""
    return any(f.is_fluid for f in spec.inputs + spec.outputs)


def place_rows(
    machines: list[MachineSpec],
    dependency_order: list[str],
    bus_width: int = 6,
) -> tuple[list[PlacedEntity], int, int]:
    """Place assembly rows for each MachineSpec, stacked vertically.

    Returns (all_entities, total_width, total_height).
    """
    entities: list[PlacedEntity] = []
    y_cursor = 0

    # Place rows in dependency order (inputs first, final product last)
    ordered_specs = []
    recipe_to_spec = {m.recipe: m for m in machines}
    for recipe in dependency_order:
        if recipe in recipe_to_spec:
            ordered_specs.append(recipe_to_spec[recipe])
    # Add any specs not in dependency_order
    for m in machines:
        if m not in ordered_specs:
            ordered_specs.append(m)

    max_width = 0

    for spec in ordered_specs:
        count = max(1, math.ceil(spec.count))
        num_solid_inputs = sum(1 for f in spec.inputs if not f.is_fluid)

        if _has_fluid(spec):
            row_ents, row_h = fluid_row(
                recipe=spec.recipe,
                machine_entity=spec.entity,
                machine_count=count,
                y_offset=y_cursor,
                x_offset=bus_width,
                inputs=spec.inputs,
            )
        elif num_solid_inputs <= 1:
            row_ents, row_h = single_input_row(
                recipe=spec.recipe,
                machine_entity=spec.entity,
                machine_count=count,
                y_offset=y_cursor,
                x_offset=bus_width,
            )
        else:
            row_ents, row_h = dual_input_row(
                recipe=spec.recipe,
                machine_entity=spec.entity,
                machine_count=count,
                y_offset=y_cursor,
                x_offset=bus_width,
            )

        entities.extend(row_ents)
        row_width = bus_width + count * 4
        max_width = max(max_width, row_width)
        y_cursor += row_h + 1  # 1-tile gap between rows

    return entities, max_width, y_cursor
