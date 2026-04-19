# Fucktorio

Automated Factorio factory blueprint generator. Takes a target item + production rate, solves recipe dependencies, generates a spatial bus layout, and exports a Factorio-importable blueprint string.

## Quick start

```bash
# Rust workspace check + unit + e2e tests
cargo test --manifest-path crates/core/Cargo.toml

# A single test with output
cargo test --manifest-path crates/core/Cargo.toml --test e2e -- \
    tier2_electronic_circuit_from_ore --exact --nocapture

# Web app (dev server at http://localhost:5173)
cd web && npm install && npm run dev
```

For full build commands (WASM rebuild, release builds), see [`docs/build-systems.md`](docs/build-systems.md).

## Development conventions

- **Primary workflow**: edit Rust, run `cargo test`, then rebuild WASM and hit the web app to eyeball the layout.
- **WASM rebuild**: `wasm-pack build crates/wasm-bindings --target web --out-dir "$(pwd)/web/src/wasm-pkg"`. Always pass an absolute `--out-dir` (see memory: `feedback_wasmpack_outdir`).
- **Pre-commit hooks**: in `.githooks/pre-commit`, activate with `git config core.hooksPath .githooks`. Runs `cargo clippy` on staged Rust and `tsc` on staged TS. Bypass with `--no-verify` only for genuine emergencies.
- **Scripts**: put exploratory snippets in `scripts/` rather than inline one-liners. Rust debug scripts go in `crates/core/examples/` or as `#[test] #[ignore]` benchmarks.
- **Snapshots**: `FUCKTORIO_DUMP_SNAPSHOTS=1 cargo test ...` writes `.fls` files under `crates/core/target/tmp/`. Decode with `tail -c +5 <file> | base64 -d | gunzip`. See [`docs/layout-snapshot-debugger.md`](docs/layout-snapshot-debugger.md).
- **Process docs**: non-trivial design work uses [`docs/rfp-template.md`](docs/rfp-template.md) — the **kill criteria** section is required, since the dominant rework shape on this project is exploration that overruns its evidence. PRs follow [`.github/pull_request_template.md`](.github/pull_request_template.md), which captures intent, scope, verification actually run, and any deviations from agreed approach. Trivial changes can omit sections explicitly rather than leaving them blank.

## Architecture

**Rust workspace** (`crates/`) is where all work happens:

- **`crates/core/`** — pure shared logic: solver, recipe DB, bus layout engine, blueprint export, A\*, validation. All new features and bug fixes land here. The `wasm` feature gates WASM-only derives.
- **`crates/wasm-bindings/`** — thin wasm-bindgen wrapper exposing `solve`, `layout`, `export_blueprint`, and recipe lookups to the browser. Consumed by `web/src/engine.ts`.

**Web app** (`web/`) is the primary interactive interface. Vite + vanilla TS + PixiJS v8 + pixi-viewport. Runs the full solver → layout → blueprint pipeline client-side via WASM. URL state encodes the recipe, rate, machine tier, external inputs, and belt tier, so links reproduce layouts exactly.

### Pipeline stages (all Rust)

1. **Solver** (`crates/core/src/solver.rs`) — Recursively resolves recipes, computes machine counts and flow rates. Loads `crates/core/data/recipes.json` via `include_str!`. Returns a `SolverResult`.
2. **Bus layout** (`crates/core/src/bus/`) — Deterministic row-based layout. Machines group by recipe into rows, trunks run on parallel columns, tap-offs are routed via the ghost router (negotiated congestion A* + region-growth junction solver). See [`docs/ghost-pipeline-contracts.md`](docs/ghost-pipeline-contracts.md) for the phase-by-phase contracts the router promises.
3. **Blueprint export** (`crates/core/src/blueprint.rs`) — Emits the JSON + zlib + base64 envelope directly (no draftsman dependency).
4. **Validation** (`crates/core/src/validate/`) — 21 functional checks: pipe isolation, fluid port connectivity, inserter chains, power coverage, belt flow/structural, underground belt pairs, lane throughput.

## Key models (`crates/core/src/models.rs`)

- `ItemFlow` — item name, rate, fluid flag
- `MachineSpec` — machine type, recipe, count, inputs/outputs
- `SolverResult` — machines, external inputs/outputs, dependency order
- `PlacedEntity` — entity name, position, direction, recipe, carries, segment id
- `LayoutResult` — entities, connections, dimensions

