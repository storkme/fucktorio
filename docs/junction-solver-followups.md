# Junction solver — session handoff

**Status at end of session 2026-04-14.** Not a spec. Notes to pick up next
session without having to re-investigate.

## 2026-04-14 follow-up: P0 occupancy threading + orphan-extend bug

P0 from the original "Next steps" is done. Two fixes landed together.

### Fix 1 — strict_obstacles thread

`solve_crossing` and `JunctionStrategyContext` gained a `strict_obstacles`
parameter alongside `hard_obstacles`. The narrow `hard` set still flows to
`PerpendicularTemplateStrategy` (which intentionally stamps over trunks /
tap-offs and would refuse to fire if it saw them). The new strict set is
built once via `Occupancy::snapshot_junction_obstacles()` — every Permanent
/ Template / SatSolved / RowEntity / HardObstacle tile, i.e. everything
except `GhostSurface`. `refresh_forbidden` adds both sets to
`junction.forbidden`, so SAT's `forced_empty` now sees the full picture.

The participating-path filter inside `refresh_forbidden` was tightened:
the original code skipped every tile in `self.tiles`, but that exempted
*interior* path tiles where another spec's earlier perp template had
stamped a Permanent belt. Now only the frontier endpoint tiles
(`port_tiles`) are exempted — those are the SAT zone's entry/exit ports
and must be free.

### Fix 2 — orphan-extend bug in step 6a stamp loop

The actual cause of the validator regressions reported in the original
TL;DR was **not** SAT stamping over trunks (the `release_for_pertile_template`
path handles trunk/tap-off claims correctly). The real bug was in the
per-tile template stamp loop in `ghost_router.rs` step 6a:

```rust
for ent in &sol.entities { ... if Template/RowEntity { continue; } ... }
entities.extend(sol.entities);  // <-- adds skipped entities too!
```

When a perp template's footprint collided with an existing Template /
RowEntity claim (e.g. a previous SAT solution at an adjacent crossing
that extended into this tile), the entity was correctly skipped from
`Occupancy.place()` but still leaked into the `entities` Vec via the
unconditional `extend`. The `step 6 sync` retain logic only drops
`ghost:` / `trunk:` / `tapoff:` segments, so the orphaned `junction:`
belts survived to the validator, which flagged them as `entity-overlap`
with the SAT belt at the same tile.

Fix: the loop now `entities.push(ent)` only inside the place branch, so
skipped entities are dropped from both `Occupancy` and `entities`
together.

### Numbers

| Recipe | Errors before | Errors after | Δ |
|---|---|---|---|
| tier2 electronic-circuit 30/s | 26 | 16 | -10 (all 12 entity-overlap gone) |
| tier4 advanced-circuit 5/s | 21 | 20 | -1 (1 entity-overlap gone) |

Remaining tier2 errors: 14× `belt-dead-end`, 1× `belt-item-isolation`,
1× `underground-belt`. Remaining tier4 errors: 5× `belt-dead-end`, 15×
`fluid-connectivity`. None of these are junction-solver bugs — separate
work. All 10 e2e tests still green.

`diagnose_junctions` was extended with `errors by code:` tally + per-error
location/entities dump for `entity-overlap` issues. Useful for spotting
this class of orphan bugs in the future.

## 2026-04-14 follow-up #2: multi-lane trunk family condition

Unrelated to junction solving, but surfaced while eyeballing the tier4
5/s AM1 case in the deployed viewer. A disconnected `fast-transport-belt`
at `(4, 35)` was the head of an unfed 2-wide copper-plate trunk extension.

**The bug:** `split_overflowing_lanes` at `bus_router.rs:592` only created
a family balancer when `n_producers < n_lanes_with_consumers` (the fan-out
case: 1→2, 2→3, etc). When `n_producers == n_lanes_with_consumers` (the
parallel case, e.g. 2→2), it fell through to the "independent direct
routing" path, where each producer gets its own `ret:` spec with goal
`(x + 1, out_y)` sideloading into its trunk column.

That goal is fine for the rightmost lane (column `x + 1` is empty) but
broken for any inner lane: the tile east of the inner lane's column is
*another lane's trunk*. A* routes the inner lane's return path as far
as it can and terminates with a West-facing belt that sideloads into the
neighbouring lane's trunk — which happily accepts the items. The inner
lane's own trunk stays unfed, and every validator check on those tiles
fires.

For tier4 5/s AM1 with red belts (no cap), copper-plate hit this shape
(2 producer rows × 2 trunk lanes) and produced 57× `lane-throughput`
errors (all of copper-plate's 25/s dumped onto lane 5's right lane)
plus the dead-belt at `(4, 35..42)`.

