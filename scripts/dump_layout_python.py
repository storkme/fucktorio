"""Print bus layout with same format as Rust dump_layout example, for diffing."""
from __future__ import annotations

import sys
from pathlib import Path
from collections import Counter, defaultdict

sys.path.insert(0, str(Path(__file__).parent.parent))

from src.bus.layout import bus_layout
from src.solver import solve

recipe = sys.argv[1] if len(sys.argv) > 1 else "iron-gear-wheel"
rate = float(sys.argv[2]) if len(sys.argv) > 2 else 10.0

print(f"=== {recipe} @ {rate:.2f}/s ===\n")

solver_result = solve(recipe, rate, ["iron-ore", "copper-ore", "coal", "crude-oil", "water", "sulfur"], "assembling-machine-3")

print(f"Solver: {len(solver_result.machines)} machines, {len(solver_result.external_inputs)} external inputs")
for m in solver_result.machines:
    import math
    print(f"  {m.recipe} × {math.ceil(m.count)} ({m.entity})")
print()

layout = bus_layout(solver_result)

print(f"Layout: {layout.width}×{layout.height} ({len(layout.entities)} entities)\n")

counts = Counter(e.name for e in layout.entities)
print("Entity counts:")
for name in sorted(counts):
    print(f"  {name:<28} {counts[name]}")
print()

belt_dirs = Counter()
for e in layout.entities:
    if "transport-belt" in e.name:
        belt_dirs[e.direction.name] += 1
if belt_dirs:
    print("Transport-belt directions:")
    for d, c in sorted(belt_dirs.items()):
        print(f"  {d:<6} {c}")
    print()

print("ASCII map:")
machines_3x3 = {"assembling-machine-1", "assembling-machine-2", "assembling-machine-3", "chemical-plant", "electric-furnace"}
machines_5x5 = {"oil-refinery"}

base_sym = {
    "transport-belt": "=", "fast-transport-belt": "=", "express-transport-belt": "=",
    "underground-belt": "U", "fast-underground-belt": "U", "express-underground-belt": "U",
    "inserter": "i", "fast-inserter": "i", "long-handed-inserter": "L",
    "pipe": "|", "pipe-to-ground": "P",
    "medium-electric-pole": "+",
    "splitter": "S", "fast-splitter": "S", "express-splitter": "S",
}

digits = "123456789abcdefghijklmnop"
recipe_to_sym = {}
def is_machine(n): return n in machines_3x3 or n in machines_5x5

for e in layout.entities:
    if is_machine(e.name) and e.recipe and e.recipe not in recipe_to_sym:
        idx = len(recipe_to_sym)
        recipe_to_sym[e.recipe] = digits[idx] if idx < len(digits) else "?"

grid = {}
for e in layout.entities:
    x, y = e.x, e.y
    if is_machine(e.name):
        sym = recipe_to_sym.get(e.recipe, "?")
        size = 5 if e.name in machines_5x5 else 3
        for dx in range(size):
            for dy in range(size):
                grid[(x+dx, y+dy)] = sym
    else:
        grid[(x, y)] = base_sym.get(e.name, "?")

if grid:
    min_x = min(x for x,_ in grid)
    max_x = max(x for x,_ in grid)
    min_y = min(y for _,y in grid)
    max_y = max(y for _,y in grid)
    print(f"   x: {min_x} → {max_x}  y: {min_y} → {max_y}")
    for y in range(min_y, max_y+1):
        print(f"{y:>4} ", end="")
        for x in range(min_x, max_x+1):
            print(grid.get((x,y), " "), end="")
        print()
    print()
    print("Legend:")
    for r, c in sorted(recipe_to_sym.items(), key=lambda x: x[1]):
        print(f"  {c} = {r}")
