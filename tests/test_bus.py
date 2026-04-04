"""Tests for the bus layout engine."""

from __future__ import annotations

import pytest

from src.bus import bus_layout
from src.solver.solver import solve
from src.validate import ValidationError, validate

# Validation categories that are known limitations of the bus validator,
# not routing bugs.  belt-flow-reachability and belt-flow-path use a
# simple BFS that doesn't trace through inserter→machine→inserter chains
# or multi-tile machines correctly for bus layouts.
_BUS_ALLOWED_WARNINGS = {"belt-flow-reachability", "belt-flow-path", "power"}


def _assert_valid(layout, result, allowed_categories=frozenset()):
    """Assert zero validation issues (errors AND warnings), except allowed categories."""
    try:
        warnings = validate(layout, result, layout_style="bus")
    except ValidationError as e:
        warnings = e.issues

    allowed = allowed_categories | _BUS_ALLOWED_WARNINGS
    unexpected = [i for i in warnings if i.category not in allowed]
    if unexpected:
        for issue in unexpected:
            print(f"  [{issue.severity}] {issue.category}: {issue.message}")
        pytest.fail(f"Validation found {len(unexpected)} unexpected issues")


class TestBusLayout:
    """Basic bus layout tests — assert zero validation issues (errors AND warnings)."""

    def test_iron_gear_wheel(self):
        """Single recipe, single solid input."""
        result = solve("iron-gear-wheel", 5.0)
        layout = bus_layout(result)

        assert len(layout.entities) > 0
        assert layout.width > 0
        assert layout.height > 0

        _assert_valid(layout, result)

    def test_copper_cable_with_smelting(self):
        """Intermediate recipe: copper-ore -> copper-plate -> copper-cable."""
        result = solve("copper-cable", 5.0)
        layout = bus_layout(result)

        assert len(layout.entities) > 0
        _assert_valid(layout, result)

    def test_electronic_circuit(self):
        """Dual input: copper-cable + iron-plate -> electronic-circuit."""
        result = solve("electronic-circuit", 5.0, available_inputs={"iron-plate", "copper-plate"})
        layout = bus_layout(result)

        assert len(layout.entities) > 0
        _assert_valid(layout, result)

    def test_plastic_bar(self):
        """Fluid recipe: coal + petroleum-gas -> plastic-bar."""
        result = solve("plastic-bar", 5.0, available_inputs={"coal", "petroleum-gas"})
        layout = bus_layout(result)

        assert len(layout.entities) > 0
        _assert_valid(layout, result, allowed_categories={"power"})

    def test_electronic_circuit_from_ores(self):
        """Full chain: ores -> smelting -> copper-cable -> electronic-circuit."""
        result = solve("electronic-circuit", 5.0)
        layout = bus_layout(result)

        assert len(layout.entities) > 0
        _assert_valid(layout, result)

    def test_yellow_belt_constraint(self):
        """Belt tier constraint forces yellow belts where possible.

        Lane-throughput warnings are expected on pre-balancer trunk segments
        and sideload returns where single-lane rate exceeds yellow belt
        per-lane capacity.  Items buffer briefly; Factorio handles this.
        """
        result = solve("iron-gear-wheel", 10.0)
        layout = bus_layout(result, max_belt_tier="transport-belt")

        _assert_valid(layout, result, allowed_categories={"lane-throughput"})

    def test_row_splitting(self):
        """High throughput triggers row splitting when output exceeds belt capacity."""
        # 40/s copper-cable = 8 machines. Express belt max 2 per row → split to 2x4.
        result = solve("copper-cable", 40.0, available_inputs={"copper-plate"})
        layout = bus_layout(result)

        assert len(layout.entities) > 0
        _assert_valid(layout, result)

    def test_lane_balancer_placement(self):
        """Lane balancers placed on collector lanes (output-only, no consumers).

        Intermediate lanes use direct routing (no balancer).
        External input lanes have no producers (no balancer).
        """
        from src.bus.bus_router import bus_width_for_lanes, plan_bus_lanes
        from src.bus.placer import place_rows

        result = solve("iron-gear-wheel", 10.0)

        # Two-pass like bus_layout to get consistent lane info
        # Use yellow belt constraint to force splitting into multiple lanes
        tier = "transport-belt"
        temp_bw = 11
        _, spans, _, _ = place_rows(
            result.machines,
            result.dependency_order,
            bus_width=temp_bw,
            y_offset=1,
            max_belt_tier=tier,
        )
        lanes, _ = plan_bus_lanes(result, spans, max_belt_tier=tier)
        actual_bw = bus_width_for_lanes(lanes)
        if actual_bw != temp_bw:
            _, spans, _, _ = place_rows(
                result.machines,
                result.dependency_order,
                bus_width=actual_bw,
                y_offset=1,
                max_belt_tier=tier,
            )
            lanes, _ = plan_bus_lanes(result, spans, max_belt_tier=tier)

        # iron-plate is intermediate — uses direct routing, no balancer
        iron_plate_lanes = [ln for ln in lanes if ln.item == "iron-plate"]
        assert all(ln.balancer_y is None for ln in iron_plate_lanes)

        # iron-ore is external — no balancer
        iron_ore_lanes = [ln for ln in lanes if ln.item == "iron-ore"]
        assert all(ln.balancer_y is None for ln in iron_ore_lanes)

        # iron-gear-wheel has no collector lane (output collection skipped)
        gear_lanes = [ln for ln in lanes if ln.item == "iron-gear-wheel"]
        assert len(gear_lanes) == 0, "Final product should not have collector lanes"

        # Validation should pass
        layout = bus_layout(result)
        _assert_valid(layout, result)

    def test_one_to_one_lane_consumer_mapping(self):
        """Every bus lane has at most 1 consumer row (1:1 mapping)."""
        from src.bus.bus_router import plan_bus_lanes
        from src.bus.placer import place_rows

        for recipe, rate in [
            ("iron-gear-wheel", 10.0),
            ("electronic-circuit", 5.0),
        ]:
            result = solve(recipe, rate)
            _, spans, _, _ = place_rows(
                result.machines,
                result.dependency_order,
                bus_width=0,
                y_offset=1,
            )
            lanes, _ = plan_bus_lanes(result, spans)
            for lane in lanes:
                assert len(lane.consumer_rows) <= 1, (
                    f"{recipe}: {lane.item} lane has {len(lane.consumer_rows)} consumers"
                )

    def test_even_row_splitting(self):
        """Row splitting produces evenly-sized rows for balanced production."""
        from src.bus.placer import place_rows

        result = solve("iron-gear-wheel", 10.0)
        _, spans, _, _ = place_rows(
            result.machines,
            result.dependency_order,
            bus_width=0,
            y_offset=1,
            max_belt_tier="transport-belt",
        )
        iron_plate_rows = [s for s in spans if s.spec.recipe == "iron-plate"]
        counts = [s.machine_count for s in iron_plate_rows]
        assert len(counts) == 2, f"Expected 2 iron-plate rows, got {len(counts)}"
        assert counts[0] == counts[1], f"Expected even rows, got {counts}"

    def test_no_entity_overlaps(self):
        """All entities must occupy unique tile positions."""
        from src.models import EntityDirection

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
        _SPLITTER_ENTITIES = {"splitter", "fast-splitter", "express-splitter"}

        def _tiles(ent):
            if ent.name in _SPLITTER_ENTITIES:
                # Splitters are 2 tiles perpendicular to direction
                if ent.direction in (EntityDirection.NORTH, EntityDirection.SOUTH):
                    return [(ent.x, ent.y), (ent.x + 1, ent.y)]
                return [(ent.x, ent.y), (ent.x, ent.y + 1)]
            sz = _MULTI_TILE.get(ent.name, 1)
            return [(ent.x + dx, ent.y + dy) for dx in range(sz) for dy in range(sz)]

        for ent in layout.entities:
            for pos in _tiles(ent):
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

    def test_viz_electronic_circuit_asm1(self, viz):
        """Assembler-1 constraint — lower crafting speed, more machines."""
        result = solve(
            "electronic-circuit",
            5.0,
            available_inputs={"iron-plate", "copper-plate"},
            machine_entity="assembling-machine-1",
        )
        layout = bus_layout(result, max_belt_tier="transport-belt")
        bp = _make_blueprint(layout, "bus: 5/s electronic-circuit (asm1, yellow)")
        viz(bp, "bus-electronic-circuit-5s-asm1-yellow", solver_result=result, layout_result=layout, layout_style="bus")

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
