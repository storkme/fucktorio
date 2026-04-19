# Phase F — In-place SAT-zone editor + fixture quality harness

Status: planned, fresh session pickup.
Depends on: Phase E v1 (fixture schema + test harness), landed on `main`.

## Goal

Build a corpus of hand-crafted SAT-zone fixtures, each pairing a real
problem instance (taken from a running layout) with a reference
solution the user considers "good." The solver is regression-tested
against this corpus on three axes: correctness (finds a solution at
all), quality (cost is at least as good as the reference), and
performance (total solve-time across the corpus doesn't drift).

The editor that produces these fixtures lives *in the existing web
app's PIXI canvas* — there is no separate `/fixture-builder` route.
Editing happens in place on the selected SAT zone, reusing the app's
entity rendering, viewport, and SAT overlay. The user spends their
time painting belts, not context-switching between views.

## Why this shape

### Use the existing canvas

We already render belts, UGs, splitters with direction + carries +
tier. Rebuilding that in a dedicated editor route duplicates code and
loses the surrounding context (the user can't see why the zone is
interesting if they're looking at it on a blank canvas). An "edit
mode" toggle on the selected zone costs ~60% less code than Phase F
as originally sketched.

### Reuse `solution_cost()` for quality

`crates/core/src/bus/junction_cost.rs` already defines the canonical
quality metric: belt=1, UG-in=5, UG-out=5. It's the same function the
solver's internal cost-descent loop minimises against. Using it for
fixture quality assertions keeps test and solver aligned — if the
solver's internal notion of "good" diverges from the test's notion of
"good," cost-descent would chase the wrong target.

### Factorio-grade drag mechanics

Painless editing is load-bearing. If the editor is clunky, the corpus
never grows. The bar: place a 12-tile belt run + auto-UG over an
obstacle in under 5 seconds. The rest of the fixture infrastructure
is worthless without this.

## Slices

### F1 — WASM `solve_fixture` binding

Expose the existing Rust SAT API to the web side so the editor can
re-solve the current painted state and show the user what SAT would
do. No UI changes in this slice — just plumbing.

New binding in `crates/wasm-bindings/src/lib.rs`:

```rust
#[wasm_bindgen]
pub fn solve_fixture(fixture_json: &str) -> Result<JsValue, JsValue> {
    // parse -> construct CrossingZone + constraints
    // call solve_crossing_zone_with_stats
    // return { entities: Vec<PlacedEntity>, cost: u32, stats: {...} } or null
}
```

Consumed by `web/src/engine.ts` as a new method:

```ts
solveFixture(json: string): Promise<{ entities: PlacedEntity[]; cost: number; stats: SatStats } | null>
```

#### Acceptance

- [ ] `cargo build --target wasm32-unknown-unknown --manifest-path crates/wasm-bindings/Cargo.toml` produces a bundle with the new export
- [ ] `npx tsc --noEmit` in `web/` clean after the `engine.ts` method is added
- [ ] Small integration smoke test: feed `sample_electronic_circuit.json` to `solveFixture`, assert a non-null result with `cost == 2` (two surface belts)

### F2 — Edit mode on the existing canvas

The inline panel's toolbar gains an "Edit" button (`✎` glyph). Clicking
enters edit mode for the currently-selected SAT zone's bbox.

#### State transitions

```
view mode        —[click Edit]→       edit mode
edit mode        —[Esc or Done]→      view mode
edit mode        —[click another zone]→  view mode (prompt to discard if dirty)
```

Edit mode is modal *within* the already-modal zone view. Clear visual
cue is mandatory: title bar of the inline panel shifts colour (yellow
tint?), cursor changes on the canvas, edit toolbar appears. No
ambiguity about which mode the user is in.

#### Canvas behaviour in edit mode

- **Entity layer dims further** (0.35 → 0.2 or so) so painted entities
  stand out against the existing stamp. The SAT overlay (bbox outline,
  boundary bars, forbidden hatch) stays at full brightness.
- **Painted entities render on a new layer** above the dimmed entity
  layer, using the existing entity renderer so they look identical to
  "real" belts. The layer is a writable working copy seeded from the
  current in-bbox entities.
- **Cursor** shows a ghost-preview of the current tool + direction at
  the hovered tile. Red tint if the placement would land on a
  boundary/forbidden tile.
- **Scope**: only tiles inside the selected bbox are editable. Hovering
  outside the bbox greys the cursor. Boundaries and forbidden tiles
  refuse placement.

#### Toolbar

Compact horizontal strip, bottom of the canvas or top of the inline
panel (decide at impl time based on screen real estate). Contents:

- Tool selector: Belt / UG-in / UG-out / Splitter / Erase
- Brush direction: N/E/S/W indicator, rotatable via `R`
- Item override: dropdown populated from the bbox's boundary items
  (painted belts carry this)
- Actions: Run SAT (live preview), Revert, Export Fixture, Done

Hotkeys mirror the toolbar plus muscle-memory shortcuts:

- `1/2/3/4/0` — tool select (belt/UG-in/UG-out/splitter/erase)
- `R` — rotate brush, AND while dragging, toggle the L-shape bend direction
- `Q` — pipette; pick the tool + direction from the tile under cursor
- `[` / `]` — prev/next item in the fixture's boundary item list
- `P` — run SAT on current painted state (live preview)
- `Ctrl+Z` / `Ctrl+Shift+Z` — undo / redo (in-memory stack; lost on reload)
- `Esc` — exit edit mode (prompt to discard unsaved changes)

#### Drag-to-paint

The core interaction. Must feel Factorio-tight.

1. **Mousedown** at tile A: record A, enter drag state, hide static
   cursor ghost.
2. **Mousemove** to tile B: compute an L-shaped path from A to B using
   the current "bend direction" (default: horizontal-first).
3. **Preview** the path as ghost entities — belts in brush direction,
   auto-forming UGs where the path crosses an obstacle (see below).
4. **R during drag**: toggle the bend direction (horizontal-first ↔
   vertical-first). Preview redraws.
5. **Mouseup**: commit the previewed entities to the edit layer. If
   any part of the path is invalid (refused UG, out-of-bounds), the
   whole placement fails and nothing commits — we don't want
   partial commits that leave the user confused.

#### Auto-UG over obstacles

This is the killer feature. The belt tier gives us `max_reach` (from
`common::ug_max_reach(belt_name)`). While painting a run:

1. Walk the L-shape tile-by-tile in drag order.
2. Classify each tile:
   - **Free**: placeable as a belt in the drag direction.
   - **Obstacle**: forbidden tile, or occupied by a non-belt permanent
     entity (splitter, machine, pole, pipe), or a boundary tile for a
     different item.
3. Partition the path into runs of consecutive Free tiles separated
   by runs of consecutive Obstacle tiles.
4. Each Obstacle run becomes a UG pair: UG-in at the last Free tile
   before the run, UG-out at the first Free tile after. The
   intervening Obstacle tiles stay untouched (we don't place anything
   on them — they're the tunnel interior).
5. Constraint: the distance from UG-in to UG-out must be ≤
   `max_reach + 1`. If an Obstacle run is too long to tunnel under,
   the whole drag fails and previews red. Split the Obstacle run into
   multiple pairs if there are intermediate Free tiles — multiple
   consecutive UG pairs is fine as long as each individual tunnel is
   within reach.

Edge cases:

- Drag ending inside an Obstacle run: refuse — needs a Free tile for
  the UG-out.
- Drag starting on an Obstacle: refuse — needs a Free tile for the
  UG-in.
- Obstacle run of length 0 (i.e. no obstacles): just a belt run, no
  UGs needed.

#### Live preview — run SAT on painted state

Hotkey `P` (or toolbar "Run SAT" button):

1. Serialise the current edit layer to a fixture JSON (same shape as
   Phase E export).
2. Call `engine.solveFixture(json)` from F1.
3. If result is non-null: replace the edit layer with the solver's
   entities, show a banner: "SAT replaced your layout · cost {N} (yours
   was {M})". Undo restores the user's painted state.
4. If null: show "SAT could not solve your setup" banner. No change to
   layer. Usually means the user erased an IN/OUT-connecting path.

This closes the loop — user paints, sees what SAT does with the same
constraints, iterates. Key insight: the editor becomes a ping-pong
between "here's my idea" and "here's SAT's interpretation."

#### Export Fixture

Toolbar "Export" button (or `E`):

1. Gather the painted entities (the edit layer).
2. Compute `max_cost = solution_cost(painted_entities)` via a web-side
   reimplementation matching `junction_cost.rs` (or expose a WASM
   helper — cheap call).
3. Build fixture JSON with all the existing fields (bbox, forbidden,
   belt_tier, boundaries, context.ghost_paths) plus:
   ```json
   "expected": {
     "mode": "solve",
     "max_cost": <computed>
   },
   "painted": {
     "entities": [ /* user's painted entities, for human reference */ ]
   }
   ```
4. Copy JSON to clipboard + download as `<name>.json`. Default name
   prompts the user.

`painted.entities` is informational — the test harness doesn't compare
against it. It's for future humans reading the fixture to see what the
author intended.

#### Test-harness update (minor)

`crates/core/tests/sat_fixtures.rs` gains one assertion for the new
quality axis:

```rust
if let Some(max_cost) = fixture.expected.max_cost {
    let actual_cost = solution_cost(&entities);
    if actual_cost > max_cost {
        failures.push(format!(
            "{name}: solver cost {actual_cost} exceeds fixture max_cost {max_cost}"
        ));
    }
}
```

Add `max_cost: Option<u32>` to the `Expected` struct in that file. When
absent (all current fixtures), no quality assertion — only the
mode-based check runs.

#### F2 acceptance

- [ ] Edit button enters edit mode, Esc/Done exits cleanly
- [ ] Drag-to-paint a straight horizontal belt run lays belts in drag direction
- [ ] Drag with an L-bend, R toggles bend direction live
- [ ] Drag over a 2-tile obstacle auto-forms a UG pair at the boundary
- [ ] Drag over a 10-tile obstacle with yellow belts (max_reach=4) refuses + flashes red
- [ ] `P` live-preview replaces the layer with SAT's output
- [ ] `Ctrl+Z` restores previous state
- [ ] Export produces a fixture JSON that re-loads via Phase E's test harness
- [ ] Fixture with `expected.max_cost` set fails the test if solver exceeds it, passes if solver matches or beats it

## What this unlocks

Once F1 + F2 land, the workflow is:

1. Run the app, spot a zone where SAT's output looks wasteful.
2. Click the zone → Edit → paint what you *think* the right layout is.
3. `P` to see what SAT does with the same constraints. Adjust.
4. Export → drop the JSON in `crates/core/tests/sat_fixtures/`.
5. `cargo test sat_fixtures` locks the bar in. If a future solver
   change increases cost on that zone, CI catches it.

Over time the corpus becomes a quality + performance benchmark we can
evaluate encoder tweaks against. The flywheel matters more than any
individual fixture.

## Open questions deferred to impl

- **Undo stack persistence**: in-memory is fine for v1. Serialise to
  session storage if it proves painful.
- **Multi-zone editing**: edit mode is single-zone. No plans for
  cross-zone editing — fixtures are scoped to a single SAT zone by
  design.
- **Fixture renaming**: users will want to rename after export. Leave
  that to the user's file manager; the browser download can default
  to the auto-generated name.
- **Version bump of fixture schema**: F2 adds `expected.max_cost` and
  `painted`. Both optional, schema stays at `version: 1`. Bump to 2
  only if a breaking change is introduced later.

## Not in scope for Phase F

- Benchmarking harness (per-fixture solve-time tracking + trend
  reports). Separate task — covered by the performance axis in intent
  but the timing infra is a different build-out.
- Batch fixture editing tools (bulk regenerate, diff two fixtures,
  etc.). Wait for the corpus to actually need these.
- Fixture sharing via URL (hash-encoded fixture in a shareable link).
  Nice but not load-bearing; skip until someone asks for it.