## Key source files

Most-visited files. Full reference in [`docs/file-reference.md`](docs/file-reference.md).

| File | Purpose |
|------|---------|
| `crates/core/src/bus/layout.rs` | Top-level `build_bus_layout`: `place_rows` → `plan_bus_lanes` → `place_poles` → `route_bus_ghost`, single pass |
| `crates/core/src/bus/ghost_router.rs` | Ghost A* + negotiated congestion routing; junction solver integration; output merger call-site |
| `crates/core/src/bus/lane_planner.rs` | `BusLane` / `LaneFamily` types, `plan_bus_lanes`, lane splitting + tap-off coordinate finding |
| `crates/core/src/bus/lane_order.rs` | Left-to-right lane column order optimiser (exact search ≤7 lanes, hill-climb above) |
| `crates/core/src/bus/balancer.rs` | `stamp_family_balancer` + splitter/UG name helpers |
| `crates/core/src/bus/trunk_renderer.rs` | `render_path` (A* path → belts), `trunk_segments`, `is_intermediate` |
| `crates/core/src/bus/output_merger.rs` | Final-product east-flowing output merger |
| `crates/core/src/bus/placer.rs` | Row placement: group machines by recipe, split for throughput, `place_rows` geometry |
| `crates/core/src/bus/templates.rs` | Belt/inserter row templates (single-input, dual-input, lane-splitting sideload bridges) |
| `crates/core/src/bus/junction_solver.rs` | Region-growth junction solver framework (trait, growth loop) |
| `crates/core/src/bus/junction_sat_strategy.rs` | SAT-backed `JunctionStrategy` fallback |
| `crates/core/src/bus/ghost_occupancy.rs` | Typed `Occupancy` map (HardObstacle / RowEntity / Permanent / GhostSurface / Template / SatSolved) |
| `crates/core/src/bus/balancer_library.rs` | Pre-generated N→M balancer templates (do not edit manually) |
| `crates/core/src/astar.rs` | `ghost_astar` + `astar_path` + `negotiate_lanes` pathfinder primitives |
| `crates/core/src/sat.rs` | Varisat-backed crossing-zone SAT solver (see memory: `project_sat_crossing_solver`) |
| `crates/core/src/validate/belt_flow.rs` | Lane-rate walker (Kahn topo sort with splitter pairing and balancer feedback-loop handling) |
| `crates/core/src/validate/` | Rest of the 21 checks: `belt_structural`, `fluids`, `inserters`, `power`, `underground` |
| `crates/core/src/trace.rs` | Thread-local trace event collector; `TraceEvent` variants drive the snapshot debugger and stress scoreboards |
| `crates/core/src/snapshot.rs` | `.fls` snapshot reader/writer for the layout debugger |
| `crates/core/tests/e2e.rs` | End-to-end test harness: tier1–4 regression tests and stress corpus with scoreboards |
| `crates/wasm-bindings/src/lib.rs` | wasm-bindgen wrapper exposing `solve`, `layout`, `export_blueprint` to the browser |
| `web/src/engine.ts` | WASM loader and typed wrappers |
| `web/src/renderer/entities.ts` | PixiJS entity renderer (bus layout view) |
| `web/src/ui/sidebar.ts` | Searchable item picker, rate input, live solve, URL state |

## Factorio game rules (constraints for the layout engine)

Physical rules the layout engine must satisfy:

- **Machines** craft recipes, need ingredients delivered and products extracted.
- **Inserters** pick from one side, drop to the other. Regular reach 1 tile; long-handed reach 2.
- **Belts** move items in a direction, connect when adjacent. Tiers: yellow 15/s, red 30/s, blue 45/s.
- **Pipes** carry fluids and merge with any adjacent pipe — separate fluid networks must be physically isolated.
- **Fluid ports** on machines are at specific tile positions that depend on direction and `mirror`.
- **Fluid-box mirroring (Space Age)** — entities with fluid boxes support `mirror: true` in the blueprint. Combined with `direction`, this gives 8 orientations (4 rotations × 2 mirrors). Only honored by Factorio 2.0+.
- **Entities** cannot overlap.
- **Power** — machines need electricity; medium-electric-pole covers a 7×7 area.
- **Belt lane mechanics** — sideloading, UG lane rules, splitter behavior — detailed in [`docs/factorio-mechanics.md`](docs/factorio-mechanics.md).

