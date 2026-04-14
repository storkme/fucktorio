# Ghost pipeline phase contracts

This is the load-bearing reference for `crates/core/src/bus/`. It lists
every phase the layout pipeline runs, what each phase consumes, what it
produces, and what invariants it promises to the next phase. Read this
before changing anything in `layout.rs`, `ghost_router.rs`, or
`bus_router.rs`.

This is **not** a tutorial or a design doc. It is a contract. When a
phase silently breaks an invariant the next phase relies on, that is a
bug in the phase that broke it, not in the phase that consumed the
broken state.

## Top-level pipeline (layout.rs)

`build_bus_layout(solver_result, max_belt_tier)` is the only public
entry point. The flow is single-pass — no retry loop, no two-pass A*
with extra-gap feedback, no direct/ghost mode switch. Just:

```
estimate_bus_width
  ↓
place_rows  (pass 1, temp width)
  ↓
plan_bus_lanes  (pass 1)
  ↓
compute_extra_gaps  (balancer height reservation)
  ↓
[if width or gaps changed]
  place_rows  (pass 2, real width + gaps)
  ↓
  plan_bus_lanes  (pass 2)
  ↓
place_poles
  ↓
route_bus_ghost
  ↓
splitter overlap filter  (drop row entities under bus splitters)
  ↓
missing-balancer warnings
  ↓
LayoutResult
```

The two-pass `place_rows` exists because `compute_extra_gaps` needs to
know which lanes will get balancer blocks before it can reserve their
vertical footprint. The first pass is just to hand `plan_bus_lanes`
something concrete to plan against.

## Phase contracts

### `place_rows` → `(row_entities, row_spans, row_width, total_height)`

**Consumes:** machine list, dependency order, target bus width, optional
extra gaps map.

**Produces:** every entity in every assembly row (machines, inserters,
internal row belts, fluid input pipes), one `RowSpan` per row.

**Promises:**
- Rows are stacked top-to-bottom in dependency order.
- Each row's `output_belt_y` is the y-coordinate of the row's WEST-flowing
  output belt (the one that feeds back into the bus).
- Each row's `input_belt_y[i]` is the y of the i-th solid input belt.
- Row entities are 100% within the row's `[y_start, y_end]` range.
- Two consecutive rows have ≥1 free y between them, more if the lower
  row is named in `extra_gaps`.

**Does NOT promise:**
- That the bus has any specific width (the caller passes a guess on
  pass 1, the real width comes from `bus_width_for_lanes` after
  `plan_bus_lanes`).
- That row belts won't be partially overwritten by bus splitters
  later — the layout-level "splitter overlap filter" handles that.

### `plan_bus_lanes(solver_result, row_spans, max_belt_tier)` → `(Vec<BusLane>, Vec<LaneFamily>)`

**Consumes:** solver output, row spans, max belt tier override.

**Produces:** one `BusLane` per item that needs trunk routing, plus
zero or more `LaneFamily` blocks describing N→M balancer requirements.

**Promises:**
- Every `BusLane.x` is unique and ≥ 1.
- Every lane's `tap_off_ys` are populated for its consumer rows
  (`find_tap_off_ys`).
- Lane ordering is optimised for fewest tap-off crossings via
  `optimize_lane_order` (exact search ≤7 lanes, hill-climb above).
- A `LaneFamily` is created when a lane's rate exceeds per-lane
  capacity AND the resulting parallel-trunk shape needs a merge block.
  After this commit's family-condition fix the rule is:
  `n_producers ≥ 1 ∧ n_lanes_with_consumers ≥ 2 ∧ n_producers ≤ n_lanes_with_consumers`.
- For every lane in a family, `lane.family_id` indexes into the
  family vector and `lane.family_balancer_range` is set to the y range
  the balancer block will occupy (so trunk segments skip those rows).
- Families' `lane_xs` are contiguous (templates assume adjacent output
  columns).

**Does NOT promise:**
- That the lane positions chosen avoid every geometric conflict —
  the router still has to negotiate. The planner picks an ordering
  that minimises *expected* crossings, not zero.
- That `family_balancer_range` is non-empty for every family. If
  no template (direct or decomposed) fits the family's `(N, M)`,
  the range is `(start, start)` and the layout warns. The trunks
  still skip that single row but no balancer entities get stamped.

### `compute_extra_gaps(families)` → `FxHashMap<row_idx, gap>`

**Consumes:** the families from `plan_bus_lanes`.

**Produces:** for each family, the number of extra rows of vertical
slack that need to be inserted *after* its last producer row so the
balancer block fits.

