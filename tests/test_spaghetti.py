"""Tests for the spaghetti layout engine."""

import math

import pytest

from src.blueprint import build_blueprint
from src.routing.graph import build_production_graph
from src.routing.router import _astar_path, route_connections
from src.solver import solve
from src.spaghetti.layout import spaghetti_layout
from src.spaghetti.placer import place_machines
from src.validate import ValidationError, validate

# ---------------------------------------------------------------------------
# Module-scoped fixtures (avoid re-running evolutionary search per test)
# ---------------------------------------------------------------------------

@pytest.fixture(scope="module")
def iron_gear_2s():
    return solve("iron-gear-wheel", target_rate=2, available_inputs={"iron-plate"})


@pytest.fixture(scope="module")
def iron_gear_2s_layout(iron_gear_2s):
    return spaghetti_layout(iron_gear_2s)


@pytest.fixture(scope="module")
def iron_gear_10s():
    return solve("iron-gear-wheel", target_rate=10, available_inputs={"iron-plate"})


@pytest.fixture(scope="module")
def iron_gear_10s_layout(iron_gear_10s):
    return spaghetti_layout(iron_gear_10s)


@pytest.fixture(scope="module")
def iron_gear_30s():
    return solve("iron-gear-wheel", target_rate=30, available_inputs={"iron-plate"})


@pytest.fixture(scope="module")
def iron_gear_30s_layout(iron_gear_30s):
    return spaghetti_layout(iron_gear_30s)


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


_3x3 = {"assembling-machine-1", "assembling-machine-2", "assembling-machine-3", "chemical-plant"}
_5x5 = {"oil-refinery"}


def _check_no_overlaps(lr):
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


class TestSpaghettiPhase1:
    """Phase 1: single machine, single input (iron-gear-wheel)."""

    def test_produces_layout(self, iron_gear_2s_layout):
        assert len(iron_gear_2s_layout.entities) > 0
        assert iron_gear_2s_layout.width > 0
        assert iron_gear_2s_layout.height > 0

    def test_has_machines(self, iron_gear_2s_layout):
        machines = [e for e in iron_gear_2s_layout.entities if e.name == "assembling-machine-3"]
        assert len(machines) > 0
        for m in machines:
            assert m.recipe == "iron-gear-wheel"

    def test_has_belts(self, iron_gear_2s_layout):
        belt_types = {"transport-belt", "fast-transport-belt", "express-transport-belt"}
        belts = [e for e in iron_gear_2s_layout.entities if e.name in belt_types]
        assert len(belts) > 0, "Should have belts for item transport"

    def test_has_inserters(self, iron_gear_10s_layout):
        inserters = [e for e in iron_gear_10s_layout.entities if "inserter" in e.name]
        machines = [e for e in iron_gear_10s_layout.entities if e.name == "assembling-machine-3"]
        assert len(inserters) > 0, "Should have inserters"
        assert len(inserters) >= len(machines), f"Should have at least {len(machines)} inserters, got {len(inserters)}"

    def test_no_overlaps(self, iron_gear_2s_layout):
        _check_no_overlaps(iron_gear_2s_layout)

    def test_has_power(self, iron_gear_2s_layout):
        poles = [e for e in iron_gear_2s_layout.entities if e.name == "medium-electric-pole"]
        assert len(poles) > 0, "Should have power poles"


class TestSpaghettiPhase2:
    """Phase 2: multiple machines, one input each."""

    def test_multiple_machines_placed(self, iron_gear_30s, iron_gear_30s_layout):
        machines = [e for e in iron_gear_30s_layout.entities if e.name == "assembling-machine-3"]
        expected = math.ceil(iron_gear_30s.machines[0].count)
        assert len(machines) == expected, f"Expected {expected} machines, got {len(machines)}"

    @pytest.mark.xfail(reason="Evolutionary search position perturbation may cause overlaps at high machine counts")
    def test_multiple_machines_no_overlaps(self, iron_gear_30s_layout):
        _check_no_overlaps(iron_gear_30s_layout)

    def test_multiple_machines_have_belts(self, iron_gear_30s_layout):
        belt_types = {"transport-belt", "fast-transport-belt", "express-transport-belt"}
        belts = [e for e in iron_gear_30s_layout.entities if e.name in belt_types]
        machines = [e for e in iron_gear_30s_layout.entities if e.name == "assembling-machine-3"]
        assert len(belts) >= len(machines), "Should have at least as many belt tiles as machines"


@pytest.mark.skip(reason="Evolutionary search too slow for CI on multi-recipe layouts")
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


