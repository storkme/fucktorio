"""Tests for the blueprint analysis pipeline."""

import pytest

from src.analysis import analyze_blueprint
from src.analysis.classify import classify_entities
from src.analysis.trace import trace_belt_networks
from src.blueprint import build_blueprint
from src.models import EntityDirection, LayoutResult, PlacedEntity

# ---------------------------------------------------------------------------
# Hand-crafted minimal blueprints for unit testing
# ---------------------------------------------------------------------------


def _simple_belt_blueprint() -> str:
    """3 belts in a line: (0,0)→(1,0)→(2,0) all EAST."""
    lr = LayoutResult(
        entities=[
            PlacedEntity(name="transport-belt", x=0, y=0, direction=EntityDirection.EAST),
            PlacedEntity(name="transport-belt", x=1, y=0, direction=EntityDirection.EAST),
            PlacedEntity(name="transport-belt", x=2, y=0, direction=EntityDirection.EAST),
        ],
        width=3,
        height=1,
    )
    return build_blueprint(lr, label="test-belts")


def _machine_with_inserter_and_belt() -> str:
    """assembling-machine-3 at (0,0) with inserter at (1,-1) facing SOUTH,
    belt at (1,-2) going EAST. Inserter drops into machine → input link."""
    lr = LayoutResult(
        entities=[
            PlacedEntity(name="assembling-machine-3", x=0, y=0, recipe="iron-gear-wheel"),
            PlacedEntity(name="inserter", x=1, y=-1, direction=EntityDirection.SOUTH),
            PlacedEntity(name="transport-belt", x=1, y=-2, direction=EntityDirection.EAST),
        ],
        width=3,
        height=5,
    )
    return build_blueprint(lr, label="test-machine")


def _two_disconnected_belt_networks() -> str:
    """Two separate belt lines with a gap between them."""
    lr = LayoutResult(
        entities=[
            # Network 1: row y=0
            PlacedEntity(name="transport-belt", x=0, y=0, direction=EntityDirection.EAST),
            PlacedEntity(name="transport-belt", x=1, y=0, direction=EntityDirection.EAST),
            # Network 2: row y=5 (gap of 4)
            PlacedEntity(name="transport-belt", x=0, y=5, direction=EntityDirection.EAST),
            PlacedEntity(name="transport-belt", x=1, y=5, direction=EntityDirection.EAST),
        ],
        width=2,
        height=6,
    )
    return build_blueprint(lr, label="test-disconnected")


def _belt_with_turn() -> str:
    """Belt going EAST then SOUTH — one turn."""
    lr = LayoutResult(
        entities=[
            PlacedEntity(name="transport-belt", x=0, y=0, direction=EntityDirection.EAST),
            PlacedEntity(name="transport-belt", x=1, y=0, direction=EntityDirection.SOUTH),
        ],
        width=2,
        height=1,
    )
    return build_blueprint(lr, label="test-turn")


# ---------------------------------------------------------------------------
# Classification tests
# ---------------------------------------------------------------------------


class TestClassification:
    def test_classify_belts(self):
        classified = classify_entities(_simple_belt_blueprint())
        assert len(classified.belt_segments) == 3
        assert len(classified.machines) == 0
        assert len(classified.inserters) == 0

    def test_classify_machine(self):
        classified = classify_entities(_machine_with_inserter_and_belt())
        assert len(classified.machines) == 1
        assert classified.machines[0].recipe == "iron-gear-wheel"
        assert "iron-plate" in classified.machines[0].inputs
        assert "iron-gear-wheel" in classified.machines[0].outputs
        assert len(classified.inserters) == 1
        assert len(classified.belt_segments) == 1

    def test_classify_unhandled(self):
        """Poles and other entities go to unhandled."""
        lr = LayoutResult(
            entities=[
                PlacedEntity(name="medium-electric-pole", x=0, y=0),
                PlacedEntity(name="transport-belt", x=2, y=0, direction=EntityDirection.EAST),
            ],
            width=3,
            height=1,
        )
        classified = classify_entities(build_blueprint(lr))
        assert len(classified.belt_segments) == 1
        assert "medium-electric-pole" in classified.unhandled


# ---------------------------------------------------------------------------
# Network tracing tests
# ---------------------------------------------------------------------------


