"""End-to-end pipeline: solve → layout → blueprint string."""

from __future__ import annotations

import math

from .blueprint import build_blueprint
from .solver import solve
from .spaghetti import spaghetti_layout
from .validate import ValidationError, validate


def produce(
    item: str,
    rate: float,
    inputs: list[str] | None = None,
    machine: str = "assembling-machine-3",
    label: str | None = None,
    visualize: bool = False,
    open_browser: bool = True,
    layout_engine: str = "spaghetti",
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

    # Print summary with dependency tree
    print(f"=== Solver: {item} @ {rate}/s ===")
    print()
    # Show production chain as a tree
    recipe_to_spec = {m.recipe: m for m in solver_result.machines}
    printed: set[str] = set()

    def _print_tree(recipe: str, depth: int = 0) -> None:
        if recipe in printed or recipe not in recipe_to_spec:
            return
        printed.add(recipe)
        m = recipe_to_spec[recipe]
        prefix = "  │ " * depth + "  ├─" if depth > 0 else "  "
        count_str = f"{m.count:.1f}" if m.count != math.ceil(m.count) else str(int(m.count))
        machine_label = (
            "chemical-plant"
            if m.entity == "chemical-plant"
            else (
                "oil-refinery"
                if m.entity == "oil-refinery"
                else ("electric-furnace" if m.entity == "electric-furnace" else "asm3")
            )
        )
        print(f"{prefix} {m.recipe}  ×{count_str} {machine_label}")
        # Show inputs — recurse into intermediate recipes, list externals inline
        for inp in m.inputs:
            if inp.item in recipe_to_spec and inp.item not in printed:
                _print_tree(inp.item, depth + 1)
            elif inp.item not in recipe_to_spec:
                inp_prefix = "  │ " * (depth + 1) + "  ← "
                fluid_tag = " (fluid)" if inp.is_fluid else ""
                print(f"{inp_prefix}{inp.item} @ {inp.rate * m.count:.1f}/s{fluid_tag}")

    # Start from the final product
    if solver_result.dependency_order:
        _print_tree(solver_result.dependency_order[-1])
    # Print any remaining recipes not reached from the root
    for m in solver_result.machines:
        _print_tree(m.recipe)

    print()
    ext_items = ", ".join(f"{f.item} @ {f.rate:.1f}/s" for f in solver_result.external_inputs)
    print(f"  External inputs: {ext_items}")

    # 2. Layout
    if layout_engine == "bus":
        from .bus import bus_layout

        layout_result = bus_layout(solver_result)
    else:
        layout_result = spaghetti_layout(solver_result)
    print(f"  Layout: {len(layout_result.entities)} entities, {layout_result.width}×{layout_result.height} tiles")

    # 3. Validate (log issues but continue with best-effort layout)
    try:
        warnings = validate(layout_result, solver_result)
    except ValidationError as e:
        warnings = e.issues
    if warnings:
        for w in warnings:
            print(f"  [{w.severity}] {w.message}")

    # 4. Blueprint
    if label is None:
        label = f"{rate}/s {item}"
    bp_string = build_blueprint(layout_result, label=label)
    print(f"  Blueprint: {len(bp_string)} chars")

    # 5. Generate HTML visualization if requested
    if visualize:
        from .visualize import visualize as viz

        viz(bp_string, solver_result=solver_result, open_browser=open_browser)

    return bp_string


if __name__ == "__main__":
    import sys

    use_html = "--html" in sys.argv
    bp = produce(
        "electronic-circuit",
        rate=30,
        inputs=["iron-plate", "copper-plate"],
        visualize=use_html,
        open_browser=False,
    )
    print(f"\n{bp}")
