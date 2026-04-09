"""Generate balancer templates using Factorio-SAT.

Invokes `belt_balancer` from external/factorio-sat for each (N, M) shape
we care about, converts each solution to a Factorio blueprint, extracts
entity positions, rotates 90° CW (SAT uses horizontal flow; the bus uses
vertical SOUTH flow), and emits `src/bus/balancer_library.py`.

Run manually:
    uv run python scripts/generate_balancer_library.py

Incremental / resumable:
    uv run python scripts/generate_balancer_library.py --skip-existing

Limit difficulty (max(N,M) <= K):
    uv run python scripts/generate_balancer_library.py --skip-existing --max-tier 6

This is an offline workflow: Factorio-SAT is NOT a runtime dependency.
The generated library ships in the repo.

Symmetry optimisation
---------------------
A (N,M) balancer reversed is a valid (M,N) balancer: flip all entity
positions 180° ((x,y) → (W-1-x, H-1-y)), keep directions unchanged, swap
underground-belt input↔output.  The generator exploits this automatically:
  - On startup, derives missing (M,N) templates from already-solved (N,M).
  - After each SAT solve, immediately derives the reverse if not yet present.
This roughly halves the number of SAT calls required.

Checkpointing
-------------
After every successful solve, the new templates are atomically written to
`scripts/balancer_checkpoint.json`.  If the run is interrupted, restarting
with --skip-existing will reload both the library file and the checkpoint so
no work is lost.  The checkpoint is deleted on clean completion.
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
CHECKPOINT_PATH = Path(__file__).parent / "balancer_checkpoint.json"
SYNC_SCRIPT = Path(__file__).parent / "sync_balancer_to_rust.py"

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


def derive_reverse(n: int, m: int, template: dict) -> dict:
    """Derive a (m, n) template from a solved (n, m) template.

    A belt balancer is reversible: running it backwards (items enter from
    the output side and exit from the input side) produces a valid balancer
    for the transposed shape.

    Transformation:
      - Position: (x, y) → (W-1-x, H-1-y) for belts and underground-belts.
        Splitters span 2 tiles, so their top-left shifts differently:
          NORTH/SOUTH splitter at (x, y): new top-left = (W-2-x, H-1-y)
          EAST/WEST  splitter at (x, y): new top-left = (W-1-x, H-2-y)
      - Directions: unchanged (180° position flip + direction flip cancel out).
      - Underground-belt io_type: input ↔ output (flow direction reversed).
      - input_tiles ↔ output_tiles, each with the position transform applied.

    The source_blueprint field is inherited from the original — it refers to
    the forward direction and is kept only as a debugging reference.
    """
    W, H = template["width"], template["height"]
    new_entities = []
    for e in template["entities"]:
        new_e = dict(e)
        if e["name"] == "splitter":
            if e["direction"] in (FACTORIO_NORTH, FACTORIO_SOUTH):
                # 2 tiles wide: top-left (x,y) + (x+1,y) → mirror left = W-2-x
                new_e["x"] = W - 2 - e["x"]
                new_e["y"] = H - 1 - e["y"]
            else:
                # 2 tiles tall: top-left (x,y) + (x,y+1) → mirror top = H-2-y
                new_e["x"] = W - 1 - e["x"]
                new_e["y"] = H - 2 - e["y"]
        else:
            new_e["x"] = W - 1 - e["x"]
            new_e["y"] = H - 1 - e["y"]
        if e["name"] == "underground-belt" and e.get("io_type"):
            new_e["io_type"] = "output" if e["io_type"] == "input" else "input"
        new_entities.append(new_e)

    # Old output_tiles (at y=H-1) map to new input_tiles (at y=0).
    # Old input_tiles (at y=0) map to new output_tiles (at y=H-1).
    new_inputs = sorted((W - 1 - x, H - 1 - y) for x, y in template["output_tiles"])
    new_outputs = sorted((W - 1 - x, H - 1 - y) for x, y in template["input_tiles"])

    return {
        "n_inputs": m,
        "n_outputs": n,
        "width": W,
        "height": H,
        "entities": new_entities,
        "input_tiles": new_inputs,
        "output_tiles": new_outputs,
        "source_blueprint": template["source_blueprint"],
    }


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
        spec = importlib.util.spec_from_file_location("_bal_lib", OUT_PATH)
        if spec is None or spec.loader is None:
            return set()
        mod = importlib.util.module_from_spec(spec)
        sys.modules["_bal_lib"] = mod  # needed for dataclass on Python 3.13+
        spec.loader.exec_module(mod)
        return set(mod.BALANCER_TEMPLATES.keys())
    except Exception:
        return set()


def _load_templates_from_library() -> dict[tuple[int, int], dict]:
    """Load all template data from the existing library file."""
    if not OUT_PATH.exists():
        return {}
    try:
        spec = importlib.util.spec_from_file_location("_bal_lib2", OUT_PATH)
        if spec is None or spec.loader is None:
            return {}
        mod = importlib.util.module_from_spec(spec)
        sys.modules["_bal_lib2"] = mod
        spec.loader.exec_module(mod)
        result = {}
        for key, tmpl in mod.BALANCER_TEMPLATES.items():
            result[key] = {
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
        return result
    except Exception as exc:
        print(f"Warning: could not load library: {exc}", file=sys.stderr)
        return {}


def _save_checkpoint(templates: dict[tuple[int, int], dict]) -> None:
    """Atomically write current templates to the checkpoint file."""
    data = {f"{n},{m}": t for (n, m), t in templates.items()}
    tmp = CHECKPOINT_PATH.with_suffix(".json.tmp")
    tmp.write_text(json.dumps(data, indent=2))
    tmp.replace(CHECKPOINT_PATH)


def _load_checkpoint() -> dict[tuple[int, int], dict]:
    """Load templates from checkpoint file, if it exists."""
    if not CHECKPOINT_PATH.exists():
        return {}
    try:
        data = json.loads(CHECKPOINT_PATH.read_text())
        result = {}
        for key_str, t in data.items():
            n, m = map(int, key_str.split(","))
            result[(n, m)] = t
        print(f"Loaded {len(result)} templates from checkpoint {CHECKPOINT_PATH}", flush=True)
        return result
    except Exception as exc:
        print(f"Warning: could not load checkpoint: {exc}", file=sys.stderr)
        return {}


def _apply_free_derivations(
    templates: dict[tuple[int, int], dict],
    target_shapes: list[tuple[int, int]],
) -> int:
    """Derive any missing (M,N) templates from already-solved (N,M) ones.

    Returns the number of templates derived.
    """
    derived = 0
    target_set = set(target_shapes)
    # Iterate over a snapshot so we can extend templates while iterating.
    for (n, m), t in list(templates.items()):
        rev = (m, n)
        if rev != (n, m) and rev not in templates and rev in target_set:
            templates[rev] = derive_reverse(n, m, t)
            print(f"  Derived ({m},{n}) from ({n},{m}) [free reversal]", flush=True)
            derived += 1
    return derived


def main() -> None:
    import argparse

    parser = argparse.ArgumentParser(description="Generate balancer templates via Factorio-SAT")
    parser.add_argument(
        "--skip-existing",
        action="store_true",
        help="Skip shapes already present in the library (incremental generation)",
    )
    parser.add_argument(
        "--max-tier",
        type=int,
        default=0,
        metavar="K",
        help="Only attempt shapes where max(N,M) <= K (0 = no limit)",
    )
    parser.add_argument(
        "--derive-reversals",
        action="store_true",
        help=(
            "Derive missing (M,N) templates from solved (N,M) via flow-reversal transform. "
            "EXPERIMENTAL: verified correct for 8/10 symmetric pairs in the current library; "
            "2 pairs ((1,5) and (2,5)) produce geometrically different (possibly valid) results. "
            "Do not use for the main library until all derived templates are tested in-game."
        ),
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

    # Determine target shape list (respecting --max-tier).
    target_shapes = SHAPES
    if args.max_tier > 0:
        target_shapes = [s for s in SHAPES if max(s) <= args.max_tier]
        print(f"--max-tier {args.max_tier}: targeting {len(target_shapes)} shapes")

    # Load existing templates.
    templates: dict[tuple[int, int], dict] = {}
    if args.skip_existing:
        templates = _load_templates_from_library()
        checkpoint = _load_checkpoint()
        new_from_checkpoint = {k: v for k, v in checkpoint.items() if k not in templates}
        if new_from_checkpoint:
            templates.update(new_from_checkpoint)
            print(f"Merged {len(new_from_checkpoint)} new template(s) from checkpoint", flush=True)
        print(f"Loaded {len(templates)} existing templates, generating missing shapes", flush=True)

    # Exploit symmetry: derive free reversals from what we already have.
    if args.derive_reversals:
        derived_count = _apply_free_derivations(templates, target_shapes)
        if derived_count:
            print(f"Derived {derived_count} template(s) for free via reversal (EXPERIMENTAL)", flush=True)

    # Build todo list, sorted by difficulty: (max(N,M), N+M) ascending.
    # Easier shapes first so workers find solutions before tackling hard ones.
    todo = [s for s in target_shapes if s not in templates]
    todo.sort(key=lambda s: (max(s), sum(s)))
    if not todo:
        print("All shapes already generated!")
        _maybe_sync()
        return

    print(f"{len(todo)} shapes to generate: {todo}", flush=True)

    workers = int(os.environ.get("BALANCER_WORKERS", "0")) or min(os.cpu_count() or 1, len(todo))

    if workers <= 1:
        for shape in todo:
            n, m = shape
            if shape in templates:
                continue
            t = build_template(n, m)
            if t is not None:
                templates[shape] = t
                _save_checkpoint(templates)
                # Immediately derive the reverse if it's a target and not yet present.
                if args.derive_reversals:
                    rev = (m, n)
                    if rev != shape and rev not in templates and rev in set(target_shapes):
                        templates[rev] = derive_reverse(n, m, t)
                        print(f"  Derived ({m},{n}) from ({n},{m}) [free reversal] (EXPERIMENTAL)", flush=True)
                        _save_checkpoint(templates)
    else:
        print(f"Parallel mode: {workers} workers for {len(todo)} shapes", flush=True)
        # Only submit shapes that still need SAT — skip ones whose reverse
        # might be solved by an earlier future completing first.
        # We eagerly derive reverses as futures complete, so re-check before submitting.
        submitted: set[tuple[int, int]] = set()
        with ProcessPoolExecutor(max_workers=workers) as pool:
            futures = {}
            for s in todo:
                if s in templates:
                    continue
                submitted.add(s)
                futures[pool.submit(_build_shape, s)] = s

            for future in as_completed(futures):
                shape = futures[future]
                n, m = shape
                try:
                    _, t = future.result()
                    if t is not None:
                        templates[shape] = t
                        _save_checkpoint(templates)
                        # Derive the reverse immediately (experimental).
                        if args.derive_reversals:
                            rev = (m, n)
                            if rev != shape and rev not in templates and rev in set(target_shapes):
                                templates[rev] = derive_reverse(n, m, t)
                                print(f"  Derived ({m},{n}) from ({n},{m}) [free reversal] (EXPERIMENTAL)", flush=True)
                                _save_checkpoint(templates)
                    else:
                        print(f"  SKIPPED: no solution for {shape}", file=sys.stderr)
                except Exception as exc:
                    print(f"  ERROR: {shape} raised {exc}", file=sys.stderr)

    emit_library(templates)

    solved_count = len([s for s in target_shapes if s in templates])
    print(f"Generated {solved_count}/{len(target_shapes)} shapes")
    missing = [s for s in target_shapes if s not in templates]
    if missing:
        print(f"Missing ({len(missing)}): {missing}")

    # Clean up checkpoint on successful completion.
    if CHECKPOINT_PATH.exists():
        CHECKPOINT_PATH.unlink()
        print(f"Removed checkpoint {CHECKPOINT_PATH}")

    _maybe_sync()


def _maybe_sync() -> None:
    """Run sync_balancer_to_rust.py if it exists."""
    if not SYNC_SCRIPT.exists():
        return
    print("\nSyncing to Rust...", flush=True)
    result = subprocess.run(
        [sys.executable, str(SYNC_SCRIPT)],
        cwd=str(SYNC_SCRIPT.parent.parent),
    )
    if result.returncode != 0:
        print("Warning: Rust sync failed — run manually:", file=sys.stderr)
        print(f"  uv run python {SYNC_SCRIPT}", file=sys.stderr)


if __name__ == "__main__":
    main()
