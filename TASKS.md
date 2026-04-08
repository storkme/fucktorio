# Future Work

Tracks for upcoming exploration and development. Items are rough — refine as we dig in.

## Visual Interactivity

The web app renderer shows entities but doesn't let you inspect or understand the layout interactively.

- [ ] Lane/belt highlight on hover — click or hover a belt segment to highlight the full connected belt network (same `segment_id` or flood-fill by adjacency)
- [ ] Item overlay — color belts by the item they carry (`carries` field), with a legend
- [ ] Throughput overlay — show per-lane throughput numbers (from solver rates) as text or heatmap on belt segments
- [ ] Hover tooltip — show entity details on hover (name, direction, carries, segment_id, coordinates)
- [ ] Machine info panel — click a machine to see its recipe, input/output rates, inserter assignments

## Factory Footprint: Special Cases & Community Blueprint Mining

The generated layout follows a general-purpose bus pattern, but many recipes have structural properties that allow significantly more compact layouts. We have a collection of downloaded community blueprints that likely demonstrate these optimisations in practice.

- [ ] **Audit downloaded blueprints** — run the analysis tools over our community blueprint collection and look for layout patterns that differ structurally from our generic bus (e.g. machine-to-machine direct insertion, shared inserters, non-standard row groupings)
- [ ] **Identify candidate "special cases"** — brainstorm and document recipe families or pipeline shapes that could be handled more compactly than the general bus (e.g. smelting columns, circuit lines with direct hand-off, fluid recipes with mirrored machines)
- [ ] **Direct machine-to-machine insertion** — when a recipe's sole consumer is a single adjacent recipe, skip the bus entirely and use a direct inserter hand-off (already exists in spaghetti engine; port the concept to bus placer)
- [ ] **Implement and test winning patterns** — pick the highest-impact special case from the audit, implement it as a layout strategy, verify with the recipe complexity ladder

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

## Blueprint Mining & Corpus Analysis

Rust analysis pipeline (`crates/core/src/analysis.rs`) and CLI tool (`crates/mining-cli/`) parse community blueprints, detect production chains, and estimate module-aware throughput. The Python pipeline (`src/analysis/`, `scripts/analysis/mine_corpus.py`) does deeper network tracing (belt BFS, inserter resolution, item inference) and exports `corpus.json` for the web app corpus browser.

- [ ] Port remaining Python analysis to Rust — network tracing (BFS over belt/pipe segments), inserter resolution, item inference, bus layout detection. Python uses `draftsman` for entity data; Rust already has entity sizes and recipes.
- [ ] Merge Python `mine_corpus.py` output with Rust analysis — or replace entirely once the Rust analysis covers the same ground
- [ ] UMAP/scatter explorer — optional `--umap` output for `mine_corpus.py` using `umap-learn` + `plotly`. Cheap way to visualize blueprint clusters without ML.

### Done

- [x] Rust blueprint parser (`blueprint_parser.rs`) — decodes all Factorio blueprint formats including books (recursive), handles 1.x and 2.0 entity formats, parses module items
- [x] Rust analysis module (`analysis.rs`) — entity classification, recipe grouping, production chain DFS, final product detection, module+beacon-aware throughput, furnace recipe inference
- [x] Rust CLI tool (`crates/mining-cli/`, binary `blueprint-analyze`) — single/batch/book modes, JSON output, reads from files or stdin, handles JSON-wrapped blueprint strings
- [x] Python analysis pipeline with bus detection, row clustering, and 40+ extracted metrics
- [x] Web app corpus browser tab with paste-to-render (WASM) and file-based corpus browsing

## Modules & Beacons

Blueprint analysis now parses modules from entities and calculates module+beacon-aware throughput. The remaining work is on the solver and layout generation side.

### Solver — module-aware machine counts

The solver (`crates/core/src/solver.rs`) calculates machine counts at base crafting speed. With modules, you need significantly fewer machines for the same throughput.

- [ ] Add `ModuleConfig` parameter to `solve()` — specify internal modules (e.g. 4× productivity-module-3) and beacon setup (count per machine, module type in beacons)
- [ ] Adjust machine count formula: `machines = target_rate / (effective_speed × recipe_output × effective_productivity / energy)` where `effective_speed = base_speed × (1 + internal_speed_bonus + beacon_count × beacon_module_speed × 0.5)` and `effective_productivity = 1 + internal_prod_bonus`
- [ ] Wire through WASM bindings so the web app can use it
- [ ] Web UI: module/beacon picker in sidebar (machine module dropdown, beacon count per machine, beacon module type)

### Layout — beacon placement

Generating layouts with beacons is a significant layout engine change. Common community patterns:

- [ ] Research common beacon arrangements — 8-beacon (2 rows of 4 flanking machines), 12-beacon (fully surrounded), inline beacon rows between machine rows
- [ ] Beacon row templates — extend `bus/templates.rs` with beacon-row variants that interleave beacon entities between machine rows
- [ ] Row dimension changes — beaconed rows are much wider (beacons add 3+ tiles on each side) and taller (beacon rows between machine rows). Pitch calculations in `bus/placer.rs` need updating.
- [ ] Power requirements — beaconed builds need far more power; pole placement (`bus/bus_router.rs`) may need substations instead of medium poles
- [ ] Module assignment in export — when generating blueprints, populate the `items` field on machines and beacons with the configured modules

### Done

- [x] Parse module items from blueprint entities (three Factorio formats: 1.x array, 1.x map, 2.0 nested id)
- [x] Module effect data in `common.rs` — speed/productivity bonuses for all 12 module types, module slot counts per machine
- [x] Beacon proximity detection — AABB overlap between beacon 9×9 supply area and machine footprint
- [x] Module-aware throughput in analysis — per-machine effective speed and productivity, beacon bonuses at 50% distribution effectivity
- [x] CLI output shows `[speed +X%, prod +Y%]` when modules are present

## Research: Verifactory

[verifactory](https://github.com/alegnani/verifactory) is a Rust library for formal verification of Factorio blueprints using bounded model checking. Worth investigating what it validates that we don't — particularly around flow rate analysis.

- [ ] Dig into verifactory's validation approach — what properties does it check? How does it model belt throughput, splitter ratios, and item flow rates?
- [ ] Compare with our validation suite — identify gaps, especially around flow rate correctness (we check structural connectivity but not whether items arrive at the right rate)
- [ ] Evaluate integration potential — could we use verifactory as a post-layout verification pass, or borrow its modeling approach for our own validators?
