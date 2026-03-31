"""Recursive recipe solver: desired output → machine counts & flows."""

from __future__ import annotations

from ..models import ItemFlow, MachineSpec, SolverResult
from .recipe_db import find_recipe_for_item, get_crafting_speed, machine_for_recipe


def solve(
    target_item: str,
    target_rate: float,
    available_inputs: set[str] | None = None,
    machine_entity: str = "assembling-machine-3",
) -> SolverResult:
    """Compute machines needed to produce *target_item* at *target_rate* items/sec.

    Recursively resolves intermediate recipes until hitting items in
    *available_inputs* (which the user must supply externally).

    *machine_entity* is the default assembler. Chemistry recipes automatically
    use chemical-plant, oil recipes use oil-refinery.
    """
    if available_inputs is None:
        available_inputs = set()

    machines: list[MachineSpec] = []
    external_inputs: dict[str, float] = {}  # item → total rate
    external_inputs_fluid: dict[str, bool] = {}  # item → is_fluid
    dependency_order: list[str] = []
    resolving: set[str] = set()  # cycle detection

    def _resolve(item: str, rate: float, is_fluid: bool = False) -> None:
        """Resolve *item* at *rate* items/sec, accumulating into outer lists."""
        if item in available_inputs:
            external_inputs[item] = external_inputs.get(item, 0.0) + rate
            external_inputs_fluid[item] = is_fluid
            return

        recipe = find_recipe_for_item(item)
        if recipe is None or item in resolving:
            external_inputs[item] = external_inputs.get(item, 0.0) + rate
            external_inputs_fluid[item] = is_fluid
            return

        resolving.add(item)

        # Pick the right machine for this recipe's category
        entity = machine_for_recipe(recipe, default=machine_entity)
        crafting_speed = get_crafting_speed(entity)

        # How many items does one craft cycle produce?
        products_per_craft = 0.0
        for prod in recipe.products:
            if prod.name == item:
                products_per_craft += prod.amount * prod.probability

        if products_per_craft <= 0:
            raise ValueError(f"Recipe {recipe.name} produces 0 of {item}")

        # Crafts per second per machine
        crafts_per_sec = crafting_speed / recipe.energy
        items_per_sec_per_machine = crafts_per_sec * products_per_craft
        count = rate / items_per_sec_per_machine

        # Per-machine input/output flows
        input_flows = [
            ItemFlow(
                item=ing.name,
                rate=ing.amount * crafts_per_sec,
                is_fluid=(ing.type == "fluid"),
            )
            for ing in recipe.ingredients
        ]
        output_flows = [
            ItemFlow(
                item=prod.name,
                rate=prod.amount * prod.probability * crafts_per_sec,
                is_fluid=(prod.type == "fluid"),
            )
            for prod in recipe.products
        ]

        # If we've already resolved this recipe, merge counts
        existing = next((m for m in machines if m.recipe == recipe.name), None)
        if existing is not None:
            existing.count += count
        else:
            machines.append(
                MachineSpec(
                    entity=entity,
                    recipe=recipe.name,
                    count=count,
                    inputs=input_flows,
                    outputs=output_flows,
                )
            )
            dependency_order.append(recipe.name)

        # Recurse into ingredients
        for ing in recipe.ingredients:
            ingredient_rate = ing.amount * crafts_per_sec * count
            _resolve(ing.name, ingredient_rate, is_fluid=(ing.type == "fluid"))

        resolving.discard(item)

    _resolve(target_item, target_rate)

    ext_in = [
        ItemFlow(item=k, rate=v, is_fluid=external_inputs_fluid.get(k, False)) for k, v in external_inputs.items()
    ]
    ext_out = [ItemFlow(item=target_item, rate=target_rate)]

    return SolverResult(
        machines=machines,
        external_inputs=ext_in,
        external_outputs=ext_out,
        dependency_order=dependency_order,
    )
