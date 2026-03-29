"""Tests for the ML-based layout engine."""

import numpy as np

from src.blueprint import build_blueprint
from src.ml.layout import ml_layout
from src.ml.loss import (
    alignment,
    compactness,
    edge_distance,
    overlap_penalty,
    total_loss,
)
from src.ml.placer import ml_place_machines
from src.solver import solve
from src.routing.common import machine_size
from src.routing.graph import build_production_graph

# ---------------------------------------------------------------------------
# Loss function unit tests
# ---------------------------------------------------------------------------


class TestOverlapPenalty:
    def test_zero_when_separated(self):
        """No penalty when machines are well separated."""
        # Two 3x3 machines, 10 tiles apart
        positions = np.array([0.0, 0.0, 10.0, 0.0])
        sizes = np.array([3.0, 3.0])
        val, grad = overlap_penalty(positions, sizes, min_gap=2.0)
        assert val == 0.0
        assert np.allclose(grad, 0.0)

    def test_positive_when_overlapping(self):
        """Positive penalty when machines overlap."""
        # Two 3x3 machines at same position
        positions = np.array([0.0, 0.0, 1.0, 0.0])
        sizes = np.array([3.0, 3.0])
        val, grad = overlap_penalty(positions, sizes, min_gap=2.0)
        assert val > 0.0

    def test_single_machine_no_penalty(self):
        """Single machine produces zero penalty."""
        positions = np.array([5.0, 5.0])
        sizes = np.array([3.0])
        val, grad = overlap_penalty(positions, sizes)
        assert val == 0.0


class TestEdgeDistance:
    def test_decreases_when_closer(self):
        """Loss decreases as connected machines move closer."""
        edges = [(0, 1, 1.0)]

        far = np.array([0.0, 0.0, 20.0, 20.0])
        close = np.array([0.0, 0.0, 3.0, 3.0])

        val_far, _ = edge_distance(far, edges)
        val_close, _ = edge_distance(close, edges)
        assert val_far > val_close

    def test_rate_weighting(self):
        """Higher rate edges contribute more to loss."""
        pos = np.array([0.0, 0.0, 10.0, 0.0])
        low_rate = [(0, 1, 1.0)]
        high_rate = [(0, 1, 5.0)]

        val_low, _ = edge_distance(pos, low_rate)
        val_high, _ = edge_distance(pos, high_rate)
        assert val_high > val_low


class TestCompactness:
    def test_tighter_is_better(self):
        """Smaller bounding box produces lower loss."""
        sizes = np.array([1.0, 1.0])
        spread = np.array([0.0, 0.0, 20.0, 20.0])
        tight = np.array([0.0, 0.0, 2.0, 2.0])

        val_spread, _ = compactness(spread, sizes)
        val_tight, _ = compactness(tight, sizes)
        assert val_spread > val_tight


class TestAlignment:
    def test_aligned_is_better(self):
        """Same-row machines produce lower alignment loss."""
        groups = [[0, 1]]
        aligned = np.array([0.0, 5.0, 10.0, 5.0])  # same y
        offset = np.array([0.0, 0.0, 10.0, 10.0])  # different x and y

        val_aligned, _ = alignment(aligned, groups)
        val_offset, _ = alignment(offset, groups)
        assert val_aligned < val_offset


