"""Debug script for bus layout inspection.

Usage:
    uv run python scripts/debug_bus_layout.py [recipe] [rate]

Prints:
- Solver result (machine counts, dependency order)
- Row spans from placer (what row lives at what y-coordinates)
- Validation issues (errors and warnings, with context)
- Belt network summary per item (column spans, trunk gaps)
- Entity dump for a specific region (pass --region x1 y1 x2 y2)

Examples:
    uv run python scripts/debug_bus_layout.py processing-unit 0.3
    uv run python scripts/debug_bus_layout.py electronic-circuit 5.0
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent.parent))

from src.bus.layout import bus_layout
from src.solver import solve
from src.validate import (
    ValidationError,
    check_belt_dead_ends,
    check_belt_flow_path,
    validate,
)


def main() -> None:
    parser = argparse.ArgumentParser(description="Debug a bus layout.")
    parser.add_argument("recipe", nargs="?", default="processing-unit")
    parser.add_argument("rate", nargs="?", type=float, default=0.3)
    parser.add_argument("--inputs", nargs="*", help="Available external inputs")
    parser.add_argument("--machine", default=None, help="Override machine entity")
    parser.add_argument("--belt", default=None, help="Max belt tier (e.g. transport-belt)")
    parser.add_argument("--item", default=None, help="Filter entity dump to this item/carries")
    parser.add_argument(
        "--region", nargs=4, type=int, metavar=("X1", "Y1", "X2", "Y2"), help="Dump all entities in this bounding box"
    )
    parser.add_argument("--row-at", type=int, metavar="Y", help="Show full row context at this y coordinate")
    args = parser.parse_args()

    # Default inputs for processing-unit-from-ores
    default_inputs = {"iron-ore", "copper-ore", "coal", "crude-oil", "water", "sulfur"}
    available_inputs = set(args.inputs) if args.inputs else default_inputs

    print(f"=== Solving {args.recipe} @ {args.rate}/s ===")
    print(f"Available inputs: {sorted(available_inputs)}")
    kwargs = {}
    if args.machine:
        kwargs["machine_entity"] = args.machine
    result = solve(args.recipe, args.rate, available_inputs=available_inputs, **kwargs)

    print(f"\n--- Machines ({len(result.machines)}) ---")
    for m in result.machines:
        solid_in = [f"{i.item}" for i in m.inputs if not i.is_fluid]
        fluid_in = [f"{i.item}(F)" for i in m.inputs if i.is_fluid]
        print(f"  {m.recipe:30s} x{m.count:5.1f}  {m.entity}  in=[{', '.join(solid_in + fluid_in)}]")

    print("\n--- Dependency order ---")
    print("  " + " → ".join(result.dependency_order))

    print("\n=== Building layout ===")
    layout_kwargs = {}
    if args.belt:
        layout_kwargs["max_belt_tier"] = args.belt
    layout = bus_layout(result, **layout_kwargs)
    print(f"Layout: {layout.width}w × {layout.height}h, {len(layout.entities)} entities")

    # Validation
    print("\n=== Validation ===")
    try:
        validate(layout, result, layout_style="bus")
        print("  validate(): clean (no exception)")
    except ValidationError as e:
        for i in e.issues:
            print(f"  [{i.severity}] {i.category}: {i.message}")

    # Belt dead-ends (our new check)
    dead_ends = check_belt_dead_ends(layout)
    if dead_ends:
        print(f"  Belt dead-ends ({len(dead_ends)}):")
        for i in dead_ends:
            print(f"    [{i.severity}] {i.message}")
    else:
        print("  Belt dead-ends: none ✓")

    # Belt flow path (all warnings including allowed ones)
    flow_issues = check_belt_flow_path(layout, result, layout_style="bus")
    print(f"  Belt flow path warnings: {len(flow_issues)}")
    for i in flow_issues:
        print(f"    {i.message[:100]}")

    # Belt network summary per item
    print("\n=== Belt networks by item ===")
    by_item: dict[str, list] = {}
    for e in layout.entities:
        if e.carries and "belt" in e.name:
            by_item.setdefault(e.carries, []).append(e)
    for item in sorted(by_item):
        ents = by_item[item]
        xs = [e.x for e in ents]
        ys = [e.y for e in ents]
        dirs = {(e.direction.name if e.direction else "-") for e in ents}
        types = {e.name for e in ents}
        print(
            f"  {item:30s}: {len(ents):4d} tiles  "
            f"x={min(xs):3d}..{max(xs):3d}  y={min(ys):4d}..{max(ys):4d}  dirs={dirs}"
        )

    # Machine listing
    print("\n=== Placed machines (sorted by y) ===")
    machines = sorted([e for e in layout.entities if e.recipe], key=lambda e: (e.y, e.x))
    for e in machines:
        print(f"  ({e.x:3d},{e.y:4d})  {e.name:25s}  {e.recipe}")

    # Region dump
    if args.region:
        x1, y1, x2, y2 = args.region
        print(f"\n=== Entity dump ({x1},{y1})..({x2},{y2}) ===")
        region = [e for e in layout.entities if x1 <= e.x <= x2 and y1 <= e.y <= y2]
        for e in sorted(region, key=lambda e: (e.y, e.x)):
            dir_name = e.direction.name if e.direction else "-"
            print(f"  ({e.x:3d},{e.y:4d})  {e.name:30s}  dir={dir_name:5s}  carries={e.carries}  recipe={e.recipe}")

    # Row-at dump
    if args.row_at is not None:
        y = args.row_at
        print(f"\n=== Full row at y={y} ===")
        row = sorted([e for e in layout.entities if e.y == y], key=lambda e: e.x)
        for e in row:
            dir_name = e.direction.name if e.direction else "-"
            print(f"  ({e.x:3d},{e.y:4d})  {e.name:30s}  dir={dir_name:5s}  carries={e.carries}  recipe={e.recipe}")

    # Item filter dump
    if args.item:
        print(f"\n=== All entities carrying '{args.item}' ===")
        filtered = [e for e in layout.entities if e.carries == args.item or e.recipe == args.item]
        by_y: dict[int, list] = {}
        for e in filtered:
            by_y.setdefault(e.y, []).append(e)
        for y in sorted(by_y):
            row = sorted(by_y[y], key=lambda e: e.x)
            xs = [e.x for e in row]
            dirs = {e.direction.name if e.direction else "-" for e in row}
            types = {e.name for e in row}
            print(f"  y={y:4d}: x={min(xs):3d}..{max(xs):3d}  {dirs}  {types}")


if __name__ == "__main__":
    main()
