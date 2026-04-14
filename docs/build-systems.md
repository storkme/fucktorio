# Build Systems

## Rust workspace

```bash
cargo check          # type-check all crates (fast)
cargo test           # run all Rust tests
cargo clippy         # lint (also runs in pre-commit hook if .rs files staged)
```

The workspace has three crates:

| Crate | Role |
|-------|------|
| `crates/core/` | Pure shared logic: models, solver, recipe DB, blueprint export, A*, bus layout, validation. The `wasm` feature gates `tsify-next` / `wasm-bindgen` derives. |
| `crates/wasm-bindings/` | wasm-bindgen wrapper → browser WASM module. |
| `crates/mining-cli/` | CLI for exploring mined-blueprint corpora. |

### WASM bundle

```bash
wasm-pack build crates/wasm-bindings --target web --out-dir "$(pwd)/web/src/wasm-pkg"
```

Always pass an absolute `--out-dir` — relative paths resolve from the
crate dir, not `cwd`. Outputs into `web/src/wasm-pkg/` and is consumed
by `web/src/engine.ts`. Exposes `solve`, `layout`, `layout_traced`,
`export_blueprint`, `validate_layout`, and the recipe lookup helpers.

## Web app (`web/`)

Stack: Vite + vanilla TypeScript + PixiJS v8 + pixi-viewport.

```bash
cd web
npm install        # install deps
npm run dev        # Vite dev server at http://localhost:5173
npm run build      # tsc --noEmit && vite build (produces web/dist/)
```

See `docs/web-app-plan.md` for design context.

## Scripts

`scripts/` contains a few Python and Node helpers. None of them are
required for a normal build, but they're useful for data refresh and
snapshot inspection:

- `generate_balancer_library.py` — regenerates `balancer_library.rs`
  from Factorio-SAT. Needs Factorio-SAT on `PATH`.
- `extract_factorio_data.py` — pulls recipes/entity data from
  `factorio-draftsman` into the JSON embedded in Rust.
- `extract_icons.py`, `extract_entity_frames.py` — asset extractors
  for the web app.
- `analyze_ghost_crossings.py`, `debug_validator_rates.py`,
  `dump_ghost_path.py`, `dump_tiles_at.py`, `inspect_ghost_spec.py`
  — stdlib-only decoders for `.fls` snapshot files from the Rust
  pipeline. See `docs/layout-snapshot-debugger.md`.
- `test_wasm.mjs`, `test_wasm_bisect.mjs` — Node.js drivers for
  timing the WASM module outside the browser.
