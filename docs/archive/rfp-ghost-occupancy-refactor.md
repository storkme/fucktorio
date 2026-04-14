# RFP: Shared occupancy map for ghost routing

**Status:** Draft — Step 1 in progress
**Owner:** ghost routing pipeline
**Tracking issue:** *(none yet)*
**Related:** [`docs/rfp-ghost-cluster-routing.md`](rfp-ghost-cluster-routing.md), [`docs/rfp-junction-solver.md`](rfp-junction-solver.md)

## Problem

The ghost routing pipeline in `crates/core/src/bus/ghost_router.rs` has three solver phases that resolve crossings between bus belts:

1. **Phase 6a-corridor** (~line 747) — stamps multi-tile UG bridges for runs of adjacent crossings on the same horizontal spec.
2. **Phase 6a-pertile** (~line 957) — stamps single-crossing perpendicular templates (3-tile footprint: surface belt + UG-in + UG-out).
3. **Phase 6b-SAT** (~line 1011) — clusters remaining crossings, pads each by ±2 tiles, runs the SAT crossing-zone solver per cluster.

Each phase has its own view of "what tiles are occupied":

| Phase | Reads obstacles from | Writes results to |
|---|---|---|
| Ghost A* (step 5) | `&hard`, `&existing_belts` | `routed_paths` (paths only) |
| Corridor template (6a) | `&hard`, `pre_existing_set`, `routed_paths` | `entities` (mutated immediately) |
| Per-tile template (6a-pertile) | `&hard`, `pre_existing_set`, `routed_paths` | `template_zones` (deferred) |
| SAT (6b) | `&hard`, `&entities`, `routed_paths` | `sat_zones` (deferred) |

