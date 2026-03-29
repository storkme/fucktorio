"""Tests for functional blueprint validation."""

from src.layout import layout
from src.models import EntityDirection, ItemFlow, LayoutResult, MachineSpec, PlacedEntity, SolverResult
from src.solver import solve
from src.validate import (
    ValidationError,
    check_belt_connectivity,
    check_belt_direction_continuity,
    check_belt_flow_path,
    check_belt_flow_reachability,
    check_belt_throughput,
    check_fluid_port_connectivity,
    check_inserter_chains,
    check_inserter_direction,
    check_pipe_isolation,
    check_power_coverage,
    validate,
)


class TestPipeIsolation:
    """Tests for adjacent pipe fluid isolation."""

    def test_same_fluid_adjacent_ok(self):
        """Adjacent pipes carrying the same fluid should not error."""
        lr = LayoutResult(
            entities=[
                PlacedEntity(name="pipe", x=0, y=0, carries="water"),
                PlacedEntity(name="pipe", x=1, y=0, carries="water"),
                PlacedEntity(name="pipe", x=2, y=0, carries="water"),
            ]
        )
        issues = check_pipe_isolation(lr)
        assert len(issues) == 0

    def test_different_fluid_adjacent_error(self):
        """Adjacent pipes carrying different fluids should error."""
        lr = LayoutResult(
            entities=[
                PlacedEntity(name="pipe", x=0, y=0, carries="water"),
                PlacedEntity(name="pipe", x=1, y=0, carries="crude-oil"),
            ]
        )
        issues = check_pipe_isolation(lr)
        assert len(issues) == 1
        assert issues[0].severity == "error"
        assert issues[0].category == "pipe-isolation"

    def test_diagonal_pipes_ok(self):
        """Diagonally adjacent pipes with different fluids should not error."""
        lr = LayoutResult(
            entities=[
                PlacedEntity(name="pipe", x=0, y=0, carries="water"),
                PlacedEntity(name="pipe", x=1, y=1, carries="crude-oil"),
            ]
        )
        issues = check_pipe_isolation(lr)
        assert len(issues) == 0

    def test_untagged_pipes_ignored(self):
        """Pipes without carries tag should not trigger errors."""
        lr = LayoutResult(
            entities=[
                PlacedEntity(name="pipe", x=0, y=0, carries="water"),
                PlacedEntity(name="pipe", x=1, y=0, carries=None),
            ]
        )
        issues = check_pipe_isolation(lr)
        assert len(issues) == 0

    def test_separated_pipes_ok(self):
        """Pipes with a gap between them should not error."""
        lr = LayoutResult(
            entities=[
                PlacedEntity(name="pipe", x=0, y=0, carries="water"),
                PlacedEntity(name="pipe", x=2, y=0, carries="crude-oil"),
            ]
        )
        issues = check_pipe_isolation(lr)
        assert len(issues) == 0


class TestFluidPortConnectivity:
    def test_connected_chemical_plant(self):
        """Chemical plant with connected pipes should pass."""
        result = solve(
            "plastic-bar",
            target_rate=10,
            available_inputs={"petroleum-gas", "coal"},
        )
        lr = layout(result)
        issues = check_fluid_port_connectivity(lr)
        errors = [i for i in issues if i.severity == "error"]
        assert len(errors) == 0, f"Unexpected errors: {errors}"

    def test_connected_refinery(self):
        """Oil refinery with connected pipes should pass."""
        result = solve(
            "petroleum-gas",
            target_rate=10,
            available_inputs={"crude-oil"},
        )
        lr = layout(result)
        issues = check_fluid_port_connectivity(lr)
        errors = [i for i in issues if i.severity == "error"]
        assert len(errors) == 0, f"Unexpected errors: {errors}"


class TestInserterChains:
    def test_machines_have_inserters(self):
        """All machines in a valid layout should have adjacent inserters."""
        result = solve(
            "electronic-circuit",
            target_rate=5,
            available_inputs={"iron-plate", "copper-plate"},
        )
        lr = layout(result)
        issues = check_inserter_chains(lr)
        assert len(issues) == 0, f"Unexpected issues: {issues}"

    def test_fluid_machines_have_inserters(self):
        """Fluid machines should have inserters for solid I/O."""
        result = solve(
            "plastic-bar",
            target_rate=10,
            available_inputs={"petroleum-gas", "coal"},
        )
        lr = layout(result)
        issues = check_inserter_chains(lr)
        assert len(issues) == 0, f"Unexpected issues: {issues}"


