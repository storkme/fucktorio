"""Smoke test for the spaghetti layout engine.

The spaghetti engine is parked — see ROADMAP.md and issue #62. This file
keeps a single sanity check so the engine doesn't bitrot silently, but the
full test suite has been removed to keep CI fast.
"""

import pytest

from src.solver import solve
from src.spaghetti.layout import spaghetti_layout


@pytest.fixture(scope="module")
def iron_gear_2s_layout():
    sr = solve("iron-gear-wheel", target_rate=2, available_inputs={"iron-plate"})
    return spaghetti_layout(sr)


def test_spaghetti_iron_gear_produces_layout(iron_gear_2s_layout):
    """Spaghetti engine can produce a basic layout without crashing."""
    assert len(iron_gear_2s_layout.entities) > 0
    assert iron_gear_2s_layout.width > 0
    assert iron_gear_2s_layout.height > 0
    machines = [e for e in iron_gear_2s_layout.entities if e.name == "assembling-machine-3"]
    assert len(machines) > 0
