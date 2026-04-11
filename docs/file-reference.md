# Key Source Files

Full reference table. The most-visited files are summarised in `CLAUDE.md`.

## Python pipeline (`src/`)

| File | Purpose |
|------|---------|
| `src/pipeline.py` | Top-level orchestrator |
| `src/models.py` | Shared data models (ItemFlow, MachineSpec, SolverResult, PlacedEntity, LayoutResult) |
| `src/routing/router.py` | Item-aware A\* pathfinding (state: x,y,forced,lane), `other_item_tiles` hard-blocks cross-item contamination, proximity penalty near foreign networks, underground belts, belt/pipe entity placement |
| `src/routing/inserters.py` | Lane-aware inserter assignment (`approach_vec`, `target_lane`, `is_direct` on `InserterAssignment`) |
| `src/routing/orchestrate.py` | Layout orchestration: batch (`build_layout`) and incremental (`build_layout_incremental`) builders, direct machine-to-machine insertion, stub placement on routing failure |
| `src/routing/graph.py` | Production graph construction from solver output |
| `src/routing/common.py` | Machine sizes, belt tier selection, direction constants, lane constants (`LANE_LEFT`/`LANE_RIGHT`), `inserter_target_lane()` |
| `src/routing/poles.py` | Power pole placement (greedy near-machine or grid fallback) |
| `src/search/layout_search.py` | Single-pass parallel random search (`random_search_layout`) + `search_with_retries()` wrapper (up to 5 independent attempts), `SearchStats` dataclass |
| `src/spaghetti/placer.py` | Incremental machine placement in dependency order |
| `src/spaghetti/layout.py` | Layout orchestrator — calls `search_with_retries()`, entry point for spaghetti engine |
| `src/validate.py` | 21 validation checks (pipe isolation, belt loops, throughput, etc.) |
| `src/bus/layout.py` | Bus layout orchestrator |
| `src/bus/placer.py` | Row placement: groups machines by recipe, splits rows for throughput |
| `src/bus/templates.py` | Belt/inserter templates for bus rows (single-input, dual-input, lane-splitting) |
| `src/bus/bus_router.py` | Trunk routing (1-tile spacing), tap-off underground crossings, N-to-M balancer families, producer-to-input wiring, output mergers, negotiated crossing map |
| `src/bus/balancer_library.py` | Pre-generated N-to-M balancer templates (SAT-solved). Regenerate via `scripts/generate_balancer_library.py` |
| `src/analysis/` | Blueprint analysis: classify, trace, infer items, build production graphs |

## Rust pipeline (`crates/core/src/`)

| File | Purpose |
|------|---------|
| `crates/core/src/models.rs` | Shared data models (Rust port of `src/models.py`) |
| `crates/core/src/common.rs` | Shared constants and helpers (belt tiers, entity sizes, direction utils) |
| `crates/core/src/astar.rs` | Rust A\* pathfinder + lane-first negotiated congestion routing (shared between PyO3 + WASM) |
| `crates/core/src/solver.rs` | Rust port of the solver (recursive recipe resolution) |
| `crates/core/src/recipe_db.rs` | Recipe DB — loads `crates/core/data/recipes.json` via `include_str!` |
| `crates/core/src/blueprint.rs` | Blueprint exporter (JSON + zlib + base64 envelope) |
| `crates/core/src/blueprint_parser.rs` | Blueprint string → LayoutResult (reverse of blueprint.rs) |
| `crates/core/src/snapshot.rs` | Layout snapshot format for debugging (see `docs/layout-snapshot-debugger.md`) |
| `crates/core/src/trace.rs` | Structured trace event collection (thread-local, zero overhead when inactive) |
| `crates/core/src/sat.rs` | SAT-based solver for bus crossing zones |
| `crates/core/src/bus/bus_router.rs` | Trunk placement, tap-offs, balancer family stamping, output mergers |
| `crates/core/src/bus/layout.rs` | Bus layout orchestrator |
| `crates/core/src/bus/placer.rs` | Row placement (Rust port) |
| `crates/core/src/bus/templates.rs` | Belt/inserter row templates (Rust port) |
| `crates/core/src/bus/balancer_library.rs` | Pre-generated balancer templates (Rust port, do not edit manually) |
| `crates/core/src/bus/tapoff_search.rs` | Brute-force search for optimal tap-off tile patterns (test/generation only) |
| `crates/core/src/validate/belt_flow.rs` | Belt connectivity, flow paths, direction continuity, reachability, topology, junctions |
| `crates/core/src/validate/belt_structural.rs` | Belt structural checks (loops, dead-ends, throughput, overlaps) |
| `crates/core/src/validate/inserters.rs` | Inserter chain and direction checks |
| `crates/core/src/validate/fluids.rs` | Pipe isolation and fluid port connectivity |
| `crates/core/src/validate/power.rs` | Power coverage and pole network connectivity |
| `crates/core/src/validate/underground.rs` | Underground belt pair and sideloading checks |

## Bindings and web

| File | Purpose |
|------|---------|
| `crates/pyo3-bindings/src/lib.rs` | PyO3 adapter exposing `astar_path` + `negotiate_lanes` to Python |
| `crates/wasm-bindings/src/lib.rs` | wasm-bindgen wrapper: `solve`, `layout`, `export_blueprint`, recipe lookups |
| `web/src/main.ts` | Web app entry: wires Pixi canvas + sidebar + engine |
| `web/src/engine.ts` | WASM loader + typed wrappers around `fucktorio_wasm` |
| `web/src/renderer/` | PixiJS renderers: `app.ts` (viewport), `grid.ts`, `graph.ts` (DAG), `entities.ts` (bus layout), `colors.ts` |
| `web/src/ui/sidebar.ts` | Searchable item picker, rate input, machine picker, live solve, URL state, totals |