class TestInserterDirection:
    """Tests for inserter direction validation."""

    def test_inserter_facing_machine_ok(self):
        """Input inserter facing toward machine should pass."""
        lr = LayoutResult(
            entities=[
                # 3x3 machine at (0,0)
                PlacedEntity(name="assembling-machine-1", x=0, y=0, recipe="iron-gear-wheel"),
                # Inserter at (1,-1) facing SOUTH → drops into machine at (1,0)
                PlacedEntity(name="inserter", x=1, y=-1, direction=EntityDirection.SOUTH),
            ]
        )
        issues = check_inserter_direction(lr)
        errors = [i for i in issues if i.severity == "error"]
        assert len(errors) == 0

    def test_inserter_facing_away_from_machine_ok(self):
        """Output inserter facing away from machine should pass (pickup side touches machine)."""
        lr = LayoutResult(
            entities=[
                PlacedEntity(name="assembling-machine-1", x=0, y=0, recipe="iron-gear-wheel"),
                # Inserter at (1,-1) facing NORTH → picks from machine at (1,0)
                PlacedEntity(name="inserter", x=1, y=-1, direction=EntityDirection.NORTH),
            ]
        )
        issues = check_inserter_direction(lr)
        errors = [i for i in issues if i.severity == "error"]
        assert len(errors) == 0

    def test_inserter_facing_parallel_error(self):
        """Inserter facing parallel to machine border should error."""
        lr = LayoutResult(
            entities=[
                PlacedEntity(name="assembling-machine-1", x=0, y=0, recipe="iron-gear-wheel"),
                # Inserter at (1,-1) facing EAST → parallel to top border, neither side hits machine
                PlacedEntity(name="inserter", x=1, y=-1, direction=EntityDirection.EAST),
            ]
        )
        issues = check_inserter_direction(lr)
        errors = [i for i in issues if i.severity == "error"]
        assert len(errors) == 1
        assert errors[0].category == "inserter-direction"

    def test_inserter_not_near_machine_error(self):
        """Inserter far from any machine should error."""
        lr = LayoutResult(
            entities=[
                PlacedEntity(name="assembling-machine-1", x=0, y=0, recipe="iron-gear-wheel"),
                PlacedEntity(name="inserter", x=10, y=10, direction=EntityDirection.SOUTH),
            ]
        )
        issues = check_inserter_direction(lr)
        errors = [i for i in issues if i.severity == "error"]
        assert len(errors) == 1

    def test_valid_layout_passes(self):
        """A real valid layout should pass inserter direction check."""
        result = solve(
            "iron-gear-wheel",
            target_rate=5,
            available_inputs={"iron-plate"},
        )
        lr = layout(result)
        issues = check_inserter_direction(lr)
        errors = [i for i in issues if i.severity == "error"]
        assert len(errors) == 0, f"Unexpected errors: {errors}"


