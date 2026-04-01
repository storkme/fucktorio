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
        """Belt tier constraint forces yellow belts, splitting rows as needed."""
        result = solve("iron-gear-wheel", 2.0)
        layout = bus_layout(result, max_belt_tier="transport-belt")

        # Verify all belts are yellow
        for e in layout.entities:
            if "belt" in e.name:
                assert e.name in ("transport-belt", "underground-belt"), (
                    f"Expected yellow belt, got {e.name} at ({e.x},{e.y})"
                )

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
        viz(bp, "bus-iron-gear-wheel-10s", solver_result=result, layout_result=layout)

    def test_viz_copper_cable_smelting(self, viz):
        result = solve("copper-cable", 5.0)
        layout = bus_layout(result)
        bp = _make_blueprint(layout, "bus: 5/s copper-cable (smelting)")
        viz(bp, "bus-copper-cable-5s", solver_result=result, layout_result=layout)

    def test_viz_electronic_circuit(self, viz):
        result = solve("electronic-circuit", 5.0, available_inputs={"iron-plate", "copper-plate"})
        layout = bus_layout(result)
        bp = _make_blueprint(layout, "bus: 5/s electronic-circuit")
        viz(bp, "bus-electronic-circuit-5s", solver_result=result, layout_result=layout)

    def test_viz_electronic_circuit_from_ores(self, viz):
        result = solve("electronic-circuit", 5.0)
        layout = bus_layout(result)
        bp = _make_blueprint(layout, "bus: 5/s electronic-circuit (from ores)")
        viz(bp, "bus-electronic-circuit-from-ores-5s", solver_result=result, layout_result=layout)


def _make_blueprint(layout, label):
    from src.blueprint import build_blueprint

    return build_blueprint(layout, label=label)