Because per-tile templates and SAT both **defer** their writes (push to side vecs that aren't merged into `entities` until line 1093, after `resolve_clusters` returns), the SAT phase cannot see per-tile template footprints when building its `forced_empty` set at lines 1820-1841.

### Concrete failure mode

Reproduced reliably by `tier2_electronic_circuit_from_ore_ghost` at 30/s on `transport-belt` (22 conflicting cluster pairs) and observed in `tier4_advanced_circuit_from_ore_am1_ghost` (2 pairs, including the user-reported clusters 11/4 and 25/34/60).

The fingerprint is consistent: a 1×3 per-tile template at `(x, y)` overlapped by a 5×5 SAT zone sourced from a different crossing tile at `(x+1, y+1)` (single-tile crossing → +2 padding → 5×5 bbox). The SAT zone's footprint engulfs the template's UG-out tile. SAT computes its solution unaware of the template, claims tiles the template is about to claim, and the post-hoc retain pass at line 1098 silently drops the conflicting SAT entities — leaving disconnected SAT belt fragments that don't physically connect to anything.

The bug is **not** padding-induced overlap inside SAT (the line 1648 comment "no zone merging — overlapping zones produce conflicting SAT solutions" diagnoses a different problem). It is a **coordination failure between the template and SAT phases** caused by deferred writes.

## Why a local fix isn't enough

Three smaller fixes were considered:

- **Inject template entities into `entities` before `resolve_clusters`.** Smallest possible change. Works for the immediate symptom but doesn't address the underlying issue: the per-phase obstacle representation will continue to drift as new template kinds are added. Each new phase needs to know about every other phase's footprint via ad-hoc parameter passing.
- **Pass `pertile_template_zone_bboxes` into `resolve_clusters` and skip overlapping SAT zones.** Adds a special case in the SAT path. Doesn't generalise.
- **Use a shared occupancy map.** Single source of truth for "what tiles are claimed and by what". This RFP.

The shared-map approach is justified because:

1. The SAT layer **already** supports the primitive we need: `CrossingZone::forced_empty` (`sat.rs:33`) is honoured by the encoder at lines 676-686 with three negative-literal clauses per tile (`!is_belt`, `!is_ug_in`, `!is_ug_out`). No SAT API changes required.
2. The corridor template phase **already** writes immediately into `entities`. Per-tile templates and SAT are the outliers; the refactor brings them in line with corridor phase behaviour.
3. The current `ghost:*`-segment-id-prefix string filtering at lines 729, 1072, 1836 is a poor man's claim-kind discriminator. A real claim-kind enum will simplify those checks rather than complicate them.

## Design

Add a private helper in `crates/core/src/bus/ghost_occupancy.rs` (new module).

```rust
pub(super) struct Occupancy {
    /// Immutable obstacles: machines, poles, fluid-lane reservations.
    /// A* never routes through these; templates and SAT never claim these.
    hard: FxHashSet<(i32, i32)>,

    /// All placed entities so far, in placement order. Indices are stable.
    entities: Vec<PlacedEntity>,

    /// Per-tile claim record. One entry per (x,y) that has a current claim.
    /// HardObstacle takes precedence; entity claims point to indices in `entities`.
    claims: FxHashMap<(i32, i32), Claim>,
}

pub(super) enum Claim {
    /// Machine, pole, fluid lane, splitter footprint. Cannot be displaced.
    HardObstacle,
    /// Trunk, row template belt, balancer block. Permanent for the duration
    /// of ghost routing — templates and SAT must route around these.
    Permanent { entity_idx: usize },
    /// Surface belt placed by ghost A* during step 5. May be replaced by a
    /// template UG-bridge or SAT solution that runs through this tile.
    GhostSurface { entity_idx: usize },
    /// Stamped by corridor or per-tile template. Permanent once placed —
    /// SAT must treat these as forced_empty.
    Template { entity_idx: usize },
    /// SAT-solved. Final, set in stone.
    SatSolved { entity_idx: usize },
}
```

### Methods

```rust
impl Occupancy {
    fn new(hard: FxHashSet<(i32, i32)>, initial_entities: Vec<PlacedEntity>) -> Self;

    // --- Queries ---
    fn is_hard_obstacle(&self, tile: (i32, i32)) -> bool;
    fn is_free(&self, tile: (i32, i32)) -> bool;             // no claim at all
    fn claim_at(&self, tile: (i32, i32)) -> Option<&Claim>;
    fn is_permanent(&self, tile: (i32, i32)) -> bool;        // HardObstacle | Permanent | Template | SatSolved
    fn entity_at(&self, tile: (i32, i32)) -> Option<&PlacedEntity>;

    // --- A* / template / SAT obstacle views ---
    /// Tiles inside `zone` that SAT must treat as forced_empty:
    /// everything except boundary ports and ghost-surface belts (which SAT replaces).
    fn forced_empty_in(&self, zone: &ClusterZone, boundaries: &FxHashSet<(i32, i32)>) -> Vec<(i32, i32)>;

    // --- Mutations ---
    /// Place an entity with a given claim kind. Panics if the tile is already
    /// claimed by anything other than a GhostSurface (which gets released).
    fn place(&mut self, entity: PlacedEntity, kind: ClaimKindTag);

    /// Release all GhostSurface claims inside a zone bbox (used by SAT before
    /// claiming tiles for its own solution).
    fn release_ghost_surface_in(&mut self, zone: &ClusterZone);

    /// Release all GhostSurface, Permanent-trunk, and Permanent-tapoff claims
    /// inside a per-tile template zone (used by per-tile template phase).
    fn release_for_pertile_template(&mut self, zone: &ClusterZone);

    // --- Drain final state ---
    fn into_entities(self) -> Vec<PlacedEntity>;
}
```

`ClaimKindTag` is a small enum that mirrors `Claim` variants but without the `entity_idx` field — used as the `place()` parameter to say "treat this entity as kind X".

### How the existing phases map

| Existing concept | New representation |
|---|---|
| `hard: FxHashSet<(i32,i32)>` (line 88) | `Occupancy::hard` (built once in step 1, never mutated) |
| `existing_belts: FxHashSet<(i32,i32)>` (line 89) | derived from `claims` (`Permanent` or `Template` with belt entity) |
| `pre_existing_set: FxHashSet<(i32,i32)>` (line 727) | `is_permanent(tile)` query — replaces both `hard.contains` and the segment-ID prefix walk |
| `pre_ghost_belts` (line 93) | snapshot of permanent claims at end of step 3 |
| `template_zones: Vec<(Vec<PlacedEntity>, ClusterZone)>` (line 738) | gone — templates write into Occupancy immediately and remember only their zone for SAT-zone-overlap accounting |
| `pertile_template_zone_bboxes` (line 742) | gone — per-phase release calls handle the trunk/tapoff stripping |
| Post-hoc retain pass at lines 1063-1083 | gone — split into per-phase release calls before each phase claims tiles |
| `forced_empty_set` build at lines 1820-1841 | one call: `occupancy.forced_empty_in(zone, &boundary_set)` |
| Final entity merge at line 1093 | `occupancy.into_entities()` |

### Phase order in the new model

1. **Step 1-3** build initial `Occupancy` from row entities, splitter stamps, balancer blocks. All claims are `HardObstacle` or `Permanent`.
2. **Step 5 (Ghost A*)** queries `is_permanent(tile)` for routing constraints, then materialises surface belts and ground UGs as `GhostSurface` claims.
3. **Step 6a-corridor** queries `is_permanent`, releases the bridged spec's `GhostSurface` belts on the corridor run tiles, then `place()`s its UG bridge as `Template`.
4. **Step 6a-pertile** for each non-corridor crossing: queries `is_permanent` for UG endpoint clearance, calls `release_for_pertile_template(zone)` to drop ghost-surface + trunk + tapoff inside the 1×3 footprint, then `place()`s the surface belt + UG pair as `Template`.
5. **Step 6b-SAT** for each remaining crossing cluster: builds the padded zone, calls `forced_empty_in(zone, &boundary_set)` to get tiles SAT must avoid (this **automatically includes** `Template` claims from step 6a-pertile, which is the bug fix), runs SAT, calls `release_ghost_surface_in(zone)` to drop the now-replaced ghost surface belts, then `place()`s SAT entities as `SatSolved`.
6. **Step 7+** continues with output rows and pole placement, consuming `occupancy.into_entities()`.

The bug fix lands at step 5: SAT's `forced_empty` now naturally contains template footprints, because the per-tile templates wrote them into the shared map at step 4 instead of deferring.

### What stays the same

- `sat::solve_crossing_zone` signature: unchanged. It already accepts `forced_empty` via `CrossingZone`.
- A* signature: unchanged externally. Internally it calls `occupancy.is_hard_obstacle` / `occupancy.entity_at` instead of consulting two separate sets.
- Trace events: unchanged. `GhostClusterSolved` still fires from the same three sites.
- The retry/feedback loop in `build_bus_layout` (the broader pipeline that wraps ghost routing): unchanged.
- Web app, WASM bindings, snapshot format: unchanged.

### What gets deleted

- `template_zones: Vec<(Vec<PlacedEntity>, ClusterZone)>`
- `pertile_template_zone_bboxes: Vec<ClusterZone>`
- `pre_existing_set: FxHashSet<(i32, i32)>`
- The string-prefix filter `segment_id.starts_with("ghost:")` at lines 729, 1072, 1836
- The post-hoc retain pass at lines 1063-1083
- The `solved_zones = template_zones; solved_zones.append(&mut sat_zones)` merge at lines 1046-1047

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| Phase ordering becomes load-bearing in a way it wasn't before. | The current code already assumes corridor → per-tile → SAT order. The refactor makes the dependency explicit, not new. |
| `place()` panicking on a double-claim hides bugs by crashing the pipeline. | `place()` should return `Result` or take a `force_replace` parameter; tests cover the legitimate replacement paths (ghost-surface release before SAT/template claims). |
| The `ghost:*` segment-id prefix is also used by the validator or downstream code. | Out of scope — the segment-id strings still get written, this RFP only changes how the **router** reads them. Audit before deleting any segment-id writes. |
| WASM build breaks. | `Occupancy` is plain Rust collections, no I/O, no threads. Verify with `wasm-pack build crates/wasm-bindings` after step 1. |
| Refactor breaks the existing tier4_ghost test in unexpected ways. | Step-by-step rollout (see Plan below). After each step the full e2e suite must stay green, and the ghost scoreboard outputs must be either identical or **strictly better** (fewer overlap pairs, fewer validator errors, no new failures). |

## Plan

Six steps. Each is a separate commit, independently verifiable, and bisectable.

### Step 1 — Land the type, no wiring

- Create `crates/core/src/bus/ghost_occupancy.rs` with `Occupancy`, `Claim`, `ClaimKindTag` and all methods listed above.
- Add `pub(super) mod ghost_occupancy;` to `crates/core/src/bus/mod.rs`.
- Unit tests in the same file:
  - Construction from a hard set and initial entities.
  - `is_free` / `is_permanent` / `claim_at` for each variant.
  - `place` happy paths for each `ClaimKindTag`.
  - `place` over a `GhostSurface` claim succeeds (replacement allowed).
  - `place` over a `Permanent` / `HardObstacle` / `Template` / `SatSolved` claim returns `Err` (or panics — TBD in implementation).
  - `release_ghost_surface_in` removes only ghost-surface claims inside the zone.
  - `release_for_pertile_template` removes ghost-surface + permanent trunk/tapoff inside the zone.
  - `forced_empty_in` produces the expected set for a constructed zone with mixed claims and a boundary set.
  - `into_entities` returns entities in placement order.
- **Not wired into `ghost_router.rs` at all.** Existing tests must stay green by being completely untouched.
- Verification: `cargo test --manifest-path crates/core/Cargo.toml` (full suite green), `cargo clippy --tests` (no new warnings in the new file), `wasm-pack build` (compiles).

### Step 2 — Build initial Occupancy in steps 1-3, run it alongside the existing `hard`/`existing_belts` sets

- At the top of `build_ghost_layout` (or wherever steps 1-3 live), build an `Occupancy` in parallel with the existing `hard`/`existing_belts`/`pre_ghost_belts` sets.
- Add a debug-only assertion at the end of step 3 that checks the new Occupancy agrees with the old sets (every tile in `hard` is `is_hard_obstacle`; every tile in `pre_ghost_belts` is `is_permanent`).
- Don't consume Occupancy anywhere yet.
- Verification: full suite green, scoreboards identical to current main.

### Step 3 — Mirror materialisation writes into Occupancy

> **Revised after reading the negotiation loop.** The original wording proposed
> replacing `ghost_astar`'s obstacle params with `&Occupancy`, but `ghost_astar`
> is a pure query function called from inside an 8-iteration negotiation loop
> with per-iteration transient obstacle sets. Cloning Occupancy 8× per iteration
> is wasteful, and A* has no semantic need to know about claim kinds — its
> contract is "given an obstacle predicate and a cost grid, return a path."
> Step 3's real goal is to exercise `Occupancy::place` in production with a
> verified mirror, which can be done at the materialisation pass alone.

- Hoist the parallel `Occupancy` binding out of the `cfg(debug_assertions)`
  block added in Step 2. Release builds will now also construct it because
  Steps 4-6 require it.
- Keep the existing debug assertions that validate construction against
  `hard` + `pre_ghost_belts`.
- In the materialisation loop (lines 606-674), restructure the per-spec write
  so the filtered entity list is collected into a temporary vec, then both
  `entities.extend(...)` and `occupancy.place(..., GhostSurface)` consume the
  same filtered vec. The filter (line 655) drops entities on `pre_ghost_belts`
  tiles or `ghost_item_at`-claimed tiles — both should already have either a
  `Permanent` claim (pre-ghost belts) or a `GhostSurface` claim from an earlier
  spec in the loop, so the entities being filtered would have failed
  `place()` anyway.
- Add a post-materialisation debug assertion: every tile in the (post-loop)
  `existing_belts` set should be `occupancy.is_claimed(tile)`. The reverse
  direction does not hold because Occupancy includes machines, poles, and
  fluid-lane reservations that `existing_belts` does not.
- **Do not touch** `ghost_astar`'s signature, the negotiation loop,
  `pre_ghost_belts`, `ghost_item_at`, or the unfiltered `existing_belts.insert`
  at line 661. `existing_belts` after materialisation has different semantics
  from Occupancy claim coverage — it tracks "a belt-tile passes through here"
  including tiles where the materialisation filter dropped the entity, while
  Occupancy tracks "an entity is placed here". Both are correct and they
  intentionally differ.
- Verification: full suite green, scoreboards identical (no behaviour change
  because Occupancy remains write-only at this step).

### Step 4 — Refactor per-tile template phase to write into Occupancy immediately

- `solve_perpendicular_template` and `try_bridge` take `&Occupancy` instead of `&hard` + `&pre_existing_set`.
- After `solve_perpendicular_template` succeeds, the caller calls `occupancy.release_for_pertile_template(zone)` and then `occupancy.place(ent, ClaimKindTag::Template)` for each entity, instead of pushing to `template_zones`.
- `template_zones` shrinks to only contain corridor-template results (or gets eliminated entirely if corridor is also migrated in this step — TBD when implementing).
- **This is where the bug fix lands.** SAT in step 5 will now see template footprints in `entities`, and its `forced_empty` build will pick them up via the existing line 1831 walk — even before SAT itself is migrated.
- Verification: tier2_ghost @ 30/s overlap pairs drop from 22 to ~0 (some residual SAT-internal overlap may still exist if the cluster-padding bug is real and separate). Full e2e suite green.

### Step 5 — Refactor SAT phase to build `forced_empty` from Occupancy

> **Revised after implementation.** The original wording assumed a clean
> swap of `&entities` + `&hard` for `&Occupancy`. In practice this needed
> two sub-steps and exposed a semantic mismatch in the boundary-port check
> that has been deferred to Step 6.

**Step 5a:** Mirror corridor template state into Occupancy.

- Push corridor UG bridge entities (built at `ents` in the corridor
  template loop) directly into `entities` and into Occupancy as
  `Template`. Replace the `template_zones.push((ents, zone))` call with
  `template_zones.push((Vec::new(), zone))` (same pattern Step 4 used for
  per-tile templates). This is what makes corridor UG bridges visible to
  the SAT phase via `forced_empty`.
- After the bridged-spec ghost belt removal at the existing
  `entities.retain` call, call `occupancy.release_ghost_surface_in(...)`
  on the corridor zone to mirror the removal.
- After the perpendicular re-add push, mirror the new `corridor-perp:*`
  belt into Occupancy as `Permanent`.
- Add `#[derive(Clone, Copy)]` to `ClusterZone` so it can be used by
  reference in multiple downstream calls without moves.

**Step 5b:** Refactor `resolve_clusters` to consume Occupancy for
forced_empty + writes.

- New `Claim::RowEntity` variant in `ghost_occupancy`. `Occupancy::new`
  now takes `(hard, row_entities, setup_entities)` and assigns
  `RowEntity` claims for the row-entity slice. Both `is_permanent` and
  `forced_empty_in` exclude `RowEntity` (the bus router *interfaces*
  with row template belts via boundary ports rather than routing around
  them — mirrors today's `pre_existing_positions` semantics, which
  excluded row entities).
- `resolve_clusters` takes `&mut Occupancy` and `&mut Vec<PlacedEntity>`
  for writes. Inside it, `forced_empty` is built via
  `occupancy.forced_empty_in(zone_rect, &boundary_set)`.
- After SAT solves a cluster, call `occupancy.release_ghost_surface_in`
  on the zone, then for each entity in the solution: skip if hard
  obstacle, skip if not `is_free`, otherwise `place(SatSolved)` and
  push to the entity output. Return `(Vec::new(), zone)` for the cluster
  so the post-hoc add loop has nothing to re-add.

**Known follow-ups for Step 6:**

1. **Boundary check still uses the entities-walk semantics.** A direct
   bisect found that switching the boundary port check from
   `hard.contains() || pre_existing_positions.contains()` to
   `Occupancy.is_permanent()` causes a tier2 regression (47 → 115
   errors, with +66 `entity-overlap` errors), even after `RowEntity` was
   added. The remaining drift is not yet fully understood. To preserve
   correctness, `resolve_clusters` currently takes two snapshot
   parameters (`boundary_check_hard`, `boundary_check_entities`) and
   uses the original entities-walk for the boundary port check only.
   Step 6 must reconcile the semantics — either find the missing
   `Claim` variant / behaviour difference, or document why the boundary
   check is intentionally different. The snapshot parameters should be
   removed when this is resolved.
2. **Tier4 has a small SAT-write delta:** 22 → 23 errors (+1
   `entity-overlap`, one categorisation shift from belt-dead-end to
   belt-item-isolation), 2194 → 2193 entities (-1). Likely caused by
   the dynamic `is_free` check in the new SAT-write path being subtly
   different from today's static `occupied` filter (today's filter is
   built once before the post-hoc add loop and not updated between
   clusters; the Occupancy path updates after each `place()` call).
   Step 6 should investigate.

