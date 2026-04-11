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

Each row corresponds to an e2e test in `crates/core/tests/e2e.rs` that runs the full pipeline (solve → layout → export → round-trip parse → validate). A test must produce zero validation errors **and** zero warnings to pass. Belt tier is auto-selected from rate unless explicitly constrained.

| Tier | Recipe @ rate | Inputs | Machine | Belt | E2E test | Status |
|------|---------------|--------|---------|------|----------|--------|
| 1 | iron-gear-wheel @ 10/s | iron-plate | AM1 | yellow (auto) | `tier1_iron_gear_wheel` | :white_check_mark: |
| 1 | iron-gear-wheel @ 10/s | iron-ore | AM2 | yellow (auto) | `tier1_iron_gear_wheel_from_ore` | :white_check_mark: |
| 1 | iron-gear-wheel @ 20/s | iron-plate | AM2 | red (auto) | `tier1_iron_gear_wheel_20s` | :white_check_mark: |
| 2 | electronic-circuit @ 10/s | iron-plate + copper-plate | AM2 | auto | `tier2_electronic_circuit` | :x: 2× lane-throughput at (4,8) (15/s on yellow) |
| 2 | electronic-circuit @ 10/s | iron-ore + copper-ore | AM1 | yellow (forced) | `tier2_electronic_circuit_from_ore` | :warning: 4× belt-direction dead spots + 35× copper-plate input-rate-delivery (rate propagation doesn't reach assembler rows at y=29) |
| 2 | electronic-circuit @ 20/s | iron-ore + copper-ore | AM2 | auto | `tier2_electronic_circuit_20s_from_ore` | :x: 1× entity-overlap + 1× belt-dead-end + 1× belt-item-isolation + 6× lane-throughput |
| 3 | plastic-bar @ 10/s | petroleum-gas + coal | chem-plant | auto | `tier3_plastic_bar` | :white_check_mark: |
| 3 | plastic-bar @ 10/s | crude-oil + coal (via refinery) | chem-plant | auto | `tier3_plastic_bar_from_crude` | :white_check_mark: |
| 3 | sulfuric-acid @ 5/s | iron-plate + sulfur + water | chem-plant | auto | `tier3_sulfuric_acid` | :white_check_mark: |
| 4 | advanced-circuit @ 10/s | iron/copper-plate + plastic-bar | AM2 | auto | `tier4_advanced_circuit_from_plates` | :x: 4× entity-overlap + 2× unpaired UG belt + 4× belt-item-isolation + 8× lane-throughput |
| 5 | processing-unit | — | — | — | — | Not attempted |
| 6 | rocket-control-unit | — | — | — | — | Not attempted |

**Legend:** :white_check_mark: passing · :x: validation errors · :warning: warnings only (test currently `#[ignore]`d)

## Test visualizations

Generated on every push to main and deployed to GitHub Pages:
https://storkme.github.io/fucktorio/viz/