**Promises:**
- For 1-producer families: `max(0, template_height - 3)` (the balancer
  starts at the producer's `output_belt_y` and extends down).
- For N-producer families: `max(0, template_height - 2)` (the balancer
  starts immediately below the last producer row).
- Falls back to `template_height = 3` if neither a direct (N, M) nor
  any decomposed template exists — i.e. assumes a small block.

**Does NOT promise:**
- That the resulting layout's row positions actually accommodate the
  balancer without overlap. This is the contract the second
  `place_rows` pass must honour. Today it does, but a regression here
  would silently produce overlapping trunks.

### `place_poles(machines, row_occupied)` → `Vec<PlacedEntity>`

**Consumes:** machine positions, the set of row + fluid-lane tiles
that poles must avoid.

**Produces:** medium-electric-poles in horizontal lines above each
machine row.

**Promises:**
- Every machine is within range of at least one pole.
- Pole tiles are disjoint from `row_occupied`.

**Does NOT promise:**
- Connectivity to the rest of the pole network. There used to be a
  `repair_pole_connectivity` pass; if it has rotted, isolated pole
  groups will surface in the validator's `power` category.

### `route_bus_ghost(...)` → `GhostRouteResult`

This is the heart of the routing pipeline. Internally it has 7 numbered
steps. They run sequentially; each step relies on the previous step's
state.

#### Step 1 — build `hard` set
Scans `row_entities`. Belt-like entities go in `existing_belts` +
`pre_ghost_belts` (transparent to A*). Machines expand to their full
footprint and become hard. Everything else (inserters, poles, pipes)
is hard. Fluid-lane columns are reserved as hard tiles.

**Promises:** every machine/pole/pipe tile is in `hard` after this step.

#### Step 2 — splitter stamps
For each non-fluid lane with multiple tap-offs, stamps a south-facing
splitter at `(x, tap_y - 1)` and a continue-belt at `(x, tap_y)` for
every non-last tap-off. These are pushed into `entities` immediately
and added to `hard` + `existing_belts` + `pre_ghost_belts`.

**Promises:** by end of step 2, the trunk's tap-off splitter geometry
is fully placed and visible to the A* obstacle set in step 5.

#### Step 3 — family balancer blocks
For each `LaneFamily` calls `stamp_family_balancer`. Belt-like outputs
go into `hard` + `existing_belts` + `pre_ghost_belts`; non-belt outputs
are hard only. All balancer entities are pushed into `entities`. The
`BalancerStamped` trace event fires per family with `template_found`.

**Promises:** balancer entities are placed before A* runs, so the
router routes around them. If `template_found = false` the stamp
produced zero entities — the warning is emitted later in `layout.rs`.

#### Step 3b — `Occupancy` construction
Builds the typed `Occupancy` map from `hard` + row belts + everything
in `entities` so far. Row belts get `RowEntity` claims (boundary ports
may land on them); machines/inserters/poles get `Permanent` claims;
splitter and balancer entities also get `Permanent` claims.

**Promises:**
- Every `hard` tile is `is_claimed` in `Occupancy`.
- Every pre-ghost belt tile is `is_claimed` (as `RowEntity` or `Permanent`).
- After this point, **`Occupancy` is the source of truth for tile
  ownership**, not the parallel `hard` / `existing_belts` sets. The
  parallel sets still get updated for backwards compatibility but
  `Occupancy.snapshot_junction_obstacles` is what step 6a hands to
  the junction solver.

#### Step 4 — build `BeltSpec` list
Walks every lane and emits `BeltSpec` entries:
- `trunk:{item}:{x}:{seg_start}` for each trunk segment (skipping
  tap-off rows + family balancer rows).
- `tap:{item}:{x}:{tap_y}` for each tap-off.
- `ret:{item}:{x}:{out_y}` for each producer row's return path
  (intermediate lanes without a family balancer).
- `feeder:{item}:{input_x}:{out_y}` for each producer row when the lane
  is in a family balancer (one per producer, targeting a specific
  template input tile).

**Promises:**
- Every spec's `start` and `goal` are routable in principle (sat on
  free or A*-permeable tiles after steps 1-3).
- Trunk specs come first in the spec list — important for negotiation
  ordering.

**Does NOT promise:**
- That every spec is geometrically routable. Some `ret:` specs land
  on tiles owned by another lane's trunk (the bug we hit on tier4
  copper-plate when n_producers == n_lanes; now mitigated by the
  family-creation condition fix). Other specs may still have similar
  latent assumptions.

#### Step 5 — negotiated A* loop
Routes every spec via `ghost_astar`, measures same-axis conflicts at
each tile, bumps a per-tile per-axis cost grid, and re-routes. Up to
`MAX_NEGOTIATION_ITERATIONS = 8` iterations. Stops when zero same-axis
conflicts or when no improvement for `MAX_NO_IMPROVEMENT = 2` iters.