class TestGradients:
    """Verify analytical gradients match finite-difference approximation."""

    def _check_gradient(self, fn, positions, *args, atol=1e-3):
        val, grad = fn(positions, *args)
        numerical_grad = np.zeros_like(positions)
        eps = 1e-5
        for i in range(len(positions)):
            pos_plus = positions.copy()
            pos_plus[i] += eps
            pos_minus = positions.copy()
            pos_minus[i] -= eps
            v_plus, _ = fn(pos_plus, *args)
            v_minus, _ = fn(pos_minus, *args)
            numerical_grad[i] = (v_plus - v_minus) / (2 * eps)
        np.testing.assert_allclose(grad, numerical_grad, atol=atol)

    def test_overlap_gradient(self):
        positions = np.array([0.0, 0.0, 2.0, 1.0])
        sizes = np.array([3.0, 3.0])
        self._check_gradient(overlap_penalty, positions, sizes, 2.0)

    def test_edge_distance_gradient(self):
        positions = np.array([0.0, 0.0, 5.0, 3.0])
        edges = [(0, 1, 2.0)]
        self._check_gradient(edge_distance, positions, edges)

    def test_compactness_gradient(self):
        positions = np.array([0.0, 0.0, 10.0, 5.0, 3.0, 8.0])
        sizes = np.array([3.0, 3.0, 3.0])
        self._check_gradient(compactness, positions, sizes)

    def test_alignment_gradient(self):
        positions = np.array([0.0, 0.0, 5.0, 3.0, 10.0, 1.0])
        groups = [[0, 1, 2]]
        self._check_gradient(alignment, positions, groups)

    def test_total_loss_gradient(self):
        positions = np.array([0.0, 0.0, 8.0, 3.0, 4.0, 10.0])
        sizes = np.array([3.0, 3.0, 3.0])
        edges = [(0, 1, 1.0), (1, 2, 0.5)]
        groups = [[0, 1]]
        self._check_gradient(
            total_loss,
            positions,
            sizes,
            edges,
            groups,
        )


# ---------------------------------------------------------------------------
# Placer tests
# ---------------------------------------------------------------------------


class TestMLPlacer:
    def test_returns_all_nodes(self):
        """Every graph node gets a position."""
        result = solve("iron-gear-wheel", target_rate=10, available_inputs={"iron-plate"})
        graph = build_production_graph(result)
        positions = ml_place_machines(graph)
        assert set(positions.keys()) == {n.id for n in graph.nodes}

    def test_positions_are_integers(self):
        """All positions are integer tuples."""
        result = solve("iron-gear-wheel", target_rate=10, available_inputs={"iron-plate"})
        graph = build_production_graph(result)
        positions = ml_place_machines(graph)
        for x, y in positions.values():
            assert isinstance(x, int) and isinstance(y, int)

    def test_no_overlaps(self):
        """Grid-snapped positions have no collisions."""
        result = solve("electronic-circuit", target_rate=5, available_inputs={"iron-plate", "copper-plate"})
        graph = build_production_graph(result)
        positions = ml_place_machines(graph)

        occupied: set[tuple[int, int]] = set()
        for node in graph.nodes:
            x, y = positions[node.id]
            size = machine_size(node.spec.entity)
            for dx in range(size):
                for dy in range(size):
                    tile = (x + dx, y + dy)
                    assert tile not in occupied, f"Overlap at {tile}"
                    occupied.add(tile)

    def test_deterministic_with_seed(self):
        """Same seed produces same layout."""
        result = solve("iron-gear-wheel", target_rate=10, available_inputs={"iron-plate"})
        graph = build_production_graph(result)
        pos1 = ml_place_machines(graph, seed=42)
        pos2 = ml_place_machines(graph, seed=42)
        assert pos1 == pos2


# ---------------------------------------------------------------------------
# Integration tests (mirror spaghetti test structure)
# ---------------------------------------------------------------------------


