# Layout Snapshot Debugger — Design Document

Turn any test failure (including hangs) into a shareable, loadable artifact that renders in the existing web debug UI. The goal: when a test fails, you should be able to *see* the factory that broke it in under a minute.

## Motivation

Debugging layout failures today is painful. The feedback loop is:

1. A test fails (or worse, hangs) on some recipe+rate+machine+belt combination
2. You guess at the cause from the error string
3. You reproduce locally by running the test, hoping it does the same thing
4. If the renderer is involved you manually paste the params into the web app sidebar, re-solve, hope the state matches

This falls apart completely for hanging tests, where you can't even get past step 1 without a debugger. And for shareable reports (GitHub issues, Slack) the best you can do today is a textual recipe spec.

The web app already has:
- A full trace event overlay (`web/src/renderer/traceOverlay.ts`)
- A debug stats panel (`web/src/ui/debugPanel.ts`)
- A validation error overlay (`web/src/renderer/validationOverlay.ts`)
- Entity rendering (`web/src/renderer/entities.ts`)

Everything you need to *display* a layout is already there. What's missing is a way to hydrate all of that from a file instead of re-running the solver.

## The core idea

Define a single self-describing binary format — a "layout snapshot" — that captures everything needed to reproduce a view of the layout without re-running the pipeline. Emit it from Rust on test failure (or on demand). Load it in the web app with a drag-drop or "Load snapshot" button.

Once loaded, every existing visualization — entity rendering, trace overlays, debug panel, validation markers — works as if the snapshot had just been generated live.

## Snapshot format

```
<snapshot blob> = "fls1" + base64(gzip(<json payload>))
```