class TestBeltConnectivity:
    """Tests for belt-to-inserter-to-machine connectivity."""

    def test_inserter_with_belt_ok(self):
        """Machine with inserter adjacent to a belt should pass."""
        lr = LayoutResult(
            entities=[
                # 3x3 machine at (0,0)
                PlacedEntity(name="assembling-machine-1", x=0, y=0, recipe="iron-gear-wheel"),
                # Inserter on top border (center top = x=1, y=-1)
                PlacedEntity(
                    name="inserter",
                    x=1,
                    y=-1,
                    direction=EntityDirection.SOUTH,
                ),
                # Belt above the inserter
                PlacedEntity(
                    name="transport-belt",
                    x=1,
                    y=-2,
                    direction=EntityDirection.EAST,
                    carries="iron-plate",
                ),
                # Extend belt so it's not isolated
                PlacedEntity(
                    name="transport-belt",
                    x=2,
                    y=-2,
                    direction=EntityDirection.EAST,
                    carries="iron-plate",
                ),
            ]
        )
        issues = check_belt_connectivity(lr)
        errors = [i for i in issues if i.severity == "error"]
        assert len(errors) == 0

    def test_inserter_without_belt_error(self):
        """Machine with inserter but no adjacent belt should error."""
        lr = LayoutResult(
            entities=[
                PlacedEntity(name="assembling-machine-1", x=0, y=0, recipe="iron-gear-wheel"),
                # Inserter on top border but no belt anywhere
                PlacedEntity(
                    name="inserter",
                    x=1,
                    y=-1,
                    direction=EntityDirection.SOUTH,
                ),
            ]
        )
        issues = check_belt_connectivity(lr)
        errors = [i for i in issues if i.severity == "error"]
        assert len(errors) >= 1
        assert errors[0].category == "belt-connectivity"

    def test_isolated_single_belt_error(self):
        """Inserter touching a single isolated belt tile should error."""
        lr = LayoutResult(
            entities=[
                PlacedEntity(name="assembling-machine-1", x=0, y=0, recipe="iron-gear-wheel"),
                PlacedEntity(
                    name="inserter",
                    x=1,
                    y=-1,
                    direction=EntityDirection.SOUTH,
                ),
                # Single belt tile, not connected to anything
                PlacedEntity(
                    name="transport-belt",
                    x=1,
                    y=-2,
                    direction=EntityDirection.EAST,
                    carries="iron-plate",
                ),
            ]
        )
        issues = check_belt_connectivity(lr)
        errors = [i for i in issues if i.severity == "error"]
        assert len(errors) >= 1
        assert "isolated" in errors[0].message

    def test_no_belts_with_machines_error(self):
        """Machines needing belts but none in layout should error."""
        lr = LayoutResult(
            entities=[
                PlacedEntity(name="assembling-machine-1", x=0, y=0, recipe="iron-gear-wheel"),
            ]
        )
        issues = check_belt_connectivity(lr)
        errors = [i for i in issues if i.severity == "error"]
        assert len(errors) >= 1

    def test_valid_layout_passes(self):
        """A real valid layout should pass belt connectivity."""
        result = solve(
            "iron-gear-wheel",
            target_rate=5,
            available_inputs={"iron-plate"},
        )
        lr = layout(result)
        issues = check_belt_connectivity(lr, result)
        errors = [i for i in issues if i.severity == "error"]
        assert len(errors) == 0, f"Unexpected errors: {errors}"