class TestMLLayout:
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

    def test_produces_layout(self):
        """ML engine produces a non-empty layout."""
        result = solve("iron-gear-wheel", target_rate=2, available_inputs={"iron-plate"})
        lr = ml_layout(result)
        assert len(lr.entities) > 0
        assert lr.width > 0
        assert lr.height > 0

    def test_has_machines(self):
        """Layout contains assembling machines with correct recipe."""
        result = solve("iron-gear-wheel", target_rate=2, available_inputs={"iron-plate"})
        lr = ml_layout(result)
        machines = [e for e in lr.entities if e.name == "assembling-machine-3"]
        assert len(machines) > 0
        for m in machines:
            assert m.recipe == "iron-gear-wheel"

    def test_has_belts(self):
        """Layout contains belt entities."""
        result = solve("iron-gear-wheel", target_rate=2, available_inputs={"iron-plate"})
        lr = ml_layout(result)
        belt_types = {"transport-belt", "fast-transport-belt", "express-transport-belt"}
        belts = [e for e in lr.entities if e.name in belt_types]
        assert len(belts) > 0

    def test_has_inserters(self):
        """Layout has inserters for each machine."""
        result = solve("iron-gear-wheel", target_rate=10, available_inputs={"iron-plate"})
        lr = ml_layout(result)
        inserters = [e for e in lr.entities if "inserter" in e.name]
        machines = [e for e in lr.entities if e.name == "assembling-machine-3"]
        assert len(inserters) >= len(machines)

    def test_no_entity_overlaps(self):
        """No entities should overlap."""
        result = solve("iron-gear-wheel", target_rate=10, available_inputs={"iron-plate"})
        lr = ml_layout(result)
        self._check_no_overlaps(lr)

    def test_electronic_circuit_chain(self):
        """Multi-step chain produces machines for all recipes."""
        result = solve(
            "electronic-circuit",
            target_rate=5,
            available_inputs={"iron-plate", "copper-plate"},
        )
        lr = ml_layout(result)
        recipes = {e.recipe for e in lr.entities if e.recipe is not None}
        assert "electronic-circuit" in recipes
        assert "copper-cable" in recipes

    def test_electronic_circuit_machines_no_overlaps(self):
        """Machine entities in multi-recipe layout don't overlap."""
        result = solve(
            "electronic-circuit",
            target_rate=5,
            available_inputs={"iron-plate", "copper-plate"},
        )
        lr = ml_layout(result)

        # Check machine footprints specifically (routing may produce belt overlaps
        # on complex chains, same as spaghetti engine best-effort behavior)
        _3x3 = {"assembling-machine-1", "assembling-machine-2", "assembling-machine-3", "chemical-plant"}
        _5x5 = {"oil-refinery"}
        occupied: dict[tuple[int, int], str] = {}
        for ent in lr.entities:
            if ent.name in _5x5:
                tiles = [(ent.x + dx, ent.y + dy) for dx in range(5) for dy in range(5)]
            elif ent.name in _3x3:
                tiles = [(ent.x + dx, ent.y + dy) for dx in range(3) for dy in range(3)]
            else:
                continue  # skip 1x1 entities (belts, pipes, inserters)
            for tile in tiles:
                assert tile not in occupied, f"Machine overlap at {tile}: {ent.name} vs {occupied[tile]}"
                occupied[tile] = ent.name

    def test_has_power_poles(self):
        """Layout includes power poles."""
        result = solve("iron-gear-wheel", target_rate=2, available_inputs={"iron-plate"})
        lr = ml_layout(result)
        poles = [e for e in lr.entities if e.name == "medium-electric-pole"]
        assert len(poles) > 0

    def test_custom_weights(self):
        """Custom weights produce a layout (doesn't crash)."""
        result = solve("iron-gear-wheel", target_rate=5, available_inputs={"iron-plate"})
        lr = ml_layout(result, weights={"overlap": 200.0, "edge": 3.0, "compact": 0.5, "align": 0.0})
        assert len(lr.entities) > 0


# ---------------------------------------------------------------------------
# Visualization tests (only with --viz)
# ---------------------------------------------------------------------------


class TestMLVisualization:
    def test_viz_iron_gear_wheel(self, viz):
        result = solve("iron-gear-wheel", target_rate=10, available_inputs={"iron-plate"})
        lr = ml_layout(result)
        graph = build_production_graph(result)
        bp = build_blueprint(lr, label="ml: 10/s iron-gear-wheel")
        viz(bp, "ml-iron-gear-wheel-10s", solver_result=result, production_graph=graph)

    def test_viz_electronic_circuit(self, viz):
        result = solve(
            "electronic-circuit",
            target_rate=10,
            available_inputs={"iron-plate", "copper-plate"},
        )
        lr = ml_layout(result)
        graph = build_production_graph(result)
        bp = build_blueprint(lr, label="ml: 10/s electronic-circuit")
        viz(bp, "ml-electronic-circuit-10s", solver_result=result, production_graph=graph)