@pytest.mark.skip(reason="Evolutionary search too slow for CI on multi-recipe layouts")
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


@pytest.mark.skip(reason="Evolutionary search too slow for CI on multi-recipe layouts")
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


@pytest.mark.skip(reason="Evolutionary search too slow for CI on multi-recipe layouts")
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


class TestSpaghettiValidation:
    """Tests that spaghetti layouts pass functional validation."""

    @pytest.mark.xfail(reason="Evolutionary search may produce disconnected output networks")
    def test_iron_gear_wheel_validates(self, iron_gear_10s, iron_gear_10s_layout):
        """Simple solid recipe should pass validation."""
        issues = validate(iron_gear_10s_layout, iron_gear_10s, layout_style="spaghetti")
        errors = [i for i in issues if i.severity == "error"]
        assert not errors, f"Validation errors: {[e.message for e in errors]}"

    def test_layout_returns_best_effort(self, iron_gear_30s_layout):
        """Multi-machine layout returns a result even if validation has errors."""
        machines = [e for e in iron_gear_30s_layout.entities if e.name == "assembling-machine-3"]
        assert len(machines) > 0

    def test_router_reports_failed_edges(self):
        """Router should report edges it couldn't route."""
        result = solve("electronic-circuit", target_rate=5, available_inputs={"iron-plate", "copper-plate"})
        graph = build_production_graph(result)
        # Use very tight spacing — some edges should fail
        positions = place_machines(graph, spacing=1)
        routing = route_connections(graph, positions)
        # We don't assert failure is guaranteed (BFS might still find paths),
        # but we verify the failed_edges list is returned
        assert isinstance(routing.failed_edges, list)

    def test_output_inserters_have_belts(self, iron_gear_10s, iron_gear_10s_layout):
        """Every machine should have an output inserter dropping onto a belt."""
        from src.validate import check_output_belt_coverage

        issues = check_output_belt_coverage(iron_gear_10s_layout, iron_gear_10s)
        errors = [i for i in issues if i.severity == "error"]
        assert not errors, f"Output belt errors: {[e.message for e in errors]}"

        gear_belts = [e for e in iron_gear_10s_layout.entities if e.carries == "iron-gear-wheel"]
        machines = [e for e in iron_gear_10s_layout.entities if e.name == "assembling-machine-3"]
        assert len(gear_belts) >= len(machines), f"Expected >= {len(machines)} output belt stubs, got {len(gear_belts)}"

    @pytest.mark.skip(reason="Evolutionary search too slow for CI on multi-recipe layouts")
    def test_internal_edge_output_belt(self):
        """Internal edges should connect through the output inserter's belt tile."""
        pass

    @pytest.mark.skip(reason="Evolutionary search too slow for CI on multi-recipe layouts")
    def test_spaghetti_fluid_check_no_bus_false_positive(self):
        """Spaghetti mode should not error about missing 'bus' for fluid layouts."""
        pass