class TestBeltFlowPath:
    """Tests for belt flow path reachability."""

    def test_connected_to_boundary_ok(self):
        """Belt network reaching layout boundary should pass."""
        lr = LayoutResult(
            entities=[
                PlacedEntity(name="assembling-machine-1", x=5, y=5, recipe="iron-gear-wheel"),
                # Input inserter dropping south into machine
                PlacedEntity(name="inserter", x=6, y=4, direction=EntityDirection.SOUTH),
                # Belt path from boundary (x=0) to inserter
                PlacedEntity(name="transport-belt", x=6, y=3, direction=EntityDirection.EAST),
                PlacedEntity(name="transport-belt", x=5, y=3, direction=EntityDirection.EAST),
                PlacedEntity(name="transport-belt", x=4, y=3, direction=EntityDirection.EAST),
                PlacedEntity(name="transport-belt", x=3, y=3, direction=EntityDirection.EAST),
            ]
        )
        issues = check_belt_flow_path(lr)
        errors = [i for i in issues if i.severity == "error"]
        assert len(errors) == 0

    def test_disconnected_input_error(self):
        """Input belt network not reaching any source should error in spaghetti mode."""
        lr = LayoutResult(
            entities=[
                PlacedEntity(name="assembling-machine-1", x=10, y=10, recipe="iron-gear-wheel"),
                # Input inserter dropping south into machine
                PlacedEntity(name="inserter", x=11, y=9, direction=EntityDirection.SOUTH),
                # Short belt not reaching boundary or other machine
                PlacedEntity(name="transport-belt", x=11, y=8, direction=EntityDirection.EAST),
                PlacedEntity(name="transport-belt", x=12, y=8, direction=EntityDirection.EAST),
                # Distant belts to push boundary far away
                PlacedEntity(name="transport-belt", x=0, y=0, direction=EntityDirection.EAST),
                PlacedEntity(name="transport-belt", x=30, y=30, direction=EntityDirection.EAST),
            ]
        )
        issues = check_belt_flow_path(lr, layout_style="spaghetti")
        errors = [i for i in issues if i.severity == "error"]
        assert len(errors) == 1
        assert errors[0].category == "belt-flow-path"

    def test_disconnected_input_warning_bus(self):
        """Disconnected input in bus mode should be warning, not error."""
        lr = LayoutResult(
            entities=[
                PlacedEntity(name="assembling-machine-1", x=10, y=10, recipe="iron-gear-wheel"),
                PlacedEntity(name="inserter", x=11, y=9, direction=EntityDirection.SOUTH),
                PlacedEntity(name="transport-belt", x=11, y=8, direction=EntityDirection.EAST),
                PlacedEntity(name="transport-belt", x=12, y=8, direction=EntityDirection.EAST),
                # Distant belts to push boundary far away
                PlacedEntity(name="transport-belt", x=0, y=0, direction=EntityDirection.EAST),
                PlacedEntity(name="transport-belt", x=30, y=30, direction=EntityDirection.EAST),
            ]
        )
        issues = check_belt_flow_path(lr, layout_style="bus")
        errors = [i for i in issues if i.severity == "error"]
        warnings = [i for i in issues if i.severity == "warning"]
        assert len(errors) == 0
        assert len(warnings) == 1

    def test_output_belts_not_flagged(self):
        """Output inserter belts should not be flagged even if disconnected."""
        lr = LayoutResult(
            entities=[
                PlacedEntity(name="assembling-machine-1", x=5, y=5, recipe="iron-gear-wheel"),
                # Input inserter (drops into machine) — with connected belt
                PlacedEntity(name="inserter", x=6, y=4, direction=EntityDirection.SOUTH),
                PlacedEntity(name="transport-belt", x=6, y=3, direction=EntityDirection.EAST),
                PlacedEntity(name="transport-belt", x=5, y=3, direction=EntityDirection.EAST),
                PlacedEntity(name="transport-belt", x=4, y=3, direction=EntityDirection.EAST),
                PlacedEntity(name="transport-belt", x=3, y=3, direction=EntityDirection.EAST),
                # Output inserter (picks from machine, drops north) — dead-end belt
                PlacedEntity(name="inserter", x=6, y=8, direction=EntityDirection.SOUTH),
                PlacedEntity(name="transport-belt", x=6, y=9, direction=EntityDirection.EAST),
                PlacedEntity(name="transport-belt", x=7, y=9, direction=EntityDirection.EAST),
            ]
        )
        issues = check_belt_flow_path(lr)
        errors = [i for i in issues if i.severity == "error"]
        assert len(errors) == 0


class TestBeltDirectionContinuity:
    """Tests for belt direction reversal detection."""

    def test_same_direction_ok(self):
        """Adjacent belts facing same direction should pass."""
        lr = LayoutResult(
            entities=[
                PlacedEntity(name="transport-belt", x=0, y=0, direction=EntityDirection.EAST),
                PlacedEntity(name="transport-belt", x=1, y=0, direction=EntityDirection.EAST),
            ]
        )
        issues = check_belt_direction_continuity(lr)
        assert len(issues) == 0

    def test_turn_ok(self):
        """90-degree turn should pass."""
        lr = LayoutResult(
            entities=[
                PlacedEntity(name="transport-belt", x=0, y=0, direction=EntityDirection.EAST),
                PlacedEntity(name="transport-belt", x=1, y=0, direction=EntityDirection.SOUTH),
            ]
        )
        issues = check_belt_direction_continuity(lr)
        assert len(issues) == 0

    def test_head_on_reversal_warning(self):
        """180-degree reversal on the same axis should warn."""
        lr = LayoutResult(
            entities=[
                # Two belts on the same row facing opposite directions (head-on)
                PlacedEntity(name="transport-belt", x=0, y=0, direction=EntityDirection.EAST),
                PlacedEntity(name="transport-belt", x=1, y=0, direction=EntityDirection.WEST),
            ]
        )
        issues = check_belt_direction_continuity(lr)
        warnings = [i for i in issues if i.severity == "warning"]
        assert len(warnings) == 1
        assert warnings[0].category == "belt-direction"

    def test_parallel_opposite_ok(self):
        """Side-by-side belts going opposite directions should not warn (common pattern)."""
        lr = LayoutResult(
            entities=[
                # Two belts in parallel (different rows), opposite directions
                PlacedEntity(name="transport-belt", x=0, y=0, direction=EntityDirection.EAST),
                PlacedEntity(name="transport-belt", x=0, y=1, direction=EntityDirection.WEST),
            ]
        )
        issues = check_belt_direction_continuity(lr)
        warnings = [i for i in issues if i.severity == "warning"]
        assert len(warnings) == 0


