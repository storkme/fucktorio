# Fucktorio

Automated Factorio factory blueprint generator. Takes a target item + production rate, solves recipe dependencies, generates a spatial layout, and exports a Factorio-importable blueprint string.

**[Try it in the browser](https://storkme.github.io/fucktorio/)** — full solver + bus layout + blueprint export runs client-side via WASM.

## Quick start

```bash
# Requires: uv (manages Python version + deps)
uv sync

# Run tests
uv run pytest tests/

# Generate a blueprint (Python pipeline)
uv run python -m src.pipeline
```

### Rust / WASM

The core logic (solver, bus layout, blueprint export, validation) is implemented in Rust (`crates/core/`) and compiled to both a Python extension (PyO3) and a WASM module for the web app.

```bash
# Check the Rust workspace
cargo check

# Build the WASM bundle
wasm-pack build crates/wasm-bindings --target web --out-dir ../../web/src/wasm-pkg
```

### Web app

Interactive browser UI (Vite + TypeScript + PixiJS). Runs the full pipeline client-side.

```bash
cd web
npm install
npm run dev        # Dev server on http://localhost:5173
```

## Architecture

Two parallel implementations share the same pipeline shape:

- **Python pipeline** (`src/`) — reference implementation, used for pytest and HTML visualizations
- **Rust pipeline** (`crates/core/`) — complete port, used by the WASM web app and PyO3 extension

Pipeline stages:

1. **Solver** — recursively resolves recipe dependencies, calculates machine counts and item flow rates
2. **Bus layout** — deterministic row-based layout with main bus pattern (machines in rows, parallel trunk belts, underground sideloading)
3. **Validation** — 21 functional checks (pipe isolation, fluid connectivity, inserter chains, power coverage, belt flow/structural, underground pairs, lane throughput)
4. **Blueprint** — serializes the layout to a Factorio-importable base64 string

## Recipe complexity ladder

| Tier | Recipe | Bus status |
|------|--------|------------|
| 1 | `iron-gear-wheel` | Solved |
| 2 | `electronic-circuit` (incl. from ores) | Solved |
| 3 | `plastic-bar` | Solved |
| 4 | `advanced-circuit` | Partial (lane-throughput warnings) |
| 5 | `processing-unit` | Not attempted |
| 6 | `rocket-control-unit` | Not attempted |

## Test visualizations

Generated on every push to main and deployed to GitHub Pages:
https://storkme.github.io/fucktorio/viz/