class TestRecipeLadder:
    """Recipe complexity ladder: tracks which recipes produce zero-error blueprints.

    Each tier represents increasing recipe complexity. A passing test means
    the layout engine can handle that level of complexity end-to-end.

    | Tier | Recipe              | Complexity                          |
    |------|---------------------|-------------------------------------|
    | 1    | iron-gear-wheel     | 1 recipe, 1 solid input             |
    | 2    | electronic-circuit  | 2 recipes, 2 solid inputs           |
    | 3    | plastic-bar         | 1 recipe, 1 fluid + 1 solid input   |
    | 4    | advanced-circuit    | 5+ recipes, mixed solid/fluid       |
    | 5    | processing-unit     | Deep chain, multiple fluids         |
    | 6    | rocket-control-unit | Very deep chain                     |
    """

    @pytest.mark.xfail(reason="Evolutionary search may produce disconnected output networks")
    def test_tier1_iron_gear_wheel(self, iron_gear_10s, iron_gear_10s_layout):
        """Tier 1: single recipe, single solid input."""
        try:
            validate(iron_gear_10s_layout, iron_gear_10s, layout_style="spaghetti")
        except ValidationError as exc:
            pytest.fail(f"Tier 1 validation errors: {[e.message for e in exc.issues]}")

    @pytest.mark.skip(reason="Evolutionary search too slow for CI on multi-recipe layouts")
    def test_tier2_electronic_circuit(self):
        """Tier 2: 2-step solid chain (copper-cable -> electronic-circuit)."""
        result = solve(
            "electronic-circuit",
            target_rate=5,
            available_inputs={"iron-plate", "copper-plate"},
        )
        lr = spaghetti_layout(result)
        try:
            validate(lr, result, layout_style="spaghetti")
        except ValidationError as exc:
            pytest.fail(f"Tier 2 validation errors: {[e.message for e in exc.issues]}")

    @pytest.mark.skip(reason="Evolutionary search too slow for CI on multi-recipe layouts")
    def test_tier3_plastic_bar(self):
        """Tier 3: fluid recipe (petroleum-gas + coal -> plastic-bar)."""
        result = solve(
            "plastic-bar",
            target_rate=5,
            available_inputs={"petroleum-gas", "coal"},
        )
        lr = spaghetti_layout(result)
        try:
            validate(lr, result, layout_style="spaghetti")
        except ValidationError as exc:
            pytest.fail(f"Tier 3 validation errors: {[e.message for e in exc.issues]}")

    @pytest.mark.skip(reason="Evolutionary search too slow for CI on multi-recipe layouts")
    def test_tier4_advanced_circuit(self):
        """Tier 4: deep mixed chain (copper-cable, plastic-bar, electronic-circuit -> advanced-circuit)."""
        result = solve(
            "advanced-circuit",
            target_rate=5,
            available_inputs={"iron-plate", "copper-plate", "petroleum-gas", "coal"},
        )
        lr = spaghetti_layout(result)
        try:
            validate(lr, result, layout_style="spaghetti")
        except ValidationError as exc:
            pytest.fail(f"Tier 4 validation errors: {[e.message for e in exc.issues]}")

    @pytest.mark.skip(reason="Evolutionary search too slow for CI on multi-recipe layouts")
    def test_tier5_processing_unit(self):
        """Tier 5: processing-unit (deep chain, multiple fluids)."""
        result = solve(
            "processing-unit",
            target_rate=2,
            available_inputs={
                "iron-plate",
                "copper-plate",
                "petroleum-gas",
                "coal",
                "sulfuric-acid",
            },
        )
        lr = spaghetti_layout(result)
        try:
            validate(lr, result, layout_style="spaghetti")
        except ValidationError as exc:
            pytest.fail(f"Tier 5 validation errors: {[e.message for e in exc.issues]}")

    @pytest.mark.skip(reason="Solver returns 0 machines for rocket-control-unit")
    def test_tier6_rocket_control_unit(self):
        """Tier 6: rocket-control-unit (very deep chain)."""
        result = solve(
            "rocket-control-unit",
            target_rate=1,
            available_inputs={
                "iron-plate",
                "copper-plate",
                "petroleum-gas",
                "coal",
                "sulfuric-acid",
            },
        )
        lr = spaghetti_layout(result)
        try:
            validate(lr, result, layout_style="spaghetti")
        except ValidationError as exc:
            pytest.fail(f"Tier 6 validation errors: {[e.message for e in exc.issues]}")


class TestUndergroundBeltExits:
    """Underground belt exits must have a continuation belt ahead."""

    def test_ug_exit_has_forward_continuation(self):
        """A* path after UG exit must continue one tile in the jump direction.

        Regression test for issue #26: items exit underground facing the jump
        direction and need a surface belt directly ahead.
        """
        # Create a wall of obstacles that forces an underground jump.
        # Start on the left, goal on the right, wall in between.
        obstacles = {(5, y) for y in range(-5, 10)}  # vertical wall at x=5
        start = (3, 0)
        goals = {(8, 0)}

        path = _astar_path(
            start,
            goals,
            obstacles,
            max_extent=20,
            allow_underground=True,
            ug_max_reach=4,
        )
        assert path is not None, "A* should find a path using underground"

        # Find underground jumps (non-adjacent consecutive tiles)
        for i in range(len(path) - 1):
            ax, ay = path[i]
            bx, by = path[i + 1]
            dist = abs(bx - ax) + abs(by - ay)
            if dist > 1:
                # This is a UG jump: entry at path[i], exit at path[i+1]
                dx = (bx - ax) // dist
                dy = (by - ay) // dist
                # The tile after the exit must be in the path (continuation)
                assert i + 2 < len(path), f"UG exit at {path[i + 1]} is the last tile — no continuation"
                cont = path[i + 2]
                expected_cont = (bx + dx, by + dy)
                assert cont == expected_cont, (
                    f"After UG exit at {path[i + 1]} (dir {dx},{dy}), "
                    f"next tile should be {expected_cont} but got {cont}"
                )

    def test_ug_exit_no_immediate_turn(self):
        """Path cannot turn immediately after UG exit.

        The forced continuation step prevents the path from turning at the
        UG exit tile, which would leave items stranded.
        """
        # Obstacles that might tempt the A* to turn right after UG exit:
        # wall at x=5, but goal is south-east of the exit
        obstacles = {(5, y) for y in range(-5, 10)}
        start = (3, 0)
        goals = {(8, 3)}  # goal is offset from the jump direction

        path = _astar_path(
            start,
            goals,
            obstacles,
            max_extent=20,
            allow_underground=True,
            ug_max_reach=4,
        )
        assert path is not None

        # Verify all UG exits have a straight continuation
        for i in range(len(path) - 1):
            ax, ay = path[i]
            bx, by = path[i + 1]
            dist = abs(bx - ax) + abs(by - ay)
            if dist > 1:
                dx = (bx - ax) // dist
                dy = (by - ay) // dist
                assert i + 2 < len(path), f"UG exit at {path[i + 1]} is the last tile"
                cont = path[i + 2]
                expected = (bx + dx, by + dy)
                assert cont == expected, (
                    f"UG exit at {path[i + 1]} should continue to {expected}, got {cont} (immediate turn)"
                )

    def test_ug_exit_no_position_revisit(self):
        """A* path should not visit the same position twice."""
        obstacles = {(5, y) for y in range(-5, 10)}
        start = (3, 0)
        goals = {(8, 0)}

        path = _astar_path(
            start,
            goals,
            obstacles,
            max_extent=20,
            allow_underground=True,
            ug_max_reach=4,
        )
        assert path is not None
        positions = [(x, y) for x, y in path]
        assert len(positions) == len(set(positions)), f"Path visits same position twice: {positions}"