class TestBeltThroughput:
    """Tests for belt throughput/overlap detection."""

    def test_no_overlap_ok(self):
        """Single belt per tile should pass."""
        lr = LayoutResult(
            entities=[
                PlacedEntity(name="transport-belt", x=0, y=0, direction=EntityDirection.EAST),
                PlacedEntity(name="transport-belt", x=1, y=0, direction=EntityDirection.EAST),
            ]
        )
        issues = check_belt_throughput(lr)
        assert len(issues) == 0

    def test_overlapping_routes_warning(self):
        """Multiple belt entities at same tile should warn."""
        lr = LayoutResult(
            entities=[
                PlacedEntity(name="transport-belt", x=0, y=0, direction=EntityDirection.EAST),
                PlacedEntity(name="transport-belt", x=0, y=0, direction=EntityDirection.SOUTH),
            ]
        )
        issues = check_belt_throughput(lr)
        warnings = [i for i in issues if i.severity == "warning"]
        assert len(warnings) == 1
        assert warnings[0].category == "belt-throughput"
        assert "2 overlapping" in warnings[0].message


class TestPowerCoverage:
    def test_machines_powered(self):
        """All machines should be within range of a power pole."""
        result = solve(
            "electronic-circuit",
            target_rate=5,
            available_inputs={"iron-plate", "copper-plate"},
        )
        lr = layout(result)
        issues = check_power_coverage(lr)
        # Power coverage is a warning, not error
        errors = [i for i in issues if i.severity == "error"]
        assert len(errors) == 0


