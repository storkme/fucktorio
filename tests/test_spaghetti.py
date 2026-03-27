"""Tests for the spaghetti layout engine."""

import math

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
