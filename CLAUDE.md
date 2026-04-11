# Fucktorio

Automated Factorio factory blueprint generator. Takes a target item + production rate, solves recipe dependencies, generates a spatial layout, and exports a Factorio-importable blueprint string.

## Quick start

```bash
# Requires: uv (manages Python version + deps)
uv sync

# Run tests
uv run pytest tests/

# Generate a blueprint
uv run python -m src.pipeline
```

For Rust builds (PyO3 extension, WASM bundle) and the web app, see [`docs/build-systems.md`](docs/build-systems.md).

## Development conventions

- **Python version**: Pinned in `.python-version`. Use `uv` to manage the venv and run commands.
- **Running snippets**: Don't use inline `python -c` for multi-line exploratory code. Instead, write a script to `scripts/` (e.g. `scripts/debug_lanes.py`) and run it with `uv run python scripts/debug_lanes.py`. This makes it easier to iterate and review.
- **Running tests**: `uv run pytest tests/`.
- **Pre-commit hooks**: Committed in `.githooks/pre-commit`. Activate with `git config core.hooksPath .githooks`. Runs ruff check + format on Python, cargo clippy on Rust (if .rs files staged), and tsc on TypeScript (if web/src/ files staged). Bypass with `git commit --no-verify`.

## Architecture

Two parallel implementations share the same pipeline shape:

**Python pipeline** (`src/pipeline.py` orchestrates) — the canonical/reference implementation; used for pytest, HTML visualizations, and analysis tooling.

**Rust pipeline** (`crates/core/`) — complete port of all pipeline stages (solver, recipe DB, bus layout, blueprint export, validation, A\*), used by the PyO3 extension and by the WASM web app.

Pipeline stages:

