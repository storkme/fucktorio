"""Tests for the blueprint exporter."""

from draftsman.blueprintable import get_blueprintable_from_string

from src.blueprint import build_blueprint
from src.models import EntityDirection, LayoutResult, PlacedEntity


class TestBlueprint:
    def test_valid_string(self):
        lr = LayoutResult(
            entities=[
                PlacedEntity(name="assembling-machine-3", x=0, y=0, recipe="iron-gear-wheel"),
                PlacedEntity(name="inserter", x=1, y=-1, direction=EntityDirection.SOUTH),
                PlacedEntity(name="transport-belt", x=1, y=-2, direction=EntityDirection.EAST),
            ],
            width=3,
            height=5,
        )
        bp_str = build_blueprint(lr, label="test")
        assert isinstance(bp_str, str)
        assert len(bp_str) > 10

        # Re-import should succeed
        bp = get_blueprintable_from_string(bp_str)
        assert len(bp.entities) == 3

    def test_roundtrip(self):
        lr = LayoutResult(
            entities=[
                PlacedEntity(name="transport-belt", x=0, y=0, direction=EntityDirection.EAST),
                PlacedEntity(name="transport-belt", x=1, y=0, direction=EntityDirection.EAST),
            ],
            width=2,
            height=1,
        )
        bp_str = build_blueprint(lr)
        bp = get_blueprintable_from_string(bp_str)
        assert bp.label == "Generated Factory"
        names = [e.name for e in bp.entities]
        assert names.count("transport-belt") == 2

    def test_end_to_end(self, viz):
        """Full pipeline produces a re-importable blueprint."""
        from src.pipeline import produce

        bp_str = produce(
            "electronic-circuit",
            rate=30,
            inputs=["iron-plate", "copper-plate"],
        )
        viz(bp_str, "electronic-circuit-30s")
        bp = get_blueprintable_from_string(bp_str)
        assert len(bp.entities) > 50

        # Check assemblers have recipes set
        asms = [e for e in bp.entities if e.name == "assembling-machine-3"]
        assert len(asms) > 0
        for asm in asms:
            assert asm.recipe is not None

    def test_advanced_circuit_full_chain(self, viz):
        """Advanced circuit: deep recipe chain with fluid intermediates.

        advanced-circuit needs:
          - electronic-circuit (assembler) ← iron-plate + copper-cable
          - copper-cable (assembler) ← copper-plate
          - plastic-bar (chemical-plant) ← petroleum-gas (fluid!) + coal

        This tests the full pipeline handling mixed solid/fluid recipes,
        chemical plants alongside assemblers, underground belts for the
        solid bus lanes, and pipe-to-ground for the fluid bus lane.
        """
        import math

        from src.layout import layout
        from src.pipeline import produce
        from src.solver import solve

        # --- Solver checks ---
        result = solve(
            "advanced-circuit",
            5,
            available_inputs={"iron-plate", "copper-plate", "petroleum-gas", "coal"},
        )

        recipes_used = {m.recipe: m for m in result.machines}
        assert "advanced-circuit" in recipes_used
        assert "electronic-circuit" in recipes_used
        assert "copper-cable" in recipes_used
        assert "plastic-bar" in recipes_used

        # advanced-circuit: asm3 speed=1.25, energy=6s, 1 product
        # crafts/s = 1.25/6 ≈ 0.2083, output ≈ 0.2083/s → 5/0.2083 = 24
        ac = recipes_used["advanced-circuit"]
        assert ac.entity == "assembling-machine-3"
        assert math.isclose(ac.count, 24.0, rel_tol=1e-6)

        # plastic-bar: chemical-plant speed=1.0, energy=1s, 2 products
        # crafts/s = 1.0, output = 2.0/s → 10/2 = 5
        pb = recipes_used["plastic-bar"]
        assert pb.entity == "chemical-plant"
        assert math.isclose(pb.count, 5.0, rel_tol=1e-6)

        # petroleum-gas should be a fluid external input
        ext = {f.item: f for f in result.external_inputs}
        assert ext["petroleum-gas"].is_fluid is True
        assert ext["coal"].is_fluid is False
        assert ext["iron-plate"].is_fluid is False
        assert ext["copper-plate"].is_fluid is False

        # --- Layout checks ---
        lr = layout(result)

        entity_names = [e.name for e in lr.entities]
        assert "chemical-plant" in entity_names, "Should have chemical plants for plastic"
        assert "assembling-machine-3" in entity_names
        assert "underground-belt" in entity_names, "Bus should use underground belts"
        assert "pipe" in entity_names, "Fluid rows should have pipes"

        # No tile overlaps (surface entities)
        _3x3 = {"assembling-machine-3", "chemical-plant"}
        occupied: dict[tuple[int, int], str] = {}
        for ent in lr.entities:
            if ent.name in _3x3:
                tiles = [(ent.x + dx, ent.y + dy) for dx in range(3) for dy in range(3)]
            else:
                tiles = [(ent.x, ent.y)]
            for tile in tiles:
                assert tile not in occupied, f"Overlap at {tile}: {ent.name} vs {occupied[tile]}"
                occupied[tile] = ent.name

        # Underground belts come in matched input/output pairs
        ug_in = sum(1 for e in lr.entities if e.name == "underground-belt" and e.io_type == "input")
        ug_out = sum(1 for e in lr.entities if e.name == "underground-belt" and e.io_type == "output")
        assert ug_in == ug_out, f"Mismatched UG belt pairs: {ug_in} in, {ug_out} out"

        # Fluid bus should have tap-off pipes connecting to rows
        pipe_count = sum(1 for e in lr.entities if e.name == "pipe")
        assert pipe_count > 0, "Should have pipes for fluid bus and rows"

        # Chemical plants should have the plastic-bar recipe
        chem_plants = [e for e in lr.entities if e.name == "chemical-plant"]
        for cp in chem_plants:
            assert cp.recipe == "plastic-bar"

        # --- Blueprint round-trip ---
        bp_str = produce(
            "advanced-circuit",
            rate=5,
            inputs=["iron-plate", "copper-plate", "petroleum-gas", "coal"],
        )
        viz(bp_str, "advanced-circuit-5s", solver_result=result)
        bp = get_blueprintable_from_string(bp_str)
        assert len(bp.entities) > 100

        # Verify recipes survived the round-trip
        bp_asms = [e for e in bp.entities if e.name == "assembling-machine-3"]
        bp_chems = [e for e in bp.entities if e.name == "chemical-plant"]
        asm_recipes = {e.recipe for e in bp_asms}
        assert "advanced-circuit" in asm_recipes
        assert "electronic-circuit" in asm_recipes
        assert "copper-cable" in asm_recipes
        for cp in bp_chems:
            assert cp.recipe == "plastic-bar"

        # Verify underground belts survived
        bp_ug = [e for e in bp.entities if e.name == "underground-belt"]
        assert len(bp_ug) > 0

        # Verify pipes survived (fluid bus + row pipes)
        bp_pipes = [e for e in bp.entities if e.name == "pipe"]
        assert len(bp_pipes) > 0

    def test_oil_refinery_end_to_end(self, viz):
        """Oil refinery: solve → layout → blueprint round-trip.

        basic-oil-processing uses oil-refinery (5x5), which is larger than
        the 3x3 assemblers and chemical plants. This tests the full pipeline
        handling the larger machine footprint correctly.
        """
        from src.layout import layout
        from src.pipeline import produce
        from src.solver import solve

        # --- Solver checks ---
        result = solve(
            "petroleum-gas",
            10,
            available_inputs={"crude-oil"},
        )
        m = result.machines[0]
        assert m.entity == "oil-refinery"
        assert m.recipe == "basic-oil-processing"

        # --- Layout checks ---
        lr = layout(result)
        entity_names = [e.name for e in lr.entities]
        assert "oil-refinery" in entity_names
        assert "pipe" in entity_names

        # No tile overlaps (5x5 refineries)
        _5x5 = {"oil-refinery"}
        _3x3 = {"assembling-machine-3", "chemical-plant"}
        occupied: dict[tuple[int, int], str] = {}
        for ent in lr.entities:
            if ent.name in _5x5:
                tiles = [(ent.x + dx, ent.y + dy) for dx in range(5) for dy in range(5)]
            elif ent.name in _3x3:
                tiles = [(ent.x + dx, ent.y + dy) for dx in range(3) for dy in range(3)]
            else:
                tiles = [(ent.x, ent.y)]
            for tile in tiles:
                assert tile not in occupied, f"Overlap at {tile}: {ent.name} vs {occupied[tile]}"
                occupied[tile] = ent.name

        # --- Blueprint round-trip ---
        bp_str = produce(
            "petroleum-gas",
            rate=10,
            inputs=["crude-oil"],
        )
        viz(bp_str, "petroleum-gas-10s", solver_result=result)
        bp = get_blueprintable_from_string(bp_str)
        assert len(bp.entities) > 10

        # Verify oil-refinery entities with recipe survived round-trip
        bp_refs = [e for e in bp.entities if e.name == "oil-refinery"]
        assert len(bp_refs) > 0
        for ref in bp_refs:
            assert ref.recipe == "basic-oil-processing"
