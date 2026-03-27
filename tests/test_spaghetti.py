"""Tests for the spaghetti layout engine."""

import math

from src.blueprint import build_blueprint
from src.solver import solve
from src.spaghetti.graph import build_production_graph
from src.spaghetti.layout import spaghetti_layout


class TestProductionGraph:
    """Tests for production graph construction."""

    def test_single_recipe_graph(self):
        """Single recipe produces correct graph structure."""
        result = solve("iron-gear-wheel", target_rate=10, available_inputs={"iron-plate"})
        graph = build_production_graph(result)

        # Should have ceil(count) machine nodes
        expected_count = math.ceil(result.machines[0].count)
        assert len(graph.nodes) == expected_count

        # All nodes should be for iron-gear-wheel
        for node in graph.nodes:
            assert node.spec.recipe == "iron-gear-wheel"

        # Should have external input edges (iron-plate → each machine)
        ext_inputs = graph.external_inputs()
        assert len(ext_inputs) == expected_count
        assert all(e.item == "iron-plate" for e in ext_inputs)

    def test_two_step_chain(self):
        """Two-step recipe chain has internal edges."""
        result = solve(
            "electronic-circuit",
            target_rate=5,
            available_inputs={"iron-plate", "copper-plate"},
        )
        graph = build_production_graph(result)

        # Should have nodes for multiple recipes
        recipes = {n.spec.recipe for n in graph.nodes}
        assert "electronic-circuit" in recipes
        assert "copper-cable" in recipes

        # Should have internal edges (copper-cable → electronic-circuit)
        internal = [e for e in graph.edges if e.from_node is not None and e.to_node is not None]
        assert len(internal) > 0
        assert any(e.item == "copper-cable" for e in internal)

    def test_external_inputs_correct(self):
        """External inputs connect to the right machines."""
        result = solve("iron-gear-wheel", target_rate=5, available_inputs={"iron-plate"})
        graph = build_production_graph(result)

        ext = graph.external_inputs()
        for e in ext:
            assert e.item == "iron-plate"
            assert e.from_node is None
            assert e.to_node is not None


class TestSpaghettiPhase1:
    """Phase 1: single machine, single input (iron-gear-wheel)."""

    def test_produces_layout(self):
        """Spaghetti engine produces a non-empty layout."""
        result = solve("iron-gear-wheel", target_rate=2, available_inputs={"iron-plate"})
        lr = spaghetti_layout(result)

        assert len(lr.entities) > 0
        assert lr.width > 0
        assert lr.height > 0

    def test_has_machines(self):
        """Layout contains assembling machines with correct recipe."""
        result = solve("iron-gear-wheel", target_rate=2, available_inputs={"iron-plate"})
        lr = spaghetti_layout(result)

        machines = [e for e in lr.entities if e.name == "assembling-machine-3"]
        assert len(machines) > 0
        for m in machines:
            assert m.recipe == "iron-gear-wheel"

    def test_has_belts(self):
        """Layout contains belt entities for item transport."""
        result = solve("iron-gear-wheel", target_rate=2, available_inputs={"iron-plate"})
        lr = spaghetti_layout(result)

        belt_types = {"transport-belt", "fast-transport-belt", "express-transport-belt"}
        belts = [e for e in lr.entities if e.name in belt_types]
        assert len(belts) > 0, "Should have belts for item transport"

    def test_has_inserters(self):
        """Layout must contain inserters to move items between belts and machines."""
        result = solve("iron-gear-wheel", target_rate=10, available_inputs={"iron-plate"})
        lr = spaghetti_layout(result)

        inserters = [e for e in lr.entities if "inserter" in e.name]
        machines = [e for e in lr.entities if e.name == "assembling-machine-3"]
        assert len(inserters) > 0, "Should have inserters"
        # Each machine needs at least one inserter
        assert len(inserters) >= len(machines), f"Should have at least {len(machines)} inserters, got {len(inserters)}"

    def test_no_overlaps(self):
        """No entities should overlap."""
        result = solve("iron-gear-wheel", target_rate=2, available_inputs={"iron-plate"})
        lr = spaghetti_layout(result)

        _3x3 = {"assembling-machine-1", "assembling-machine-2", "assembling-machine-3", "chemical-plant"}
        _5x5 = {"oil-refinery"}
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

    def test_has_power(self):
        """Layout contains power poles."""
        result = solve("iron-gear-wheel", target_rate=2, available_inputs={"iron-plate"})
        lr = spaghetti_layout(result)

        poles = [e for e in lr.entities if e.name == "medium-electric-pole"]
        assert len(poles) > 0, "Should have power poles"


