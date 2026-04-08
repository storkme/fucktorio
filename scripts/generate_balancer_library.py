"""Generate balancer templates using Factorio-SAT.

Invokes `belt_balancer` from external/factorio-sat for each (N, M) shape
we care about, converts each solution to a Factorio blueprint, extracts
entity positions, rotates 90° CW (SAT uses horizontal flow; the bus uses
vertical SOUTH flow), and emits `src/bus/balancer_library.py`.

Run manually:
    uv run python scripts/generate_balancer_library.py

This is an offline workflow: Factorio-SAT is NOT a runtime dependency.
The generated library ships in the repo.
"""

from __future__ import annotations

import base64
import importlib.util
import json
import os
import subprocess
import sys
import zlib
from concurrent.futures import ProcessPoolExecutor, as_completed
from dataclasses import dataclass
from pathlib import Path

SAT_DIR = Path(__file__).parent.parent / "external" / "factorio-sat"
SAT_PY = SAT_DIR / ".venv" / "bin" / "python"
OUT_PATH = Path(__file__).parent.parent / "src" / "bus" / "balancer_library.py"

# Factorio direction encoding (1.0 blueprint format):
#   0 = NORTH, 2 = EAST, 4 = SOUTH, 6 = WEST
FACTORIO_NORTH, FACTORIO_EAST, FACTORIO_SOUTH, FACTORIO_WEST = 0, 2, 4, 6

# Shapes to generate: (N inputs, M outputs)
# Cover all combinations up to 8×8 (except 1×1 identity).
SHAPES: list[tuple[int, int]] = [(n, m) for n in range(1, 9) for m in range(1, 9) if (n, m) != (1, 1)]


@dataclass
class RawEntity:
    """Entity as extracted from SAT's blueprint output (pre-rotation)."""

    name: str
    x: float  # tile-center in SAT's grid (horizontal flow, EAST = +x)
    y: float
    direction: int  # Factorio direction (0/2/4/6)
    io_type: str | None = None  # "input" or "output" for underground-belt


def run_sat(n: int, m: int, width: int, height: int, fast: bool = True, timeout: int = 300) -> str | None:
    """Run belt_balancer then blueprint encode.

    Returns the encoded blueprint string, or None if SAT fails / unsat.
    """
    network = SAT_DIR / "networks" / f"{n}x{m}"
    if not network.exists():
        raise FileNotFoundError(f"No network file for {n}x{m}: {network}")

    cmd = [str(SAT_PY), "-m", "factorio_sat.belt_balancer"]
    if fast:
        cmd.append("--fast")
    cmd.extend([str(network), str(width), str(height)])
    try:
        bb = subprocess.run(
            cmd,
            capture_output=True,
            cwd=str(SAT_DIR),
            timeout=timeout,
        )
    except subprocess.TimeoutExpired:
        print(f"    timeout after {timeout}s at {width}x{height} (fast={fast})", flush=True)
        return None
    if bb.returncode != 0 or not bb.stdout:
        return None
    enc = subprocess.run(
        [str(SAT_PY), "-m", "factorio_sat.blueprint", "encode"],
        input=bb.stdout,
        capture_output=True,
        cwd=str(SAT_DIR),
        timeout=30,
    )
    # blueprint encode reads stdin in a loop and always exits with EOFError;
    # what matters is that it produced stdout.
    if not enc.stdout:
        return None
    out = enc.stdout.decode().strip().splitlines()[0]
    if not out or not out.startswith("0"):
        return None
    return out


def find_balancer(n: int, m: int) -> tuple[str, int, int] | None:
    """Search for a compact balancer by increasing width.

    Returns (blueprint_string, width, height) for the first solution found.

    For small shapes (max(n,m) < 6), fast mode alone finds solutions
    reliably. For larger shapes, the full solver is tried at each width
    after fast mode times out, since CaDiCaL's heuristics sometimes
    struggle where the plain solver succeeds.
    """
    base_h = max(n, m)
    # Scale search space with shape complexity.
    if base_h >= 7:
        max_width = 40
        extra_heights = 4
        fast_timeout = 120
        use_full_interleave = True
    elif base_h >= 6:
        max_width = 30
        extra_heights = 3
        fast_timeout = 120
        use_full_interleave = True
    elif base_h >= 5:
        max_width = 25
        extra_heights = 3
        fast_timeout = 120
        use_full_interleave = False
    else:
        max_width = 21
        extra_heights = 2
        fast_timeout = 120
        use_full_interleave = False
    heights = [base_h + i for i in range(extra_heights + 1)]
    # Phase 1: fast probe, optionally interleaved with full solver.
    for height in heights:
        for width in range(3, max_width + 1):
            print(f"  probing {n}x{m} at {width}x{height} (fast)...", flush=True)
            bp = run_sat(n, m, width, height, fast=True, timeout=fast_timeout)
            if bp is not None:
                print(f"  -> solved at {width}x{height} (fast)")
                return bp, width, height
            if use_full_interleave and width >= base_h:
                print(f"  probing {n}x{m} at {width}x{height} (full)...", flush=True)
                bp = run_sat(n, m, width, height, fast=False, timeout=300)
                if bp is not None:
                    print(f"  -> solved at {width}x{height} (full)")
                    return bp, width, height
    # Phase 2: full solver sweep (only if not already interleaved).
    if not use_full_interleave:
        for height in heights:
            for width in range(3, max_width + 1):
                print(f"  probing {n}x{m} at {width}x{height} (full)...", flush=True)
                bp = run_sat(n, m, width, height, fast=False, timeout=300)
                if bp is not None:
                    print(f"  -> solved at {width}x{height} (full)")
                    return bp, width, height
    return None


