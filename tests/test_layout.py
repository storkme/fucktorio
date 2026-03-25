"""Tests for the layout engine."""

import math
from src.solver import solve
from src.layout import layout


class TestLayout:
    def test_no_overlaps(self):
        result = solve(
            "electronic-circuit", target_rate=30,
            available_inputs={"iron-plate", "copper-plate"},
        )
        lr = layout(result)

        # Collect all occupied tiles
        _3x3 = {
            "assembling-machine-1", "assembling-machine-2", "assembling-machine-3",
            "chemical-plant",
        }
        occupied: dict[tuple[int, int], str] = {}
        for ent in lr.entities:
            if ent.name in _3x3:
                tiles = [(ent.x + dx, ent.y + dy) for dx in range(3) for dy in range(3)]
            else:
                tiles = [(ent.x, ent.y)]

            for tile in tiles:
                assert tile not in occupied, (
                    f"Overlap at {tile}: {ent.name} vs {occupied[tile]}"
                )
                occupied[tile] = ent.name

    def test_has_entities(self):
        result = solve(
            "iron-gear-wheel", target_rate=10,
            available_inputs={"iron-plate"},
        )
        lr = layout(result)
        assert len(lr.entities) > 0
        assert lr.width > 0
        assert lr.height > 0

    def test_inserters_aligned_with_machines(self):
        """Each inserter should be adjacent to a machine tile."""
        result = solve(
            "electronic-circuit", target_rate=5,
            available_inputs={"iron-plate", "copper-plate"},
        )
        lr = layout(result)

        _3x3 = {
            "assembling-machine-1", "assembling-machine-2", "assembling-machine-3",
            "chemical-plant",
        }
        machine_tiles: set[tuple[int, int]] = set()
        for ent in lr.entities:
            if ent.name in _3x3:
                for dx in range(3):
                    for dy in range(3):
                        machine_tiles.add((ent.x + dx, ent.y + dy))

        for ent in lr.entities:
            if ent.name == "inserter":
                # Inserter should have a machine tile within 2 tiles
                # (inserters pick/drop across 1 tile gap)
                nearby = set()
                for dx in range(-1, 2):
                    for dy in range(-2, 3):
                        nearby.add((ent.x + dx, ent.y + dy))
                assert nearby & machine_tiles, (
                    f"Inserter at ({ent.x}, {ent.y}) not near any machine"
                )

    def test_no_overlaps_fluid(self):
        """Fluid recipe layouts should have no overlapping entities."""
        result = solve(
            "plastic-bar", target_rate=10,
            available_inputs={"petroleum-gas", "coal"},
        )
        lr = layout(result)

        _3x3 = {"chemical-plant"}
        occupied: dict[tuple[int, int], str] = {}
        for ent in lr.entities:
            if ent.name in _3x3:
                tiles = [(ent.x + dx, ent.y + dy) for dx in range(3) for dy in range(3)]
            else:
                tiles = [(ent.x, ent.y)]

            for tile in tiles:
                assert tile not in occupied, (
                    f"Overlap at {tile}: {ent.name} vs {occupied[tile]}"
                )
                occupied[tile] = ent.name

    def test_fluid_layout_has_pipes(self):
        """Fluid recipes should produce pipe entities."""
        result = solve(
            "plastic-bar", target_rate=10,
            available_inputs={"petroleum-gas", "coal"},
        )
        lr = layout(result)
        pipe_count = sum(1 for e in lr.entities if e.name == "pipe")
        assert pipe_count > 0, "Fluid layout should contain pipes"

    def test_fluid_layout_has_chemical_plant(self):
        """Chemistry recipes should use chemical-plant entities."""
        result = solve(
            "sulfuric-acid", target_rate=50,
            available_inputs={"sulfur", "iron-plate", "water"},
        )
        lr = layout(result)
        chem_count = sum(1 for e in lr.entities if e.name == "chemical-plant")
        assert chem_count > 0, "Should have chemical plants"

    def test_bus_uses_underground_belts(self):
        """Bus lanes should use underground belts to tunnel through rows."""
        result = solve(
            "electronic-circuit", target_rate=30,
            available_inputs={"iron-plate", "copper-plate"},
        )
        lr = layout(result)
        ug_count = sum(1 for e in lr.entities if e.name == "underground-belt")
        assert ug_count > 0, "Bus should use underground belts"
        # Should come in input/output pairs
        inputs = sum(1 for e in lr.entities if e.name == "underground-belt" and e.io_type == "input")
        outputs = sum(1 for e in lr.entities if e.name == "underground-belt" and e.io_type == "output")
        assert inputs == outputs, f"Mismatched UG belt pairs: {inputs} in, {outputs} out"

    def test_bus_uses_pipe_to_ground(self):
        """Fluid bus lanes should use pipe-to-ground to tunnel through rows."""
        result = solve(
            "plastic-bar", target_rate=10,
            available_inputs={"petroleum-gas", "coal"},
        )
        lr = layout(result)
        ptg_count = sum(1 for e in lr.entities if e.name == "pipe-to-ground")
        assert ptg_count > 0, "Fluid bus should use pipe-to-ground"
        # Should come in pairs
        assert ptg_count % 2 == 0, f"Odd number of pipe-to-ground: {ptg_count}"