**Verification:** tier1/2/3 metrics are byte-for-byte identical to the
Step 5a baseline (which itself matches the Step 4 baseline). tier4 has
the small +1 delta noted above. Full suite + lib tests + clippy + WASM
all green.

### Step 6 — Delete the obsolete plumbing

**What landed:**

- **Reconciled the boundary-check semantics.** Root cause: the
  materialisation loop in Step 3 was placing *trunk* belts as
  `GhostSurface`, but trunks are load-bearing vertical bus backbones
  that templates and SAT must route around. The fix: check the spec
  key in the materialisation loop and use `ClaimKindTag::Permanent`
  for `trunk:*` specs, `GhostSurface` for everything else. With this,
  `occupancy.is_permanent` matches today's `hard ∪
  pre_existing_positions` semantics, and the bisect snapshot
  parameters on `resolve_clusters` are gone.
- **Fixed row entity classification.** `Occupancy::new` now takes
  `(hard, row_belts, permanent_entities)`. The call site splits
  `row_entities` into belt-like (→ `RowEntity`) and non-belt (→
  `Permanent`) so that row machines, inserters, poles, and pipes are
  correctly treated as obstacles, not permeable row belts.
- **Deleted `pre_existing_set`.** It was only actively used by the
  corridor endpoint check (replaced with `occupancy.is_permanent`);
  the `solve_perpendicular_template` → `try_bridge` chain threaded it
  but `try_bridge` ignored it. Both dead params removed.
- **Deleted `template_zones` and `pertile_template_zone_bboxes`.**
  Template zones are now tracked only via a `template_count: usize`
  counter for `cluster_count` accounting. The per-tile/corridor
  template phases push their entities directly into `entities` +
  `Occupancy`, so the per-zone `(ents, zone)` storage is gone.
- **Deleted the post-hoc retain pass + add loop.** Replaced with an
  Occupancy-driven retain: walk `entities`, drop any `ghost:*` entity
  whose Occupancy claim is no longer `GhostSurface` (released by a
  template/SAT write), drop any `trunk:*`/`tapoff:*` entity whose
  Occupancy claim is no longer `Permanent`. This captures the same
  semantics as the old zone-bbox retain but reads from a single
  source of truth.
- **Shrunk `resolve_clusters`' return type.** It now returns
  `(Vec<LayoutRegion>, usize)` instead of `(Vec<(Vec<PlacedEntity>,
  ClusterZone)>, Vec<LayoutRegion>, usize)`. SAT solutions are
  written directly into `entities` + `Occupancy` in the SAT loop;
  there's no longer a side-channel to merge later.
- **Snapshot the permanent obstacle set at the top of
  `resolve_clusters`.** A new `Occupancy::snapshot_permanent_tiles()`
  method freezes the set of `is_permanent` tiles before any SAT
  cluster writes. The boundary-port check uses this snapshot so that
  later clusters see the same obstacle view as earlier ones, matching
  today's static `pre_existing_positions` behaviour.
- **Partially addressed the entity-overlap-on-hard-tiles bug.** The
  materialisation filter now excludes hard tiles too, which prevents
  ghost belts from landing on fluid-lane reservations or machine
  tiles. The underlying A* bug (allowing start/goal on hard
  obstacles at `astar.rs:695/658`) is tracked in TASKS.md as a
  remaining follow-up — the clean fix is to make `ghost_astar`
  reject hard-tile start/goal outright.
- **Removed the `#![allow(dead_code)]`** at the top of
  `ghost_occupancy.rs`. The few remaining unused methods
  (`entity_count`, `entity_at`, `into_entities`, `Claim::entity_idx`)
  have per-item `#[allow(dead_code)]` with a note that they're API
  surface for tests + future callers.