def decode_blueprint(bp: str) -> dict:
    raw = zlib.decompress(base64.b64decode(bp[1:]))
    return json.loads(raw)


def extract_entities(bp: str) -> list[RawEntity]:
    data = decode_blueprint(bp)
    result = []
    for ent in data["blueprint"]["entities"]:
        result.append(
            RawEntity(
                name=ent["name"],
                x=ent["position"]["x"],
                y=ent["position"]["y"],
                direction=ent.get("direction", 0),
                io_type=ent.get("type"),
            )
        )
    return result


# 90° CW rotation: (x, y) -> (H - y, x), direction: N->E->S->W->N
# (a vector pointing up, after CW rotation, points right)
_DIR_ROTATE_CW = {
    FACTORIO_NORTH: FACTORIO_EAST,
    FACTORIO_EAST: FACTORIO_SOUTH,
    FACTORIO_SOUTH: FACTORIO_WEST,
    FACTORIO_WEST: FACTORIO_NORTH,
}


def rotate_cw(entities: list[RawEntity], grid_h: int) -> list[RawEntity]:
    """Rotate entities 90° clockwise around the grid origin.

    Original grid is width W, height H. After rotation, new grid is
    width H, height W. Coordinate transform: (x, y) -> (H - y, x).

    SAT generates horizontal-flow balancers (input WEST, output EAST).
    After 90° CW, they become vertical (input NORTH, output SOUTH) —
    what the bus needs.
    """
    out = []
    for e in entities:
        nx = grid_h - e.y
        ny = e.x
        out.append(
            RawEntity(
                name=e.name,
                x=nx,
                y=ny,
                direction=_DIR_ROTATE_CW[e.direction],
                io_type=e.io_type,
            )
        )
    return out


def normalize(entities: list[RawEntity]) -> tuple[list[RawEntity], float, float]:
    """Shift entities so min x/y are >= 0.5 (top-left tile)."""
    min_x = min(e.x for e in entities)
    min_y = min(e.y for e in entities)
    # Target: smallest belt tile should be at (0.5, 0.5).
    dx = 0.5 - min_x
    dy = 0.5 - min_y
    shifted = [RawEntity(e.name, e.x + dx, e.y + dy, e.direction, e.io_type) for e in entities]
    return shifted, dx, dy


def entity_tile(e: RawEntity) -> tuple[int, int]:
    """Top-left tile position for a belt/underground/splitter.

    Belts and underground-belts have center (tile_x + 0.5, tile_y + 0.5).
    Splitters are 2 tiles wide perpendicular to flow:
      - NORTH/SOUTH (direction 0/4): center at (tile_x + 1.0, tile_y + 0.5)
      - EAST/WEST (direction 2/6): center at (tile_x + 0.5, tile_y + 1.0)
    """
    if e.name == "splitter":
        if e.direction in (0, 4):  # NORTH/SOUTH
            return (int(round(e.x - 1.0)), int(round(e.y - 0.5)))
        else:  # EAST/WEST
            return (int(round(e.x - 0.5)), int(round(e.y - 1.0)))
    return (int(round(e.x - 0.5)), int(round(e.y - 0.5)))


