"""Tests for the recipe solver."""

import math

from src.solver import solve
from src.solver.recipe_db import (
    find_recipe_for_item,
    get_crafting_speed,
    get_recipe,
    machine_for_recipe,
    recipe_has_fluid,
)


class TestRecipeDB:
    def test_get_recipe(self):
        r = get_recipe("electronic-circuit")
        assert r.name == "electronic-circuit"
        assert len(r.ingredients) == 2
        ing_names = {i.name for i in r.ingredients}
        assert "iron-plate" in ing_names
        assert "copper-cable" in ing_names

    def test_find_recipe_for_item(self):
        r = find_recipe_for_item("electronic-circuit")
        assert r is not None
        assert r.name == "electronic-circuit"

    def test_find_recipe_missing(self):
        r = find_recipe_for_item("nonexistent-item-xyz")
        assert r is None

    def test_crafting_speed(self):
        speed = get_crafting_speed("assembling-machine-3")
        assert speed == 1.25

    def test_crafting_speed_default(self):
        speed = get_crafting_speed("nonexistent-machine")
        assert speed == 1.0


class TestSolver:
    def test_electronic_circuit(self):
        result = solve(
            "electronic-circuit",
            30,
            available_inputs={"iron-plate", "copper-plate"},
        )
        # Should have 2 machine specs: electronic-circuit and copper-cable
        recipes = {m.recipe for m in result.machines}
        assert "electronic-circuit" in recipes
        assert "copper-cable" in recipes

        # Check machine counts
        ec = next(m for m in result.machines if m.recipe == "electronic-circuit")
        # asm3 speed=1.25, energy=0.5 → crafts/s=2.5, output=2.5/s per machine
        # 30/2.5 = 12 machines
        assert math.isclose(ec.count, 12.0, rel_tol=1e-6)

        cc = next(m for m in result.machines if m.recipe == "copper-cable")
        # Each EC craft needs 3 copper-cable, 12 machines * 2.5 crafts/s = 30 crafts/s
        # → 90 copper-cable/s needed
        # CC recipe: 1 copper-plate → 2 cable, energy=0.5
        # crafts/s per machine = 2.5, output = 5/s per machine
        # 90/5 = 18 machines
        assert math.isclose(cc.count, 18.0, rel_tol=1e-6)

    def test_external_inputs(self):
        result = solve(
            "electronic-circuit",
            30,
            available_inputs={"iron-plate", "copper-plate"},
        )
        ext = {f.item: f.rate for f in result.external_inputs}
        assert "iron-plate" in ext
        assert "copper-plate" in ext
        assert math.isclose(ext["iron-plate"], 30.0, rel_tol=1e-6)
        assert math.isclose(ext["copper-plate"], 45.0, rel_tol=1e-6)

    def test_single_recipe(self):
        result = solve(
            "iron-gear-wheel",
            10,
            available_inputs={"iron-plate"},
        )
        assert len(result.machines) == 1
        m = result.machines[0]
        assert m.recipe == "iron-gear-wheel"
        # energy=0.5 (default), 1 product, speed=1.25
        # crafts/s = 2.5, output = 2.5/s → 10/2.5 = 4
        assert math.isclose(m.count, 4.0, rel_tol=1e-6)

    def test_fluid_recipe_uses_chemical_plant(self):
        """Chemistry recipes should auto-select chemical-plant."""
        result = solve(
            "plastic-bar",
            10,
            available_inputs={"petroleum-gas", "coal"},
        )
        assert len(result.machines) == 1
        m = result.machines[0]
        assert m.entity == "chemical-plant"
        assert m.recipe == "plastic-bar"
        # chemical-plant speed=1.0, energy=1s, 2 products per craft
        # crafts/s = 1.0, output = 2.0/s → 10/2 = 5
        assert math.isclose(m.count, 5.0, rel_tol=1e-6)

    def test_fluid_inputs_marked(self):
        """Fluid external inputs should have is_fluid=True."""
        result = solve(
            "plastic-bar",
            10,
            available_inputs={"petroleum-gas", "coal"},
        )
        ext = {f.item: f for f in result.external_inputs}
        assert ext["petroleum-gas"].is_fluid is True
        assert ext["coal"].is_fluid is False

    def test_machine_for_recipe_db(self):
        r = get_recipe("plastic-bar")
        assert machine_for_recipe(r) == "chemical-plant"
        assert recipe_has_fluid(r) is True

        r2 = get_recipe("iron-gear-wheel")
        assert machine_for_recipe(r2) == "assembling-machine-3"
        assert recipe_has_fluid(r2) is False

    def test_sulfuric_acid(self):
        """Sulfuric acid: fluid output recipe."""
        result = solve(
            "sulfuric-acid",
            50,
            available_inputs={"sulfur", "iron-plate", "water"},
        )
        m = result.machines[0]
        assert m.entity == "chemical-plant"
        # energy=1s, speed=1.0, 50 per craft → crafts/s=1, output=50/s → 1 machine
        assert math.isclose(m.count, 1.0, rel_tol=1e-6)

    def test_processing_unit_mixed(self):
        """Processing unit uses assembler (electronics-with-fluid category)."""
        result = solve(
            "processing-unit",
            1,
            available_inputs={"iron-plate", "copper-plate", "plastic-bar", "sulfuric-acid"},
        )
        pu = next(m for m in result.machines if m.recipe == "processing-unit")
        # electronics-with-fluid is handled by assembling-machine-3
        assert pu.entity == "assembling-machine-3"
        # Has fluid input
        fluid_inputs = [f for f in pu.inputs if f.is_fluid]
        assert len(fluid_inputs) > 0

    def test_basic_oil_processing(self):
        """Basic oil processing should use oil-refinery."""
        result = solve(
            "petroleum-gas",
            10,
            available_inputs={"crude-oil"},
        )
        assert len(result.machines) == 1
        m = result.machines[0]
        assert m.entity == "oil-refinery"
        assert m.recipe == "basic-oil-processing"
        # oil-refinery speed=1.0, energy=5s → crafts/s=0.2, output=45*0.2=9/s per machine
        # 10/9 ≈ 1.111
        assert math.isclose(m.count, 10 / 9, rel_tol=1e-6)

    def test_oil_recipe_multi_output(self):
        """Advanced oil processing produces multiple fluid outputs."""
        r = get_recipe("advanced-oil-processing")
        assert machine_for_recipe(r) == "oil-refinery"
        assert recipe_has_fluid(r) is True
        # Should have 3 products
        product_names = {p.name for p in r.products}
        assert "heavy-oil" in product_names
        assert "light-oil" in product_names
        assert "petroleum-gas" in product_names

    def test_oil_external_inputs_fluid(self):
        """Oil recipe external inputs should be marked as fluid."""
        result = solve(
            "petroleum-gas",
            10,
            available_inputs={"crude-oil"},
        )
        ext = {f.item: f for f in result.external_inputs}
        assert "crude-oil" in ext
        assert ext["crude-oil"].is_fluid is True

    def test_coal_liquefaction_uses_refinery(self):
        """Coal liquefaction (oil-processing category) should use oil-refinery."""
        r = get_recipe("simple-coal-liquefaction")
        assert machine_for_recipe(r) == "oil-refinery"
        # Has both solid and fluid inputs
        solid_ings = [i for i in r.ingredients if i.type == "item"]
        fluid_ings = [i for i in r.ingredients if i.type == "fluid"]
        assert len(solid_ings) > 0
        assert len(fluid_ings) > 0
