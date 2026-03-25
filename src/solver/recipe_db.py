"""Recipe and entity lookups backed by draftsman data."""

from __future__ import annotations

from dataclasses import dataclass
from draftsman.data import recipes as _recipes
from draftsman.data import entities as _entities


@dataclass
class Ingredient:
    name: str
    amount: float
    type: str = "item"  # "item" or "fluid"


@dataclass
class Product:
    name: str
    amount: float
    type: str = "item"
    probability: float = 1.0


@dataclass
class Recipe:
    name: str
    category: str
    energy: float          # crafting time in seconds
    ingredients: list[Ingredient]
    products: list[Product]


# Default crafting time when not specified in data
_DEFAULT_ENERGY = 0.5


def get_recipe(name: str) -> Recipe:
    """Look up a recipe by its internal name. Raises KeyError if not found."""
    raw = _recipes.raw[name]

    ingredients = [
        Ingredient(
            name=ing["name"],
            amount=ing["amount"],
            type=ing.get("type", "item"),
        )
        for ing in raw["ingredients"]
    ]

    products = [
        Product(
            name=prod["name"],
            amount=prod["amount"],
            type=prod.get("type", "item"),
            probability=prod.get("probability", 1.0),
        )
        for prod in raw.get("results", [])
    ]

    return Recipe(
        name=raw["name"],
        category=raw.get("category", "crafting"),
        energy=raw.get("energy_required", _DEFAULT_ENERGY),
        ingredients=ingredients,
        products=products,
    )


def find_recipe_for_item(item: str) -> Recipe | None:
    """Find the first recipe whose products include *item*.

    Returns None if no recipe produces this item.
    """
    for name, raw in _recipes.raw.items():
        results = raw.get("results", [])
        for prod in results:
            if prod["name"] == item:
                return get_recipe(name)
    return None


def get_crafting_speed(entity: str) -> float:
    """Return the crafting_speed of an assembler / furnace entity."""
    raw = _entities.raw.get(entity, {})
    return raw.get("crafting_speed", 1.0)


def recipe_exists(name: str) -> bool:
    return name in _recipes.raw


# Maps recipe categories to the machine that can craft them.
# Order matters: first match wins when multiple machines could work.
_CATEGORY_TO_MACHINE: list[tuple[set[str], str]] = [
    (
        {"chemistry", "chemistry-or-cryogenics", "organic-or-chemistry"},
        "chemical-plant",
    ),
    (
        {"oil-processing"},
        "oil-refinery",
    ),
    # assembling-machine-3 covers everything else we care about
]


def machine_for_recipe(recipe: Recipe, default: str = "assembling-machine-3") -> str:
    """Choose the right machine entity for a recipe based on its category."""
    cat = recipe.category
    for categories, machine in _CATEGORY_TO_MACHINE:
        if cat in categories:
            return machine
    return default


def recipe_has_fluid(recipe: Recipe) -> bool:
    """True if any ingredient or product is a fluid."""
    for ing in recipe.ingredients:
        if ing.type == "fluid":
            return True
    for prod in recipe.products:
        if prod.type == "fluid":
            return True
    return False
