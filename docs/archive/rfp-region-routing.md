# RFP: Region Routing (Junction Solver Algorithms)

## Summary

Frame the "junction solver" work as an instance of **switchbox routing** — a
well-studied VLSI detailed-routing problem — and propose a three-tier solver
architecture (templates → negotiated A* → bounded DFS) to replace the per-tile
SAT approach that was torn out in fa57ed8.

This document is the **algorithmic companion** to
[`rfp-junction-solver.md`](rfp-junction-solver.md). That doc defines the
template catalogue (T1 perpendicular, T2 same-direction, T3 fallback). This
doc frames the overall approach, connects the problem to prior art, and
commits to an escalation path that isn't solver-dependent.

**Scope:** framing, strategy, sample data, non-goals. No implementation. No
code changes.

## Context

The belt-routing pipeline lands ghost paths first and then has to resolve
shared-tile crossings into real underground-bridge entities. Three failed
attempts preceded this RFP:

1. **Heuristic SAT crossing zones** (`extract_and_solve_crossings`) —
   hand-picked rectangular zones around crossing tiles, per-tile CNF, varisat.
   Multiple independent failure modes: zone extraction missed adjacent lanes,
   SAT filled unconstrained tiles with loops to "park" items with nowhere to
   go.

2. **Ghost-cluster routing**
   ([`rfp-ghost-cluster-routing.md`](rfp-ghost-cluster-routing.md)) —
   union-find the crossings, one SAT solve per cluster. Unified the code path
   but the same formulation problems carried over: ~93 residual errors on
   `tier4_advanced_circuit_from_ore_am1`, 78 of them traceable to zones
   straddling the bus/machine-row boundary where machine footprints created
   2–3 tile corridors that the CNF couldn't express cleanly.

3. **Band regions** ([`rfp-band-regions.md`](rfp-band-regions.md),
   [`sat-band-investigation.md`](sat-band-investigation.md)) — partition the
   bus strip into horizontal bands and solve each band as a switchbox. Ran
   into the same per-tile decomposition problem at band edges.

On `origin/main` today, the SAT cluster resolver is gone (fa57ed8 — replaced
with per-tile "unresolved" telemetry). The scaffolding types exist but
nothing consumes them:

- `crates/core/src/bus/junction.rs` — `Junction`, `SpecCrossing`, `PortPoint`,
  `BeltTier`, `Rect` types with documented invariants (1 entry + 1 exit per
  spec, flow conservation by class, forbidden-tile carve-outs).
- `crates/core/src/models.rs` — `LayoutRegion` carries `kind`, `ports`,
  `variables`, `clauses`, `solve_time_us`.
- `crates/core/examples/diagnose_junctions.rs` — "Step 0" diagnostic grouping
  regions by `(kind × class)`, cross-referenced with validator errors.
- `web/src/renderer/regionClassify.ts` — 8-variant classifier (perpendicular,
  corridor, same-direction, complex, single-item, unbalanced, no-ports,
  unknown).
- `web/src/renderer/regionOverlay.ts` — HUD showing region kind, class,
  boundary ports.

What's missing is **a clear algorithmic framing** of what the solver is
actually doing, so that future work doesn't repeat the failed experiments.
That's what this doc provides.

## The problem, properly named: switchbox routing

The junction solver is solving a textbook problem:

> Given a rectangular region (the **switchbox**) and a set of **nets**
> (specs), each with fixed **terminals** (entry + exit ports) on the region
> boundary, find a set of pairwise-disjoint paths on a 2D grid inside the
> region, such that each path connects its net's terminals and avoids
> **obstacles** (machines, pre-placed belts, power poles).

This is **detailed routing** in the VLSI sense. The entire VLSI
physical-design literature applies, give or take the exotic move primitives.
Relevant prior art:

- **Lee's algorithm** (1961) — BFS maze routing. Optimal for a single net, no
  congestion awareness. Our existing `astar.rs` is already this, with A* for
  speed.
