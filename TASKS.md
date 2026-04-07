# Future Work

Tracks for upcoming exploration and development. Items are rough — refine as we dig in.

## Visual Interactivity

The web app renderer shows entities but doesn't let you inspect or understand the layout interactively.

- [ ] Lane/belt highlight on hover — click or hover a belt segment to highlight the full connected belt network (same `segment_id` or flood-fill by adjacency)
- [ ] Item overlay — color belts by the item they carry (`carries` field), with a legend
- [ ] Throughput overlay — show per-lane throughput numbers (from solver rates) as text or heatmap on belt segments
- [ ] Hover tooltip — show entity details on hover (name, direction, carries, segment_id, coordinates)
- [ ] Machine info panel — click a machine to see its recipe, input/output rates, inserter assignments

## Layout Improvements

The bus layout works but is wider/taller than necessary in many cases.

- [ ] Compact row spacing — current inter-row gap is fixed; investigate tighter packing where inserter reach allows
- [ ] Bus width optimization — lanes are currently 1-tile spaced; explore whether reordering lanes to minimize tap-off underground hops reduces total width
- [ ] Dead lane elimination — remove bus lanes that carry zero throughput after splitting/balancing
- [ ] Multi-tile entity positioning — assemblers and chemical plants use center-offset positioning; verify blueprint export handles 3x3 and 3x3+ footprints correctly (there's a TODO in blueprint.rs)
- [ ] Row merging — when two recipe groups have compatible input sets, consider placing them in adjacent rows sharing tap-offs

## Space Age Machine Support (Rust Pipeline)

The Rust solver and bus layout engine don't handle Space Age (DLC) machines. Three areas need fixes before Space Age recipes can be solved and laid out correctly.

### `crates/core/src/recipe_db.rs` — category → machine mapping

`machine_for_recipe()` only maps `chemistry`, `oil-processing`, `crafting`, `smelting` etc. All Space Age categories fall through to the default assembling machine. Add:

- [ ] `"electromagnetics"` → `"electromagnetic-plant"`
- [ ] `"cryogenics"` → `"cryogenic-plant"`
- [ ] `"metallurgy"` → `"foundry"`
- [ ] `"organic"` → `"biochamber"`
- [ ] `"recycling"` → `"recycler"`
- [ ] `"crushing"` → `"crusher"`
- [ ] Hybrid categories (`"cryogenics-or-assembling"`, `"organic-or-assembling"`, `"metallurgy-or-assembling"`) — prefer the Space Age machine, fall back to assembling-machine-3

### `crates/core/src/common.rs` — machine sizes and entity name list

`machine_size()` returns 3 for all unrecognised slugs (wrong for non-3×3 Space Age machines). `MACHINE_ENTITY_NAMES` doesn't include Space Age machines so they never appear in layouts.

- [ ] Add `machine_size()` cases: `electromagnetic-plant` → 4, `cryogenic-plant` → 5, `foundry` → 5, `recycler` → (2, 4), `crusher` → (2, 3) (recycler/crusher are non-square — may need `machine_size()` to return `(w, h)` instead of a single `u32`)
- [ ] Add Space Age slugs to `MACHINE_ENTITY_NAMES`: electromagnetic-plant, cryogenic-plant, foundry, biochamber, recycler, crusher

### `crates/core/src/bus/placer.rs` — row pitch and row kind

`row_kind()` only special-cases `oil-refinery`; all Space Age machines get treated as 3×3 assemblers. Machine pitch is hardcoded as oil-refinery=5, else=3.

- [ ] Fix `row_kind()` to check machine footprint (via `machine_size()`) rather than slug name, so 5×5 machines get the same wide-row treatment as oil-refinery
- [ ] Fix pitch calculation to use `machine_size().max_dim()` so 4×4 and 5×5 machines get correct inter-row spacing
- [ ] Handle non-square machines (recycler 2×4, crusher 2×3) — inserter placement and pitch may need separate w/h

## Kovarex

Kovarex enrichment process (`kovarex-enrichment-process`) is a self-feeding recipe: it consumes 40 U-235 + 5 U-238 and produces 41 U-235 + 2 U-238. The solver currently doesn't handle cyclic recipes.

- [ ] Research how community builds handle kovarex (centrifuge arrays with productivity modules, uranium processing chains)
- [ ] Solver support for cyclic/self-feeding recipes — the solver needs to detect and handle the net-positive loop (1 U-235 + consumed U-238 per cycle)
- [ ] Layout support — kovarex machines need belt loops or chest buffers to recirculate U-235; this is fundamentally different from the current one-directional bus pattern

## Debugging / Tracing

Layout bugs are hard to diagnose because the placement engine doesn't emit intermediate state.

- [ ] Trace events for bus router — emit structured events (JSON or log lines) for each phase: lane planning, balancer stamping, feeder routing, tap-off placement, trunk segments
- [ ] A* path visualization — optionally dump explored nodes and final path for each A* call, renderable as an overlay in the web app
- [ ] Negotiation round logging — `negotiate_lanes` runs multiple A* iterations with congestion updates; log per-round congestion maps and path changes
- [ ] Validation issue overlay — render validation errors/warnings as positioned markers on the web app canvas (using `x`/`y` from `ValidationIssue`)
- [ ] Export intermediate layout states — snapshot `LayoutResult` at each routing phase for step-through debugging

## Visual Polish: Game Icons & Entity Graphics

The web app currently renders entities as colored rectangles. Using actual Factorio sprites/icons would make layouts much easier to read and validate visually.

- [ ] Research sprite extraction — Factorio game assets are in `.png` sprite sheets; figure out legal/practical way to extract or reference them (factorio-icon-browser, wiki assets, or ship a curated subset)
- [ ] Item icons in sidebar — show item icons next to names in the item picker, input checkboxes, and solver result display
- [ ] Entity sprites on canvas — render machines, belts, inserters, pipes etc. with their Factorio sprites instead of colored boxes (direction-aware)
- [ ] Belt direction indicators — at minimum, add arrow overlays showing belt flow direction if full sprites aren't feasible
- [ ] Recipe icons — show recipe icons on machines (the output item icon overlaid on the machine sprite)

## Research: Verifactory

[verifactory](https://github.com/alegnani/verifactory) is a Rust library for formal verification of Factorio blueprints using bounded model checking. Worth investigating what it validates that we don't — particularly around flow rate analysis.

- [ ] Dig into verifactory's validation approach — what properties does it check? How does it model belt throughput, splitter ratios, and item flow rates?
- [ ] Compare with our validation suite — identify gaps, especially around flow rate correctness (we check structural connectivity but not whether items arrive at the right rate)
- [ ] Evaluate integration potential — could we use verifactory as a post-layout verification pass, or borrow its modeling approach for our own validators?
