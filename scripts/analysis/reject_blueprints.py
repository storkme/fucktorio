"""Move old-format and mod blueprints to a rejected/ subfolder."""

from __future__ import annotations

import base64
import json
import shutil
import zlib
from pathlib import Path

# Old pre-rename Factorio recipe names (0.15–1.x era, removed/renamed in 2.0)
OLD_RECIPES = {
    # Science packs (renamed in 1.0)
    "science-pack-1", "science-pack-2", "science-pack-3",
    "high-tech-science-pack",
    # Removed in 2.0
    "rocket-control-unit",
    # Modules (renamed in 1.0)
    "effectivity-module", "effectivity-module-2", "effectivity-module-3",
    # Inserters (renamed in 2.0)
    "filter-inserter", "stack-filter-inserter",
    # Logistics chests (renamed in 2.0)
    "logistic-chest-active-provider", "logistic-chest-passive-provider",
    "logistic-chest-requester", "logistic-chest-storage",
    # Barrels (old style)
    "fill-sulfuric-acid-barrel", "fill-mineral-water-barrel",
    "empty-sulfuric-acid-barrel", "empty-mineral-water-barrel", "empty-barrel",
    # Other old names
    "steel-gear-wheel",
}

# Mod recipe name patterns (Krastorio 2, etc.)
MOD_PREFIXES = ("kr-",)

# Standalone mod recipe names
MOD_RECIPES = {
    "steel-beam", "iron-beam",
    "automation-core", "ai-core",
    "nitric-acid", "ammonia",
    "mining-drone", "mining-depot",
    "glass", "biusart-lab",
}


def decode_blueprint(bp_string: str) -> dict | str | None:
    """Returns parsed JSON dict, 'lua' for old Lua-format blueprints, or None on failure."""
    try:
        s = bp_string
        if s.startswith("0"):
            s = s[1:]
        raw = base64.b64decode(s + "==")
        # Try standard zlib (modern format)
        try:
            data = zlib.decompress(raw)
            return json.loads(data)
        except Exception:
            pass
        # Try gzip/zlib with auto-detect (wbits=47) — old factorio format
        try:
            data = zlib.decompress(raw, 47)
            # If it starts with 'do local' it's Lua, not JSON
            if data[:2] == b"do" or data[:5] == b"do lo":
                return "lua"
            return json.loads(data)
        except Exception:
            pass
        return None
    except Exception:
        return None


def extract_bp_string(data: dict) -> str | None:
    for key in ("blueprintString", "blueprint_string", "blueprint-string", "string"):
        if key in data:
            return data[key]
    if isinstance(data.get("data"), str):
        return data["data"]
    return None


def get_recipes(bp_data: dict) -> set[str]:
    """Get all recipe names from a blueprint or blueprint book (recursively)."""
    recipes: set[str] = set()
    # Direct blueprint
    for entity in bp_data.get("blueprint", {}).get("entities", []):
        if entity.get("recipe"):
            recipes.add(entity["recipe"])
    # Blueprint book — iterate nested blueprints
    for item in bp_data.get("blueprint_book", {}).get("blueprints", []):
        for entity in item.get("blueprint", {}).get("entities", []):
            if entity.get("recipe"):
                recipes.add(entity["recipe"])
    return recipes


def is_reject(recipes: set[str]) -> tuple[bool, str]:
    for r in recipes:
        if r in OLD_RECIPES:
            return True, f"old recipe: {r}"
        if any(r.startswith(p) for p in MOD_PREFIXES):
            return True, f"mod recipe: {r}"
        if r in MOD_RECIPES:
            return True, f"mod recipe: {r}"
    return False, ""


def main() -> None:
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("blueprints_dir", type=Path)
    parser.add_argument("--dry-run", action="store_true", help="Print moves without executing")
    args = parser.parse_args()

    blueprints_dir: Path = args.blueprints_dir
    rejected_dir = blueprints_dir / "rejected"

    if not args.dry_run:
        rejected_dir.mkdir(exist_ok=True)

    moved = 0
    kept = 0
    errors = 0

    for path in sorted(blueprints_dir.glob("*.json")):
        try:
            raw = json.loads(path.read_text())
        except Exception as e:
            print(f"  ERROR reading {path.name}: {e}")
            errors += 1
            continue

        bp_string = extract_bp_string(raw)
        if not bp_string:
            print(f"  SKIP {path.name}: no blueprint string found")
            errors += 1
            continue

        bp_data = decode_blueprint(bp_string)
        if bp_data is None:
            print(f"  SKIP {path.name}: failed to decode")
            errors += 1
            continue
        if bp_data == "lua":
            reject, reason = True, "old Lua format (pre-0.14)"
        else:
            recipes = get_recipes(bp_data)
            reject, reason = is_reject(recipes)

        if reject:
            print(f"  REJECT {path.name}  ({reason})")
            if not args.dry_run:
                shutil.move(str(path), str(rejected_dir / path.name))
            moved += 1
        else:
            kept += 1

    print(f"\nDone: {moved} moved to rejected/, {kept} kept, {errors} errors")
    if args.dry_run:
        print("(dry run — no files moved)")


if __name__ == "__main__":
    main()
