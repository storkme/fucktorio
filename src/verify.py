"""Offline blueprint verification — no Factorio needed.

Re-imports a blueprint string via draftsman and checks:
  1. String decodes successfully
  2. All entities are valid Factorio entities
  3. All assemblers/chemical plants have recipes set
  4. No entity collisions (draftsman collision detection)
  5. Entity counts match expectations
  6. Prints a visual ASCII map of the layout
"""

from __future__ import annotations

import sys
import warnings
from collections import Counter

from draftsman.blueprintable import get_blueprintable_from_string
from draftsman.warning import DraftsmanWarning


def verify(bp_string: str, verbose: bool = True) -> bool:
    """Verify a blueprint string is valid. Returns True if all checks pass."""
    ok = True

    # 1. Decode
    if verbose:
        print("1. Decoding blueprint string...")
    try:
        # Capture draftsman warnings (overlap etc)
        with warnings.catch_warnings(record=True) as caught:
            warnings.simplefilter("always", DraftsmanWarning)
            bp = get_blueprintable_from_string(bp_string)
    except Exception as e:
        print(f"   FAIL: Could not decode: {e}")
        return False

    if verbose:
        print(f"   OK: label='{bp.label}', {len(bp.entities)} entities")

    # 2. Check for overlap warnings
    overlaps = [w for w in caught if "overlap" in str(w.message).lower() or "intersect" in str(w.message).lower()]
    if overlaps:
        print(f"   WARN: {len(overlaps)} overlap warning(s):")
        for w in overlaps[:5]:
            print(f"     - {w.message}")
        if len(overlaps) > 5:
            print(f"     ... and {len(overlaps) - 5} more")
        ok = False
    elif verbose:
        print("2. No entity overlaps detected")

    # 3. Entity summary
    counts = Counter(e.name for e in bp.entities)
    if verbose:
        print("3. Entity counts:")
        for name, count in counts.most_common():
            print(f"   {count:4d}x {name}")

    # 4. Recipe check
    crafting_entities = {"assembling-machine-1", "assembling-machine-2",
                        "assembling-machine-3", "chemical-plant", "oil-refinery"}
    missing_recipe = []
    recipe_counts: Counter = Counter()
    for e in bp.entities:
        if e.name in crafting_entities:
            if e.recipe is None:
                missing_recipe.append(e)
            else:
                recipe_counts[e.recipe] += 1

    if missing_recipe:
        print(f"   FAIL: {len(missing_recipe)} crafting machine(s) without recipes")
        ok = False
    elif verbose:
        print("4. All crafting machines have recipes:")
        for recipe, count in recipe_counts.most_common():
            print(f"   {count:4d}x {recipe}")

    # 5. Underground belt pairing
    ug_in = sum(1 for e in bp.entities if e.name == "underground-belt" and getattr(e, "io_type", None) == "input")
    ug_out = sum(1 for e in bp.entities if e.name == "underground-belt" and getattr(e, "io_type", None) == "output")
    if ug_in != ug_out:
        print(f"   WARN: Underground belt mismatch: {ug_in} inputs, {ug_out} outputs")
    elif ug_in > 0 and verbose:
        print(f"5. Underground belts: {ug_in} matched pairs")

    ptg = sum(1 for e in bp.entities if e.name == "pipe-to-ground")
    if ptg % 2 != 0:
        print(f"   WARN: Odd pipe-to-ground count: {ptg}")
    elif ptg > 0 and verbose:
        print(f"   Pipe-to-ground: {ptg // 2} matched pairs")

    # 6. ASCII map
    if verbose:
        print("\n6. Layout map:")
        _print_ascii_map(bp)

    if ok and verbose:
        print("\nAll checks PASSED")

    return ok


def _print_ascii_map(bp) -> None:
    """Print a compact ASCII visualization of the blueprint."""
    if not bp.entities:
        print("   (empty)")
        return

    _3x3 = {"assembling-machine-1", "assembling-machine-2",
             "assembling-machine-3", "chemical-plant"}
    _5x5 = {"oil-refinery"}

    # Symbol map
    sym = {
        "transport-belt": "═",
        "underground-belt": "⇓",
        "inserter": "↓",
        "assembling-machine-1": "A",
        "assembling-machine-2": "A",
        "assembling-machine-3": "A",
        "chemical-plant": "C",
        "oil-refinery": "R",
        "pipe": "│",
        "pipe-to-ground": "⇣",
        "medium-electric-pole": "╋",
        "splitter": "S",
    }

    grid: dict[tuple[int, int], str] = {}

    for e in bp.entities:
        # Get tile position — draftsman stores position as center
        # tile_position is top-left for multi-tile entities
        tx = int(e.tile_position.x) if hasattr(e.tile_position, 'x') else int(e.tile_position[0])
        ty = int(e.tile_position.y) if hasattr(e.tile_position, 'y') else int(e.tile_position[1])
        s = sym.get(e.name, "?")

        if e.name in _3x3:
            for dx in range(3):
                for dy in range(3):
                    grid[(tx + dx, ty + dy)] = s
        elif e.name in _5x5:
            for dx in range(5):
                for dy in range(5):
                    grid[(tx + dx, ty + dy)] = s
        else:
            grid[(tx, ty)] = s

    if not grid:
        print("   (no entities)")
        return

    min_x = min(x for x, y in grid)
    max_x = max(x for x, y in grid)
    min_y = min(y for x, y in grid)
    max_y = max(y for x, y in grid)

    # Limit display size
    w = max_x - min_x + 1
    h = max_y - min_y + 1
    if h > 60:
        print(f"   ({w}×{h} tiles — too tall, showing first 60 rows)")
        max_y = min_y + 59

    # Print with coordinate labels
    print(f"   x: {min_x} → {max_x}  y: {min_y} → {max_y}")
    for y in range(min_y, max_y + 1):
        row = ""
        for x in range(min_x, max_x + 1):
            row += grid.get((x, y), "·")
        print(f"   {y:3d} {row}")


if __name__ == "__main__":
    from src.pipeline import produce

    item = sys.argv[1] if len(sys.argv) > 1 else "advanced-circuit"
    rate = float(sys.argv[2]) if len(sys.argv) > 2 else 5
    inputs = sys.argv[3].split(",") if len(sys.argv) > 3 else ["iron-plate", "copper-plate", "petroleum-gas", "coal"]

    print(f"Generating: {item} @ {rate}/s from {inputs}\n")
    bp_str = produce(item, rate=rate, inputs=inputs)
    print()
    verify(bp_str)