class TestBeltFlowReachability:
    """Tests for directional belt flow reachability."""

    def _machine_with_inserter_and_belts(self, belt_dirs, belt_x_range):
        """Helper: 3x3 machine at (3,0) with input inserter at (4,-1) picking from (4,-2).

        belt_dirs: list of (x, y, EntityDirection) for belt tiles.
        """
        entities = [
            PlacedEntity(name="assembling-machine-3", x=3, y=0, recipe="iron-gear-wheel"),
            PlacedEntity(name="inserter", x=4, y=-1, direction=EntityDirection.SOUTH),
        ]
        for x, y, d in belt_dirs:
            entities.append(PlacedEntity(name="transport-belt", x=x, y=y, direction=d))
        return LayoutResult(entities=entities)

    def test_straight_belt_input_ok(self):
        """East-facing belts from boundary to input inserter should pass."""
        sr = SolverResult(
            machines=[
                MachineSpec(
                    entity="assembling-machine-3",
                    recipe="iron-gear-wheel",
                    count=1,
                    inputs=[ItemFlow(item="iron-plate", rate=5.0)],
                    outputs=[ItemFlow(item="iron-gear-wheel", rate=2.5)],
                )
            ],
            external_inputs=[ItemFlow(item="iron-plate", rate=5.0)],
            external_outputs=[ItemFlow(item="iron-gear-wheel", rate=2.5)],
        )
        # Belts: (0,-2)→ (1,-2)→ (2,-2)→ (3,-2)→ (4,-2)→  — inserter picks from (4,-2)
        belt_dirs = [(x, -2, EntityDirection.EAST) for x in range(5)]
        lr = self._machine_with_inserter_and_belts(belt_dirs, range(5))
        issues = check_belt_flow_reachability(lr, sr)
        input_errors = [i for i in issues if "can't reach input" in i.message]
        assert not input_errors

    def test_reversed_belt_input_fails(self):
        """West-facing belts can't deliver items eastward to the machine."""
        sr = SolverResult(
            machines=[
                MachineSpec(
                    entity="assembling-machine-3",
                    recipe="iron-gear-wheel",
                    count=1,
                    inputs=[ItemFlow(item="iron-plate", rate=5.0)],
                    outputs=[ItemFlow(item="iron-gear-wheel", rate=2.5)],
                )
            ],
            external_inputs=[ItemFlow(item="iron-plate", rate=5.0)],
            external_outputs=[ItemFlow(item="iron-gear-wheel", rate=2.5)],
        )
        # Belts all face WEST — items flow away from the machine
        belt_dirs = [(x, -2, EntityDirection.WEST) for x in range(5)]
        lr = self._machine_with_inserter_and_belts(belt_dirs, range(5))
        issues = check_belt_flow_reachability(lr, sr)
        input_errors = [i for i in issues if "can't reach input" in i.message]
        assert len(input_errors) == 1

    def test_sideload_upstream_ok(self):
        """A south-facing belt sideloading onto an east-facing trunk is valid upstream."""
        sr = SolverResult(
            machines=[
                MachineSpec(
                    entity="assembling-machine-3",
                    recipe="iron-gear-wheel",
                    count=1,
                    inputs=[ItemFlow(item="iron-plate", rate=5.0)],
                    outputs=[ItemFlow(item="iron-gear-wheel", rate=2.5)],
                )
            ],
            external_inputs=[ItemFlow(item="iron-plate", rate=5.0)],
            external_outputs=[ItemFlow(item="iron-gear-wheel", rate=2.5)],
        )
        # Trunk: (2,-2)→ (3,-2)→ (4,-2)→  with sideload from (2,-3)↓ onto (2,-2)
        belt_dirs = [
            (2, -3, EntityDirection.SOUTH),  # sideload feeder
            (2, -2, EntityDirection.EAST),
            (3, -2, EntityDirection.EAST),
            (4, -2, EntityDirection.EAST),
        ]
        lr = self._machine_with_inserter_and_belts(belt_dirs, range(5))
        issues = check_belt_flow_reachability(lr, sr)
        input_errors = [i for i in issues if "can't reach input" in i.message]
        assert not input_errors

    def test_output_downstream_reaches_boundary(self):
        """Output belt facing toward boundary should pass downstream check."""
        sr = SolverResult(
            machines=[
                MachineSpec(
                    entity="assembling-machine-3",
                    recipe="iron-gear-wheel",
                    count=1,
                    inputs=[ItemFlow(item="iron-plate", rate=5.0)],
                    outputs=[ItemFlow(item="iron-gear-wheel", rate=2.5)],
                )
            ],
            external_inputs=[ItemFlow(item="iron-plate", rate=5.0)],
            external_outputs=[ItemFlow(item="iron-gear-wheel", rate=2.5)],
        )
        entities = [
            PlacedEntity(name="assembling-machine-3", x=3, y=0, recipe="iron-gear-wheel"),
            # Input inserter + belts (so input check passes)
            PlacedEntity(name="inserter", x=4, y=-1, direction=EntityDirection.SOUTH),
            *[PlacedEntity(name="transport-belt", x=x, y=-2, direction=EntityDirection.EAST) for x in range(5)],
            # Output inserter dropping onto belt going south toward boundary
            PlacedEntity(name="inserter", x=4, y=3, direction=EntityDirection.SOUTH),
            *[PlacedEntity(name="transport-belt", x=4, y=y, direction=EntityDirection.SOUTH) for y in range(4, 8)],
        ]
        lr = LayoutResult(entities=entities)
        issues = check_belt_flow_reachability(lr, sr)
        output_errors = [i for i in issues if "can't leave output" in i.message]
        assert not output_errors

    def test_output_dead_end_fails(self):
        """Output belt facing away from boundary (into machines) should fail."""
        sr = SolverResult(
            machines=[
                MachineSpec(
                    entity="assembling-machine-3",
                    recipe="iron-gear-wheel",
                    count=1,
                    inputs=[ItemFlow(item="iron-plate", rate=5.0)],
                    outputs=[ItemFlow(item="iron-gear-wheel", rate=2.5)],
                )
            ],
            external_inputs=[ItemFlow(item="iron-plate", rate=5.0)],
            external_outputs=[ItemFlow(item="iron-gear-wheel", rate=2.5)],
        )
        entities = [
            PlacedEntity(name="assembling-machine-3", x=3, y=0, recipe="iron-gear-wheel"),
            PlacedEntity(name="inserter", x=4, y=-1, direction=EntityDirection.SOUTH),
            *[PlacedEntity(name="transport-belt", x=x, y=-2, direction=EntityDirection.EAST) for x in range(5)],
            # Output inserter drops onto a NORTH-facing belt (goes toward machine, dead end)
            PlacedEntity(name="inserter", x=4, y=3, direction=EntityDirection.SOUTH),
            PlacedEntity(name="transport-belt", x=4, y=4, direction=EntityDirection.NORTH),
        ]
        lr = LayoutResult(entities=entities)
        issues = check_belt_flow_reachability(lr, sr)
        output_errors = [i for i in issues if "can't leave output" in i.message]
        assert len(output_errors) == 1


