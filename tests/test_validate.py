"""Tests for functional blueprint validation."""

from src.layout import layout
from src.models import LayoutResult, PlacedEntity
from src.solver import solve
from src.validate import (
    ValidationError,
    check_fluid_port_connectivity,
    check_inserter_chains,
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
