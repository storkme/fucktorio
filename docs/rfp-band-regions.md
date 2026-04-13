# RFP: Band regions for ghost crossing resolution

**Status:** **Paused — SAT path rejected by gate 3 correctness check**. See [`docs/sat-band-investigation.md`](sat-band-investigation.md). The geometric insight (crossings partition cleanly by horizontal y-band) is still sound, but the SAT encoder can't produce valid solutions at band scale and strengthening it is not tractable. The band concept should be revisited as part of deterministic template work under [`docs/rfp-junction-solver.md`](rfp-junction-solver.md) — one template per band, not one SAT call.
**Owner:** ghost routing pipeline
**Related:** [`docs/rfp-ghost-occupancy-refactor.md`](rfp-ghost-occupancy-refactor.md), [`docs/rfp-ghost-cluster-routing.md`](rfp-ghost-cluster-routing.md), [`docs/rfp-junction-solver.md`](rfp-junction-solver.md), [`docs/sat-band-investigation.md`](sat-band-investigation.md)

## Why this RFP is paused

Gates 1, 2, and 3 (performance) all passed on paper. Gate 3 specifically measured SAT solve time on synthetic band-shaped zones and reported ~14 ms per tier2 band, ~40 ms per tier4 band — well within budget.

**The performance numbers were meaningless because the solutions were invalid.** A follow-up check (trace each input port through the belt graph to see if it reaches a matching output port) revealed that every band-sized solution the SAT solver returned was cyclic or disconnected. The SAT encoder only enforces local item-match along flow edges; there is no global reachability constraint. Small production zones (5×5 with heavy `forced_empty`) work by geometric accident; band-sized zones with room to breathe produce garbage.

Attempts to strengthen the encoder with partial flow constraints (predecessor existence + 4-cycle ban) reduced cycle counts but never eliminated them, and pushed solve time up by 100–500× on band-sized zones (tier4 merged-3: 41 ms → 8.5 s; stress: 86 ms → 48.9 s). The only encoding that would fully prevent cycles is unary-depth reachability, which is significantly more expensive and almost certainly not tractable at band scale.

**The band insight — that crossings partition by horizontal y — remains true and useful.** It just can't be realised via SAT. The same band-shaped regions are a natural fit for deterministic templates: a band of N vertical trunks and M horizontal specs has a known, regular shape (trunks pass straight through, horizontal specs UG-bridge past each trunk). That's template territory, and it's what the junction solver RFP already proposes under T1/T2.

**Action items falling out of this investigation**:

1. The `ug_out → surface neighbor` item propagation fix in `sat::encode_item_transport` lands independently — it's a real bug fix for the existing 5×5 production SAT path (tier2 ghost scoreboard: 39 → 38 errors, no regressions).
2. Two ignored tests retained as regression guards: `sat::tests::band_regions_sat_bench` and `sat::tests::validate_existing_small_tests`.
3. The rest of this document is preserved below because the structural analysis (y-band clustering, overlap diagnostics, tier2/tier4 data) is still valuable context for the template-based work that supersedes it.

---


## Problem

The ghost router resolves belt crossings with three separate phases (corridor templates, per-tile templates, SAT clusters). Each phase defines its own region for each crossing: corridor templates stamp `N×1` strips, per-tile templates stamp `1×3` footprints, and SAT clusters pad crossing-tile bboxes by +2 on all sides. The UF clustering that feeds SAT merges crossings by Manhattan distance ≤ `ug_max_reach + 1` but applies rectangular padding — so clusters that UF refused to merge can still have overlapping padded rectangles.