class TestLaneThroughput:
    """Tests for per-lane belt throughput simulation."""

    def _make_solver_result(self, rate=2.5):
        return SolverResult(
            machines=[
                MachineSpec(
                    entity="assembling-machine-3",
                    recipe="iron-gear-wheel",
                    count=1,
                    inputs=[ItemFlow(item="iron-plate", rate=5.0)],
                    outputs=[ItemFlow(item="iron-gear-wheel", rate=rate)],
                )
            ],
            external_inputs=[ItemFlow(item="iron-plate", rate=rate * 2)],
            external_outputs=[ItemFlow(item="iron-gear-wheel", rate=rate)],
        )

    def test_single_inserter_within_capacity(self):
        """One inserter at 2.5/s on yellow belt (7.5/s per lane) — should pass."""
        from src.validate import check_lane_throughput

        sr = self._make_solver_result(rate=2.5)
        entities = [
            PlacedEntity(name="assembling-machine-3", x=3, y=0, recipe="iron-gear-wheel"),
            # Output inserter on the left side of an east-facing belt
            PlacedEntity(name="inserter", x=4, y=3, direction=EntityDirection.SOUTH),
            PlacedEntity(name="transport-belt", x=4, y=4, direction=EntityDirection.EAST, carries="iron-gear-wheel"),
            PlacedEntity(name="transport-belt", x=5, y=4, direction=EntityDirection.EAST, carries="iron-gear-wheel"),
        ]
        lr = LayoutResult(entities=entities)
        issues = check_lane_throughput(lr, sr)
        assert not issues

    def test_same_side_inserters_overload(self):
        """Two inserters on the same side, combined rate > per-lane capacity."""
        from src.validate import check_lane_throughput

        sr = SolverResult(
            machines=[
                MachineSpec(
                    entity="assembling-machine-3",
                    recipe="iron-gear-wheel",
                    count=2,
                    inputs=[ItemFlow(item="iron-plate", rate=5.0)],
                    outputs=[ItemFlow(item="iron-gear-wheel", rate=5.0)],
                )
            ],
            external_inputs=[ItemFlow(item="iron-plate", rate=10.0)],
            external_outputs=[ItemFlow(item="iron-gear-wheel", rate=10.0)],
        )
        # Two machines both dropping onto the same belt from the same side (north)
        # Each at 5.0/s → 10.0/s on one lane, exceeding 7.5/s yellow belt lane cap
        entities = [
            PlacedEntity(name="assembling-machine-3", x=0, y=0, recipe="iron-gear-wheel"),
            PlacedEntity(name="assembling-machine-3", x=7, y=0, recipe="iron-gear-wheel"),
            # Both inserters on the north side of the east-facing belt
            PlacedEntity(name="inserter", x=1, y=3, direction=EntityDirection.SOUTH),
            PlacedEntity(name="transport-belt", x=1, y=4, direction=EntityDirection.EAST, carries="iron-gear-wheel"),
            PlacedEntity(name="inserter", x=8, y=3, direction=EntityDirection.SOUTH),
            PlacedEntity(name="transport-belt", x=8, y=4, direction=EntityDirection.EAST, carries="iron-gear-wheel"),
            # Connect them with east-facing belts
            *[
                PlacedEntity(name="transport-belt", x=x, y=4, direction=EntityDirection.EAST, carries="iron-gear-wheel")
                for x in range(2, 8)
            ],
            PlacedEntity(name="transport-belt", x=9, y=4, direction=EntityDirection.EAST, carries="iron-gear-wheel"),
        ]
        lr = LayoutResult(entities=entities)
        issues = check_lane_throughput(lr, sr)
        lane_errors = [i for i in issues if "lane" in i.category]
        assert len(lane_errors) > 0, "Expected lane overload errors"

    def test_opposite_side_inserters_ok(self):
        """Two inserters on opposite sides — rate splits across lanes."""
        from src.validate import check_lane_throughput

        sr = SolverResult(
            machines=[
                MachineSpec(
                    entity="assembling-machine-3",
                    recipe="iron-gear-wheel",
                    count=2,
                    inputs=[ItemFlow(item="iron-plate", rate=5.0)],
                    outputs=[ItemFlow(item="iron-gear-wheel", rate=5.0)],
                )
            ],
            external_inputs=[ItemFlow(item="iron-plate", rate=10.0)],
            external_outputs=[ItemFlow(item="iron-gear-wheel", rate=10.0)],
        )
        # Two inserters on opposite sides of the belt
        entities = [
            PlacedEntity(name="assembling-machine-3", x=0, y=0, recipe="iron-gear-wheel"),
            PlacedEntity(name="assembling-machine-3", x=7, y=6, recipe="iron-gear-wheel"),
            # First inserter from north (puts on right/far lane)
            PlacedEntity(name="inserter", x=1, y=3, direction=EntityDirection.SOUTH),
            PlacedEntity(name="transport-belt", x=1, y=4, direction=EntityDirection.EAST, carries="iron-gear-wheel"),
            # Second inserter from south (puts on left/far lane)
            PlacedEntity(name="inserter", x=8, y=5, direction=EntityDirection.NORTH),
            PlacedEntity(name="transport-belt", x=8, y=4, direction=EntityDirection.EAST, carries="iron-gear-wheel"),
            # Connect with east-facing belts
            *[
                PlacedEntity(name="transport-belt", x=x, y=4, direction=EntityDirection.EAST, carries="iron-gear-wheel")
                for x in range(2, 8)
            ],
            PlacedEntity(name="transport-belt", x=9, y=4, direction=EntityDirection.EAST, carries="iron-gear-wheel"),
        ]
        lr = LayoutResult(entities=entities)
        issues = check_lane_throughput(lr, sr)
        lane_errors = [i for i in issues if "lane" in i.category]
        assert not lane_errors, f"Unexpected lane errors: {[e.message for e in lane_errors]}"

    def test_no_solver_result_skips(self):
        """Without solver_result, returns empty."""
        from src.validate import check_lane_throughput

        lr = LayoutResult(entities=[])
        issues = check_lane_throughput(lr, None)
        assert not issues