Lead with a 4-byte magic prefix (`"fls1"` = "Fucktorio Layout Snapshot v1") so we can detect and version-check without parsing. Gzip-wrapped JSON keeps the blob small (a typical layout is 5–20 KB compressed, under 30 KB base64'd) and remains debuggable with standard tools.

### JSON payload schema

```typescript
interface LayoutSnapshot {
  version: 1;                           // format version
  created_at: string;                   // ISO 8601
  source: "test" | "manual" | "ci";     // origin tag

  params: {                             // enough to re-run if needed
    item: string;
    rate: number;
    machine: string;
    belt_tier: string | null;
    inputs: string[];
    module_config?: ModuleConfig | null;
  };

  context: {                            // human-readable identifier
    test_name?: string;                 // if dumped from a test
    label?: string;                     // user-provided label for manual dumps
    git_sha?: string;                   // repo HEAD when dumped
    rust_version?: string;
  };

  layout: LayoutResult;                 // same shape as WASM returns

  validation: {
    issues: ValidationIssue[];          // all errors + warnings
    truncated: boolean;                 // true if the validator panicked/timed out
  };

  trace: {
    events: TraceEvent[];               // whatever fired before the hang
    complete: boolean;                  // false if pipeline didn't finish
  };

  solver?: SolverResult | null;         // optional — the solver output
}
```

The `truncated` and `complete` flags are critical: for hanging tests, the snapshot might be incomplete, and the UI needs to know to warn the user "this trace stopped at phase X".

## Producer side (Rust)

### 1. Serialization helpers

New module `crates/core/src/snapshot.rs`:

```rust
pub struct LayoutSnapshot { /* fields above */ }

impl LayoutSnapshot {
    pub fn from_run(params: SnapshotParams, layout: &LayoutResult, ...) -> Self { ... }
    pub fn encode(&self) -> Result<String, SnapshotError> { /* "fls1" + b64(gzip(json)) */ }
    pub fn write_to_file(&self, path: &Path) -> std::io::Result<()> { ... }
}
```

The `LayoutResult`, `ValidationIssue`, and `TraceEvent` types already derive `Serialize` (for WASM), so serialization is free — just wrap them.

### 2. Test harness integration

Extend the e2e test helper at `crates/core/tests/e2e.rs`:

```rust
fn run_e2e_with_snapshot(...) -> Result<E2EResult, String> {
    let result = run_e2e(...)?;
    if env::var("FUCKTORIO_DUMP_SNAPSHOTS").is_ok() || result.issues.iter().any(is_error) {
        let snapshot = LayoutSnapshot::from_run(params, &result.layout, ...);
        let path = PathBuf::from(env::var("CARGO_TARGET_TMPDIR").unwrap())
            .join(format!("snapshot-{}.fls", test_name));
        snapshot.write_to_file(&path).ok();
        eprintln!("snapshot: {}", path.display());
    }
    Ok(result)
}
```

Opt-in via env var for passing tests; automatic for failing tests. Files land in `target/tmp/` where they're easy to find and gitignored by default.

### 3. Handling hangs

For hanging tests, we need a different strategy — if the test body hangs, the normal cleanup path never runs. Two options:

**(a) Background thread snapshot writer.** Spawn a thread at test start that periodically serializes the *current* trace buffer to a file. If the main thread hangs, the background thread's last write captures whatever made it out. Tradeoff: duplicates memory, adds test overhead.

**(b) Signal handler / panic hook.** Install a custom panic hook (triggered by `ntest::timeout`'s thread-kill) that dumps the snapshot before unwinding. Cleaner but relies on `ntest`'s timeout actually triggering a panic (it does — it uses `std::thread::spawn` + channel recv timeout, and kills the test thread with an unwind panic).

Recommend **(b)** — less invasive, works with our existing setup. The hook reads the thread-local trace collector (same one `buildLayoutTraced()` uses) and writes whatever events it has at timeout time.

### 4. Manual dumping

Add a small CLI under `crates/mining-cli/` or a new `crates/snapshot-cli/`:

```bash
cargo run -p snapshot-cli -- \
    --item electronic-circuit --rate 10 --machine assembling-machine-1 \
    --inputs iron-ore,copper-ore --belt-tier transport-belt \
    --output debug.fls
```

For reproducing issues without writing a test.

## Consumer side (Web app)

### 1. Snapshot loader UI

New file `web/src/ui/snapshotLoader.ts`. Two entry points:

- **Drag-drop target** on the main canvas — drop a `.fls` file anywhere on the viewport
- **"Load snapshot" button** in the sidebar header, near the existing "Debug" checkbox

Both call the same `loadSnapshot(blob: File | string)` function.

### 2. Snapshot decoder

```typescript
async function decodeSnapshot(input: string | ArrayBuffer): Promise<LayoutSnapshot> {
  const text = typeof input === "string" ? input : new TextDecoder().decode(input);
  if (!text.startsWith("fls1")) throw new Error("Not a layout snapshot");
  const b64 = text.slice(4);
  const gz = Uint8Array.from(atob(b64), c => c.charCodeAt(0));
  const json = new TextDecoder().decode(await gunzip(gz));  // pako or CompressionStream API
  return JSON.parse(json);
}
```

Use the browser's native `DecompressionStream` where available, falling back to `pako` for older browsers. Bundle size matters — `pako` is ~45 KB but we can gate the import.

### 3. Hydration path

New function in `web/src/main.ts`:

```typescript
function renderSnapshot(snap: LayoutSnapshot) {
  currentLayout = {
    layout: snap.layout,
    validation: snap.validation.issues,
    trace: snap.trace.events,
  };
  renderLayout(entityLayer, snap.layout);
  updateTraceOverlay();
  updateValidationOverlay();
  renderDebugPanel(snap.trace.events);
  showSnapshotBanner(snap);  // see below
}
```

Most of this is just calling existing render functions with the snapshot's data instead of solver output. The key insight: we bypass `engine.solve()` and `engine.layout()` entirely.

### 4. Snapshot banner

When a snapshot is loaded, show a non-intrusive banner at the top of the viewport:

```
📸 Snapshot: tier2_electronic_circuit_from_ore  (git: 5072a85, 2026-04-10)
   electronic-circuit @ 10/s · asm1 · yellow belt · from iron-ore, copper-ore
   ⚠ Incomplete trace — layout stopped at phase "route" (hang timeout)
   [Re-solve from params] [Clear snapshot]
```

The banner has two actions:
- **Re-solve from params** — run the pipeline live with the snapshot's params. Useful for "did my fix work?"
- **Clear snapshot** — returns to normal sidebar-driven solving

### 5. Sidebar behavior

When a snapshot is loaded, grey out the sidebar item picker — the current view is a frozen snapshot, not a live solve. Typing in the sidebar either does nothing or prompts "Clear snapshot to start a new solve?".

### 6. Warnings for incomplete snapshots

If `trace.complete === false`, the debug panel shows a warning banner at the top:

> ⚠ **Incomplete trace** — the snapshot was captured during a pipeline failure or timeout. Trace events end at phase `{last_phase}`. Validation issues may be missing.

If `validation.truncated === true`, the validation overlay shows a similar warning and the debug panel's validation section is marked as partial.

## Use cases

### Debugging hanging tests

1. Test hangs, `ntest` timeout fires, panic hook writes `target/tmp/snapshot-tier2_electronic_circuit_from_ore.fls`
2. Developer drags the file into the web app
3. They see the layout that was being validated when the hang happened
4. They see the trace events up to the hang — "last event: `NegotiateComplete` at iteration 3"
5. They can now zoom in on the specific belt topology that's tripping the validator

### Sharing failure reports

Someone in chat says "electronic-circuit 20/s from ore fails with lane-throughput errors". Instead of describing it, they attach a `.fls` file. Anyone can load it and see exactly what they saw — no repro-hunting.

### Regression investigation

When a previously-passing test starts failing, capture snapshots on the good and bad commits. Diff them visually side-by-side. The same snapshot format supports the existing trace debug comparison view (Phase 4 of `trace-debug-ui.md`).

### Offline review / slow internet

Snapshots are 5–30 KB. You can email them, paste them in a gist, drop them in a Google Doc. They're durable artifacts that don't require the backend pipeline to view.

## Phased implementation

### Phase 1 — Snapshot format + Rust producer (2–3 days)

- [ ] Define `LayoutSnapshot` struct in `crates/core/src/snapshot.rs`
- [ ] Implement `encode()` / `decode()` with `"fls1"` magic + gzip + base64
- [ ] Add `write_to_file()` helper
- [ ] Unit tests for round-trip encode/decode
- [ ] Tiny `snapshot-cli` binary for manual captures

**Validation:** Hand-run the CLI on `electronic-circuit` at 10/s, open the file, confirm it's the expected shape.

### Phase 2 — Test integration (1–2 days)

- [ ] Wrap `run_e2e` to dump snapshots on failure
- [ ] Opt-in env var for dumping passing tests too
- [ ] Panic hook integration with `ntest::timeout` so hanging tests also dump
- [ ] Verify the panic hook actually fires on timeout (write a deliberately-hanging test as a check)

**Validation:** Run the e2e suite. Failing/hanging tests leave `.fls` files in `target/tmp/`. Successful tests don't (unless `FUCKTORIO_DUMP_SNAPSHOTS=1`).

### Phase 3 — Web app loader + hydration (2–3 days)

- [ ] `snapshotLoader.ts` with drag-drop + button
- [ ] `decodeSnapshot()` using `DecompressionStream` / `pako`
- [ ] `renderSnapshot()` hydration path in `main.ts`
- [ ] Snapshot banner UI
- [ ] Sidebar greying when snapshot is loaded

**Validation:** Drop a `.fls` file from Phase 2 into the web app. See the layout render. See trace overlays populate. See validation markers appear.

### Phase 4 — Polish & convenience (1 day)

- [ ] Re-solve-from-params button wires up to existing `engine.solve()` + `engine.layout()`
- [ ] Incomplete trace warnings
- [ ] Keyboard shortcut (`Ctrl+O` to open snapshot file picker)
- [ ] URL fragment support — drop a base64 snapshot in `#snapshot=…` and the app loads it (useful for sharing via URL; size-limited by browser)

**Validation:** All polish items work end-to-end. A teammate can receive a URL and see the same view you do.

## Key design decisions

### Why a single blob and not a directory of files?

A single self-describing blob is shareable (drag-drop, email, gist). A directory needs zipping anyway. The 30KB ceiling easily covers realistic layouts.

### Why JSON over MessagePack / binary?

Debuggability. You can `gunzip | jq` a snapshot to inspect it without any tooling. For 30KB payloads the size overhead is negligible and the format version is dead simple to evolve.

### Why magic prefix?

Distinguishes our snapshots from arbitrary base64 on the clipboard. Lets us add `fls2` later without breaking old blobs. Lets the web app reject non-snapshot drops with a clear error.

### Why include both `layout` and `solver`?

`layout` is needed to render. `solver` is useful for the debug panel's "Solver Summary" section and for the re-solve button. It's optional because for hang-time snapshots the solver may have completed but the layout phase didn't finish writing its output — so include whatever's available.

### Why store git sha and rust version?

Provenance. "This snapshot was captured at commit X" is essential for understanding whether a regression is your fault or inherited. Cheap to include.

## Open questions

1. **Where do `.fls` files land in CI?** Probably uploaded as artifacts on failure via `actions/upload-artifact`. Needs a CI workflow change.
2. **Should snapshots be loadable in the Pixi viewport alone, or only through the full app?** Starting with "full app" is simpler — the viewport needs the debug panel to show real context.
3. **What about multi-snapshot comparison?** Deferred. Phase 4 of `trace-debug-ui.md` already describes run-history comparison — that mechanism could be extended to compare snapshots rather than re-runs.
4. **Security / sandboxing** — loading arbitrary JSON from a file is low-risk (no `eval`), but we should still validate the shape defensively in `decodeSnapshot` before passing to renderers. Untrusted `TraceEvent` unions with wrong discriminants could crash the UI.
5. **Do snapshots need to be diffable as text?** If yes, maybe emit a companion `.fls.json` (ungzipped) alongside the binary for git-diff-friendliness. Decide based on actual use.

## Non-goals

- **Full blueprint replay.** A snapshot is a frozen view, not a time-travel debugger. If you want to step through the pipeline again, use the existing step-through debugger.
- **Snapshot editing.** You can't modify a loaded snapshot in the UI. Re-solve with adjusted params instead.
- **Long-term storage.** Snapshots are artifacts for debugging, not a database. They're meant to be thrown away after the bug is fixed.

## Related docs

- [`docs/trace-debug-ui.md`](trace-debug-ui.md) — the trace overlay + debug panel that this feature hydrates
- [`crates/core/src/trace.rs`](../crates/core/src/trace.rs) — the 22 trace events that populate the debug panel
- [`web/src/renderer/validationOverlay.ts`](../web/src/renderer/validationOverlay.ts) — the validation markers the snapshot populates
