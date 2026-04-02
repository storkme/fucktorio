"""Tests for the bus layout engine."""

from __future__ import annotations

import pytest

from src.bus import bus_layout
from src.solver.solver import solve
from src.validate import ValidationError, validate


class TestBusLayout:
    """Basic bus layout tests — assert zero validation errors."""

    def test_iron_gear_wheel(self):
        """Single recipe, single solid input."""
        result = solve("iron-gear-wheel", 5.0)
        layout = bus_layout(result)

        assert len(layout.entities) > 0
        assert layout.width > 0
        assert layout.height > 0

        # Should produce zero validation errors
        try:
            validate(layout, result, layout_style="bus")
        except ValidationError as e:
            for issue in e.issues:
                print(f"  [{issue.severity}] {issue.category}: {issue.message}")
            pytest.fail(f"Validation failed with {len(e.issues)} errors")

    def test_copper_cable_with_smelting(self):
        """Intermediate recipe: copper-ore -> copper-plate -> copper-cable."""
        result = solve("copper-cable", 5.0)
        layout = bus_layout(result)

        assert len(layout.entities) > 0

        try:
            validate(layout, result, layout_style="bus")
        except ValidationError as e:
            for issue in e.issues:
                print(f"  [{issue.severity}] {issue.category}: {issue.message}")
            pytest.fail(f"Validation failed with {len(e.issues)} errors")

    def test_electronic_circuit(self):
        """Dual input: copper-cable + iron-plate -> electronic-circuit."""
        result = solve("electronic-circuit", 5.0, available_inputs={"iron-plate", "copper-plate"})
        layout = bus_layout(result)

        assert len(layout.entities) > 0

        try:
            validate(layout, result, layout_style="bus")
        except ValidationError as e:
            for issue in e.issues:
                print(f"  [{issue.severity}] {issue.category}: {issue.message}")
            pytest.fail(f"Validation failed with {len(e.issues)} errors")

    def test_plastic_bar(self):
        """Fluid recipe: coal + petroleum-gas -> plastic-bar."""
        result = solve("plastic-bar", 5.0, available_inputs={"coal", "petroleum-gas"})
        layout = bus_layout(result)

        assert len(layout.entities) > 0

        try:
            validate(layout, result, layout_style="bus")
        except ValidationError as e:
            for issue in e.issues:
                print(f"  [{issue.severity}] {issue.category}: {issue.message}")
            pytest.fail(f"Validation failed with {len(e.issues)} errors")

    def test_electronic_circuit_from_ores(self):
        """Full chain: ores -> smelting -> copper-cable -> electronic-circuit."""
        result = solve("electronic-circuit", 5.0)
        layout = bus_layout(result)

        assert len(layout.entities) > 0

        try:
            validate(layout, result, layout_style="bus")
        except ValidationError as e:
            for issue in e.issues:
                print(f"  [{issue.severity}] {issue.category}: {issue.message}")
            pytest.fail(f"Validation failed with {len(e.issues)} errors")

    def test_yellow_belt_constraint(self):
        """Belt tier constraint forces yellow belts where possible.

        Output collector trunks and underground crossings may auto-upgrade
        to higher tiers when yellow belt capacity is insufficient.
        """
        result = solve("iron-gear-wheel", 10.0)
        layout = bus_layout(result, max_belt_tier="transport-belt")

        try:
            validate(layout, result, layout_style="bus")
        except ValidationError as e:
            for issue in e.issues:
                print(f"  [{issue.severity}] {issue.category}: {issue.message}")
            pytest.fail(f"Validation failed with {len(e.issues)} errors")

    def test_row_splitting(self):
        """High throughput triggers row splitting when output exceeds belt capacity."""
        # 40/s copper-cable = 8 machines. Express belt max 2 per row → split to 2x4.
        result = solve("copper-cable", 40.0, available_inputs={"copper-plate"})
        layout = bus_layout(result)

        assert len(layout.entities) > 0

        try:
            validate(layout, result, layout_style="bus")
        except ValidationError as e:
            for issue in e.issues:
                print(f"  [{issue.severity}] {issue.category}: {issue.message}")
            pytest.fail(f"Validation failed with {len(e.issues)} errors")

    def test_no_entity_overlaps(self):
        """All entities must occupy unique tile positions."""
        result = solve("iron-gear-wheel", 5.0)
        layout = bus_layout(result)

        occupied: dict[tuple[int, int], str] = {}
        _MULTI_TILE = {
            "assembling-machine-1": 3,
            "assembling-machine-2": 3,
            "assembling-machine-3": 3,
            "chemical-plant": 3,
            "electric-furnace": 3,
            "oil-refinery": 5,
        }

        for ent in layout.entities:
            sz = _MULTI_TILE.get(ent.name, 1)
            for dx in range(sz):
                for dy in range(sz):
                    pos = (ent.x + dx, ent.y + dy)
                    if pos in occupied:
                        pytest.fail(f"Overlap at {pos}: {ent.name} conflicts with {occupied[pos]}")
                    occupied[pos] = ent.name


