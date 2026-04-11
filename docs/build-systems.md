# Build Systems

## Python

```bash
uv sync                          # install/update deps
uv run pytest tests/             # run full test suite
uv run pytest tests/ --viz       # run tests + generate HTML visualizations in test_viz/
uv run python -m src.pipeline    # generate a blueprint
uv run python scripts/<name>.py  # run a one-off script
```

## Rust workspace

```bash
cargo check          # type-check all crates (fast)
cargo test           # run all Rust tests
cargo clippy         # lint (also runs in pre-commit hook if .rs files staged)
```

### PyO3 extension (`crates/pyo3-bindings/`)

Compiled into `fucktorio_native.so` and imported by the Python pipeline for A\* pathfinding and
lane negotiation.

```bash
uv run maturin develop --manifest-path crates/pyo3-bindings/Cargo.toml
```

> **Note:** Editable installs don't always refresh `target/*.so`. If changes aren't picked up,
> copy the compiled `.so` manually. See agent memory `feedback_maturin.md`.

Exposes to Python: `astar_path`, `negotiate_lanes`.

### WASM bundle (`crates/wasm-bindings/`)

Used by the web app. Outputs into `web/src/wasm-pkg/`.

```bash
wasm-pack build crates/wasm-bindings --target web --out-dir ../../web/src/wasm-pkg
```

Exposes to the browser: `solve`, `layout`, `export_blueprint`, recipe lookups. Loaded by
`web/src/engine.ts`.

## Web app (`web/`)

Stack: Vite + vanilla TypeScript + PixiJS v8 + pixi-viewport.

```bash
cd web
npm install        # install deps (or pnpm / bun)
npm run dev        # Vite dev server
npm run build      # tsc --noEmit && vite build
```

See `docs/web-app-plan.md` for design context.

## Crate structure

| Crate | Role |
|-------|------|
| `crates/core/` | Pure shared logic: models, solver, recipe DB, blueprint export, A\*, bus layout, validation. The `wasm` feature gates `tsify-next`/`wasm-bindgen` derives so `core` compiles for both PyO3 and WASM. |
| `crates/pyo3-bindings/` | Thin PyO3 adapter → `fucktorio_native.so` |
| `crates/wasm-bindings/` | wasm-bindgen wrapper → browser WASM module |