**The fix:** Widen the family condition to `n_producers >= 1 &&
n_lanes_with_consumers >= 2 && n_producers <= n_lanes_with_consumers`.
Parallel multi-lane cases now stamp a proper balancer block between the
producers and the trunks. The `(2, 2)` template already exists in the
library — it's literally a single splitter with both sides in/out.

### Numbers

| Recipe | Errors before | Errors after | Δ |
|---|---|---|---|
| tier2 ec 30/s (yellow) | 16 | 16 | 0 |
| tier3 pb 30/s | 24 | 24 | 0 |
| tier4 ac 5/s (yellow) | 20 | 25 | +5 (regression) |
| tier4 ac 5/s (no cap, user URL) | 74 | 17 | -57 |

**The tier4 yellow regression** is +2 `belt-dead-end`, +2
`belt-item-isolation`, +1 `underground-belt`. Caused by a new
`plastic-bar (2, 2)` balancer at `y=28..30` that wasn't being stamped
before — the family placement shifts the downstream layout and causes
routing conflicts in unrelated rows. Worth investigating separately;
the plastic-bar balancer block is probably stepping on something
around `y=38..51`. Not catastrophic — net -52 validator errors across
the tier4 AC 5/s cases.

`diagnose_junctions` now also prints `BalancerStamped` shapes and y
ranges per case. Useful for spotting which families are being created
vs falling back to direct routing.

### What this means for the rest of the original plan

- **P0** (this fix): done.
- **P1 verify SAT output in viewer**: still worth doing if the next
  session picks up another regression — the diagnose tally now makes
  it cheap to spot the remaining `belt-dead-end` clusters and judge
  whether they're SAT artefacts or pre-existing.
- **P1 tier3** (no-op): unchanged, still null baseline.
- **P2 SAT gating**: unchanged. SAT still fires whenever `tile_count > 1`.
- **P3 turn-shifter strategy / encountered promotion**: unchanged.

The original P0 caveat about "decide whether to expose Occupancy or build
forbidden in ghost_router" was answered by going closer to option (a):
the snapshot is built in ghost_router but `Occupancy` had to grow a new
`snapshot_junction_obstacles()` accessor to expose the right tile classes.
The framework still doesn't see `Occupancy` directly — it sees a frozen
`FxHashSet` — which keeps `junction_solver.rs` ignorant of ghost_router
internals.

---

## TL;DR

Region-growth outer loop + strategy framework are in. Two strategies are
wired: `PerpendicularTemplateStrategy` (wraps the existing per-tile
template, preserves current behaviour) and `SatStrategy` (wraps
`crate::sat::solve_crossing_zone` after PR #145 merged in its general-routing
CNF fixes). SAT now solves every previously-unresolved perpendicular
crossing on tier2 (12/12) and tier4 (1/1) — but introduces **new validator
errors** because its stamped entities overlap pipeline state that the
strategy can't see. All 10 e2e tests still pass because they don't cover
the recipes whose baselines moved.

## What's in

### Files
- `crates/core/src/bus/junction_solver.rs` — new. `GrowingRegion`,
  `JunctionStrategy` trait, `JunctionStrategyContext`, `JunctionSolution`,
  `solve_crossing` outer loop. `MAX_GROWTH_ITERS=5`, `MAX_REGION_TILES=64`.
- `crates/core/src/bus/junction_sat_strategy.rs` — new. `SatStrategy` impl
  mapping `Junction` → `CrossingZone` → `sat::solve_crossing_zone`. Skips
  when `region.tile_count() <= 1`.
- `crates/core/src/bus/junction.rs` — added `BeltTier::belt_name()` and
  `BeltTier::rank()` helpers.
- `crates/core/src/bus/ghost_router.rs` — `PerpendicularTemplateStrategy`
  (wraps `solve_perpendicular_template`). Step 6a rewritten to call
  `junction_solver::solve_crossing` with `[&perp_strategy, &sat_strategy]`.
  `try_bridge` emits `JunctionTemplateRejected` trace events with a reason
  string on every early return; `ug_endpoint_conflicts` now returns
  `Option<&'static str>` so the caller can forward the precise reason.
- `crates/core/src/trace.rs` — new variants `JunctionTemplateRejected`,
  `JunctionSolved`, `JunctionGrowthCapped`.
- `crates/core/examples/diagnose_junctions.rs` — prints per-tile rejection
  reasons for unresolved perpendicular regions, plus a per-tier tally of
  `JunctionSolved { strategy }` and `JunctionGrowthCapped { reason }`
  counts.
- Merged PR #145 — three new CNF constraints in `sat.rs`
  (`encode_single_incoming`, `ug_out` pairing, input-boundary back-flow
  block) + `test_4x4_electronic_circuit_routing` which passes.

### Behaviour flow for one unresolved crossing today