The cost grid has two layers:
- **History** (`HISTORY_PENALTY_K = 4` per repeat): carries across
  iterations. Every tile that had a same-axis conflict last iteration
  pays a permanent extra cost on the over-crowded axis.
- **Present** (`PRESENT_PENALTY_K = 6` per repeat): rebuilt each iter,
  bumped after each spec routes. Subsequent specs in the same iter
  pay a higher cost on tiles already used by earlier specs.

`TURN_PENALTY = 8` discourages turn-heavy paths.

**Promises:**
- Every successfully-routed spec lands a path that doesn't overlap a
  `hard` tile.
- `routed_paths` contains the best (lowest same-axis-conflict) routing
  across all iterations.
- `unroutable_specs` lists every spec that couldn't be routed at all.

**Does NOT promise:**
- That the routing has zero same-axis conflicts. The negotiation can
  give up after the no-improvement streak. Remaining conflicts are
  a candidate set for step 6a's junction solver.
- That the routed paths are *optimal*. The solver is greedy + history-
  weighted.
- That routes for distinct items don't sideload onto each other. The
  validator catches this as `belt-item-isolation`.

#### Step 6 — materialise paths into entities
After step 5, the chosen `routed_paths` are walked. Surface tiles
become belts; gaps become UG pairs. New entities are stamped into
`Occupancy` as `GhostSurface` claims. Every materialised entity
inherits a `ghost:tap:` / `ghost:ret:` / `ghost:trunk:` / etc.
segment-id prefix from its spec.

**Promises:**
- Every tile in a routed path now holds either a `GhostSurface` claim
  (on a fresh ghost belt) or a `Permanent` claim (the path passed
  through an existing trunk/splitter).