- **Deleted the `entities()` method** that was never used anywhere.

**What stayed:**

- `existing_belts` and `pre_ghost_belts` in `route_bus_ghost` — still
  used by the ghost A* negotiation loop (per-iteration transient
  obstacle sets) and the materialisation loop's per-tile filter.
  These are iteration-local and don't overlap Occupancy's role as
  shared obstacle state, so deleting them would require refactoring
  `ghost_astar` which is out of scope.
- A few comments and variable names still reference the legacy
  `pre_existing_positions` concept — harmless but can be tidied
  later.

**Verification:**

| Test | Baseline (Step 4) | Step 6 | Δ |
|---|---|---|---|
| 366 lib unit tests | green | green (+2 net to 368) | ✅ |
| 10/10 default e2e | green | green | ✅ |
| tier1_ghost | 822 ents / 0 err | 822 / 0 | identical |
| tier2_ghost | 5536 ents / 39 err | **5536 / 39** | identical |
| tier3_ghost | 337 ents / 24 err | 336 / 24 | **−1 entity** from hard-tile filter |
| tier4_ghost | 2194 ents / 22 err | 2194 / 22 | identical |
| Clippy | 0 new warnings | 0 new | ✅ |
| WASM release build | clean | clean | ✅ |

The Step 5b tier4 regression (+1 entity-overlap, −1 entity) is
**gone** — the trunk-as-Permanent fix reconciled it automatically.
The Step 4 bug fix (per-tile template vs SAT collisions, tier2:
47 → 39 errors) is preserved across the entire refactor.

