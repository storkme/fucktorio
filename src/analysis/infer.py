"""Step 4: Infer what item each transport network carries."""

from __future__ import annotations

import logging

from .models import (
    AnalyzedMachine,
    FluidLink,
    InserterLink,
    TransportNetwork,
)

logger = logging.getLogger(__name__)


def infer_items(
    machines: list[AnalyzedMachine],
    networks: list[TransportNetwork],
    inserter_links: list[InserterLink],
    fluid_links: list[FluidLink],
) -> None:
    """Infer and set inferred_item on networks, inserter links, and fluid links.

    Strategy:
    1. Seed from fluid links (fluid port type + recipe → exact fluid)
    2. Seed from inserter links where machine has only one possible item for that role
    3. Propagate: if a network has a known item, label all links on it
    4. Disambiguate: if a machine has N input links on N different networks
       and N-1 are labeled, the last one gets the remaining item
    5. Repeat until stable
    """
    machine_map = {m.id: m for m in machines}
    net_map = {n.id: n for n in networks}

    # Track known items per network
    net_items: dict[int, str] = {}

    # --- Phase 1: Seed from fluid links ---
    for fl in fluid_links:
        machine = machine_map.get(fl.machine_id)
        if machine is None or machine.recipe is None:
            continue

        if fl.role == "input":
            # Fluid input — check which fluid ingredients this machine has
            fluid_inputs = [item for item in machine.inputs if _is_fluid_item(item, machine)]
            if len(fluid_inputs) == 1:
                net_items[fl.network_id] = fluid_inputs[0]
                fl.inferred_item = fluid_inputs[0]
        elif fl.role == "output":
            fluid_outputs = [item for item in machine.outputs if _is_fluid_item(item, machine)]
            if len(fluid_outputs) == 1:
                net_items[fl.network_id] = fluid_outputs[0]
                fl.inferred_item = fluid_outputs[0]

    # --- Phase 2: Seed from unambiguous inserter links ---
    for link in inserter_links:
        if link.network_id is None:
            continue
        if link.network_id in net_items:
            link.inferred_item = net_items[link.network_id]
            continue

        machine = machine_map.get(link.machine_id)
        if machine is None:
            continue

        candidates = machine.inputs if link.role == "input" else machine.outputs
        # Filter out fluids — inserters only carry solids
        solid_candidates = [item for item in candidates if not _is_fluid_item(item, machine)]
        if len(solid_candidates) == 1:
            net_items[link.network_id] = solid_candidates[0]
            link.inferred_item = solid_candidates[0]

    # --- Phase 3+4: Propagate and disambiguate until stable ---
    for _ in range(10):  # max iterations
        changed = False

        # Propagate network labels to links
        for link in inserter_links:
            if link.network_id is not None and link.inferred_item is None:
                if link.network_id in net_items:
                    link.inferred_item = net_items[link.network_id]
                    changed = True

        for fl in fluid_links:
            if fl.inferred_item is None and fl.network_id in net_items:
                fl.inferred_item = net_items[fl.network_id]
                changed = True

        # Disambiguate: for each machine, check if unlabeled links can be resolved
        for machine in machines:
            for role in ("input", "output"):
                candidates = machine.inputs if role == "input" else machine.outputs
                solid_candidates = [item for item in candidates if not _is_fluid_item(item, machine)]

                # Get all links for this machine+role
                role_links = [
                    lk
                    for lk in inserter_links
                    if lk.machine_id == machine.id and lk.role == role and lk.network_id is not None
                ]
                if not role_links:
                    continue

                # Find which items are already claimed
                claimed = {lk.inferred_item for lk in role_links if lk.inferred_item is not None}
                unclaimed = [item for item in solid_candidates if item not in claimed]
                unlabeled = [lk for lk in role_links if lk.inferred_item is None]

                if len(unclaimed) == 1 and len(unlabeled) >= 1:
                    # Only one item left — assign to all unlabeled links on distinct networks
                    item = unclaimed[0]
                    for lk in unlabeled:
                        if lk.network_id not in net_items:
                            net_items[lk.network_id] = item
                            lk.inferred_item = item
                            changed = True

        if not changed:
            break

    # --- Final: apply network labels ---
    for net in networks:
        if net.id in net_items:
            net.inferred_item = net_items[net.id]


def _is_fluid_item(item: str, machine: AnalyzedMachine) -> bool:
    """Check if an item is a fluid based on the machine's recipe data."""
    from ..solver.recipe_db import get_recipe

    if machine.recipe is None:
        return False
    try:
        recipe = get_recipe(machine.recipe)
    except KeyError:
        return False

    for ing in recipe.ingredients:
        if ing.name == item and ing.type == "fluid":
            return True
    for prod in recipe.products:
        if prod.name == item and prod.type == "fluid":
            return True
    return False