## Recipe complexity ladder

Tracks which recipes produce zero-error bus blueprints. Moving up = real progress. Tests for each tier live in `crates/core/tests/e2e.rs`.

| Tier | Recipe | Complexity | Bus status |
|------|--------|-----------|-----|
| 1 | `iron-gear-wheel` | 1 recipe, 1 solid input | SOLVED |
| 2 | `electronic-circuit` | 2 recipes, 2 solid inputs | SOLVED (incl. from ores) |
| 3 | `plastic-bar` | 1 recipe, 1 fluid + 1 solid input | SOLVED |
| 4 | `advanced-circuit` | 5+ recipes, mixed solid/fluid | Partial (from plates: lane-throughput warnings, needs [#65](https://github.com/storkme/fucktorio/issues/65); from ores blocked by [#68](https://github.com/storkme/fucktorio/issues/68) and missing balancer shapes [#136](https://github.com/storkme/fucktorio/issues/136)) |
| 5 | `processing-unit` | Deep chain, multiple fluids | Not attempted |
| 6 | `rocket-control-unit` | Very deep chain | Not attempted |

Open tracking issues for layout quality: [#135 balancer templates are oversized](https://github.com/storkme/fucktorio/issues/135), [#136 missing coprime balancer shapes](https://github.com/storkme/fucktorio/issues/136), [#68 fluid row 3-tile pitch](https://github.com/storkme/fucktorio/issues/68).

## Verification protocol for layout engine changes

Layout bugs are easy to get wrong — zero validation errors can mean the check was wrong, not that the layout is. Follow this protocol:

1. **Run the full e2e suite** — `cargo test --manifest-path crates/core/Cargo.toml`. All 10 non-ignored tests must stay green.
2. **Load the case in the browser** — start the dev server, open the URL for the recipe you changed, and look at the layout with your eyes. A zero-warning layout that visibly has disconnected belts is a validator bug, not a success.
3. **Check the snapshot for the exact bug you intended to fix** — `FUCKTORIO_DUMP_SNAPSHOTS=1 cargo test ... --nocapture <test>` then decode with the snippet in [`docs/layout-snapshot-debugger.md`](docs/layout-snapshot-debugger.md). Inspect entities at the suspect coordinates, not just the warning count.
4. **Trace events are reliable signals** — `RouteFailure`, `BridgeDropped`, `CrossingZoneSkipped`, `BalancerStamped` are emitted by the pipeline and land in the snapshot's `trace.events`. Use them to confirm the specific failure mode before theorizing.
5. **Don't trust an error-count drop alone** — if warnings go 5 → 0, ask *why*. Does the topology still make sense? Were belts actually re-routed, or did a check get silently skipped? Check the specific change caused the fix you wanted.
6. **Clippy + WASM builds are checks, not nits** — a layout change that clippy-fails or breaks the WASM build is not done.

## Where to find X

| Looking for | Location |
|-------------|----------|
| Recipe data | `crates/core/data/recipes.json` (embedded via `include_str!`) |
| Balancer templates | `crates/core/src/bus/balancer_library.rs`. Regenerate: `python scripts/generate_balancer_library.py` (needs Factorio-SAT on `PATH`). |
| Belt tier thresholds | `crates/core/src/common.rs` (`belt_entity_for_rate`, `ug_max_reach`) |
| Entity sizes | `crates/core/src/common.rs` (`entity_size`) |
| Validation checks | `crates/core/src/validate/` (21 checks) |
| Snapshot format | `crates/core/src/snapshot.rs` + [`docs/layout-snapshot-debugger.md`](docs/layout-snapshot-debugger.md) |
| Belt lane physics | [`docs/factorio-mechanics.md`](docs/factorio-mechanics.md) |
| Ghost pipeline contracts | [`docs/ghost-pipeline-contracts.md`](docs/ghost-pipeline-contracts.md) |
| Build commands | [`docs/build-systems.md`](docs/build-systems.md) |
| Full source file list | [`docs/file-reference.md`](docs/file-reference.md) |

## Visualizations

The web app at `http://localhost:5173` is the primary visualization — any URL (`?item=...&rate=...&in=...&belt=...`) renders a live layout with entity overlays, segment highlighting, and validation markers. The same app is deployed to GitHub Pages on every push to main: https://storkme.github.io/fucktorio/
