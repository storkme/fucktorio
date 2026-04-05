# Python → Rust port status

**Status: Complete (2026-04-05)**

The full pipeline is ported to `crates/core` and runs in the browser via WASM. See `crates/core/src/` for the current Rust implementation.

## What's ported

| Module | Rust location |
|---|---|
| Models | `core/models.rs` |
| Recipe DB | `core/recipe_db.rs` |
| Solver | `core/solver.rs` |
| Blueprint export | `core/blueprint.rs` |
| A* pathfinder | `core/astar.rs` |
| Common utilities | `core/common.rs` |
| Bus placer | `core/bus/placer.rs` |
| Bus templates | `core/bus/templates.rs` |
| Balancer library | `core/bus/balancer_library.rs` |
| Bus router | `core/bus/bus_router.rs` |
| Bus layout orchestrator | `core/bus/layout.rs` |
| Validation (all 6 families) | `core/validate/` |

## What stays Python

| Module | Why |
|---|---|
| `src/analysis/` | Debug/research tool for parsing existing blueprints |
| `src/visualize.py`, `src/showcase.py` | Draftsman-based HTML viz for pytest; browser has its own Pixi renderer |
| `src/verify.py` | ASCII-map structural debug tool |
| `src/spaghetti/`, `src/routing/`, `src/search/` | Parked; spaghetti engine work is paused per [#62](https://github.com/storkme/fucktorio/issues/62) |
| Python pipeline + pytest suite | Reference implementation and regression test harness |