class TestBusVisualization:
    """Visualization tests — run with --viz flag."""

    def test_viz_iron_gear_wheel(self, viz):
        result = solve("iron-gear-wheel", 10.0)
        layout = bus_layout(result)
        bp = _make_blueprint(layout, "bus: 10/s iron-gear-wheel")
        viz(bp, "bus-iron-gear-wheel-10s", solver_result=result, layout_result=layout, layout_style="bus")

    def test_viz_copper_cable_smelting(self, viz):
        result = solve("copper-cable", 5.0)
        layout = bus_layout(result)
        bp = _make_blueprint(layout, "bus: 5/s copper-cable (smelting)")
        viz(bp, "bus-copper-cable-5s", solver_result=result, layout_result=layout, layout_style="bus")

    def test_viz_electronic_circuit(self, viz):
        result = solve("electronic-circuit", 5.0, available_inputs={"iron-plate", "copper-plate"})
        layout = bus_layout(result)
        bp = _make_blueprint(layout, "bus: 5/s electronic-circuit")
        viz(bp, "bus-electronic-circuit-5s", solver_result=result, layout_result=layout, layout_style="bus")

    def test_viz_electronic_circuit_from_ores(self, viz):
        result = solve("electronic-circuit", 5.0)
        layout = bus_layout(result)
        bp = _make_blueprint(layout, "bus: 5/s electronic-circuit (from ores)")
        viz(bp, "bus-electronic-circuit-from-ores-5s", solver_result=result, layout_result=layout, layout_style="bus")

    def test_viz_iron_gear_wheel_yellow_belt(self, viz):
        """Yellow belt constraint forces row + trunk splitting."""
        result = solve("iron-gear-wheel", 10.0)
        layout = bus_layout(result, max_belt_tier="transport-belt")
        bp = _make_blueprint(layout, "bus: 10/s iron-gear-wheel (yellow belt)")
        viz(bp, "bus-iron-gear-wheel-10s-yellow", solver_result=result, layout_result=layout, layout_style="bus")

    def test_viz_electronic_circuit_yellow_belt(self, viz):
        """Yellow belt constraint on multi-input recipe."""
        result = solve("electronic-circuit", 5.0, available_inputs={"iron-plate", "copper-plate"})
        layout = bus_layout(result, max_belt_tier="transport-belt")
        bp = _make_blueprint(layout, "bus: 5/s electronic-circuit (yellow belt)")
        viz(bp, "bus-electronic-circuit-5s-yellow", solver_result=result, layout_result=layout, layout_style="bus")

    def test_viz_iron_gear_wheel_20s(self, viz):
        """High rate with auto belt — overflow handling splits trunks."""
        result = solve("iron-gear-wheel", 20.0)
        layout = bus_layout(result)
        bp = _make_blueprint(layout, "bus: 20/s iron-gear-wheel")
        viz(bp, "bus-iron-gear-wheel-20s", solver_result=result, layout_result=layout, layout_style="bus")

    def test_viz_electronic_circuit_20s(self, viz):
        """High rate multi-input layout."""
        result = solve("electronic-circuit", 20.0, available_inputs={"iron-plate", "copper-plate"})
        layout = bus_layout(result)
        bp = _make_blueprint(layout, "bus: 20/s electronic-circuit")
        viz(bp, "bus-electronic-circuit-20s", solver_result=result, layout_result=layout, layout_style="bus")


def _make_blueprint(layout, label):
    from src.blueprint import build_blueprint

    return build_blueprint(layout, label=label)
