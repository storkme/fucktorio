"""Tests for the layout engine."""

import math

from src.layout import layout
from src.solver import solve


class TestLayout:
    def test_no_overlaps(self):
        result = solve(
            "electronic-circuit",
            target_rate=30,
            available_inputs={"iron-plate", "copper-plate"},
        )
        lr = layout(result)

        # Collect all occupied tiles
        _3x3 = {
            "assembling-machine-1",
            "assembling-machine-2",
            "assembling-machine-3",
            "chemical-plant",
        }
        occupied: dict[tuple[int, int], str] = {}
        for ent in lr.entities:
            if ent.name in _3x3:
                tiles = [(ent.x + dx, ent.y + dy) for dx in range(3) for dy in range(3)]
            else:
                tiles = [(ent.x, ent.y)]

            for tile in tiles:
                assert tile not in occupied, f"Overlap at {tile}: {ent.name} vs {occupied[tile]}"
                occupied[tile] = ent.name

    def test_has_entities(self):
        result = solve(
            "iron-gear-wheel",
            target_rate=10,
            available_inputs={"iron-plate"},
        )
        lr = layout(result)
        assert len(lr.entities) > 0
        assert lr.width > 0
        assert lr.height > 0

    def test_inserters_aligned_with_machines(self):
        """Each inserter should be adjacent to a machine tile."""
        result = solve(
            "electronic-circuit",
            target_rate=5,
            available_inputs={"iron-plate", "copper-plate"},
        )
        lr = layout(result)

        _3x3 = {
            "assembling-machine-1",
            "assembling-machine-2",
            "assembling-machine-3",
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
                assert nearby & machine_tiles, f"Inserter at ({ent.x}, {ent.y}) not near any machine"

    def test_no_overlaps_fluid(self):
        """Fluid recipe layouts should have no overlapping entities."""
        result = solve(
            "plastic-bar",
            target_rate=10,
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
                assert tile not in occupied, f"Overlap at {tile}: {ent.name} vs {occupied[tile]}"
                occupied[tile] = ent.name

    def test_fluid_layout_has_pipes(self):
        """Fluid recipes should produce pipe entities."""
        result = solve(
            "plastic-bar",
            target_rate=10,
            available_inputs={"petroleum-gas", "coal"},
        )
        lr = layout(result)
        pipe_count = sum(1 for e in lr.entities if e.name == "pipe")
        assert pipe_count > 0, "Fluid layout should contain pipes"

    def test_fluid_layout_has_chemical_plant(self):
        """Chemistry recipes should use chemical-plant entities."""
        result = solve(
            "sulfuric-acid",
            target_rate=50,
            available_inputs={"sulfur", "iron-plate", "water"},
        )
        lr = layout(result)
        chem_count = sum(1 for e in lr.entities if e.name == "chemical-plant")
        assert chem_count > 0, "Should have chemical plants"

    def test_chemical_plant_pipes_adjacent_to_ports(self):
        """Pipes must be placed adjacent to chemical-plant fluid ports."""
        result = solve(
            "plastic-bar",
            target_rate=10,
            available_inputs={"petroleum-gas", "coal"},
        )
        lr = layout(result)

        # chemical-plant ports (relative to tile_position):
        #   input:  (0, 0) north, (2, 0) north → pipe at (0, -1), (2, -1)
        #   output: (0, 2) south, (2, 2) south → pipe at (0, 3), (2, 3)
        pipe_tiles = {(e.x, e.y) for e in lr.entities if e.name == "pipe"}

        for ent in lr.entities:
            if ent.name != "chemical-plant":
                continue
            mx, my = ent.x, ent.y
            # Check input ports (north side) have adjacent pipes
            for dx in (0, 2):
                expected = (mx + dx, my - 1)
                assert expected in pipe_tiles, (
                    f"Chemical plant at ({mx},{my}): missing input pipe at {expected}"
                )
            # Check output ports (south side) have adjacent pipes
            for dx in (0, 2):
                expected = (mx + dx, my + 3)
                assert expected in pipe_tiles, (
                    f"Chemical plant at ({mx},{my}): missing output pipe at {expected}"
                )

    def test_bus_uses_underground_belts(self):
        """Bus lanes should use underground belts to tunnel through rows."""
        result = solve(
            "electronic-circuit",
            target_rate=30,
            available_inputs={"iron-plate", "copper-plate"},
        )
        lr = layout(result)
        ug_count = sum(1 for e in lr.entities if e.name == "underground-belt")
        assert ug_count > 0, "Bus should use underground belts"
        # Should come in input/output pairs
        inputs = sum(1 for e in lr.entities if e.name == "underground-belt" and e.io_type == "input")
        outputs = sum(1 for e in lr.entities if e.name == "underground-belt" and e.io_type == "output")
        assert inputs == outputs, f"Mismatched UG belt pairs: {inputs} in, {outputs} out"

    def test_fluid_bus_tap_off(self):
        """Fluid bus lanes should have horizontal tap-off pipes into consuming rows."""
        result = solve(
            "plastic-bar",
            target_rate=10,
            available_inputs={"petroleum-gas", "coal"},
        )
        lr = layout(result)

        # Find the fluid bus lane x (petroleum-gas lane)
        fluid_idx = next(
            i for i, f in enumerate(result.external_inputs) if f.item == "petroleum-gas"
        )
        bus_x = fluid_idx * 2

        # The bus should have surface pipes at bus_x
        bus_pipes = {e.y for e in lr.entities if e.name == "pipe" and e.x == bus_x}
        assert len(bus_pipes) > 0, "Fluid bus should have surface pipes"

        # There should be horizontal tap-off pipes between bus and row
        # (pipes at x positions between bus_x and bus_width)
        bus_width = max(2, len(result.external_inputs) * 2 + 1)
        tap_pipes = [
            e for e in lr.entities
            if e.name == "pipe" and bus_x < e.x < bus_width
        ]
        assert len(tap_pipes) > 0, "Should have horizontal tap-off pipes"


class TestOilRefineryLayout:
    """Tests for oil-refinery (5x5) layout handling."""

    def _check_no_overlaps(self, lr):
        """Helper: verify no tile overlaps, accounting for 5x5 refineries."""
        _5x5 = {"oil-refinery"}
        _3x3 = {
            "assembling-machine-1",
            "assembling-machine-2",
            "assembling-machine-3",
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
                assert tile not in occupied, f"Overlap at {tile}: {ent.name} vs {occupied[tile]}"
                occupied[tile] = ent.name

    def test_refinery_no_overlaps(self):
        """Oil refinery 5x5 entities should not overlap with anything."""
        result = solve(
            "petroleum-gas",
            target_rate=10,
            available_inputs={"crude-oil"},
        )
        lr = layout(result)
        self._check_no_overlaps(lr)

    def test_refinery_entity_present(self):
        """Layout should contain oil-refinery entities."""
        result = solve(
            "petroleum-gas",
            target_rate=10,
            available_inputs={"crude-oil"},
        )
        lr = layout(result)
        refinery_count = sum(1 for e in lr.entities if e.name == "oil-refinery")
        assert refinery_count > 0, "Should have oil refineries"

    def test_refinery_correct_count(self):
        """Layout should have the right number of refinery entities."""
        result = solve(
            "petroleum-gas",
            target_rate=10,
            available_inputs={"crude-oil"},
        )
        lr = layout(result)
        expected_count = math.ceil(result.machines[0].count)
        refinery_count = sum(1 for e in lr.entities if e.name == "oil-refinery")
        assert refinery_count == expected_count

    def test_refinery_has_pipes(self):
        """Oil refinery layout should have pipe entities for fluid IO."""
        result = solve(
            "petroleum-gas",
            target_rate=10,
            available_inputs={"crude-oil"},
        )
        lr = layout(result)
        pipe_count = sum(1 for e in lr.entities if e.name == "pipe")
        assert pipe_count > 0, "Oil refinery layout should contain pipes"

    def test_refinery_inserter_alignment(self):
        """Each inserter should be adjacent to an oil-refinery tile."""
        result = solve(
            "petroleum-gas",
            target_rate=10,
            available_inputs={"crude-oil"},
        )
        lr = layout(result)

        _5x5 = {"oil-refinery"}
        _3x3 = {
            "assembling-machine-1",
            "assembling-machine-2",
            "assembling-machine-3",
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
                assert nearby & machine_tiles, f"Inserter at ({ent.x}, {ent.y}) not near any machine"

    def test_refinery_fluid_bus_tap_off(self):
        """Fluid bus lanes for oil recipes should tap off into the refinery row."""
        result = solve(
            "petroleum-gas",
            target_rate=10,
            available_inputs={"crude-oil"},
        )
        lr = layout(result)

        # Find the fluid bus lane x (crude-oil lane)
        fluid_idx = next(
            i for i, f in enumerate(result.external_inputs) if f.item == "crude-oil"
        )
        bus_x = fluid_idx * 2

        # The bus should have surface pipes
        bus_pipes = {e.y for e in lr.entities if e.name == "pipe" and e.x == bus_x}
        assert len(bus_pipes) > 0, "Fluid bus should have surface pipes"

        # There should be horizontal tap-off pipes
        bus_width = max(2, len(result.external_inputs) * 2 + 1)
        tap_pipes = [
            e for e in lr.entities
            if e.name == "pipe" and bus_x < e.x < bus_width
        ]
        assert len(tap_pipes) > 0, "Should have horizontal tap-off pipes"

    def test_refinery_recipe_set(self):
        """Oil refinery entities should have the correct recipe set."""
        result = solve(
            "petroleum-gas",
            target_rate=10,
            available_inputs={"crude-oil"},
        )
        lr = layout(result)
        refineries = [e for e in lr.entities if e.name == "oil-refinery"]
        for ref in refineries:
            assert ref.recipe == "basic-oil-processing"

    def test_refinery_pipes_adjacent_to_ports(self):
        """Pipes must be placed adjacent to oil-refinery fluid ports."""
        result = solve(
            "petroleum-gas",
            target_rate=10,
            available_inputs={"crude-oil"},
        )
        lr = layout(result)

        # Oil refinery ports (relative to tile_position):
        #   input:  (1, 4) south, (3, 4) south → pipe at (1, 5), (3, 5)
        #   output: (0, 0) north, (2, 0) north, (4, 0) north → pipe at (0, -1), (2, -1), (4, -1)
        pipe_tiles = {(e.x, e.y) for e in lr.entities if e.name == "pipe"}

        for ent in lr.entities:
            if ent.name != "oil-refinery":
                continue
            mx, my = ent.x, ent.y
            # Check input ports (south side) have adjacent pipes
            for dx in (1, 3):
                expected = (mx + dx, my + 5)
                assert expected in pipe_tiles, (
                    f"Refinery at ({mx},{my}): missing input pipe at {expected}"
                )
            # Check output ports (north side) have adjacent pipes
            for dx in (0, 2, 4):
                expected = (mx + dx, my - 1)
                assert expected in pipe_tiles, (
                    f"Refinery at ({mx},{my}): missing output pipe at {expected}"
                )

    def test_refinery_wider_spacing(self):
        """Oil refineries (5x5) should be spaced wider than 3x3 machines."""
        result = solve(
            "petroleum-gas",
            target_rate=10,
            available_inputs={"crude-oil"},
        )
        lr = layout(result)
        refineries = [e for e in lr.entities if e.name == "oil-refinery"]
        if len(refineries) >= 2:
            # Sort by x position
            refineries.sort(key=lambda e: e.x)
            spacing = refineries[1].x - refineries[0].x
            # Must be at least 6 (5-wide + 1-gap) to avoid overlap
            assert spacing >= 6, f"Refinery spacing {spacing} too narrow for 5x5 machines"
