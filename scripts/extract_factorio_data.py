"""Extract Factorio recipe and entity data from draftsman into a static JSON file.

This JSON is embedded in the Rust WASM build so the solver can run without draftsman.
Run: uv run python scripts/extract_factorio_data.py
"""

import json
import sys
from pathlib import Path

from draftsman.data import entities as _entities
from draftsman.data import recipes as _recipes

# Recipe categories to exclude (not useful for production chains)
EXCLUDED_CATEGORIES = {"recycling", "crushing", "recycling-or-hand-crafting"}

# Machines we care about for crafting speed lookups
MACHINES = [
    "assembling-machine-1",
    "assembling-machine-2",
    "assembling-machine-3",
    "chemical-plant",
    "electric-furnace",
    "oil-refinery",
    "stone-furnace",
    "steel-furnace",
    "foundry",
    "electromagnetic-plant",
    "cryogenic-plant",
    "biochamber",
    "recycler",
    "crusher",
]


def extract_recipes() -> dict:
    """Extract all non-excluded recipes."""
    recipes = {}
    for name, raw in _recipes.raw.items():
        category = raw.get("category", "crafting")
        if category in EXCLUDED_CATEGORIES:
            continue

        ingredients = []
        for ing in raw.get("ingredients", []):
            ingredients.append({
                "name": ing["name"],
                "amount": ing["amount"],
                "type": ing.get("type", "item"),
            })

        results = []
        for prod in raw.get("results", []):
            entry = {
                "name": prod["name"],
                "amount": prod["amount"],
                "type": prod.get("type", "item"),
            }
            prob = prod.get("probability", 1.0)
            if prob != 1.0:
                entry["probability"] = prob
            results.append(entry)

        recipe = {
            "name": raw["name"],
            "category": category,
            "energy": raw.get("energy_required", 0.5),
            "ingredients": ingredients,
            "results": results,
        }
        recipes[name] = recipe

    return recipes


def extract_machines() -> dict:
    """Extract crafting speeds and fluid box definitions for relevant machines."""
    machines = {}
    for entity_name in MACHINES:
        raw = _entities.raw.get(entity_name)
        if raw is None:
            continue

        entry = {
            "crafting_speed": raw.get("crafting_speed", 1.0),
        }

        # Extract fluid box definitions if present
        fluid_boxes = raw.get("fluid_boxes")
        if fluid_boxes:
            boxes = []
            for fb in fluid_boxes:
                if not isinstance(fb, dict):
                    continue
                connections = []
                for pc in fb.get("pipe_connections", []):
                    conn = {}
                    if "position" in pc:
                        conn["position"] = pc["position"]
                    if "direction" in pc:
                        conn["direction"] = pc["direction"]
                    if conn:
                        connections.append(conn)
                if connections:
                    boxes.append({
                        "pipe_connections": connections,
                        "production_type": fb.get("production_type", "input"),
                    })
            if boxes:
                entry["fluid_boxes"] = boxes

        machines[entity_name] = entry

    return machines


def main():
    data = {
        "recipes": extract_recipes(),
        "machines": extract_machines(),
    }

    out_path = Path(__file__).parent.parent / "crates" / "core" / "data" / "recipes.json"
    out_path.parent.mkdir(parents=True, exist_ok=True)

    with open(out_path, "w") as f:
        json.dump(data, f, indent=2)

    recipe_count = len(data["recipes"])
    machine_count = len(data["machines"])
    size_kb = out_path.stat().st_size / 1024
    print(f"Extracted {recipe_count} recipes, {machine_count} machines -> {out_path} ({size_kb:.1f} KB)")


if __name__ == "__main__":
    main()