1. Step 6a calls `solve_crossing(tile, keys_at_tile, ...)`.
2. Iter 0: region is 1 tile. Perp wrapper runs (it only fires on
   `tile_count == 1`), fails, emits `JunctionTemplateRejected`. SAT
   skipped (`tile_count <= 1`). Region grows.
3. Iter 1+: region is 3–5 tiles. Perp wrapper skips. SAT runs, returns
   `Some(solution)`. `JunctionSolved { strategy: "sat" }` fires.
4. Step 6a stamps the SAT entities into `entities` + `occupancy` via
   `release_for_pertile_template` + `occupancy.place` (same path the
   old per-tile template used).

## Current bug: SAT vs. occupancy state

**Symptom.** Baseline shifts with SAT enabled:

| Recipe | Regions | Errors | JunctionTemplate err-touching |
|---|---|---|---|
| tier2 electronic-circuit 30/s (pre-SAT) | 183 | 25 | 11 |
| tier2 electronic-circuit 30/s (post-SAT) | 183 | 26 | 24 |
| tier4 advanced-circuit 5/s (pre-SAT) | 61 | 21 | 0 |
| tier4 advanced-circuit 5/s (post-SAT) | 61 | 21 | 2 |

SAT adds 12 new `JunctionTemplate` regions on tier2 (all were `Unresolved`
pre-SAT) and 1 on tier4. The error count is basically flat on tier4 but
the number of regions that *touch* errors jumps — i.e. SAT's output is
stamping belts on tiles that downstream validation already flags.

**Diagnosis.** `SatStrategy` builds `CrossingZone.forced_empty` from
`junction.forbidden`, which only contains two things:

1. `hard_obstacles` — the set ghost_router assembles from row_entities +
   fluid lane reservations. This does NOT include trunk belts, tap-off
   belts, corridor template entities, or anything else the ghost router
   itself placed.
2. Non-participating *ghost-routed* path tiles — minus any that land on a
   participating path (fixed in this session; without that fix SAT could
   not solve anything because ports sat on forced-empty tiles).

`Occupancy` has the full picture — `Permanent`, `GhostSurface`, `Template`,
`RowEntity` — but we don't thread it into the strategy context. So when
SAT's bbox grows wide enough to touch a trunk column, it happily stamps
surface belts on the trunk, and the validator flags the result.

**Evidence.** `JunctionTemplate err-touching` jumped from 11→24 tier2, not
11→23. That means some of the pre-existing perp regions are *now* erring
too — probably because SAT's output occupies tiles that the pipeline's
post-Step-6a sync step then drops, leaving gaps in previously-valid
junctions. Worth confirming by eyeballing the snapshot.

## Next steps (ranked)

### P0 — Thread full occupancy into the strategy context

`JunctionStrategyContext` currently carries `hard_obstacles: &FxHashSet`.
That's not enough. Options:

- **(a) Pass `&Occupancy`** and let strategies query
  `occupancy.claim_at(tile)`. Strategies decide which claim kinds count as
  "forbidden for my purposes". `SatStrategy` would treat `Permanent`,
  `Template`, `RowEntity` as forbidden and only allow `GhostSurface` /
  `Free` tiles.
- **(b) Compute a per-crossing `forbidden` set** in `ghost_router.rs`
  before calling `solve_crossing`, and pass it alongside (or replacing)
  `hard_obstacles`. Simpler; doesn't leak the full occupancy abstraction
  to the framework.

I'd go with (b) — keeps `junction_solver.rs` ignorant of `ghost_router`
internals, keeps the trait simple. The forbidden set for a given crossing
tile is essentially "every tile in the bbox with any Permanent / Template /
RowEntity claim that isn't a participating path." `refresh_forbidden`
already has the shape for this; the fix is in how the caller builds the
obstacle set it hands to `solve_crossing`.

### P1 — Verify what SAT is actually producing on tier2/tier4

Before trusting any "fix", load a failing tier2 case in the web viewer or
dump the snapshot and eyeball the SAT-stamped regions. The `sat:` strategy
tally in `diagnose_junctions` shows SAT fired 12 times on tier2 and 1 on
tier4 — those are the exact tiles to inspect. Questions:

- Do the stamped entities match the spec's item?
- Do they connect cleanly at the bbox boundary to the surrounding trunks
  / tap-offs / row belts?
- Are any of the "new" errors actually pre-existing errors that got
  *moved* because SAT reshuffled the region, or are they genuinely new?

### P1 — Re-run tier3 (it's a no-op today)

tier3 plastic-bar shows `0 regions, 24 validator errors` — no junction
regions at all because the layout doesn't cross anything. But the error
count is the same as before the scaffold, so that's a null baseline, not a
regression. Mention here just so it's not mistaken for a signal.

### P2 — Decide whether SAT should gate on region size or shape