def identify_ports(
    entities: list[RawEntity],
) -> tuple[list[tuple[int, int]], list[tuple[int, int]]]:
    """Find input (top-edge SOUTH belts) and output (bottom-edge SOUTH belts).

    Post-rotation, the bus's flow convention is SOUTH:
      - inputs are belts facing SOUTH at the topmost y (items enter here)
      - outputs are belts facing SOUTH at the bottommost y (items exit here)
    """
    belts = [e for e in entities if e.name == "transport-belt"]
    if not belts:
        return [], []
    belt_tiles = [(entity_tile(e), e) for e in belts]
    min_y = min(ty for (_, ty), _ in belt_tiles)
    max_y = max(ty for (_, ty), _ in belt_tiles)

    inputs = sorted((tx, ty) for (tx, ty), e in belt_tiles if ty == min_y and e.direction == FACTORIO_SOUTH)
    outputs = sorted((tx, ty) for (tx, ty), e in belt_tiles if ty == max_y and e.direction == FACTORIO_SOUTH)
    return inputs, outputs


def build_template(n: int, m: int) -> dict | None:
    print(f"Generating ({n},{m})...", flush=True)
    found = find_balancer(n, m)
    if found is None:
        print(f"  FAILED: no solution for ({n},{m})", file=sys.stderr)
        return None
    bp, width, height = found
    entities = extract_entities(bp)
    rotated = rotate_cw(entities, height)
    normalized, _, _ = normalize(rotated)
    inputs, outputs = identify_ports(normalized)

    # Compute bounding box from actual entity tiles (post-rotation).
    tiles = []
    for e in normalized:
        tx, ty = entity_tile(e)
        tiles.append((tx, ty))
        if e.name == "splitter":
            # add the second tile of the splitter
            if e.direction in (FACTORIO_NORTH, FACTORIO_SOUTH):
                tiles.append((tx + 1, ty))
            else:
                tiles.append((tx, ty + 1))
    tpl_width = max(tx for tx, _ in tiles) + 1
    tpl_height = max(ty for _, ty in tiles) + 1

    template = {
        "n_inputs": n,
        "n_outputs": m,
        "width": tpl_width,
        "height": tpl_height,
        "entities": [
            {
                "name": e.name,
                "x": entity_tile(e)[0],
                "y": entity_tile(e)[1],
                "direction": e.direction,
                "io_type": e.io_type,
            }
            for e in normalized
        ],
        "input_tiles": inputs,
        "output_tiles": outputs,
        "source_blueprint": bp,
    }
    print(f"  ports: inputs={inputs}, outputs={outputs}")
    print(f"  footprint: {template['width']}W x {template['height']}H")
    return template


def emit_library(templates: dict[tuple[int, int], dict]) -> None:
    lines = [
        '"""Pre-generated N-to-M balancer templates.',
        "",
        "DO NOT EDIT MANUALLY. Regenerate with:",
        "    uv run python scripts/generate_balancer_library.py",
        "",
        "Shapes are oriented for vertical SOUTH flow: inputs at the top",
        "(facing SOUTH), outputs at the bottom (facing SOUTH).",
        '"""',
        "from __future__ import annotations",
        "",
        "from dataclasses import dataclass",
        "",
        "",
        "@dataclass(frozen=True)",
        "class BalancerTemplateEntity:",
        "    name: str",
        "    x: int  # top-left tile (splitters span 2 tiles in their broad axis)",
        "    y: int",
        "    direction: int  # Factorio 1.0 direction (0=N, 2=E, 4=S, 6=W)",
        "    io_type: str | None = None  # 'input'/'output' for underground-belt",
        "",
        "",
        "@dataclass(frozen=True)",
        "class BalancerTemplate:",
        "    n_inputs: int",
        "    n_outputs: int",
        "    width: int",
        "    height: int",
        "    entities: tuple[BalancerTemplateEntity, ...]",
        "    input_tiles: tuple[tuple[int, int], ...]  # (dx, dy) relative",
        "    output_tiles: tuple[tuple[int, int], ...]",
        "    source_blueprint: str  # for debugging / regeneration",
        "",
        "",
        "BALANCER_TEMPLATES: dict[tuple[int, int], BalancerTemplate] = {",
    ]

    for (n, m), t in sorted(templates.items()):
        lines.append(f"    ({n}, {m}): BalancerTemplate(")
        lines.append(f"        n_inputs={t['n_inputs']},")
        lines.append(f"        n_outputs={t['n_outputs']},")
        lines.append(f"        width={t['width']},")
        lines.append(f"        height={t['height']},")
        lines.append("        entities=(")
        for e in t["entities"]:
            io_suffix = f', io_type="{e["io_type"]}"' if e["io_type"] is not None else ""
            lines.append(
                f'            BalancerTemplateEntity(name="{e["name"]}", '
                f"x={e['x']}, y={e['y']}, direction={e['direction']}{io_suffix}),"
            )
        lines.append("        ),")
        it = ", ".join(f"({x}, {y})" for x, y in t["input_tiles"])
        ot = ", ".join(f"({x}, {y})" for x, y in t["output_tiles"])
        lines.append(f"        input_tiles=({it}{',' if len(t['input_tiles']) == 1 else ''}),")
        lines.append(f"        output_tiles=({ot}{',' if len(t['output_tiles']) == 1 else ''}),")
        lines.append(f'        source_blueprint="{t["source_blueprint"]}",')
        lines.append("    ),")
    lines.append("}")
    lines.append("")
    OUT_PATH.write_text("\n".join(lines))
    print(f"Wrote {OUT_PATH} ({len(templates)} templates)")