1. **Solver** (`src/solver/`, `crates/core/src/solver.rs`) — Recursively resolves recipes, calculates machine counts and flow rates. Returns `SolverResult`. Python uses `draftsman.data`; Rust loads a pre-extracted `crates/core/data/recipes.json`.
2. **Layout** — Two engines, both return `LayoutResult`:
   - **Bus** (`src/bus/`, `crates/core/src/bus/`) — Deterministic row-based layout with main bus pattern. The primary focus, and the engine exposed to the web app. See [`docs/layout-engine-deep-dive.md`](docs/layout-engine-deep-dive.md) for internals.
   - **Spaghetti** (`src/spaghetti/`, `src/routing/`, `src/search/`) — Python-only. Random search with A\* place-and-route. Parked pending routing rework ([#62](https://github.com/storkme/fucktorio/issues/62)).
3. **Blueprint** (`src/blueprint/`, `crates/core/src/blueprint.rs`) — Converts `LayoutResult` to a base64 blueprint string. Python version wraps draftsman; Rust version emits the JSON + zlib + base64 envelope directly.
4. **Validation** (`src/validate.py`, `crates/core/src/validate/`) — 21 functional checks: pipe isolation, fluid port connectivity, inserter chains, power coverage, belt flow/structural, underground belt pairs, lane throughput.
5. **Analysis** (`src/analysis/`) — Python-only. Parses real Factorio blueprints into production graphs for studying community layouts.

## Key models (`src/models.py`)

- `ItemFlow` — item name, rate, fluid flag
- `MachineSpec` — machine type, recipe, count, inputs/outputs
- `SolverResult` — machines, external inputs/outputs, dependency order
- `PlacedEntity` — entity name, position, direction, recipe, carries (what item/fluid it transports)
- `LayoutResult` — entities, connections, dimensions

## Key source files

Most-visited files. For the full reference table see [`docs/file-reference.md`](docs/file-reference.md).

| File | Purpose |
|------|---------|
| `src/bus/bus_router.py` / `crates/core/src/bus/bus_router.rs` | Trunk placement, tap-offs, balancer families, output mergers |
| `src/bus/templates.py` / `crates/core/src/bus/templates.rs` | Belt/inserter row templates |
| `src/validate.py` / `crates/core/src/validate/` | 21 validation checks |
| `crates/core/src/astar.rs` | Rust A\* pathfinder (shared PyO3 + WASM) |
| `crates/core/src/models.rs` / `src/models.py` | Shared data models |
| `crates/core/src/common.rs` / `src/routing/common.py` | Shared constants and helpers (belt tiers, entity sizes) |
| `src/bus/balancer_library.py` / `crates/core/src/bus/balancer_library.rs` | Pre-generated N-to-M balancer templates (SAT-solved) |

## Factorio game rules (constraints for the layout engine)

Physical rules the layout engine must satisfy:

- **Machines** craft recipes, need ingredients delivered and products extracted
- **Inserters** pick from one side, drop to the other. Regular inserters reach 1 tile; long-handed inserters reach 2 tiles
- **Belts** move items in a direction, connect when adjacent. Tiers: yellow 15/s, red 30/s, blue 45/s
- **Pipes** carry fluids, connect to any adjacent pipe (and merge — separate fluid networks must be physically isolated)
- **Fluid ports** on machines are at specific tile positions (queryable from `draftsman.data.entities`)
- **Fluid-box mirroring (Space Age)** — entities with fluid boxes support a `mirror: true` blueprint attribute that flips fluid port positions along the entity's primary axis. Combined with `direction`, this gives 8 orientations (4 rotations × 2 mirrors). Only effective in Factorio Space Age (2.0+); ignored in 1.1.
- **Entities** cannot overlap
- **Power** — machines need electricity; medium-electric-pole covers a 7×7 area
- **Belt lane mechanics** — see [`docs/factorio-mechanics.md`](docs/factorio-mechanics.md) for detailed lane-level rules (sideloading, underground lanes, splitter behavior)

## Recipe complexity ladder

Tracks which recipes produce zero-error blueprints. Each tier represents increasing complexity. Moving up = real progress.

| Tier | Recipe | Complexity | Spaghetti | Bus |
|------|--------|-----------|-----------|-----|
| 1 | `iron-gear-wheel` | 1 recipe, 1 solid input | Inconsistent (xfail) — loops, contamination, unpaired UG | SOLVED |
| 2 | `electronic-circuit` | 2 recipes, 2 solid inputs | Failing — belt-flow-reachability | SOLVED (incl. from ores) |
| 3 | `plastic-bar` | 1 recipe, 1 fluid + 1 solid input | Failing — pipe isolation | SOLVED |
| 4 | `advanced-circuit` | 5+ recipes, mixed solid/fluid | Failing — massive routing failures | Partial (from plates: lane-throughput warnings, needs [#65](https://github.com/storkme/fucktorio/issues/65)) |
| 5 | `processing-unit` | Deep chain, multiple fluids | Not attempted | Not attempted |
| 6 | `rocket-control-unit` | Very deep chain | Not attempted | Not attempted |

### Known failure modes

1. **Belt-flow-reachability** (current #1 blocker): Continuation routing connects tiles topologically but belt directions point the wrong way for input edges. The belt path exists physically but items can't flow upstream. Fix needed at `route_connections` level.
2. **Belt cross-item contamination**: SOLVED for tier 1 via `other_item_tiles` hard-blocking in A\*. Proximity penalty (+3.0) and contamination guard in `_fix_belt_directions()` prevent residual cases.
3. **Belt loops**: A\* routing accidentally creates cycles in the belt network.
4. **Disconnected networks**: Output belts from different machines don't merge into a connected trunk.
5. **Throughput violations**: Too many inserters feeding the same belt lane.
6. **Failed routing edges**: A\* can't find any path from source to destination.
7. **Pipe merge contamination**: Adjacent pipes carrying different fluids merge (fluids connect to all adjacent pipes).

## Verification protocol for layout engine changes

Layout changes are easy to get wrong — errors can be masked by other changes, imports can shadow silently, and error counts can drop to zero for the wrong reason. Follow this protocol:

1. **Check the viz**: After any routing/placement change, generate `test_viz/iron-gear-wheel-10s.html` and visually inspect it. Zero validation errors mean nothing if the layout looks wrong.
   ```bash
   pytest tests/test_spaghetti.py::TestSpaghettiVisualization::test_viz_iron_gear_wheel --viz -x
   ```
2. **Verify the fix is running**: If you add new logic inside the incremental builder, confirm it actually executes (e.g. add a temporary `logger.info` or check that output changes). The `_evaluate` function catches ALL exceptions silently, so broken code just scores 10000 and gets ignored.
3. **Watch for import shadowing**: `build_layout_incremental()` is a long function. A `from .router import X` anywhere inside it makes `X` a local variable for the ENTIRE function, shadowing any module-level import of `X`. All imports from router should be at the top of the file.
4. **Don't trust error count drops alone**: If errors go from 5 to 0, ask WHY. Check that belt types are correct (should be yellow transport-belt for 10/s), check that the topology makes sense, check that the specific fix you intended is the reason errors dropped.
5. **One search attempt should take <2s** with the Rust A\*. If it takes >10s, something is wrong (likely N×A\* instead of multi-start, or an infinite loop).

## Where to find X

| Looking for | Location |
|-------------|----------|
| Recipe data | `crates/core/data/recipes.json` (Rust, embedded via `include_str!`). Python uses `draftsman.data` at runtime. |
| Balancer templates | `src/bus/balancer_library.py` / `crates/core/src/bus/balancer_library.rs`. Regenerate: `uv run python scripts/generate_balancer_library.py` |
| Belt tier thresholds | `crates/core/src/common.rs` (`belt_entity_for_rate`) / `src/routing/common.py` |
| Entity sizes | `crates/core/src/common.rs` (`entity_size`) / `src/routing/common.py` (`MACHINE_SIZES`) |
| Validation checks | `src/validate.py` (Python reference) ↔ `crates/core/src/validate/` (Rust port, 1:1 check parity) |
| Layout snapshot format | `crates/core/src/snapshot.rs` + `docs/layout-snapshot-debugger.md` |
| Belt lane physics | `docs/factorio-mechanics.md` |
| Bus layout internals | `docs/layout-engine-deep-dive.md` |
| Build commands | `docs/build-systems.md` |
| Full source file list | `docs/file-reference.md` |

## Verification & validation

- `src/verify.py` — structural blueprint validation (overlap detection, unpaired undergrounds, ASCII map)
- `src/validate.py` — functional validation (21 checks: pipe isolation, fluid connectivity, inserter chains, power coverage, belt flow/structural, underground pairs, lane throughput)
- `crates/core/src/validate/` — Rust mirror of the same check suite, used by the WASM web app
- `tests/` — pytest suite with `--viz` flag for HTML visualizations, deployed to GitHub Pages via CI

## Visualizations

Test visualizations are deployed to GitHub Pages on main branch pushes:
https://storkme.github.io/fucktorio/