Today SAT fires as soon as `tile_count > 1` (i.e. at iter 1). For a 5-tile
"+" shape, SAT is probably overkill — a cheap template would do the job
with no occupancy coupling. Two knobs to consider once the occupancy fix
lands:

- **Minimum bbox size** (e.g. `bbox.w >= 3 && bbox.h >= 3`) — SAT only on
  regions with genuine interior slack.
- **`growth_iter` floor** — give cheap templates the first two iters to
  swing, fall to SAT at iter 3+.

### P3 — Wire a third strategy (A* reroute or a turn-shifter template)

The whole point of the scaffold is that new strategies slot in. The
original turn-shifter idea from the conversation (move the copper-plate
turn one tile back so the UG-in sits on a straight tile) is a natural
third strategy — cheap, targeted, handles the common tier2 shape without
SAT. It wants a template that rewrites the first few tiles of a
participating path within the bbox.

### P3 — Promote encountered specs to participating

`GrowingRegion` tracks `encountered` (non-participating specs whose tiles
landed in the bbox) but never promotes them. Strategies that can rewrite
three or more specs at once (A* reroute, SAT with a bigger net set) will
want this. The framework stub is there — wire it up when a strategy needs
it.

## Open threads / things I noticed but didn't touch

- **`MAX_GROWTH_ITERS=5` and `MAX_REGION_TILES=64` are arbitrary.** Picked
  to keep per-crossing cost bounded. Revisit once we have a feel for how
  much growth real junctions need — tier2 unresolved regions all hit
  `iter_cap`, tier4's one region also hits `iter_cap`. A histogram of
  "region size at success vs failure" would tell us a lot.
- **Mixed-tier junctions pick the max tier.** `SatStrategy` passes one
  `belt_name` to `solve_crossing_zone`, derived from the highest-rank
  participating spec. If a junction carries yellow + red, everything gets
  stamped as red. Throughput-neutral but may affect validation. Not
  observed as a bug yet.
- **Growth loop emits `JunctionGrowthCapped { reason: "iter_cap" }`**
  once per failed crossing. It's 13 events on tier2+tier4 today —
  pre-fix. Post-SAT, those events disappear (everything solves). If SAT
  gets gated tighter, they'll come back. Nothing to do; mentioning so it
  doesn't look like a bug if you see them.
- **`encode_single_incoming` is O(feeders²) per tile** (8 feeders × 8
  = 28 clauses). At `MAX_REGION_TILES=64` that's ~1800 extra clauses on a
  full-size region. Cheap today, worth watching if SAT becomes the default
  escalation.
- **`PerpendicularTemplateStrategy` short-circuits on `tile_count > 1`**
  to avoid duplicate `JunctionTemplateRejected` traces across growth
  iterations. If someone wants the perp template to re-try on grown
  regions (e.g. because growth might have moved a turn), this guard has
  to come out.
- **The `JunctionSolution.strategy_name` field is `#[allow(dead_code)]`.**
  I kept it because the outer loop's `JunctionSolved` trace event already
  carries the strategy name, but downstream consumers of the solution
  might want it. Trivial to wire up when needed.

## Where to look

| Question | File / symbol |
|---|---|
| How does the growth loop work? | `bus/junction_solver.rs::solve_crossing` |
| How is a `Junction` built from a `GrowingRegion`? | `GrowingRegion::to_junction` |
| Where does `forbidden` come from? | `GrowingRegion::refresh_forbidden` |
| Where does the perp template plug in? | `bus/ghost_router.rs::PerpendicularTemplateStrategy` |
| Where does SAT plug in? | `bus/junction_sat_strategy.rs` |
| Where does ghost_router call into the framework? | `bus/ghost_router.rs` — search for `junction_solver::solve_crossing` (Step 6a) |
| How to see which strategy solved what? | `cargo run --release --example diagnose_junctions` — look for the `junction-solver success by strategy` line per tier |
| Per-rejection traces? | Same diagnostic — `(x,y) reasons: ...` lines under each `Unresolved` region |
| The new SAT constraints? | `sat.rs::encode_single_incoming`, `encode_underground` (ug_out pairing), `encode_boundaries` (input-boundary back-flow block) |
| The 4×4 test | `sat.rs::tests::test_4x4_electronic_circuit_routing` |

## Related docs

- `docs/rfp-region-routing.md` — the RFP the scaffold implements. Framing
  still valid; "SAT is not a tier" claim is stale after this session, but
  the P0 bug above is proof the RFP's caution about SAT formulation
  wasn't wrong, it was just pointing at a different cliff.
- `docs/rfp-junction-solver.md` — the sister RFP on template catalogue.
  New templates plug in alongside `PerpendicularTemplateStrategy` via the
  same trait.
