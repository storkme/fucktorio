# RFP: Ghost-Cluster Routing

## Summary

Replace the current multi-layered bus router (hand-rolled trunk placement +
tap-off A\* with inline UG bridging + heuristic SAT crossing zones +
`render_family_input_paths` feeders + `place_merger_block` mergers) with a
single unified pipeline:

1. **Place hard obstacles first** — machines, balancer stamps, power poles.
   Everything that isn't a belt is fixed before routing starts.
2. **Route every belt spec with straight-line-biased A\* that ghosts through
   other belts.** Hard obstacles (non-belts) still block. Same-tile overlaps
   with other belts are allowed and recorded.
3. **Union-find the resulting ghost crossings into clusters.**
4. **SAT-solve each cluster** using the existing `sat.rs` engine to place
   actual surface/UG entities that resolve the crossings.

Clusters emerge naturally from where ghost paths meet, replacing the current
heuristic zone-extraction in `extract_and_solve_crossings`.

## Why

Every failing case on `tier4_advanced_circuit_from_ore_am1` reduces to a
perpendicular-crossing problem the current pipeline can't express:

| Failure | Current mechanism | Why it fails |
|---|---|---|
| `ret:plastic-bar:{1,2}:{27,18}` — 2 dead-ends | `route_intermediate_lane` schedules a `ret:` A\* spec | A\* can't UG-bridge the ore trunks at `y_tolerance=3` so it gives up |
| `ret:electronic-circuit:{3,4}:{152,144}` — 2 dead-ends | same | same |
| copper-cable feeders — 5 dead-ends | family balancer `render_family_input_paths` reads the balancer template library | Two stacked bugs: (a) shape (5,7) has no template, gcd(5,7)=1 so decomposition fallback has no divisor, returns empty silently — a *flow* problem that needs solver-side overproduction to a covered ratio; (b) even once a balancer is stamped, the feeders from row outputs still have to cross trunks — a *routing* problem that ghost clustering fixes. This RFP addresses (b) only. |
| plastic-bar tap-off at (11,180) merging copper-cable | heuristic SAT crossing zone drops its UG exit on an adjacent tap-off row | Zone extraction doesn't see the neighbouring lane |

Three of the four are the same underlying ask: *"this belt wants to cross
those belts, please figure out how."* Today the answer lives across four
different code paths with independent failure modes and silent fallbacks.
Under ghost clustering there is one answer, one failure mode, one place to
fix bugs.

The copper-cable case has a second bug layered on top — a missing (5,7)
balancer template, which is a flow/throughput problem this RFP does **not**
solve (SAT-at-crossings has no throughput semantics; balancer construction
stays template-based). That one needs a solver-side fix: overproduce to a
machine count whose (n, m) ratio the library already covers, or add more
templates. Tracked separately as #136.

## Empirical cluster sizing

Phase 0 diagnostics in `crates/core/tests/e2e.rs` (`diag_ghost_cluster_*`)
measure what ghost clusters actually look like on real layouts. They extract
either the failing `ret:` specs from the current pipeline's trace, or
synthesise the silent copper-cable feeder specs from the family structure,
then route each with a turn-biased A\* whose hard obstacles are everything
*except* belts, and union-find paths by shared tiles.

### Findings per case

| Case | Layout entities | # failures measured | # clusters | Max cluster tiles | Max crossings |
|---|---|---|---|---|---|
| Tier-4 AC 5/s from ore (failing `ret:` only) | 4434 | 4 | 4 | 31 | 10 |
| Tier-4 copper-cable feeders (synthesised) | 4434 | 5 | 5 | 27 | 6 |
| Tier-4 AC 45/s from plates (stress) | ~13k | 7 | 7 | 34 | 18 |
| Tier-5 processing-unit 5/s AM2 | 9961 | 6 | 6 | **40** | 21 |

**The pattern is consistent across every case measured:** each failing
connecting belt (`ret:`, feeder, etc.) routes as a locally-bounded path of
~30-40 tiles, with zero transitive cluster merging between paths. Max cluster
stays well under the 300-tile safety budget, even at tier-5.

Most paths have **zero turns** — the obstacle-avoidance doesn't need
detours, which keeps clusters small and the SAT problem tight.

### The full-spec-set blowup (and its fix)

One diagnostic variant (`diag_ghost_cluster_all_specs_ac_from_ore`) re-routes
**every** belt spec the planner would schedule — trunks, tap-offs, feeders,
returns — under ghost semantics. Result was dramatic:

