"""End-to-end pipeline: solve → layout → blueprint string."""

from __future__ import annotations

import math

from .solver import solve
from .layout import layout
from .blueprint import build_blueprint
from .models import SolverResult, LayoutResult


def produce(
    item: str,
    rate: float,
    inputs: list[str] | None = None,
    machine: str = "assembling-machine-3",
    label: str | None = None,
) -> str:
    """One-call interface: item + rate → Factorio blueprint string.

    Args:
        item: Target item name (e.g. "electronic-circuit").
        rate: Desired production rate in items/sec.
        inputs: Items the user supplies externally. If None, raw ores are used.
        machine: Assembler entity to use.
        label: Blueprint label. Defaults to "{rate}/s {item}".

    Returns:
        Factorio-importable blueprint string.
    """
    available = set(inputs) if inputs else set()

    # 1. Solve
    solver_result = solve(item, rate, available, machine)

    # Print summary
    print(f"=== Solver: {item} @ {rate}/s ===")
    for m in solver_result.machines:
        print(f"  {m.recipe}: {m.count:.2f} machines → {math.ceil(m.count)} placed")
    print(f"  External inputs: {', '.join(f'{f.item} @ {f.rate:.1f}/s' for f in solver_result.external_inputs)}")

    # 2. Layout
    layout_result = layout(solver_result)
    print(f"  Layout: {len(layout_result.entities)} entities, {layout_result.width}×{layout_result.height} tiles")

    # 3. Blueprint
    if label is None:
        label = f"{rate}/s {item}"
    bp_string = build_blueprint(layout_result, label=label)
    print(f"  Blueprint: {len(bp_string)} chars")

    return bp_string


if __name__ == "__main__":
    bp = produce("electronic-circuit", rate=30, inputs=["iron-plate", "copper-plate"])
    print(f"\n{bp}")
