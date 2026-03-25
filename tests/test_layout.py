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


class TestOilRefineryLayout:
    """Tests for oil-refinery (5x5) layout handling."""

    def _check_no_overlaps(self, lr):
        """Helper: verify no tile overlaps, accounting for 5x5 refineries."""
        _5x5 = {"oil-refinery"}
        _3x3 = {
            "assembling-machine-1", "assembling-machine-2", "assembling-machine-3",
            "chemical-plant",
        }
        occupied: dict[tuple[int, int], str] = {}
        for ent in lr.entities:
            if ent.name in _5x5:
                tiles = [(ent.x + dx, ent.y + dy) for dx in range(5) for dy in range(5)]
            elif ent.name in _3x3:
                tiles = [(ent.x + dx, ent.y + dy) for dx in range(3) for dy in range(3)]
            else:
                tiles = [(ent.x, ent.y)]

            for tile in tiles:
                assert tile not in occupied, (
                    f"Overlap at {tile}: {ent.name} vs {occupied[tile]}"
                )
                occupied[tile] = ent.name

    def test_refinery_no_overlaps(self):
        """Oil refinery 5x5 entities should not overlap with anything."""
        result = solve(
            "petroleum-gas", target_rate=10,
            available_inputs={"crude-oil"},
        )
        lr = layout(result)
        self._check_no_overlaps(lr)

    def test_refinery_entity_present(self):
        """Layout should contain oil-refinery entities."""
        result = solve(
            "petroleum-gas", target_rate=10,
            available_inputs={"crude-oil"},
        )
        lr = layout(result)
        refinery_count = sum(1 for e in lr.entities if e.name == "oil-refinery")
        assert refinery_count > 0, "Should have oil refineries"

    def test_refinery_correct_count(self):
        """Layout should have the right number of refinery entities."""
        result = solve(
            "petroleum-gas", target_rate=10,
            available_inputs={"crude-oil"},
        )
        lr = layout(result)
        import math
        expected_count = math.ceil(result.machines[0].count)
        refinery_count = sum(1 for e in lr.entities if e.name == "oil-refinery")
        assert refinery_count == expected_count

    def test_refinery_has_pipes(self):
        """Oil refinery layout should have pipe entities for fluid IO."""
        result = solve(
            "petroleum-gas", target_rate=10,
            available_inputs={"crude-oil"},
        )
        lr = layout(result)
        pipe_count = sum(1 for e in lr.entities if e.name == "pipe")
        assert pipe_count > 0, "Oil refinery layout should contain pipes"

    def test_refinery_inserter_alignment(self):
        """Each inserter should be adjacent to an oil-refinery tile."""
        result = solve(
            "petroleum-gas", target_rate=10,
            available_inputs={"crude-oil"},
        )
        lr = layout(result)

        _5x5 = {"oil-refinery"}
        _3x3 = {
            "assembling-machine-1", "assembling-machine-2", "assembling-machine-3",
            "chemical-plant",
        }
        machine_tiles: set[tuple[int, int]] = set()
        for ent in lr.entities:
            if ent.name in _5x5:
                for dx in range(5):
                    for dy in range(5):
                        machine_tiles.add((ent.x + dx, ent.y + dy))
            elif ent.name in _3x3:
                for dx in range(3):
                    for dy in range(3):
                        machine_tiles.add((ent.x + dx, ent.y + dy))

        for ent in lr.entities:
            if ent.name == "inserter":
                nearby = set()
                for dx in range(-2, 3):
                    for dy in range(-2, 3):
                        nearby.add((ent.x + dx, ent.y + dy))
                assert nearby & machine_tiles, (
                    f"Inserter at ({ent.x}, {ent.y}) not near any machine"
                )

    def test_refinery_bus_pipe_to_ground(self):
        """Fluid bus lanes for oil recipes should use pipe-to-ground."""
        result = solve(
            "petroleum-gas", target_rate=10,
            available_inputs={"crude-oil"},
        )
        lr = layout(result)
        ptg_count = sum(1 for e in lr.entities if e.name == "pipe-to-ground")
        assert ptg_count > 0, "Oil refinery bus should use pipe-to-ground"
        assert ptg_count % 2 == 0, f"Odd pipe-to-ground count: {ptg_count}"

    def test_refinery_recipe_set(self):
        """Oil refinery entities should have the correct recipe set."""
        result = solve(
            "petroleum-gas", target_rate=10,
            available_inputs={"crude-oil"},
        )
        lr = layout(result)
        refineries = [e for e in lr.entities if e.name == "oil-refinery"]
        for ref in refineries:
            assert ref.recipe == "basic-oil-processing"

    def test_refinery_wider_spacing(self):
        """Oil refineries (5x5) should be spaced wider than 3x3 machines."""
        result = solve(
            "petroleum-gas", target_rate=10,
            available_inputs={"crude-oil"},
        )
        lr = layout(result)
        refineries = [e for e in lr.entities if e.name == "oil-refinery"]
        if len(refineries) >= 2:
            # Sort by x position
            refineries.sort(key=lambda e: e.x)
            spacing = refineries[1].x - refineries[0].x
            # Must be at least 6 (5-wide + 1-gap) to avoid overlap
            assert spacing >= 6, (
                f"Refinery spacing {spacing} too narrow for 5x5 machines"
            )