class TestSpaghettiPhase2:
    """Phase 2: multiple machines, one input each."""

    def _check_no_overlaps(self, lr):
        _3x3 = {"assembling-machine-1", "assembling-machine-2", "assembling-machine-3", "chemical-plant"}
        _5x5 = {"oil-refinery"}
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

    def test_multiple_machines_placed(self):
        """Higher rate should produce multiple machines."""
        result = solve("iron-gear-wheel", target_rate=30, available_inputs={"iron-plate"})
        lr = spaghetti_layout(result)

        machines = [e for e in lr.entities if e.name == "assembling-machine-3"]
        expected = math.ceil(result.machines[0].count)
        assert len(machines) == expected, f"Expected {expected} machines, got {len(machines)}"

    def test_multiple_machines_no_overlaps(self):
        """Multiple machines should not overlap."""
        result = solve("iron-gear-wheel", target_rate=30, available_inputs={"iron-plate"})
        lr = spaghetti_layout(result)
        self._check_no_overlaps(lr)

    def test_multiple_machines_have_belts(self):
        """Each machine should be reachable by belts."""
        result = solve("iron-gear-wheel", target_rate=30, available_inputs={"iron-plate"})
        lr = spaghetti_layout(result)

        belt_types = {"transport-belt", "fast-transport-belt", "express-transport-belt"}
        belts = [e for e in lr.entities if e.name in belt_types]
        machines = [e for e in lr.entities if e.name == "assembling-machine-3"]
        # Should have at least some belts per machine
        assert len(belts) >= len(machines), "Should have at least as many belt tiles as machines"


class TestSpaghettiPhase3:
    """Phase 3: intermediates (multi-step chains)."""

    def _check_no_overlaps(self, lr):
        _3x3 = {"assembling-machine-1", "assembling-machine-2", "assembling-machine-3", "chemical-plant"}
        _5x5 = {"oil-refinery"}
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

    def test_intermediate_chain(self):
        """Electronic circuit chain should produce machines for both recipes."""
        result = solve(
            "electronic-circuit",
            target_rate=5,
            available_inputs={"iron-plate", "copper-plate"},
        )
        lr = spaghetti_layout(result)

        recipes = {e.recipe for e in lr.entities if e.recipe is not None}
        assert "electronic-circuit" in recipes
        assert "copper-cable" in recipes

    def test_intermediate_no_overlaps(self):
        """Multi-recipe layout should have no overlaps."""
        result = solve(
            "electronic-circuit",
            target_rate=5,
            available_inputs={"iron-plate", "copper-plate"},
        )
        lr = spaghetti_layout(result)
        self._check_no_overlaps(lr)

    def test_intermediate_has_connecting_belts(self):
        """Should have belts carrying the intermediate product (copper-cable)."""
        result = solve(
            "electronic-circuit",
            target_rate=5,
            available_inputs={"iron-plate", "copper-plate"},
        )
        lr = spaghetti_layout(result)

        belt_types = {"transport-belt", "fast-transport-belt", "express-transport-belt"}
        copper_cable_belts = [e for e in lr.entities if e.name in belt_types and e.carries == "copper-cable"]
        assert len(copper_cable_belts) > 0, "Should have belts carrying copper-cable between machines"