class TestIntegration:
    """Integration tests: full validation on real layouts."""

    def test_electronic_circuit_validates(self):
        """Full validation on electronic-circuit layout should pass."""
        result = solve(
            "electronic-circuit",
            target_rate=5,
            available_inputs={"iron-plate", "copper-plate"},
        )
        lr = layout(result)
        # Should not raise
        warnings = validate(lr, result)
        errors = [w for w in warnings if w.severity == "error"]
        assert len(errors) == 0

    def test_plastic_bar_validates(self):
        """Full validation on plastic-bar (fluid) layout should pass."""
        result = solve(
            "plastic-bar",
            target_rate=10,
            available_inputs={"petroleum-gas", "coal"},
        )
        lr = layout(result)
        warnings = validate(lr, result)
        errors = [w for w in warnings if w.severity == "error"]
        assert len(errors) == 0

    def test_petroleum_gas_validates(self):
        """Full validation on petroleum-gas (refinery) layout should pass."""
        result = solve(
            "petroleum-gas",
            target_rate=10,
            available_inputs={"crude-oil"},
        )
        lr = layout(result)
        warnings = validate(lr, result)
        errors = [w for w in warnings if w.severity == "error"]
        assert len(errors) == 0

    def test_validation_error_raised(self):
        """ValidationError should be raised on critical issues."""
        import pytest

        # Create a layout with adjacent pipes carrying different fluids
        lr = LayoutResult(
            entities=[
                PlacedEntity(name="pipe", x=0, y=0, carries="water"),
                PlacedEntity(name="pipe", x=1, y=0, carries="crude-oil"),
            ]
        )
        with pytest.raises(ValidationError) as exc_info:
            validate(lr)
        assert len(exc_info.value.issues) > 0
        assert exc_info.value.issues[0].category == "pipe-isolation"