def _build_shape(shape: tuple[int, int]) -> tuple[tuple[int, int], dict | None]:
    """Worker function for parallel generation. Returns (shape, template_or_None)."""
    return shape, build_template(*shape)


def _load_existing() -> set[tuple[int, int]]:
    """Load already-generated template keys from the library file."""
    if not OUT_PATH.exists():
        return set()
    try:
        # Import the existing library to get its keys.
        import importlib.util

        spec = importlib.util.spec_from_file_location("_bal_lib", OUT_PATH)
        if spec is None or spec.loader is None:
            return set()
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        return set(mod.BALANCER_TEMPLATES.keys())
    except Exception:
        return set()


def main() -> None:
    import argparse

    parser = argparse.ArgumentParser(description="Generate balancer templates via Factorio-SAT")
    parser.add_argument(
        "--skip-existing",
        action="store_true",
        help="Skip shapes already present in the library (incremental generation)",
    )
    args = parser.parse_args()

    if not SAT_PY.exists():
        print(f"Factorio-SAT venv not found at {SAT_PY}", file=sys.stderr)
        print("Set it up with:", file=sys.stderr)
        print(f"  cd {SAT_DIR}", file=sys.stderr)
        print("  uv venv .venv --python 3.12", file=sys.stderr)
        print("  .venv/bin/python -m ensurepip --upgrade", file=sys.stderr)
        print("  .venv/bin/python -m pip install --editable .", file=sys.stderr)
        sys.exit(1)

    # When --skip-existing, seed `templates` from the library and only
    # generate shapes that are missing.
    existing_keys: set[tuple[int, int]] = set()
    templates: dict[tuple[int, int], dict] = {}
    if args.skip_existing:
        existing_keys = _load_existing()
        # Re-read the existing library to preserve its data for emit.
        if existing_keys:
            spec = importlib.util.spec_from_file_location("_bal_lib", OUT_PATH)
            mod = importlib.util.module_from_spec(spec)  # type: ignore[arg-type]
            spec.loader.exec_module(mod)  # type: ignore[union-attr]
            for key, tmpl in mod.BALANCER_TEMPLATES.items():
                templates[key] = {
                    "n_inputs": tmpl.n_inputs,
                    "n_outputs": tmpl.n_outputs,
                    "width": tmpl.width,
                    "height": tmpl.height,
                    "entities": [
                        {"name": e.name, "x": e.x, "y": e.y, "direction": e.direction, "io_type": e.io_type}
                        for e in tmpl.entities
                    ],
                    "input_tiles": list(tmpl.input_tiles),
                    "output_tiles": list(tmpl.output_tiles),
                    "source_blueprint": tmpl.source_blueprint,
                }
        print(f"Loaded {len(existing_keys)} existing templates, generating missing shapes", flush=True)

    todo = [s for s in SHAPES if s not in existing_keys]
    if not todo:
        print("All shapes already generated!")
        return

    workers = int(os.environ.get("BALANCER_WORKERS", "0")) or min(os.cpu_count() or 1, len(todo))

    if workers <= 1:
        for shape in todo:
            t = build_template(*shape)
            if t is not None:
                templates[shape] = t
    else:
        print(f"Parallel mode: {workers} workers for {len(todo)} shapes", flush=True)
        with ProcessPoolExecutor(max_workers=workers) as pool:
            futures = {pool.submit(_build_shape, s): s for s in todo}
            for future in as_completed(futures):
                shape = futures[future]
                try:
                    _, t = future.result()
                    if t is not None:
                        templates[shape] = t
                    else:
                        print(f"  SKIPPED: no solution for {shape}", file=sys.stderr)
                except Exception as exc:
                    print(f"  ERROR: {shape} raised {exc}", file=sys.stderr)

    emit_library(templates)
    print(f"Generated {len(templates)}/{len(SHAPES)} templates")
    missing = [s for s in SHAPES if s not in templates]
    if missing:
        print(f"Missing ({len(missing)}): {missing}")


if __name__ == "__main__":
    main()