class TestSpaghettiPhase4:
    """Phase 4: multiple inputs per machine."""

    def test_multiple_inputs(self):
        """Electronic circuit with both iron-plate and copper-cable inputs."""
        result = solve(
            "electronic-circuit",
            target_rate=5,
            available_inputs={"iron-plate", "copper-plate"},
        )
        lr = spaghetti_layout(result)

        belt_types = {"transport-belt", "fast-transport-belt", "express-transport-belt"}
        # Should have belts carrying iron-plate (external input to electronic-circuit)
        iron_belts = [e for e in lr.entities if e.name in belt_types and e.carries == "iron-plate"]
        assert len(iron_belts) > 0, "Should have belts carrying iron-plate"

        # And copper-cable (intermediate)
        cable_belts = [e for e in lr.entities if e.name in belt_types and e.carries == "copper-cable"]
        assert len(cable_belts) > 0, "Should have belts carrying copper-cable"


class TestSpaghettiPhase5:
    """Phase 5: fluid recipes."""

    def test_fluid_recipe_has_pipes(self):
        """Plastic-bar recipe should produce pipe entities."""
        result = solve(
            "plastic-bar",
            target_rate=5,
            available_inputs={"petroleum-gas", "coal"},
        )
        lr = spaghetti_layout(result)

        pipes = [e for e in lr.entities if e.name == "pipe"]
        assert len(pipes) > 0, "Should have pipes for fluid transport"

    def test_fluid_recipe_has_chemical_plant(self):
        """Plastic-bar should use chemical-plant."""
        result = solve(
            "plastic-bar",
            target_rate=5,
            available_inputs={"petroleum-gas", "coal"},
        )
        lr = spaghetti_layout(result)

        chems = [e for e in lr.entities if e.name == "chemical-plant"]
        assert len(chems) > 0, "Should have chemical plants"

    def test_fluid_pipes_tagged(self):
        """Pipes should be tagged with the fluid they carry."""
        result = solve(
            "plastic-bar",
            target_rate=5,
            available_inputs={"petroleum-gas", "coal"},
        )
        lr = spaghetti_layout(result)

        petro_pipes = [e for e in lr.entities if e.name == "pipe" and e.carries == "petroleum-gas"]
        assert len(petro_pipes) > 0, "Should have pipes tagged with petroleum-gas"

    def test_fluid_no_overlaps(self):
        """Fluid layout should have no overlaps."""
        result = solve(
            "plastic-bar",
            target_rate=5,
            available_inputs={"petroleum-gas", "coal"},
        )
        lr = spaghetti_layout(result)

        _3x3 = {"assembling-machine-1", "assembling-machine-2", "assembling-machine-3", "chemical-plant"}
        occupied: dict[tuple[int, int], str] = {}
        for ent in lr.entities:
            if ent.name in _3x3:
                tiles = [(ent.x + dx, ent.y + dy) for dx in range(3) for dy in range(3)]
            else:
                tiles = [(ent.x, ent.y)]
            for tile in tiles:
                assert tile not in occupied, f"Overlap at {tile}: {ent.name} vs {occupied[tile]}"
                occupied[tile] = ent.name