- **PathFinder** (Ebeling & McMurchie, 1995) — negotiated congestion routing
  for FPGA/ASIC. Route all nets greedily, penalize shared tiles, rip up and
  re-route the losers with escalating penalties. Terminates when no shared
  tiles remain. **This is what `astar::negotiate_lanes` already implements**
  — extending it to handle all ghost-crossings is the most direct path
  forward.
- **Switchbox routing algorithms** — Greedy (Rivest, 1982), Hightower's
  line-probing, BEAVER (Cohoon & Heck, 1988). Classical decade-ish-old,
  tractable at our scale.
- **OARSMT** (Obstacle-Avoiding Rectilinear Steiner Minimum Tree) — overkill
  for us. Our nets are 2-pin; we don't need Steiner trees.
- **Sherwani**, *Algorithms for VLSI Physical Design Automation* — survey
  reference; useful if we get stuck.

NP-hardness caveat: switchbox routing is NP-hard in general. The
**at-our-scale** story is that every known hard instance has dozens of nets
and hundreds of tiles. A tier4 junction has 2–6 specs and 10–20 tile paths.
The practical complexity floor for small instances is far below the worst
case.

## What Factorio adds to the classical problem

Two twists that the VLSI literature mostly doesn't cover:

1. **Underground belts are not a second layer.** The canonical VLSI move is
   "jump to the back layer, route in the orthogonal direction, jump back."
   Factorio underground belts are a **long-range move primitive on a single
   grid**: "from tile X facing east, jump N tiles east (N ∈ 2..max_reach),
   placing an entrance entity at X and an exit entity at X+N." The tile at
   X+N must be empty, tiles between X and X+N must not contain another UG
   belt of the same tier in the same orientation (lane pairing). This is
   cleaner as an A* **successor function** than as a 2-layer product graph.
   Concretely: at state `(x, y, dir)`, generate successors
   `(x+k·Δ, y+k·Δ, dir)` for k in 2..reach and stamp the entrance/exit pair
   on acceptance.

2. **Per-spec belt tier.** Two ghost specs in the same junction can have
   different belt tiers (yellow/red/blue). That changes `max_reach` per spec
   and prevents lane-pairing between tiers. The solver has to track tier as
   part of each spec's state, which the existing negotiated router doesn't do
   yet.

Everything else — grid topology, obstacles, forbidden tiles, flow
conservation — is classical switchbox territory.

## Why the SAT experiment failed (and why SAT isn't doomed)

Restate the Phase 3 finding from
[`rfp-ghost-cluster-routing.md`](rfp-ghost-cluster-routing.md): varisat
solved clusters with 1000–2000 variables in under 100 ms. **Speed was never
the problem.** The failures were:

- **Boundary straddle.** Zones were centered on crossing tiles and padded
  uniformly, so they often spanned the bus/machine-row boundary. Machine
  footprints got pinned as `forced_empty`, leaving 2–3 tile corridors that
  the SAT formulation treated as free space — and then "filled" with belt
  loops to absorb items from missing output ports.

- **Missing output ports.** The `occupied_by_existing` filter skipped ports
  where ghost paths exited the zone into an adjacent lane. The solver
  received N inputs and M < N outputs. No assignment can balance that, so
  the solver created local loops to park excess items. **This is
  formulation, not solver quality.**

- **Per-tile granularity.** The crossing set treated every shared tile as
  independent. Two specs running antiparallel for 100 tiles produced 100
  crossings, union-found into one 33×21 zone with 200+ ports. Even a correct
  formulation would struggle — the right answer is "bridge one of the two
  specs underground for its entire run," which the per-tile zones can't
  express because they don't know about spec runs.

**Takeaway:** SAT solves the wrong sub-problem cheaply. Switching solvers
doesn't help. What helps is changing the sub-problem — which is what the
rest of this doc proposes.

## Prerequisite: growing regions to spec-run granularity

This is the elephant in the room, and it deserves its own section.

From the 2026-04-12 investigation appended to
[`rfp-junction-solver.md`](rfp-junction-solver.md):

> The remaining crossings are not "two paths cross at a point." They are
> **long parallel or anti-parallel overlaps** where many specs share the
> same tiles for extended runs. The crossing set treats each shared tile as
> independent, but the correct resolution operates at the spec-run level:
> bridge one spec underground for an entire run, not per-tile.

