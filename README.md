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

| Tier | Recipe | Inputs | Machine | Belt | E2E test | Status |
|------|--------|--------|---------|------|----------|--------|
| 1 | iron-gear-wheel | iron-plate | AM1 | yellow | `tier1_iron_gear_wheel` | :white_check_mark: |
| 1 | iron-gear-wheel | iron-ore | AM2 | yellow | `tier1_iron_gear_wheel_from_ore` | :white_check_mark: |
| 2 | electronic-circuit | iron/copper-plate | AM2 | yellow | `tier2_electronic_circuit` | :x: lane-throughput |
| 2 | electronic-circuit | iron/copper-ore | AM1 | yellow | `tier2_electronic_circuit_from_ore` | :white_check_mark: |
| 3 | plastic-bar | petroleum-gas, coal | chem-plant | yellow | `tier3_plastic_bar` | :white_check_mark: |
| 4 | advanced-circuit | iron/copper-plate, plastic-bar | AM2 | yellow | `tier4_advanced_circuit_from_plates` | :x: lane-throughput |
| 5 | processing-unit | — | — | — | — | Not attempted |
| 6 | rocket-control-unit | — | — | — | — | Not attempted |

Each row corresponds to an e2e test in `crates/core/tests/e2e.rs` that runs the full pipeline (solve, layout, export, round-trip parse, validate). Status reflects zero-error validation on the generated blueprint.

## Test visualizations

Generated on every push to main and deployed to GitHub Pages:
https://storkme.github.io/fucktorio/viz/