class TestSpaghettiPhase6:
    """Phase 6: complex chains (mixed solid + fluid, multi-step)."""

    def _check_no_overlaps(self, lr):
        _3x3 = {"assembling-machine-1", "assembling-machine-2", "assembling-machine-3", "chemical-plant"}
        _5x5 = {"oil-refinery"}
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

    def test_advanced_circuit(self):
        """Advanced circuit: deep chain with fluid intermediates."""
        result = solve(
            "advanced-circuit",
            target_rate=5,
            available_inputs={"iron-plate", "copper-plate", "petroleum-gas", "coal"},
        )
        lr = spaghetti_layout(result)

        recipes = {e.recipe for e in lr.entities if e.recipe is not None}
        assert "advanced-circuit" in recipes
        assert "electronic-circuit" in recipes
        assert "copper-cable" in recipes
        assert "plastic-bar" in recipes

    def test_advanced_circuit_no_overlaps(self):
        """Advanced circuit layout should have no overlaps."""
        result = solve(
            "advanced-circuit",
            target_rate=5,
            available_inputs={"iron-plate", "copper-plate", "petroleum-gas", "coal"},
        )
        lr = spaghetti_layout(result)
        self._check_no_overlaps(lr)

    def test_advanced_circuit_has_mixed_transport(self):
        """Should have both belts and pipes for mixed solid/fluid chain."""
        result = solve(
            "advanced-circuit",
            target_rate=5,
            available_inputs={"iron-plate", "copper-plate", "petroleum-gas", "coal"},
        )
        lr = spaghetti_layout(result)

        belt_types = {"transport-belt", "fast-transport-belt", "express-transport-belt"}
        belts = [e for e in lr.entities if e.name in belt_types]
        pipes = [e for e in lr.entities if e.name == "pipe"]
        assert len(belts) > 0, "Should have belts for solids"
        assert len(pipes) > 0, "Should have pipes for fluids"

    def test_petroleum_gas(self):
        """Petroleum gas: oil refinery (5x5) recipe."""
        result = solve(
            "petroleum-gas",
            target_rate=10,
            available_inputs={"crude-oil"},
        )
        lr = spaghetti_layout(result)

        refineries = [e for e in lr.entities if e.name == "oil-refinery"]
        assert len(refineries) > 0, "Should have oil refineries"
        for r in refineries:
            assert r.recipe == "basic-oil-processing"

    def test_petroleum_gas_no_overlaps(self):
        """Oil refinery layout should have no overlaps."""
        result = solve(
            "petroleum-gas",
            target_rate=10,
            available_inputs={"crude-oil"},
        )
        lr = spaghetti_layout(result)
        self._check_no_overlaps(lr)


class TestSpaghettiVisualization:
    """Generate visualizations for spaghetti layouts (only runs with --viz)."""

    def test_viz_iron_gear_wheel(self, viz):
        result = solve("iron-gear-wheel", target_rate=10, available_inputs={"iron-plate"})
        lr = spaghetti_layout(result)
        graph = build_production_graph(result)
        bp = build_blueprint(lr, label="spaghetti: 10/s iron-gear-wheel")
        viz(bp, "spaghetti-iron-gear-wheel-10s", solver_result=result, production_graph=graph)

    def test_viz_electronic_circuit(self, viz):
        result = solve(
            "electronic-circuit",
            target_rate=10,
            available_inputs={"iron-plate", "copper-plate"},
        )
        lr = spaghetti_layout(result)
        graph = build_production_graph(result)
        bp = build_blueprint(lr, label="spaghetti: 10/s electronic-circuit")
        viz(bp, "spaghetti-electronic-circuit-10s", solver_result=result, production_graph=graph)

    def test_viz_plastic_bar(self, viz):
        result = solve(
            "plastic-bar",
            target_rate=5,
            available_inputs={"petroleum-gas", "coal"},
        )
        lr = spaghetti_layout(result)
        graph = build_production_graph(result)
        bp = build_blueprint(lr, label="spaghetti: 5/s plastic-bar")
        viz(bp, "spaghetti-plastic-bar-5s", solver_result=result, production_graph=graph)

    def test_viz_advanced_circuit(self, viz):
        result = solve(
            "advanced-circuit",
            target_rate=5,
            available_inputs={"iron-plate", "copper-plate", "petroleum-gas", "coal"},
        )
        lr = spaghetti_layout(result)
        graph = build_production_graph(result)
        bp = build_blueprint(lr, label="spaghetti: 5/s advanced-circuit")
        viz(bp, "spaghetti-advanced-circuit-5s", solver_result=result, production_graph=graph)

    def test_viz_petroleum_gas(self, viz):
        result = solve(
            "petroleum-gas",
            target_rate=10,
            available_inputs={"crude-oil"},
        )
        lr = spaghetti_layout(result)
        graph = build_production_graph(result)
        bp = build_blueprint(lr, label="spaghetti: 10/s petroleum-gas")
        viz(bp, "spaghetti-petroleum-gas-10s", solver_result=result, production_graph=graph)