Concretely, the tier4 baseline on origin/main still shows:

- **141 residual crossing tiles** after T1 template handles the simple point
  crossings (15 zones, all 1×3).
- Histogram of specs-per-tile: 3 specs @ 43 tiles, 4 @ 6, 5 @ 47, 6 @ 4,
  7 @ 15, 8 @ 20, and 12 tiles of anti-parallel 2-spec overlap.
- Worst offenders: `ret:plastic-bar:2:18` (path length 233, **100
  crossings**), `ret:electronic-circuit:4:144` (path length 95, **93
  crossings**). These single specs cross through almost every other spec in
  the layout for their entire horizontal run.

**A per-tile junction solver — of any quality — cannot solve this.** The
right answer is a decision at the spec-run level: "this spec goes
underground for tiles [x0..x1]." That's one UG pair regardless of how many
specs it crosses under. The per-tile solver, no matter how smart, will try
to resolve every crossing locally and either (a) produce one zone that spans
a third of the layout and is computationally intractable, or (b) produce
many small zones that each "solve" their slice without coordinating with
neighbours.

**This doc does not design the region-growth pass.** That's a separate
strategy and the right framing for it is "how do we identify spec-runs," not
"how do we route them." The remaining sections assume a future
region-identification pass produces regions of the right granularity:

- A point-crossing region is still a point-crossing region.
- A long overlap gets identified as a single spec-run region: bbox covers
  the overlap, ports are the entry/exit of each spec into/out of the
  overlap, the solver's job is to decide who goes underground and for how
  long.
- Output merger zones (y≈178–196 on tier4) get identified as one region per
  merger, not one per tile.

Until that pass lands, the solver proposed here will only help with the
~80% of cases that are already point crossings — which is worth doing, but
doesn't move tier4 to zero errors by itself.

## Proposed solver architecture: 3-tier escalation

Given a `Junction` (as already defined in
`crates/core/src/bus/junction.rs`), the solver attempts strategies in order.
The first one that succeeds returns a `JunctionSolution`. The tier numbering
is intentional: each tier handles more cases, pays more compute, and is
invoked less often.

### Tier 1: Template matcher

**Input**: `Junction` + a classification (reusing the `RegionClassification`
logic already in `web/src/renderer/regionClassify.ts`, ported to Rust).

**What it does**: pattern-matches the junction against a catalogue of named
shapes. Each shape has a constant-time stamp routine that produces entities
directly. No search.

Candidate shapes (names are indicative, not exhaustive):

- **T1 perpendicular** — 1 horiz spec × 1 vert spec, 1-in/1-out each.
  Already implemented.
- **T1-N perpendicular-multi** — 1 horiz × N vert (or inverse) where all
  verts are single-port. The "corridor" class from the existing classifier.
- **T2 parallel-lane** — 2 specs same direction, same row, different items.
  Bridge one for the length of the overlap.
- **T2 antiparallel** — 2 specs opposite directions on the same row. Bridge
  one.
- **T3 corner-swap** — 2 specs meeting at a corner with conflicting turns.
  Determined by port positions.

Tier 1 is the existing [`rfp-junction-solver.md`](rfp-junction-solver.md)
proposal in full. This RFP defers to that doc for the template catalogue —
it is the *what*, this section is the *where it sits*. Tier 1 handles the
~80% tail that point-crossing regions already produce.

**Output**: success (stamp entities, done) or "no template matched"
(escalate to Tier 2).

### Tier 2: Negotiated A* with rip-up and reroute (PathFinder-family)

**Input**: junctions where no template matches. Typically 3+ specs with
mixed directions, or weird port configurations.

**What it does**: iterative negotiated routing. Each spec is routed with A*
as if it had the whole bbox to itself. Conflicts (shared tiles) are detected
after each round. Conflicting specs are ripped up and rerouted with a
history penalty on the shared tiles. Penalties escalate per iteration.
Terminates when no conflicts remain or an iteration cap is hit.

