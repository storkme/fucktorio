"""Shared pytest fixtures and CLI options."""

from __future__ import annotations

import shutil
from pathlib import Path

import pytest

VIZ_DIR = Path("test_viz")


def pytest_addoption(parser: pytest.Parser) -> None:
    parser.addoption(
        "--viz",
        action="store_true",
        default=False,
        help="Generate HTML visualizations for tests that produce blueprints",
    )


@pytest.fixture
def viz(request: pytest.FixtureRequest):
    """Fixture that returns a callable to save a blueprint visualization.

    Usage in a test:
        def test_something(viz):
            bp_str = produce(...)
            viz(bp_str, "my-test-name")       # optional solver_result kwarg
            viz(bp_str, solver_result=result)  # name auto-derived from test

    When --viz is not passed, the callable is a no-op (tests run at full speed).
    """
    enabled = request.config.getoption("--viz")

    def _save(
        bp_string: str,
        name: str | None = None,
        solver_result=None,
        production_graph=None,
        layout_result=None,
        layout_style="spaghetti",
    ):
        if not enabled:
            return
        if name is None:
            name = request.node.name
        # Sanitize filename
        safe_name = name.replace("/", "_").replace(" ", "_").replace(":", "_")
        out_dir = VIZ_DIR
        out_dir.mkdir(exist_ok=True)
        out_path = str(out_dir / f"{safe_name}.html")

        # Run validation to collect issues for display
        validation_issues = None
        lane_rates = None
        if layout_result is not None and solver_result is not None:
            from src.validate import ValidationError, compute_lane_rates, validate

            try:
                validation_issues = validate(layout_result, solver_result, layout_style=layout_style)
            except ValidationError as exc:
                validation_issues = exc.issues
            lane_rates = compute_lane_rates(layout_result, solver_result)

        from src.visualize import visualize

        visualize(
            bp_string,
            output_path=out_path,
            open_browser=False,
            solver_result=solver_result,
            production_graph=production_graph,
            validation_issues=validation_issues,
            layout_result=layout_result,
            lane_rates=lane_rates,
        )

    return _save


def pytest_configure(config: pytest.Config) -> None:
    """Clean up old viz output at start of run when --viz is used."""
    if config.getoption("--viz", default=False) and VIZ_DIR.exists():
        shutil.rmtree(VIZ_DIR)


def pytest_terminal_summary(terminalreporter, exitstatus, config) -> None:
    """Print viz output location after test run."""
    if config.getoption("--viz", default=False) and VIZ_DIR.exists():
        count = len(list(VIZ_DIR.glob("*.html")))
        if count:
            terminalreporter.write_sep("=", f"{count} HTML visualization(s) in {VIZ_DIR}/")


# ---------------------------------------------------------------------------
# Shared layout fixtures (session-scoped to avoid redundant evolutionary search)
# ---------------------------------------------------------------------------


@pytest.fixture(scope="session")
def iron_gear_solver_result():
    from src.solver import solve

    return solve("iron-gear-wheel", target_rate=10, available_inputs={"iron-plate"})


@pytest.fixture(scope="session")
def iron_gear_layout(iron_gear_solver_result):
    pytest.skip("Spaghetti layout is parked; search hangs on CI")
    from src.spaghetti import spaghetti_layout

    return spaghetti_layout(iron_gear_solver_result)


@pytest.fixture(scope="session")
def electronic_circuit_solver_result():
    from src.solver import solve

    return solve("electronic-circuit", target_rate=5, available_inputs={"iron-plate", "copper-plate"})


@pytest.fixture(scope="session")
def electronic_circuit_layout(electronic_circuit_solver_result):
    from src.spaghetti import spaghetti_layout

    return spaghetti_layout(electronic_circuit_solver_result)


def pytest_sessionfinish(session, exitstatus) -> None:
    """Generate visual showcase when --viz is used."""
    if session.config.getoption("--viz", default=False):
        VIZ_DIR.mkdir(exist_ok=True)
        from src.showcase import generate_showcase

        generate_showcase(
            output_path=str(VIZ_DIR / "showcase.html"),
            open_browser=False,
        )
