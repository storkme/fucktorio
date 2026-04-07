#!/usr/bin/env python3
"""Sync the Python balancer library to Rust.

Reads src/bus/balancer_library.py and regenerates
crates/core/src/bus/balancer_library.rs, keeping the header
(struct defs, impl blocks) intact and regenerating only the data section
+ registry function.

Usage:
    uv run python scripts/sync_balancer_to_rust.py
"""
from __future__ import annotations

import sys
from pathlib import Path

# ---------------------------------------------------------------------------
# Paths
# ---------------------------------------------------------------------------
REPO_ROOT = Path(__file__).resolve().parent.parent
PY_SRC = REPO_ROOT / "src" / "bus" / "balancer_library.py"
RS_OUT = REPO_ROOT / "crates" / "core" / "src" / "bus" / "balancer_library.rs"

# ---------------------------------------------------------------------------
# Import the Python library directly
# ---------------------------------------------------------------------------
sys.path.insert(0, str(REPO_ROOT))
from src.bus.balancer_library import BALANCER_TEMPLATES, BalancerTemplate, BalancerTemplateEntity  # noqa: E402


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def key_to_const(n: int, m: int) -> str:
    """Return the Rust static array name prefix, e.g. 'T_2_3'."""
    return f"T_{n}_{m}"


def format_entity(e: BalancerTemplateEntity) -> str:
    """Render one BalancerTemplateEntity as a Rust struct literal."""
    if e.io_type is None:
        io_type = "None"
    else:
        io_type = f'Some("{e.io_type}")'
    return (
        f'    BalancerTemplateEntity {{ '
        f'name: "{e.name}", x: {e.x}, y: {e.y}, '
        f'direction: {e.direction}, io_type: {io_type} }},'
    )


def format_tile(t: tuple[int, int]) -> str:
    return f"({t[0]}, {t[1]})"


def generate_data_section(templates: dict) -> str:
    """Generate all static arrays for every template."""
    lines: list[str] = []
    lines.append("// " + "-" * 75)
    lines.append("// Template data")
    lines.append("// " + "-" * 75)
    lines.append("")

    # Sort by key for deterministic output
    for (n, m), tmpl in sorted(templates.items()):
        prefix = key_to_const(n, m)

        # Entity array
        lines.append(f"static {prefix}_ENTITIES: &[BalancerTemplateEntity] = &[")
        for e in tmpl.entities:
            lines.append(format_entity(e))
        lines.append("];")

        # Input tiles
        tile_str = ", ".join(format_tile(t) for t in tmpl.input_tiles)
        lines.append(f"static {prefix}_INPUT: &[(i32, i32)] = &[{tile_str}];")

        # Output tiles
        tile_str = ", ".join(format_tile(t) for t in tmpl.output_tiles)
        lines.append(f"static {prefix}_OUTPUT: &[(i32, i32)] = &[{tile_str}];")

        lines.append("")

    return "\n".join(lines)


def generate_registry(templates: dict) -> str:
    """Generate the build_templates() function."""
    count = len(templates)
    lines: list[str] = []
    lines.append("// " + "-" * 75)
    lines.append("// Global registry")
    lines.append("// " + "-" * 75)
    lines.append("")
    lines.append("/// Lazily-initialised map from (n_inputs, n_outputs) to [`BalancerTemplate`].")
    lines.append("pub fn balancer_templates() -> &'static FxHashMap<(u32, u32), BalancerTemplate> {")
    lines.append("    static MAP: OnceLock<FxHashMap<(u32, u32), BalancerTemplate>> = OnceLock::new();")
    lines.append("    MAP.get_or_init(build_templates)")
    lines.append("}")
    lines.append("")
    lines.append(f"fn build_templates() -> FxHashMap<(u32, u32), BalancerTemplate> {{")
    lines.append(f"    let mut m = FxHashMap::with_capacity_and_hasher({count}, Default::default());")

    for (n, m), tmpl in sorted(templates.items()):
        prefix = key_to_const(n, m)
        # Escape backslashes in the blueprint string (none expected, but be safe)
        bp = tmpl.source_blueprint.replace('\\', '\\\\').replace('"', '\\"')
        lines.append("")
        lines.append(f"    m.insert(({n}, {m}), BalancerTemplate {{")
        lines.append(f"        n_inputs: {tmpl.n_inputs}, n_outputs: {tmpl.n_outputs}, width: {tmpl.width}, height: {tmpl.height},")
        lines.append(f"        entities: {prefix}_ENTITIES, input_tiles: {prefix}_INPUT, output_tiles: {prefix}_OUTPUT,")
        lines.append(f'        source_blueprint: "{bp}",')
        lines.append("    });")

    lines.append("")
    lines.append("    m")
    lines.append("}")

    return "\n".join(lines)


# ---------------------------------------------------------------------------
# Read existing Rust file and locate the header / test sections
# ---------------------------------------------------------------------------

HEADER_SENTINEL = "// Template data"
TESTS_SENTINEL = "#[cfg(test)]"


def split_rust_file(text: str) -> tuple[str, str]:
    """Return (header, tests_and_after).

    'header' ends just before the first '// Template data' comment line.
    'tests_and_after' starts at the '#[cfg(test)]' line.
    """
    header_end = text.find("// " + "-" * 75 + "\n// Template data")
    if header_end == -1:
        # Try with fewer dashes in case formatting differs
        for width in range(70, 80):
            sentinel = "// " + "-" * width + "\n// Template data"
            pos = text.find(sentinel)
            if pos != -1:
                header_end = pos
                break
    if header_end == -1:
        raise RuntimeError(
            "Could not find '// Template data' section in Rust file. "
            "Check the file format."
        )

    tests_start = text.find(f"\n{TESTS_SENTINEL}")
    if tests_start == -1:
        raise RuntimeError(
            f"Could not find '{TESTS_SENTINEL}' section in Rust file."
        )
    # Include the leading newline before #[cfg(test)] in the tail
    tests_section = text[tests_start:]

    return text[:header_end], tests_section


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main() -> None:
    print(f"Reading Python library from: {PY_SRC}")
    print(f"  Found {len(BALANCER_TEMPLATES)} templates: {sorted(BALANCER_TEMPLATES.keys())}")

    print(f"Reading existing Rust file from: {RS_OUT}")
    existing = RS_OUT.read_text(encoding="utf-8")

    header, tests_section = split_rust_file(existing)

    data_section = generate_data_section(BALANCER_TEMPLATES)
    registry_section = generate_registry(BALANCER_TEMPLATES)

    # Update the test that checks count
    # Find "assert_eq!(templates.len(), NN);" and replace with new count
    count = len(BALANCER_TEMPLATES)

    new_content = (
        header
        + data_section
        + "\n"
        + registry_section
        + "\n"
        + tests_section
    )

    # Fix the template count assertion in the tests
    import re
    new_content = re.sub(
        r'assert_eq!\(templates\.len\(\), \d+\)',
        f'assert_eq!(templates.len(), {count})',
        new_content,
    )

    RS_OUT.write_text(new_content, encoding="utf-8")
    print(f"Written {len(new_content)} bytes to: {RS_OUT}")
    print(f"  Templates: {count}")
    print("Done.")


if __name__ == "__main__":
    main()