This is **the PathFinder algorithm**, which
`crates/core/src/astar.rs::negotiate_lanes` already implements for bus-level
routing. The junction-solver version is a restricted application of the
same algorithm on a much smaller grid, with three differences:

1. **State includes incoming direction** so the A* can model belt turn
   costs and emit UG successors correctly.
2. **UG successors are first-class** — at any belt-facing state, the
   successor function generates "jump k tiles in direction d for k in
   2..reach" as well as the normal 1-tile moves. Reach comes from the
   spec's belt tier.
3. **Per-spec belt tier** — each iteration processes one spec at that
   spec's tier, with its own `max_reach`.

Order matters. The router tries multiple spec orderings (longest path
first, most-constrained first, and a few random permutations) and picks
the best result. For n ≤ 6 specs, all n! orderings are trivially
enumerable in parallel via Rayon.

**Why PathFinder**: it's the most battle-tested congestion-aware router in
the VLSI/FPGA space. It handles the "many specs, most already fit but a
few conflict" case elegantly. It degrades gracefully — if it can't find a
conflict-free solution, it returns its best attempt with a conflict list,
which Tier 3 can consume.

**Output**: success (all specs routed, no conflicts) or "conflicts remain"
(escalate to Tier 3).

### Tier 3: Bounded DFS fallback

**Input**: the tiny minority of junctions where Tier 2 can't converge
within a few iterations. Expected to be rare — by the time Tier 2 can't
solve it, something is weird about the region.

**What it does**: exhaustive search over spec orderings and UG-bridge
decisions within a hard wall-clock budget (target: 50 ms per junction).
Uses Tier 2's best partial solution as a warm start and an upper bound.
Aggressive pruning — if the current partial cost already exceeds the
upper bound, cut.

This tier exists to **never silently fail**. If Tier 3 times out, the
region is reported as `unresolved` with a specific trace event explaining
why (per-tile telemetry already exists in `emit_unresolved_junctions`). No
loops, no placeholder entities, no lies in the output.

**Output**: success or explicit `unresolved` with diagnostic trace.

### What's explicitly NOT in the tier list

**SAT is not a tier.** The Phase 3 results and the investigation in
`rfp-junction-solver.md` together show that SAT's formulation mismatch,
not its solver quality, was the problem. Re-adding varisat as a T4
fallback would reintroduce the same failure mode. If the Tier 3 DFS
proves inadequate in practice and a future investigation shows a
CNF-friendly sub-problem, reconsider — but start without it.

**ILP, CP-SAT, HiGHS are not tiers.** WASM-compatibility rules them out
(OR-Tools is C++, HiGHS is C++, CP-SAT is C++). See Implementation
Constraints.

**GPU acceleration is not a tier.** At our scale (junctions < 20×20,
nets < 10) a single A* call is sub-millisecond. The bottleneck is
inter-junction parallelism, not intra-junction compute.

## Implementation constraints

- **WASM first.** The Rust core runs in the browser. Pure Rust, no C++
  dependencies, no native-only crates. `varisat` happens to be pure Rust,
  which is why it was viable in the first place. Any new solver
  dependency has to clear the same bar.
- **No new heavy dependencies.** The existing Rust ecosystem gives us
  everything the 3-tier plan needs: `rustc_hash` (fast hash maps),
  `rayon` (inter-junction parallelism, already a dependency via
  workspace), `std::collections::BinaryHeap` for A*.
- **Parallelism is inter-junction.** `rayon::par_iter` over the regions
  array in `ghost_router.rs::route_bus_ghost`. Sort regions by
  `(y, x, bbox size)` before dispatch so the output entity list is
  deterministic regardless of thread schedule. Inside a single junction,
  Tier 2's spec-ordering trials parallelize trivially (n! for n ≤ 6).
- **Determinism is non-negotiable.** Every test and every snapshot depends
  on stable entity ordering. All iteration orders are BTreeMap/Vec, never
  HashMap/HashSet. Rayon results collect into sorted Vecs before merge.