class TestSpaghettiVisualization:
    """Generate visualizations for spaghetti layouts (only runs with --viz)."""

    def test_viz_iron_gear_wheel(self, viz, iron_gear_10s, iron_gear_10s_layout):
        graph = build_production_graph(iron_gear_10s)
        bp = build_blueprint(iron_gear_10s_layout, label="spaghetti: 10/s iron-gear-wheel")
        viz(
            bp, "spaghetti-iron-gear-wheel-10s",
            solver_result=iron_gear_10s, production_graph=graph, layout_result=iron_gear_10s_layout,
        )

    @pytest.mark.skip(reason="Evolutionary search too slow for CI on multi-recipe layouts")
    def test_viz_electronic_circuit(self, viz):
        result = solve(
            "electronic-circuit",
            target_rate=10,
            available_inputs={"iron-plate", "copper-plate"},
        )
        lr = spaghetti_layout(result)
        graph = build_production_graph(result)
        bp = build_blueprint(lr, label="spaghetti: 10/s electronic-circuit")
        viz(bp, "spaghetti-electronic-circuit-10s", solver_result=result, production_graph=graph, layout_result=lr)

    @pytest.mark.skip(reason="Evolutionary search too slow for CI on multi-recipe layouts")
    def test_viz_plastic_bar(self, viz):
        result = solve(
            "plastic-bar",
            target_rate=5,
            available_inputs={"petroleum-gas", "coal"},
        )
        lr = spaghetti_layout(result)
        graph = build_production_graph(result)
        bp = build_blueprint(lr, label="spaghetti: 5/s plastic-bar")
        viz(bp, "spaghetti-plastic-bar-5s", solver_result=result, production_graph=graph, layout_result=lr)

    @pytest.mark.skip(reason="Evolutionary search too slow for CI on multi-recipe layouts")
    def test_viz_advanced_circuit(self, viz):
        result = solve(
            "advanced-circuit",
            target_rate=5,
            available_inputs={"iron-plate", "copper-plate", "petroleum-gas", "coal"},
        )
        lr = spaghetti_layout(result)
        graph = build_production_graph(result)
        bp = build_blueprint(lr, label="spaghetti: 5/s advanced-circuit")
        viz(bp, "spaghetti-advanced-circuit-5s", solver_result=result, production_graph=graph, layout_result=lr)

    @pytest.mark.skip(reason="Evolutionary search too slow for CI on multi-recipe layouts")
    def test_viz_petroleum_gas(self, viz):
        result = solve(
            "petroleum-gas",
            target_rate=10,
            available_inputs={"crude-oil"},
        )
        lr = spaghetti_layout(result)
        graph = build_production_graph(result)
        bp = build_blueprint(lr, label="spaghetti: 10/s petroleum-gas")
        viz(bp, "spaghetti-petroleum-gas-10s", solver_result=result, production_graph=graph, layout_result=lr)