### Outcome

The occupancy refactor is complete. `Occupancy` is the single source
of truth for "what claims this tile" across all template and SAT
phases. `ghost_router.rs` has:

- 1 initial Occupancy construction in steps 1-3
- 0 deferred entity queues (everything writes to `entities` +
  `Occupancy` directly)
- 0 post-hoc merge loops (the old `solved_zones` merge + add loop
  is gone)
- 1 retain pass that syncs `entities` to `Occupancy`'s released
  state (much simpler than the old zone-bbox-walking variant)

Net code delta: roughly −80 lines of plumbing, +150 lines of
`ghost_occupancy.rs` module (most of which is comments + unit
tests). The bug fix at the heart of the refactor (per-tile templates
and SAT no longer stamp on top of each other) is preserved. The
follow-ups that remain (`ghost_astar` hard-tile handling,
`existing_belts`/`pre_ghost_belts` removal) are out of scope for
this refactor and tracked in TASKS.md.

## Verification protocol (applies to every step)

1. **Default suite green:** `cargo test --manifest-path crates/core/Cargo.toml` — all 10 non-ignored tests pass.
2. **Ghost suite green:** all four `tierN_ghost` tests pass with `--ignored`.
3. **Scoreboard comparison:** capture the ghost scoreboard outputs (`zone overlap pairs`, `clusters`, `validator errors/warnings`) before and after the step. They must be identical OR strictly better (fewer overlaps, fewer errors). Never strictly worse.
4. **Snapshot eyeball:** for steps 4 and 5, decode the new `tier2_ghost.fls` and `tier4_ghost.fls` and load them in the web app's snapshot debugger to confirm overlapping clusters are visually gone.
5. **Clippy:** `cargo clippy --tests` — no new warnings in changed code.
6. **WASM:** `wasm-pack build crates/wasm-bindings --target web --out-dir "$(pwd)/web/src/wasm-pkg"` — clean build.

## Out of scope for this RFP

- Detection logic for "should this crossing be a corridor vs per-tile vs SAT cluster" (Phase 6a dispatch). The current dispatch stays as-is. This RFP is only about coordination between the phases that actually run.
- Cluster merging when SAT-cluster bboxes overlap each other (the `line 1648` "no zone merging" comment). If overlap pairs persist after step 5, that's a separate follow-up and should be tracked in its own issue.
- Splitting the trace events to distinguish template-emitted vs SAT-emitted clusters in the snapshot debugger UI. Useful but not required for correctness.
- Renaming `ghost:` segment-id prefixes. The strings stay; only how the router reads them changes.