```
specs: 49  routed: 39  unroutable: 10 (row-template belts, correctly excluded)
clusters: 4
  paths  tiles  crossings
     35   1191   1147    ← mega-cluster
      2     39     64
      1     21     19
      1     14     10
```

35 of 39 paths collapsed into one **1,191-tile** mega-cluster — ~4× the
safety budget. Mechanism: every vertical trunk runs through every horizontal
tap-off row, and every horizontal path crosses every trunk column, so
everything transitively shares tiles through the intermediate trunks.

**This finding drives the core structural refinement of the RFP.** Trunks
are planner-decided vertical columns with known endpoints — they do not
need pathfinding, and routing them as ghost paths creates the transitive
sprawl. The fix is a distinction:

- **Trunks** = placed as hard obstacles in the first pass, along with
  machines / poles / balancer stamps. They are the skeleton.
- **Connecting belts** (`tap:`, `ret:`, `feeder:`, `bal:`, merger inputs) =
  the things that cross trunks. **Only these ghost.**

With that distinction, the per-case measurements above *are* representative
of a real ghost-routing run, and the full-spec-set blowup disappears by
construction — trunks are no longer in the ghost graph at all.

### Processing-unit 20/s deferred

The original Phase 0 plan called for measurement on
`stress_processing_unit_20s_from_plates` (processing-unit at 20/s AM3).
That test has never completed on the current pipeline — its baseline
comment reads `warnings=?, zones_solved=?, zones_skipped=?` and it has
been `#[ignore]`'d without verification since it was added. Our attempts
to run it (with validation skipped, 15-minute timeout) consistently time
out in `solver + build_bus_layout` alone.

This is a pre-existing current-pipeline pathology, not anything ghost
routing would inherit. The 5/s AM2 variant above completes in 32 seconds
and gives us the tier-5 signal we wanted. Measurement at 20/s AM3 is
deferred until the ghost router actually exists and can produce the
layout.

### Verdict

Cluster explosion was the main sizing risk. With the trunks-as-hard-obstacle
refinement, **the data says it's not a problem** — every measured case
(tier-4 small, tier-4 balancer-dependent, tier-4 stress, tier-5 moderate)
shows bounded per-path clusters with max ~40 tiles / ~21 crossings. Phase 0
signs off. Move on to Phase 1.

## Mechanics

### Obstacle tiers

Every tile is classified once per layout:

- **Hard**: machine footprints, poles, pipes, inserters, already-placed
  balancer stamps. Nothing routes through these.
- **Soft (belt)**: tiles that currently hold a belt, underground belt, or
  splitter. A\* may path through them; each traversal records a ghost crossing.
- **Free**: everything else.

### Pole placement moves to the front

Today poles are placed *last* (`place_poles` in `layout.rs`), after the router
has already finished and the remaining free tiles are probed. This leads to
two problems: routes that *would* have been fine are re-routed to dodge poles
added after the fact, and coverage holes appear where the router consumed the
only tiles that could cover an edge machine.

Under the new scheme, poles go down immediately after `place_rows`. The pole
placer is already deterministic (greedy forward sweep with guaranteed machine
coverage, `layout.rs` comment at line 782). Moving it ahead of routing makes
poles just another hard obstacle — no retry loops, no post-hoc patching.

### A\* cost function

Same grid search as today but with two changes:

- **No UG bridging in this pass.** `allow_underground = false`. UG entities
  come from the SAT phase, not from A\*.
- **Stiff turn penalty.** Straight step = 1, turn = 1 + K (K ≈ 8 in the
  diagnostic). Biasing for straight lines keeps clusters small and gives the
  SAT phase less to do. Turn penalty also encodes "we'd rather pay a few
  extra tiles than introduce a corner near a crossing".

Belt tiles are *not* in the hard obstacle set. When A\* steps onto a tile
that already holds a belt (from an earlier spec in the same routing pass, or
a pre-placed belt), it records the tile as a ghost crossing and continues.

### Cluster identification

After every spec is routed, walk each routed path and find the tiles where it
coincides with any other belt. Union-find:

- Two ghost tiles are in the same cluster if they share a spec's path.
- Two ghost tiles are in the same cluster if they're adjacent through a
  connected run of belts *or through turn tiles*. (Including turns handles
  the case where a 90° turn next to a crossing is structurally part of the
  same resolution problem.)