- Adjacent steps in a path are at Manhattan distance 1 (no gaps that
  aren't UG bridges).

#### Step 6a — per-tile junction solver
Walks every tile in `crossing_set` (tiles where two ghost-routed paths
overlap, after corridor templates have handled their share). For each
crossing tile:
1. Collects keys of every spec passing through it.
2. Calls `junction_solver::solve_crossing` with the strategy stack
   (currently `[PerpendicularTemplateStrategy, SatStrategy]`).
3. The growth loop seeds a `GrowingRegion` with one tile, then expands
   the participating specs' frontier outward up to `MAX_GROWTH_ITERS = 5`
   iterations or `MAX_REGION_TILES = 64` tiles.
4. Each iteration tries every strategy in order. On success, the
   solver returns a `JunctionSolution` (entities + footprint).
5. The caller releases trunk/tap-off entities in the footprint via
   `release_for_pertile_template`, then stamps the new entities.

**Promises:**
- The strategies see `Occupancy.snapshot_junction_obstacles()` — every
  Permanent / Template / SatSolved / RowEntity / HardObstacle tile
  except `GhostSurface` ones — as the strict-obstacle set in
  `junction.forbidden`. Frontier endpoint tiles (entry/exit ports)
  are exempted so the SAT solver can place its boundary belts there.
- Entities placed by a successful strategy are stamped to `Occupancy`
  AND pushed to the entities list. **They are not pushed to the
  entities list if the place call was skipped** (Template /
  RowEntity collision). This is the orphan-extend fix from earlier
  in the session — violating it leaks ghost belts that the validator
  flags as overlaps.

**Does NOT promise:**
- That every crossing is resolved. Tiles that no strategy can solve
  go into `remaining_crossings` and become `Unresolved` regions.

#### Step 6b — emit unresolved regions
Each remaining crossing becomes a 1×1 `LayoutRegion` of kind
`Unresolved` so the UI can render a marker.

#### Step 6 sync — drop orphaned ghost / trunk / tap-off entities
Walks `entities` and drops any belt whose segment id starts with
`ghost:`, `trunk:`, or `tapoff:` if its current `Occupancy` claim is
no longer compatible (e.g. a template stamped over a ghost surface
belt). Other segment ids are kept as-is.

**Promises:**
- After this step, `entities` and `Occupancy` agree on every
  `ghost:` / `trunk:` / `tapoff:` belt. Other segment ids may still
  diverge but the load-bearing categories are consistent.

#### Step 7 — output mergers
Final-product items get east-flowing output belts merged into a single
south-facing splitter chain at the bottom-right via `merge_output_rows`.

**Promises:** layout `max_y` is updated to include the merger block;
`merge_max_x` is the rightmost x used by any merger entity.

## Data type contracts

### `Occupancy` (`ghost_occupancy.rs`)

The typed obstacle map. Every tile holds one of:
- `HardObstacle` — never placeable.
- `RowEntity` — row template belt; boundary ports may land on it but
  routes don't go through it.
- `Permanent` — trunk, tap-off, splitter, balancer, machine, pole.
  Templates and SAT must avoid stamping over these unless they
  release the tile first via `release_for_pertile_template`.
- `GhostSurface` — A*-routed surface belt. May be replaced by a
  template / SAT solution that runs through this tile.
- `Template` — stamped by a per-tile / corridor template. Permanent
  once placed.
- `SatSolved` — stamped by SAT. Final.

**Invariant:** at any point in the pipeline, the `entities` Vec and the
`Occupancy` claims should agree on every belt-bearing tile for the
load-bearing segment-id prefixes (`ghost:`, `trunk:`, `tapoff:`,
`junction:`, `crossing:`, `balancer:`). The step 6 sync pass enforces
this for the first three; the rest are anchored by the place-loop in
step 6a (skip + don't push, never push + skip).

### `BusLane` (`bus_router.rs`)

One per item that needs trunk routing.
- `x`: trunk column.
- `source_y`: top of the trunk (where the belt starts flowing south).
- `tap_off_ys`: y values where consumers tap east.
- `producer_row` + `extra_producer_rows`: row indices producing this item.
- `family_id`: index into the family vector if this lane is part of a
  balancer block.
- `family_balancer_range`: `(y_start, y_end)` of the balancer the
  trunk must skip over.
- `is_fluid`: true means trunk is pipe-based, not belt.

### `LaneFamily` (`bus_router.rs`)

One N→M balancer block.
- `item`: the item all member lanes carry.
- `shape`: `(n_producers, n_lanes_with_consumers)`.
- `producer_rows`: the rows feeding the family's input belts.
- `lane_xs`: the trunk columns the family's outputs feed (contiguous,
  populated by `plan_bus_lanes` after lane x assignment).
- `balancer_y_start`, `balancer_y_end`: vertical footprint reserved.

### `BeltSpec` (`ghost_router.rs`, internal)

One A* routing job. `key` is a stable identifier. `start`, `goal`
are the endpoints. `belt_name` picks the belt tier. The spec list is
built once in step 4 and consumed by step 5.

### `Junction` / `GrowingRegion` (`junction*.rs`)

The state passed to a `JunctionStrategy`. `Junction` is the immutable
snapshot strategies see (bbox + spec crossings + forbidden tiles).
`GrowingRegion` is the mutable state the solve loop walks. See
`junction_solver.rs` for the precise expansion semantics.

## What this pipeline does NOT do

If you find yourself wondering "why isn't X working" in ghost mode and
the answer isn't here, it's probably because the pipeline doesn't
guarantee X. A few common ones:

- **Lane balancing across producer outputs.** Each `BeltSpec` is routed
  independently; the negotiator only minimises *spatial* conflicts,
  not throughput distribution. Items can pile up on one lane of a
  belt while the other sits idle. The validator catches the obvious
  cases as `lane-throughput`.
- **Cross-item contamination prevention.** A* doesn't know about item
  identity. Two specs carrying different items can converge on the
  same tile and the validator flags it as `belt-item-isolation`. The
  negotiator's per-axis cost grid mitigates this but doesn't prevent
  it.
- **Power network connectivity beyond row-line continuity.**
  `place_poles` lays one horizontal pole line per machine row. Lines
  for non-adjacent rows may end up disconnected; the validator
  flags this as a `power` warning.
- **Output of a recipe that the solver doesn't recognise as final.**
  Only items in `solver_result.external_outputs` get an east-flowing
  output row. Intermediate items returning to the bus go via the
  `ret:` spec mechanism in step 4.

## Glossary

- **ghost belt** — a belt placed by step 5 with a `ghost:` segment-id
  prefix. May be replaced by step 6a's strategies.
- **trunk** — a vertical south-flowing belt for one item. Lives at
  `lane.x`, segment id `trunk:{item}:{x}:{seg_start}`.
- **tap-off** — an east-flowing horizontal belt branching off a trunk
  to feed a consumer row. Segment id `tap:{item}:{x}:{tap_y}` (specs)
  or `ghost:tap:{item}:{x}:{tap_y}` (placed entities).
- **return** — a horizontal west-flowing belt carrying a producer's
  output back to its trunk column. Segment id `ret:` (spec) or
  `ghost:ret:` (entity).
- **feeder** — a horizontal belt routing a producer's output into a
  family balancer's input tile. Segment id `feeder:` / `ghost:feeder:`.
- **junction** — a tile (or region) where two ghost-routed paths
  cross. Resolved by step 6a's strategies.
- **family** — a `LaneFamily` block of N producer rows merging into
  M sibling trunk lanes via a pre-stamped balancer template.
