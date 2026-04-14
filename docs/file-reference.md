# Key Source Files

Full reference table. The most-visited files are summarised in `CLAUDE.md`.

## Rust pipeline (`crates/core/src/`)

| File | Purpose |
|------|---------|
| `models.rs` | Shared data models: `ItemFlow`, `MachineSpec`, `SolverResult`, `PlacedEntity`, `LayoutResult`, `EntityDirection` |
| `common.rs` | Shared constants and helpers (belt tiers, entity sizes, direction utils, UG reach) |
| `astar.rs` | `ghost_astar` — turn-penalty + per-axis cost A* used by the ghost router |
| `solver.rs` | Recursive recipe resolution producing `SolverResult` |
| `recipe_db.rs` | Recipe DB — loads `crates/core/data/recipes.json` via `include_str!` |
| `blueprint.rs` | Blueprint exporter (JSON + zlib + base64 envelope) |
| `blueprint_parser.rs` | Blueprint string → `LayoutResult` (reverse of `blueprint.rs`) |
| `snapshot.rs` | `.fls` layout snapshot format for debugging (see `docs/layout-snapshot-debugger.md`) |
| `trace.rs` | Structured trace event collection (thread-local, zero overhead when inactive) |
| `sat.rs` | Varisat-backed SAT solver for bus crossing zones |
| `zone_cache.rs` | On-disk memo for expensive SAT solves (native only) |
| `analysis.rs` | Post-layout analytics used by tests + snapshot tooling |

### Bus layout subsystem (`crates/core/src/bus/`)

| File | Purpose |
|------|---------|
| `layout.rs` | Top-level orchestrator: `place_rows → plan_bus_lanes → place_poles → route_bus_ghost` |
| `placer.rs` | Row placement — groups machines by recipe, splits rows for throughput |
| `templates.rs` | Belt/inserter row templates (single-input, dual-input, lane-splitting) |
| `lane_planner.rs` | `BusLane` / `LaneFamily` types, `plan_bus_lanes`, lane splitting + tap-off coordinate finding |
| `lane_order.rs` | Left-to-right lane column order optimiser |
| `balancer.rs` | `stamp_family_balancer` + splitter/UG belt-tier name helpers |
| `balancer_library.rs` | Pre-generated N→M balancer templates (do not edit manually) |
| `trunk_renderer.rs` | Path → entity rendering (`render_path`, `trunk_segments`, `is_intermediate`) |
| `output_merger.rs` | Final-product east-flowing output merger |
| `ghost_router.rs` | Ghost A* + negotiation loop, crossing set construction, junction solver integration |
| `ghost_occupancy.rs` | Typed `Occupancy` map: `HardObstacle` / `RowEntity` / `Permanent` / `GhostSurface` / `Template` / `SatSolved` |
| `junction.rs` | `Junction` / `SpecCrossing` / `Rect` / `BeltTier` — the snapshot strategies consume |
| `junction_solver.rs` | Region-growth outer loop + `JunctionStrategy` trait |
| `junction_sat_strategy.rs` | SAT-backed `JunctionStrategy` fallback |
| `tapoff_search.rs` | Brute-force search for optimal tap-off tile patterns (test/generation only) |

### Validation (`crates/core/src/validate/`)

| File | Purpose |
|------|---------|
| `belt_flow.rs` | Belt connectivity, flow paths, direction continuity, reachability, topology, junctions |
| `belt_structural.rs` | Belt structural checks (loops, dead-ends, throughput, overlaps) |
| `inserters.rs` | Inserter chain and direction checks |
| `fluids.rs` | Pipe isolation and fluid port connectivity |
| `power.rs` | Power coverage and pole network connectivity |
| `underground.rs` | Underground belt pair and sideloading checks |

## Bindings and web

| File | Purpose |
|------|---------|
| `crates/wasm-bindings/src/lib.rs` | wasm-bindgen wrapper: `solve`, `layout`, `layout_traced`, `export_blueprint`, `validate_layout`, recipe lookups |
| `web/src/main.ts` | Web app entry: wires Pixi canvas + sidebar + engine |
| `web/src/engine.ts` | WASM loader + typed wrappers around `fucktorio_wasm` |
| `web/src/renderer/` | PixiJS renderers: `app.ts` (viewport), `grid.ts`, `graph.ts` (DAG), `entities.ts` (bus layout), `colors.ts`, `validationOverlay.ts`, `ghostRoutingOverlay.ts` |
| `web/src/ui/sidebar.ts` | Searchable item picker, rate input, machine picker, live solve, URL state, totals |

## Tests

| File | Purpose |
|------|---------|
| `crates/core/tests/e2e.rs` | End-to-end test harness: tier 1–4 regression tests + stress corpus with scoreboards |
| `crates/core/examples/diagnose_junctions.rs` | Offline diagnostic: runs tier 2/3/4 layouts and dumps junction-solver breakdown + balancer stamps |