- **Trace events feed the HUD.** The web renderer already shows region
  kind/class and boundary ports. The solver should emit `TierOneHit`,
  `TierTwoSolved`, `TierThreeSolved`, `TierThreeTimeout` trace events so
  the Ghost Mode overlay can colour regions by which tier solved them.
  This gives us a live view of coverage as the template catalogue grows.

## Sample regions from real layouts

These are drawn from the Step 0 diagnostic
([`crates/core/examples/diagnose_junctions.rs`](../crates/core/examples/diagnose_junctions.rs))
and the investigation section of
[`rfp-junction-solver.md`](rfp-junction-solver.md). The tier2/tier4 numbers
come from the current origin/main baseline.

### Sample A — small perpendicular crossing (tier2, Tier 1 target)

**Layout:** `tier2_electronic_circuit_from_ore_am1` (electronic-circuit,
30/s, AM1, ore inputs).

**Region**: 3×1 at approximately the copper-cable × iron-plate
intersection. 2 specs, 1 entry + 1 exit each, perpendicular axes.

```
Before (ghost paths overlap at center):     After (bridge vertical):
   .  ↓Cu  .                                   .  U↓Cu  .
  →Fe →Fe →Fe                                 →Fe →Fe →Fe
   .  ↓Cu  .                                   .  U↑Cu  .
```

**Classification**: `Perpendicular`. **Tier 1 verdict**: already solved by
the existing T1 template. This is the baseline success case — tier2
diagnostic shows `junction_template` handling 165 of 182 regions cleanly.

### Sample B — the canonical large point cluster (tier4, historical)

**Layout:** `tier4_advanced_circuit_from_ore_am1` (advanced-circuit, 5/s,
AM1, ore inputs).

**Region**: 5×5 at (29, 73), one crossing tile at (31, 73) where
copper-plate (east) meets plastic-bar (south). ASCII from
`rfp-junction-solver.md`:

```
      28 29 30 31 32 33 34
  73:  .  ·  ↓C U↓  →C ↓C  .     pre-fix SAT output: 5×5 zone
  74:  ▾  ·  →C →C  ↑C ←C  .     2×2 loop at (31-32, 73-74)
  75:  → →C  ↑C U↑   ▸  ▸  ▸    row template belts at x=32+
  76:  .  ·  ·  →C  ↑C  ⊥  .     inserter
  77:  .  ·  ·  ↓P  ███ ·  .     machine
```

**Correct solution** (3-tile UG bridge):

```
  74:  →C →C →C          copper-plate continues east on surface
  75:  .  U↓   .          plastic-bar goes underground
```

**Classification**: `Perpendicular` if re-identified at 3×1 instead of
5×5. **Tier 1 verdict**: solved. **Lesson**: region extraction produced
the wrong bbox (5×5 with machine straddle), not the solver. This is a
region-growth/region-shrinking question, not an algorithm question.

### Sample C — spec-run overlap (tier4, blocked on region growth)

**Layout**: same as B. **Worst offender**: `ret:plastic-bar:2:18` — a
return path from a plastic-bar machine row back to the bus trunk.

**Shape**: 233 tiles long. Crosses **100 other specs** along its
horizontal run. If we per-tile-crossing this, we get 100 tiny
point-crossings, most of which T1 can solve in isolation, but the
cumulative UG-pair stamps conflict with each other because neighbouring
templates don't know about each other.

**Correct solution**: a single UG bridge covering some substantial
portion of the 233 tiles (determined by which overlap has the fewest
conflicts above or below). One decision, one UG pair, dozens of crossings
resolved simultaneously.

**Classification today**: 100 `Perpendicular` 1×3 regions.
**Classification needed**: 1 `SpecRunOverlap` region with bbox covering
the run, ports at the entry/exit of the long spec into the overlap.
**Status**: blocked on the region-growth pass.

### Sample D — output merger zone (tier4, blocked on region growth)

**Layout**: same as B. **Location**: y ≈ 178–196 at the south edge of
the layout, where product belts converge toward output mergers.

**Shape**: a rectangular zone covering the final ~18 rows of the bus.
Diagnostic shows:

- (4, 196): 4 specs at same tile
- (11, 196): 7 specs at same tile
- (30, 196): 8 specs at same tile
- (31, 184): 5 specs at same tile