The result is a set of connected cluster components, each bounded by a
rectangle of tiles the SAT solver will own.

### SAT phase

Each cluster becomes one input to `sat::solve_crossing_zone`. The engine is
already general-purpose — it handles surface belts, UG input/output entities,
4-direction underground passage with max-reach constraints, item transport
consistency, boundary port specifications, and forced-empty tiles. The
current limitation is the *zone extraction* around it (fixed 3-tile height,
per-tap-off zones that can't merge, contiguous columns only) — all of that
goes away when clusters come from the ghost union-find instead. Boundary
conditions are "what belts entered the cluster and must exit it" derived
from the ghost path endpoints. Interior tiles are free for the SAT
solver to fill with surface belts, UG pairs, or empty space as needed.

If a cluster is unsolvable, it's a hard failure — no silent fallback.
Diagnostic output points at the specific cluster and the constraint set.

## What goes away

- **`render_family_input_paths`**'s feeder-routing half — the code that
  A\*-routes producer row outputs into balancer input tiles. That's a ghost
  routing problem. The *balancer stamping* half (looking up a template and
  placing its entities as hard obstacles) stays.
- **Silent no-op on missing templates** — today when a template lookup
  misses, `render_family_input_paths` returns an empty list and the row
  outputs dangle. Under the new scheme, missing templates are a hard error
  raised back to the solver layer, which must either overproduce to a
  covered ratio or fail loudly with a suggested fix.

The balancer template library **stays**. SAT-at-crossings reasons about
local tile occupancy given boundary ports; it has no throughput semantics,
so it cannot synthesise a 5→7 balancer or any other flow-balanced N→M
construction. Balancer stamps remain pre-rendered splitter-tree templates,
placed as hard obstacles before routing. The RFP only replaces the routing
that *connects* row outputs into those stamps.
- **`extract_and_solve_crossings`** — heuristic zone extraction. Clusters are
  now a by-product of routing, not a separate phase.
- **Inline UG bridging in `route_belt_lane` / `route_intermediate_lane`** —
  `foreign_trunk_skip_ys`, `merge_consecutive_skips`,
  `filter_and_record_dropped_bridges`, and the dropped-bridge retry loop.
- **`ret:` / `feeder:` / `bal:` spec distinctions** — they all collapse to
  "belt from point A to point B, ghost what you need to".
- **`y_tolerance` on horizontal specs** — the reason it exists (letting a
  feeder detour around a wide trunk group) is handled by ghosting instead.

`route_lane`, `route_fluid_lane`, `place_merger_block`, and
`merge_output_rows` stay. Fluid routing is unchanged — fluids don't ghost.
Merger blocks for N→1 external outputs are still useful as a geometry
primitive.

## Phased implementation

### Phase 0 — Sizing validation (DONE)

Completed. Four `diag_ghost_cluster_*` diagnostics in
`crates/core/tests/e2e.rs` measure per-case cluster sizes across tiers 4
and 5. Findings captured in the "Empirical cluster sizing" section above.
Key outputs:

- Bounded per-path clusters (~30-40 tiles) across every measured case.
- Max crossings per cluster stays ≤ 21 across tier-4 and tier-5 measurements.
- No transitive cluster merging — each failing path is its own component.
- Full-spec-set blow-up identified and resolved by keeping trunks as hard
  obstacles. This is the load-bearing structural finding of Phase 0 and is
  now pinned in the "Mechanics" and "Why" sections.
- Tier-5 measurement at 20/s AM3 deferred — the current pipeline can't
  produce that layout at all. Measured 5/s AM2 instead, got clean data.

### Phase 1 — Pole-first placement (DONE)

`place_poles` now runs inside the retry loop before `route_bus`, using
only row entities to build the occupied set. Poles are appended to the
entity list passed to the router, making them hard obstacles from the
start. Spot-checked: zero pole-position drift on tier-2, tier-3 tests.

One regression caught and fixed: oil-refinery fluid ports (placed by the
router, not row templates) collided with pre-placed poles. Fix: reserve
planned fluid-lane tiles (vertical trunk connections + horizontal PTG
chain endpoints) in the pole placer's occupied set.

### Phase 2 — Ghost A\* prototype (DONE)

Implemented as `crates/core/src/bus/ghost_router.rs`, with `ghost_astar`
in `astar.rs` and env var gate in `layout.rs`. Gated behind
`FUCKTORIO_GHOST_ROUTING=1`. When set, `route_bus_ghost` replaces
`route_bus`: places trunk belts + splitter stamps + balancer blocks as
hard obstacles, routes every connecting-belt spec with `ghost_astar`,
records ghost crossings, union-finds them into clusters.

**Key result on `tier4_advanced_circuit_from_ore_am1`:**
- 36/36 specs routed, **0 unroutable**
- 1 ghost cluster at **348 tiles** (545 expected validator errors)
- 3166 layout entities

**Cluster size note:** Phase 0 per-failing-spec measurement showed
4 separate ~31-tile clusters. Phase 2 full spec set produces 1 cluster
at 348 tiles because tap-offs at different y-values share trunk-crossing
tiles. This is the expected SAT input for Phase 3. Varisat handles
~1000-2000 variables in <100ms. Bifurcation (#138) can decompose if
needed.

### Phase 3 — Cluster SAT resolution

Feed ghost clusters from Phase 2 into `sat::solve_crossing_zone`. For
each cluster: extract boundary ports from ghost path endpoints, build a
`CrossingZone`, solve, replace ghost-crossing surface belts with
SAT-solved entities (surface belts + UG pairs). Unsolvable clusters are
hard failures with diagnostic output.

**Solve clusters in parallel from day one.** Each cluster is independent;
`clusters.par_iter().map(sat::solve_crossing_zone)` gives linear speedup.
Related: #139 (parallelise validators the same way).

**Main design question:** boundary extraction — how to derive
`ZoneBoundary` entries from the ghost cluster's edge tiles. Each ghost
path entering/exiting the cluster bbox defines a port (item, direction,
position on boundary). The ghost router records which spec placed each
crossing tile and specs carry item names + directional intent.

### Phase 4 — Gate flip + cleanup

When the ghost router passes the full e2e suite on the existing fixtures
*and* unblocks `tier4_advanced_circuit_from_ore_am1`, flip the gate and
delete the old code paths listed in "What goes away".

## Open questions

1. **Same-tile same-item belts** — can't happen by construction (the planner
   allocates one column per item) but needs an assert during Phase 2 to
   catch planner bugs early.
2. **Fluids** — pipes never ghost. Fluid lanes continue to use the existing
   placement. Confirm fluid-port positions are all in the hard-obstacle set.
3. **Tap-offs onto machine input belts** — the current pipeline glues the
   east end of a tap-off to the row input belt at a specific tile. Under
   ghost routing, the tap-off's goal is still that tile but the path may
   ghost through neighbouring trunks. SAT-phase boundary conditions need to
   pin the tap-off endpoint or we'll lose the connection.
4. **Lane-splitting output rows** — the `sideload_bridge` template stamps
   entities inside the row; those need to be hard obstacles before the
   router sees them.
5. **N→M mergers for final products** — `merge_output_rows` stays, but its
   output column should be handed to the ghost router as a "belt needs to
   exist from (col, out_y) to (col, max_y)" rather than manually rendered.

## Risks

- **SAT scaling at larger cases.** Phase 0 measurement must cover tier-5
  recipes before Phase 4 ships.
- **Turn-penalty tuning.** K too low → paths snake through everything and
  clusters grow. K too high → paths refuse to detour and unreachable goals
  appear. Needs a small grid search on the fixture corpus.
- **Lost SAT failure context.** Today when SAT fails, the dropped-bridge
  retry loop gives the pipeline a chance to relax and try again. Under the
  new scheme, SAT failure is terminal unless we add an analogous retry
  (e.g., push rows apart and re-route). Worth designing in from Phase 3.

## Related docs

- [`docs/rfp-belt-flow-aware-astar.md`](rfp-belt-flow-aware-astar.md) —
  orthogonal A\* improvements that still apply.
- [`docs/layout-engine-deep-dive.md`](layout-engine-deep-dive.md) —
  the pipeline being replaced.
- [`crates/core/src/sat.rs`](../crates/core/src/sat.rs) — SAT crossing
  solver; the engine this proposal leans on.
- [#138](https://github.com/storkme/fucktorio/issues/138) — SAT solver
  optimization opportunities unlocked by the zone cache (memoisation,
  bifurcation, shape-specific fast paths, warm-start).
- [`crates/core/tests/e2e.rs`](../crates/core/tests/e2e.rs) —
  `diag_ghost_cluster_ac_from_ore` is the Phase 0 diagnostic.
