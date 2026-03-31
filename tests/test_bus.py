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
                        pytest.fail(
                            f"Overlap at {pos}: {ent.name} conflicts with {occupied[pos]}"
                        )
                    occupied[pos] = ent.name


class TestBusVisualization:
    """Visualization tests — run with --viz flag."""

    @pytest.fixture(autouse=True)
    def _skip_without_viz(self, request):
        if not request.config.getoption("--viz", default=False):
            pytest.skip("Use --viz to generate visualizations")

    def test_viz_iron_gear_wheel(self):
        result = solve("iron-gear-wheel", 10.0)
        layout = bus_layout(result)
        _generate_viz(layout, result, "bus-iron-gear-wheel-10s")

    def test_viz_copper_cable_smelting(self):
        result = solve("copper-cable", 5.0)
        layout = bus_layout(result)
        _generate_viz(layout, result, "bus-copper-cable-5s")

    def test_viz_electronic_circuit(self):
        result = solve("electronic-circuit", 5.0, available_inputs={"iron-plate", "copper-plate"})
        layout = bus_layout(result)
        _generate_viz(layout, result, "bus-electronic-circuit-5s")


def _generate_viz(layout, solver_result, name):
    """Generate HTML visualization for a bus layout."""
    from src.validate import ValidationError, validate
    from src.verify import ascii_map

    errors = []
    try:
        validate(layout, solver_result, layout_style="bus")
    except ValidationError as e:
        errors = e.issues

    print(f"\n=== {name} ===")
    print(f"Entities: {len(layout.entities)}, Size: {layout.width}x{layout.height}")
    print(f"Errors: {len(errors)}")
    for err in errors:
        print(f"  [{err.severity}] {err.category}: {err.message}")

    print("\nASCII map:")
    print(ascii_map(layout.entities))
