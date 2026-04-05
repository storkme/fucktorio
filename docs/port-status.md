# Python → Rust port status

Tracking which Python modules have been ported to `crates/core` for use by the WASM web app. Updated as PRs land.

## Summary

| | Python LOC | Rust LOC | % |
|---|---:|---:|---:|
| **Ported** | ~1,190 | ~1,640 | — |
| **To port (needed for web MVP)** | ~8,900 | — | — |
| **Not needed in browser** | ~3,400 | — | — |

~8% of the meaningful Python codebase is in Rust today. A functional browser MVP (solver → bus layout → blueprint export → validation) needs another ~8,900 LOC ported.

## What's ported

| Module | Python | Rust | Status |
|---|---:|---:|---|
| `src/models.py` | 89 | 86 (`core/models.rs`) | ✅ complete |
| `src/solver/recipe_db.py` | 134 | 176 (`core/recipe_db.rs`) | ✅ complete |
| `src/solver/solver.py` | 118 | 218 (`core/solver.rs`) | ✅ complete |
| `src/blueprint/blueprint.py` | 49 | 155 (`core/blueprint.rs`) | ✅ complete (1x1 entity centering limitation) |
| `src/routing/router.py` (A* only) | ~800 of 1,400 | 1,007 (`core/astar.rs`) | ✅ A* + negotiate_lanes |

## What's left to port (needed for web MVP)

### Tier 1 — Bus layout engine (primary focus)

Blocks: browser can run solver + export a blueprint string, but can't generate a layout. All of these are in `src/bus/`.

| Module | Python LOC | Dependencies | Notes |
|---|---:|---|---|
| `src/bus/placer.py` | 390 | models, solver result | Groups machines by recipe into rows, splits for throughput. Pure data transformation — easy port |
| `src/bus/templates.py` | 963 | models, common | Belt/inserter stamp templates (single-input, dual-input, lane-splitting). Pure data/tables, mechanical port |
| `src/bus/balancer_library.py` | 595 | models | Pre-generated SAT templates — mostly a const data embed + stamping helper |
| `src/bus/bus_router.py` | 2,183 | placer, templates, balancer_library, astar | Trunk routing, tap-offs, N→M balancer families, output mergers, negotiated crossings. **The big one.** Consume core A* via `astar::negotiate_lanes` |
| `src/bus/layout.py` | 182 | all above | Top-level orchestrator |
| `src/routing/common.py` | 123 | — | Machine sizes, belt tier selection, direction/lane constants. Shared infrastructure, port first |
| **Total** | **~4,440** | | |

Estimated Rust: ~5,500 LOC (Rust is typically 20–25% longer than equivalent Python).

### Tier 2 — Validation

Blocks: generated layouts can't be checked for correctness before export. All in `src/validate.py` (2,776 LOC, 21 check functions).

| Check family | Functions | Approx LOC | Complexity |
|---|---|---:|---|
| Belts — flow & connectivity | `check_belt_connectivity`, `check_belt_flow_path`, `check_belt_direction_continuity`, `check_belt_flow_reachability`, `check_belt_network_topology`, `check_belt_junctions` | ~800 | BFS/graph traversal, directional flow |
| Belts — structural | `check_belt_loops`, `check_belt_dead_ends`, `check_belt_item_isolation`, `check_belt_inserter_conflict`, `check_belt_throughput`, `check_lane_throughput`, `check_output_belt_coverage` | ~700 | Geometry + rate math |
| Underground belts | `check_underground_belt_pairs`, `check_underground_belt_sideloading`, `check_underground_belt_entry_sideload` | ~300 | Paired-entity geometry |
| Pipes / fluids | `check_pipe_isolation`, `check_fluid_port_connectivity` | ~400 | Needs fluid port data (already in recipes.json) |
| Inserters | `check_inserter_chains`, `check_inserter_direction` | ~300 | Tile-adjacency |
| Power | `check_power_coverage` | ~100 | 7x7 coverage grid |

Each check is independent — these can be ported in **parallel batches of 3–5 checks**. Natural first batch: power + underground pairs (smallest, self-contained).

Estimated Rust: ~3,500 LOC.

## What's NOT needed for the web app

These stay Python forever (or get trimmed eventually):

| Module | Python LOC | Why not |
|---|---:|---|
| `src/analysis/` | 1,749 | Parses existing Factorio blueprints — debug/research tool, not generator |
| `src/visualize.py` + `src/showcase.py` | 934 | Draftsman-based HTML viz for pytest. Browser has its own Pixi renderer |
| `src/verify.py` | 234 | ASCII-map structural debug tool |
| `src/spaghetti/` + `src/search/` | 874 | Parked per CLAUDE.md; spaghetti engine is unused |
| `src/routing/*` (non-router) | 2,000 | Only used by spaghetti. Keep until spaghetti work resumes |

**Total: ~5,800 LOC** that can stay Python.

## What's also left: thin layer to make wasm usable

Not a "port" but needed before the browser can actually call anything:

| Task | Files | LOC estimate |
|---|---|---:|
| Wire wasm `solve()` → `core::solver::solve` | `crates/wasm-bindings/src/lib.rs` | ~10 |
| Wire `all_producible_items` / `all_producer_machines` | same | ~10 |
| Wire wasm `export_blueprint` → `core::blueprint::export` | same | ~5 |
| Relax `machine_entity: &'static str` → `&str` in solver | `core/solver.rs` | ~5 |
| Web UI: engine.ts loader, sidebar.ts, entities.ts | `web/src/` | ~400 |

This is the deferred "end-to-end wire-up" from batch 1 — do it as soon as one person is free. Unblocks the demo even without the layout engine (user picks item + rate → sees machine count summary, no canvas entities yet).

## Execution

The sequential unit list with per-unit briefs lives in **`docs/port-plan.md`** — 16 canonical units (bus layout engine + validation) ready to be delegated to Sonnet subagents via the `/port <unit-name>` skill.

## Suggested sequencing

The port work and the web UI work don't block each other. Rough parallel tracks:

**Track A — Port to Rust (heavy lifting)**
1. `src/routing/common.py` + `src/bus/placer.py` (single PR, foundations)
2. `src/bus/templates.py` (single PR, self-contained data)
3. `src/bus/balancer_library.py` (single PR, data embed)
4. `src/bus/bus_router.py` phases (3–4 PRs: trunks → balancers → mergers → fluid lanes)
5. `src/bus/layout.py` orchestrator (small PR, ties it together)
6. Validation checks (batches of 3–5, multiple parallel PRs)

**Track B — Web app (UI/UX)**
1. Wire wasm stubs + engine.ts (minimal but gets solver running in browser)
2. Sidebar UI with item/rate/machine pickers
3. Entity renderer (colored rectangles from `PlacedEntity[]`)
4. Blueprint export button with clipboard copy
5. Sprite-based entities (replacing colored rectangles)
6. Validation error display panel (depends on Track A step 6)

Track A step 4 (bus_router) is the longest pole — that's ~2,200 LOC of routing logic with non-trivial dependencies on A* + balancer templates. Everything else is mechanical.

## Key reference files

| File | Purpose |
|---|---|
| `src/bus/bus_router.py` | Biggest remaining port (2,183 LOC) |
| `src/validate.py` | All 21 validation checks (2,776 LOC) |
| `src/routing/common.py` | Shared constants needed first (123 LOC) |
| `docs/web-app-plan.md` | Overall web app plan |
| `docs/belt-mechanics.md` | Physics rules the layout engine must satisfy |