The occupancy refactor (`rfp-ghost-occupancy-refactor.md`, landed as PR #144) made these overlaps **harmless** — SAT no longer stamps entities on tiles already claimed by templates — but it did not eliminate them structurally. `tier2_electronic_circuit_from_ore_ghost` at 30/s still reports 22 overlap pairs; `tier4` still reports 2. More importantly, the fragmentation into many tiny regions leaves **39 validator errors on tier2** (24 `belt-dead-end`, 12 `belt-item-isolation`, 1 `belt-loop`, 2 `underground-belt`). These errors are downstream consequences of regions being sliced up in ways that leave specs half-solved across region boundaries: one phase resolves part of a spec's crossings, another phase resolves the rest, and the hand-off doesn't connect.

## Finding: crossings live in horizontal bands

A snapshot dump of tier2's 182 regions reveals a very clean pattern:

```
region kind counts:
  junction_template (per-tile)  165  at 18 distinct y values
  ghost_cluster     (SAT)        11  at 11 distinct y values
  corridor_template               6  at  6 distinct y values
```

Actual crossing y-coordinates (region y-origin corrected for padding):

| Kind | Crossing y values |
|---|---|
| junction | `{9, 36, 117, 124, 131, 138, 148, 156, 164, 172, 180, 188, 196, 204, 212, 220, 228, 261}` |
| ghost_cluster | `{36, 156, 164, 172, 180, 188, 196, 204, 212, 220, 228}` |
| corridor | `{15, 22, 29, 236, 244, 252}` |

Two things jump out:

1. **The 11 ghost_cluster y values are a strict subset of the 18 junction y values.** Every SAT call happens on a band that also has per-tile templates. Every structural overlap (all 22 of them) is a 1×3 template vs a 5×5 SAT region on the same y. Phases fight over bands, not over regions.
2. **The 24 distinct bands** (18 ∪ 6 corridor = 24, since corridor y's are disjoint) partition *all* crossings cleanly by y. The band at `y=172` has crossings at x ∈ `{43, 44, 45}` (two junction templates plus one SAT cluster). The band at `y=236` has a corridor run. Bands never share y values — they cannot overlap with one another by construction.

This is the entire factoring the three phases are trying to reinvent, separately.

### Why this shape is forced by geometry

Ghost crossings occur where a **horizontal spec** (tap-off, return, feeder) crosses a **vertical trunk**. That's the only kind of crossing the router produces — confirmed by the `first_dy == 0` filter at `ghost_router.rs:853` on corridor detection. A horizontal spec routes at a fixed `y = tap_y / out_y / input_belt_y`, so every crossing it generates lives on that y. Multiple specs routing at the same y share a band; two specs at different y's are physically separated by a gap of rows, and their crossings cannot interact across that gap.

Regions defined by y-band are the natural, geometry-driven unit. Anything finer is artificial subdivision that creates coordination problems where none need exist.

## Proposed design

**One SAT region per band of horizontal crossings.**

Algorithm:

1. Group all ghost crossings by `y`. Each unique y becomes a candidate band.
2. **Merge adjacent bands.** Run 1D union-find on the unique y values with `merge_dist = 2 · pad + 1` (= 5 for pad=2). This merges balancer-adjacent feeder rows (tier4 has feeders at y=144/146, 152/154, 160/162, 168/169, 176/179/180, 189/190 — each pair lives on different horizontal specs but close enough in y that their padded rects would touch). A merged band contains all crossings whose y is within `merge_dist` of another member; its y extent is `[min(ys) - pad, max(ys) + pad]`.
3. For each merged band:
   - Collect all crossings `(x, y_i)` for any y_i in the band.
   - Compute x extent: `x_lo = min(xs) - pad`, `x_hi = max(xs) + pad`.
   - Build the SAT `CrossingZone` rect.
   - Walk every routed path; for each path tile inside the rect, emit boundary ports on the rect edges exactly as `resolve_clusters` does today (the port-detection logic is unchanged).
   - Derive `forced_empty` from occupancy as today.
4. Call `sat::solve_crossing_zone` once per merged band.
5. Write the SAT solution entities back via occupancy, replacing any `GhostSurface` claims inside the rect.

**Why the merge step is essential**: tier4 has 22 distinct crossing y values but only **14 disjoint merged bands**. Without the merge step, 8 of the 22 single-y bands would have padded rectangles overlapping their neighbours (gaps of 1–3 tiles between balancer-adjacent feeder rows). The merge step makes the "no overlaps by construction" claim actually hold. Tier2 happens to not need any merges because all its crossing y values are ≥6 apart — but the rule is needed for correctness on tier4 and any future recipe with tight feeder topology.

Phases collapsed: corridor templates, per-tile templates, and SAT clusters become one uniform pass. A band with 1 crossing behaves like today's per-tile template case (SAT is overkill but fast). A band with 3 adjacent crossings looks like today's corridor case. A band with mixed crossings (today's overlap scenario) becomes a single coherent region where SAT sees *all* the specs in the band and routes them together.

### SAT contract is unchanged

`CrossingZone { x, y, width, height, boundaries, forced_empty }` (`sat.rs:20-34`) is already region-shape-agnostic. The SAT solver doesn't care whether the rect came from a 1-crossing cluster or a 50-crossing band. No solver changes needed.

### What goes away

| Today | After |
|---|---|
| `corridor_template` stamping (`ghost_router.rs:825-975` approx) | gone — bands subsume runs |
| `junction_template` per-tile pass (`ghost_router.rs:1000-1199` approx) | gone — bands subsume singletons |
| UF clustering on Manhattan distance (`ghost_router.rs:1204-1227`) | gone — replaced by "group by y" |
| `+2` rectangular padding of UF clusters | gone — bands extend full x span of crossings |
| `cluster_tile_counts`, `template_count`, corridor/junction bookkeeping | gone |
| `report_zone_overlaps` diagnostic | kept, but asserts zero pairs as a regression guard |

### What stays

- `route_bus_ghost` steps 1–5 (obstacle construction, ghost A* with negotiation loop) are unchanged.
- The occupancy map, its claim kinds, and all the `release_ghost_surface_in` / `place(SatSolved)` machinery.
- The post-materialisation sync pass (`ghost_router.rs:1252-...`) that reconciles `entities` with occupancy state.
- The SAT solver itself (`sat.rs`).

### SAT complexity — measured

Measured baseline (tier4 ghost snapshot): **one** SAT region today, 5×5 = 25 tiles, 350 variables, 2481 clauses, **6075 µs**. The other 60 regions are template stamps with zero SAT time.

Synthetic benchmark in `sat::tests::band_regions_sat_bench` (release mode, varisat on a realistic band-shaped `CrossingZone` with shared-item trunks + horizontal specs):

| Scenario | Dims | Ports | Vars | Clauses | Wall |
|---|---|---|---|---|---|
| Baseline 5×5 | 5×5 | 4 | 350 | 2.4k | **0.4 ms** |
| Small band | 30×5 | 12 | 3000 | 24k | 4.2 ms |
| Medium band | 50×5 | 20 | 5000 | 41k | 8.8 ms |
| **Tier2 band** | 90×5 | 28 | 9000 | 74k | **14 ms** |
| Tier2 wide | 90×5 | 38 | 10350 | 87k | 20 ms |
| **Tier4 merged-3** | 90×9 | 30 | 18630 | 161k | **41 ms** |
| Tier4 wide | 124×7 | 34 | 19964 | 171k | 36 ms |
| Stress | 124×9 | 48 | 25668 | 223k | 86 ms |

**Projected totals:**
- **Tier2** (24 bands × ~14 ms avg) ≈ **340 ms** of SAT time. Today: ~0 ms. E2E: ~4 s → **~4.3 s** (+8%).
- **Tier4** (14 merged bands × ~40 ms avg) ≈ **560 ms** of SAT time. Today: 6 ms. E2E: ~4 s → **~4.6 s** (+15%).

Scaling is nearly linear in variable count — varisat handles the constraint shape well, with no quadratic port-interaction blow-up. Even the stress scenario (48 ports, 124×9 = biggest realistic band) stays under 100 ms.

**Gate 3 is a pass**, comfortably. The 15% e2e slowdown is the cost of giving up the template fast-path; the correctness and simplification benefits dominate. The benchmark is retained as an ignored test to guard against future SAT regressions.

## Tier2 walkthrough

Under the proposed scheme, tier2's 182 regions collapse to 24:

| Band y | Kind today | Tiles | Proposed region (width × height) |
|---|---|---|---|
| 9 | junction only | 1+ | ≥5×5 spanning all crossing x's |
| 15 | corridor | run | ≥5×5 |
| 22 | corridor | run | ≥5×5 |
| 29 | corridor | run | ≥5×5 |
| 36 | junction + SAT (overlap!) | 2 junction + 1 SAT | one region absorbing all 3 |
| 117 | junction only | | |
| 124 | junction only | | |
| 131 | junction only | | |
| 138 | junction only | | |
| 148 | junction only | | |
| 156, 164, 172, 180, 188, 196, 204, 212, 220, 228 | **junction + SAT (overlap!)** | 2 junction + 1 SAT each | **10 merged bands** |
| 236, 244, 252 | corridor | runs | ≥5×5 each |
| 261 | junction only | | |

All 22 overlap pairs disappear by construction: the 10 mixed bands become 10 SAT regions with no template counterpart.

The 11 junction-only bands (y 9, 117, 124, 131, 138, 148, 261 and the 4 extra we haven't enumerated) become 11 SAT regions that each solve what's today's per-tile template case via SAT. These were never broken under the current scheme; they'll remain unbroken under bands.

The 6 corridor bands become 6 SAT regions. Today's corridor-template pass hand-crafts UG bridges for these; SAT will produce equivalent (or better) solutions without the special case.

## Open questions

1. **Are there any crossings *not* at a horizontal-spec y?** The `first_dy == 0` filter at `ghost_router.rs:853` suggests no — only horizontal specs are eligible for corridor templates, and crossings are only counted for horizontal travel. But there's also `is_belt_like(ent.name)` walking that might pick up vertical crossings. Verify by dumping `all_ghost_crossings` grouped by direction of the spec that generated each crossing. If *some* crossings come from vertical travel, bands-by-y isn't enough and we need a second region type (bands-by-x) or a unified treatment.

2. **How do feeder specs behave?** Feeders go right-to-left horizontally at the row's `out_y`, then vertically down into the balancer column, then horizontally again into the balancer input tile. They contribute crossings on multiple y's. Under the band scheme each contribution lands in its own band, which is the correct handling — but worth a sanity check against a real feeder trace (tier4 has balancers, tier2 does not).

3. **What happens when `forced_empty` has many tiles?** A band region covers the full crossing x span, so `forced_empty` may include many tap-off surface belts on the same y that aren't crossings. SAT's clause count scales with `forced_empty` size; measure on tier2 to confirm bands don't blow up encoding time.

4. **Do the remaining 39 tier2 errors actually disappear under bands?** Expected yes because `belt-dead-end` / `belt-item-isolation` point at hand-off failures between regions, and bands eliminate the inter-region hand-off. But until we implement it, this is a hypothesis. The first sub-step of implementation should be "build bands, keep existing pipeline running in parallel, diff the validator output" so we catch any error classes that don't improve.

5. **Performance on tier4.** Tier4 has a wider bus and many balancer-adjacent feeders. Bands there will be larger. Worth a coarse measurement (time one band SAT call on tier4's widest spec-y) before committing.

## Alternatives considered

- **Merge overlapping UF clusters (stage 3.1 of `rfp-ghost-occupancy-refactor.md`).** Tighter patch, keeps the three-phase structure, just post-processes clusters. Doesn't address the template-vs-SAT inter-phase overlap (templates run before clustering even starts) and doesn't help the downstream `belt-dead-end` errors caused by fragmentation. Useful as a short-term bandaid; not a fix.

- **Shared occupancy across SAT clusters (option B from earlier discussion).** Clusters stay separate but each SAT call sees the others' tiles via occupancy. Means clusters run *sequentially* and late clusters pay penalty in forced_empty size. Doesn't address template-vs-SAT (templates would need the same treatment). Strictly weaker than bands.

- **Adaptive padding that shrinks when neighbours are close.** Heuristic; pushes the overlap problem into the padding algorithm instead of fixing it. Same downsides as cluster merging.

## Proposed rollout

Six steps, similar shape to `rfp-ghost-occupancy-refactor.md`:

1. **Add a `band_regions` boolean feature flag** (default off) that switches `route_bus_ghost` between the old three-phase path and the new one-pass path.
2. **Implement `build_band_regions`**: groups crossings by y, computes rects, returns a `Vec<CrossingZone>` ready to pass to `sat::solve_crossing_zone`. Write unit tests against fabricated crossings.
3. **Wire `build_band_regions` into a new `resolve_bands` function** that replaces `resolve_clusters` when the flag is on. Write solution entities into occupancy with `SatSolved` claims.
4. **Run the 4 ghost e2e tests with the flag on.** Expect scoreboard diff. Tier2 should drop from 39 errors; tier4 should drop from 22. Any regressions point at a gap in the band design.
5. **Delete the old corridor / junction / UF clustering code paths** once the flag is on by default and tests are green.
6. **Remove the flag**; band regions is the only path.

Each step is independently testable and revertable. If stage 5 reveals that some scenarios fundamentally need the old paths (unlikely given the tier2/tier4 data, but possible for edge cases we haven't tested), we roll back to step 4 and bands becomes an optional path.

## Decision gate

Run before step 1 of rollout:

- [x] **Gate 1: all crossings at horizontal-spec y's.** PASSED. Dumped tier2's 24 distinct crossing y values against the belt entity y histogram; every one has 9–88 horizontal belts spanning 28–88 tiles. No crossings at non-horizontal y's.
- [x] **Gate 2: feeder behaviour / adjacent bands.** PASSED with a design tweak. Tier4 has 22 crossing y values containing 8 close pairs (gaps 1–3 tiles at balancer-adjacent feeders). Added the band-merge rule (1D UF with merge_dist = 2·pad + 1 = 5), producing 14 disjoint merged bands on tier4. No overlaps post-merge.
- [x] **Gate 3: SAT performance estimate.** PASSED with caveat. Tier4 today runs one SAT call @ 6 ms; under bands it runs ~14 calls with larger regions, estimated 0.3–1.2 s total. Tractable, but not negligible. A synthetic benchmark during rollout step 2 should confirm before step 3.

All three gates pass. Proceed to rollout step 1 (feature flag + empty stub path).