class TestNetworkTracing:
    def test_single_belt_network(self):
        classified = classify_entities(_simple_belt_blueprint())
        networks = trace_belt_networks(classified.belt_segments)
        assert len(networks) == 1
        assert networks[0].path_length == 3
        assert networks[0].type == "belt"

    def test_disconnected_networks(self):
        classified = classify_entities(_two_disconnected_belt_networks())
        networks = trace_belt_networks(classified.belt_segments)
        assert len(networks) == 2
        assert all(n.path_length == 2 for n in networks)

    def test_turn_counting(self):
        classified = classify_entities(_belt_with_turn())
        networks = trace_belt_networks(classified.belt_segments)
        assert len(networks) == 1
        assert networks[0].turn_count == 1

    def test_straight_no_turns(self):
        classified = classify_entities(_simple_belt_blueprint())
        networks = trace_belt_networks(classified.belt_segments)
        assert networks[0].turn_count == 0


# ---------------------------------------------------------------------------
# Full pipeline tests
# ---------------------------------------------------------------------------


class TestAnalyzePipeline:
    def test_simple_belts(self):
        graph = analyze_blueprint(_simple_belt_blueprint())
        assert len(graph.machines) == 0
        assert len(graph.networks) == 1
        assert graph.networks[0].path_length == 3

    def test_machine_with_inserter(self):
        graph = analyze_blueprint(_machine_with_inserter_and_belt())
        assert len(graph.machines) == 1
        assert graph.machines[0].recipe == "iron-gear-wheel"
        # Should have an inserter link
        assert len(graph.inserter_links) == 1
        link = graph.inserter_links[0]
        assert link.machine_id == 0
        assert link.role == "input"  # inserter facing SOUTH drops into machine


# ---------------------------------------------------------------------------
# Round-trip tests: generate blueprint → analyze → verify graph structure
# ---------------------------------------------------------------------------


class TestRoundTrip:
    def test_iron_gear_roundtrip(self, iron_gear_solver_result, iron_gear_layout):
        """Generate iron-gear-wheel, analyze the blueprint, verify structure."""
        bp_str = build_blueprint(iron_gear_layout, label="iron-gear-test")
        graph = analyze_blueprint(bp_str)

        # Should find assembling machines with iron-gear-wheel recipe
        gear_machines = [m for m in graph.machines if m.recipe == "iron-gear-wheel"]
        assert len(gear_machines) > 0, "Expected at least one iron-gear-wheel machine"

        # Should have belt networks
        belt_nets = [n for n in graph.networks if n.type == "belt"]
        assert len(belt_nets) > 0, "Expected at least one belt network"

        # Should have inserter links
        assert len(graph.inserter_links) > 0, "Expected inserter links"

        # Should have some edges resolved
        # (item inference may not fully resolve everything, but should get some)
        iron_plate_nets = [n for n in graph.networks if n.inferred_item == "iron-plate"]
        gear_nets = [n for n in graph.networks if n.inferred_item == "iron-gear-wheel"]
        inferred_count = len(iron_plate_nets) + len(gear_nets)

        # Print diagnostic info
        print("\nAnalysis results:")
        print(f"  Machines: {len(graph.machines)}")
        print(f"  Networks: {len(graph.networks)} ({len(belt_nets)} belt)")
        print(f"  Inserter links: {len(graph.inserter_links)}")
        print(f"  Fluid links: {len(graph.fluid_links)}")
        print(f"  Edges: {len(graph.edges)}")
        print(f"  Inferred networks: {inferred_count}")
        print(f"  iron-plate networks: {len(iron_plate_nets)}")
        print(f"  iron-gear-wheel networks: {len(gear_nets)}")
        for n in graph.networks:
            print(f"    net {n.id} ({n.type}): {n.path_length} tiles, {n.turn_count} turns, item={n.inferred_item}")

    @pytest.mark.skip(reason="Electronic circuit layout search too slow without Rust A*")
    def test_electronic_circuit_roundtrip(self, electronic_circuit_solver_result, electronic_circuit_layout):
        """Generate electronic-circuit, analyze, verify multi-recipe structure."""
        bp_str = build_blueprint(electronic_circuit_layout, label="ecircuit-test")
        graph = analyze_blueprint(bp_str)

        # Should find machines for both recipes
        recipes_found = {m.recipe for m in graph.machines if m.recipe is not None}
        assert "electronic-circuit" in recipes_found, f"Missing electronic-circuit in {recipes_found}"
        # copper-cable is an intermediate
        assert "copper-cable" in recipes_found, f"Missing copper-cable in {recipes_found}"

        print("\nElectronic circuit analysis:")
        print(f"  Machines: {len(graph.machines)}")
        print(f"  Recipes: {recipes_found}")
        print(f"  Networks: {len(graph.networks)}")
        print(f"  Inserter links: {len(graph.inserter_links)}")
        print(f"  Edges: {len(graph.edges)}")
        for n in graph.networks:
            print(f"    net {n.id} ({n.type}): {n.path_length} tiles, item={n.inferred_item}")