**Pre-fix SAT zone**: (1, 178) 33×21 — a 693-tile rectangle with 200+
boundary ports, traversed by most product lanes simultaneously. The old
SAT solver chewed on this for tens of milliseconds and produced
loop-riddled garbage.

**Classification today**: one giant `ghost_cluster` or, post-template,
as many 1×1 `unresolved` regions as there are shared tiles.
**Classification needed**: one `OutputMerger` region per actual merger,
with the solver reasoning about each merger's inputs in isolation.
**Status**: blocked on region-growth pass.

### Sample E — antiparallel overlap (tier4, blocked on region growth)

**Shape**: 2 specs running in opposite directions on the same row for
~12 tiles. `ret:electronic-circuit:4:144` on its way back through the
bus crosses an inbound feeder.

**Classification today**: 12 per-tile regions, all classified
`SameDirection` or `Complex`. **Classification needed**: 1
`Antiparallel` region. Trivial to solve — bridge one of the two specs
for the length of the overlap — **once the region identification groups
the tiles together**.

### Current baselines (origin/main, post-fa57ed8)

From the Step 0 diagnostic (`cargo run --example diagnose_junctions
--release`):

| Tier | Regions | Errors | Region mix                                                                              |
|------|---------|--------|------------------------------------------------------------------------------------------|
| tier2 electronic-circuit 30/s from ore | 183 | 25 | 165 junction_template (11 errs) + 6 corridor_template (0 errs) + 12 unresolved (0 errs) |
| tier4 advanced-circuit 5/s from ore    | 61  | 21 | 48 junction_template (0 errs) + 12 corridor_template (0 errs) + 1 unresolved (0 errs)   |

Observations:

- **Tier 1 already eats most of tier4** at the current region
  granularity. The residual unresolved region carries most of the error
  mass, and it's large (spec-run overlap).
- **The tier2 unresolved regions aren't causing validator errors.** This
  was a misread in the original RFP draft. The 25 tier2 errors live
  inside successful `JunctionTemplate` regions — they're lane-flow /
  throughput issues from the templates the matcher *accepted*, not from
  ones it rejected. The 12 unresolved regions pass through with zero
  err-touching tiles.

#### Tier2 perpendicular-but-unresolved: investigation (2026-04-13)

Added `TraceEvent::JunctionTemplateRejected` to `try_bridge` and extended
the diagnostic to print the rejection reason for each unresolved
perpendicular region. Result on tier2 (12 regions, not 11 — small
baseline drift since the original count):

- **1 region** at (32, 36): mixed — vertical attempt hit
  `turn_at_ug_in`, horizontal attempt hit `ug_in_axis_conflict`.
- **11 regions** at x ∈ [38..48], y ∈ [148, 156, 164, 172, 180, 188,
  196, 204, 212, 220, 228] (y step 8, x step 1): **every one** rejected
  both attempts with `ug_out_axis_conflict` and `ug_in_axis_conflict`
  respectively. Identical failure mode across all 11 — clearly one spec
  weaving diagonally through a row of machine/belt rows, with a third
  spec sitting on the UG endpoint tile on a perpendicular axis.

Tier4's single unresolved region at (11, 20) falls into the same
`ug_out_axis_conflict` bucket.

**Conclusion: the classifier is not lying about perpendicular, and
port extraction is fine.** The template matcher is correctly refusing
bridges that would place a UG-in/out on top of a third spec on the
orthogonal axis. The classifier operates on the region's own ports and
has no view into neighbouring tiles — that's the gap, and it's a real
one, not a bug.

**Implication for this RFP:** these 12 regions are *not* cheap wins for
Tier 1. A smarter template would either (a) shift the UG endpoints
further from the crossing center to avoid the third spec, or (b) grow
the region to include the conflicting spec and route all three together
via Tier 2 (negotiated A*). Option (b) is what the 3-tier architecture
is for — but because these regions don't contribute to validator errors
today, they're not the ones driving error count down either. **The tier2
error count is a lane-flow problem in accepted templates, not a
rejected-template problem.** The 3-tier plan should be justified by
tier4 spec-run overlaps (Samples C/D/E), not by the tier2 anomaly.

