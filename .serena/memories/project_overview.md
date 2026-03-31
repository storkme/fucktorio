# Fucktorio - Project Overview

Automated Factorio factory blueprint generator. Takes a target item + production rate, solves recipe dependencies, generates a spatial layout, and exports a Factorio-importable blueprint string.

## Tech Stack
- **Python 3.12+** (main language)
- **Rust** via Maturin/PyO3 — native A* pathfinding module (`fucktorio_native`, built from `rust_src/lib.rs`)
- **factorio-draftsman** — Factorio data access and blueprint encoding
- **pytest** — test framework with custom `--viz` flag for HTML visualizations
- **ruff** — linting and formatting

## Architecture (3-stage pipeline)
1. **Solver** (`src/solver/`) — Resolves recipes, calculates machine counts and flow rates → `SolverResult`
2. **Layout** (`src/spaghetti/`, `src/routing/`, `src/search/`) — Parallel random search with incremental place-and-route → `LayoutResult`
3. **Blueprint** (`src/blueprint/`) — Converts `LayoutResult` to base64 blueprint string
4. **Validation** (`src/validate.py`) — 18 functional checks (pipe isolation, fluid connectivity, etc.)

Also has a newer **bus layout** engine (`src/bus/`) as an alternative to the spaghetti layout.

## Key Directories
- `src/` — main source code
- `src/routing/` — A* router, inserter assignment, production graph, orchestration
- `src/spaghetti/` — spaghetti-style layout engine
- `src/bus/` — bus-style layout engine
- `src/search/` — parallel random search
- `tests/` — pytest test suite
- `rust_src/` — Rust native module (A* pathfinding)
- `test_viz/` — generated HTML visualizations