Reproduce with:
```bash
cargo run --manifest-path crates/core/Cargo.toml \
    --release --example diagnose_junctions
```

## Non-goals

- **Region identification / growth.** Handled separately. This doc
  assumes the regions it receives are the right granularity, and
  acknowledges that until the region-growth pass lands, the
  tier2/tier4 improvements from this work are bounded.
- **Template catalogue enumeration.**
  [`rfp-junction-solver.md`](rfp-junction-solver.md) owns this. This
  doc refers to Tier 1 as "whatever templates are in the catalogue at
  the time."
- **Re-adding varisat or any new solver dependency.** Pure Rust,
  hand-rolled search, no CNF.
- **Trace events / UI design.** Mentioned in passing in Implementation
  Constraints, but not spec'd here.
- **Belt tier changes.** The solver respects per-spec tier, but this
  doc doesn't propose new tiers or reach changes.
- **Fluid routing.** Pipes have their own geometry and don't
  participate in ghost routing. Out of scope.

## Open questions

1. **Is `web/src/renderer/regionClassify.ts` the source of truth for
   classification, or do we port it to Rust?** The existing classifier
   runs in the browser; the solver needs it in Rust. Option A: port the
   8-variant classifier to Rust and have the web renderer call the Rust
   version via WASM. Option B: keep two implementations and hope they
   stay in sync. Option A is obviously right; it just needs doing.

2. **How does Tier 2's rip-up-and-reroute coexist with
   `astar::negotiate_lanes`?** The bus-level router already uses
   negotiated routing for trunk placement. The junction solver's Tier
   2 is the same algorithm at smaller scale. Do they share code? If
   yes, where does the abstraction boundary sit? If no, why are we
   implementing PathFinder twice?

3. **Trace-event design.** How much detail does the HUD need to show
   which tier solved what? Minimal: one variant per tier
   (`TierOneHit`, `TierTwoSolved`, etc.). Maximal: per-iteration spec
   orderings, history penalties, conflict maps. Start minimal.

4. **Tier 3 warm-start from Tier 2's best partial.** Tier 2 might exit
   with some specs routed and a few conflicting. Tier 3 could continue
   from there. Worth the plumbing, or easier to start Tier 3 from
   scratch?

5. **Rayon parallelism per junction vs per spec-ordering trial.** At
   n ≤ 6 specs, n! orderings is at most 720, trivially parallel. But
   most junctions have n ≤ 3, so the parallelism wants to be at the
   junction level, not the trial level. Needs measurement.

## Related

- [`rfp-junction-solver.md`](rfp-junction-solver.md) — template
  catalogue and per-tile template strategy. The *what* to this doc's
  *how*.
- [`rfp-ghost-cluster-routing.md`](rfp-ghost-cluster-routing.md) —
  the Phase 3 ghost-cluster rewrite that this work builds on.
- [`rfp-band-regions.md`](rfp-band-regions.md),
  [`sat-band-investigation.md`](sat-band-investigation.md) — earlier
  attempts at region partitioning; superseded.
- [`layout-engine-deep-dive.md`](layout-engine-deep-dive.md) — full
  bus layout pipeline context.
- [`crates/core/src/bus/junction.rs`](../crates/core/src/bus/junction.rs)
  — current scaffolding types.
- [`crates/core/src/bus/ghost_router.rs`](../crates/core/src/bus/ghost_router.rs)
  — where the solver plugs in.
- [`crates/core/examples/diagnose_junctions.rs`](../crates/core/examples/diagnose_junctions.rs)
  — Step 0 diagnostic.
- [`crates/core/src/astar.rs`](../crates/core/src/astar.rs) — existing
  A* + `negotiate_lanes` (PathFinder-family, bus-level).

## Status

**Draft.** Requires review before any implementation work begins.
Implementation will probably happen in phases along the tier lines,
but the region-growth prerequisite (Sample C/D/E cases) is the first
thing to attack — everything in this doc is downstream of that.

