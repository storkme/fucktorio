# Layout Engine Deep Dive

Detailed internals of both layout engines. For the high-level overview see `CLAUDE.md`.

## Bus layout engine (`src/bus/`, `crates/core/src/bus/`)

Deterministic row-based layout using the main bus pattern. Machines grouped by recipe into rows,
items transported on parallel trunk belts, with underground sideloading for branches.

### How it works

1. **Solver** produces `SolverResult` (same as spaghetti)
2. **Placer** (`placer.py`) groups machines by recipe into rows, splits rows when throughput exceeds belt capacity (lane-splitting for both lanes of a belt)
3. **Templates** (`templates.py`) stamp out belt/inserter patterns for each row type (single-input, dual-input, with optional lane splitting via sideload bridges)
4. **Bus router** (`bus_router.py`) places trunk belts, routes branches from trunks to rows via underground belt pairs
5. **Validation** runs the same checks as spaghetti (`validate.py`)

### Current status

Bus passes tiers 1–3 with zero validation errors (iron-gear-wheel, electronic-circuit incl. from ores, plastic-bar). Tier 4 (advanced-circuit from plates) has lane-throughput warnings from single-lane sideload bottleneck ([#64](https://github.com/storkme/fucktorio/issues/64)). Tier 4 via ores is blocked by [#68](https://github.com/storkme/fucktorio/issues/68) (fluid rows can't fit 2+ solid inputs in 3-tile pitch).

### Bus routing details

- **Lane spacing**: 1-tile (trunks on adjacent columns, no gaps)
- **N-to-M balancer families** (`bus_router.LaneFamily`): when an item has `N_producers != N_lanes`, the planner creates a balancer family and stamps a pre-generated template from `src/bus/balancer_library.py` (SAT-generated via Factorio-SAT, see `scripts/generate_balancer_library.py`). Covers 1→M, N→M asymmetric shapes. Producer WEST belts are wired to template input tiles via explicit path rendering (topmost producer → leftmost input tile to avoid crossings). Balancer footprint + feeder paths are registered as A\* obstacles so tap-offs route around them.
- **Row reflow for tall templates**: families whose template height exceeds the default 2-tile recipe gap trigger `extra_gap_after_row` reservations in the two-pass `place_rows` call. Formula: `max(0, H-3)` for N=1 families (balancer overlaps producer output row), `max(0, H-2)` for N≥2 (balancer sits below producer rows).
- **Contiguous family lanes**: `_optimize_lane_order` rejects permutations that split a family's lanes non-contiguously — templates stamp at fixed width and require adjacent output columns.
- **Even row splitting**: Producer rows are evenly sized for balanced throughput per lane.
- **Tap-off underground crossings**: Tap-offs go EAST on the surface; first tile is always a surface belt (turn point from trunk) to avoid sideloading onto UG inputs. Underground hops cross other trunks with short spans.
- **Output returns (N→1 only, hand-rolled)**: the single remaining hand-rolled balancer path is the N→1 Z-wrap (`_route_intermediate_lane` `balance_y` code) — triggered when 2+ producers feed a single lane. Not yet migrated to template framework because N→1 templates have output-not-at-edge, requiring column reservation.
- **Output mergers**: Final product rows use EAST-flowing output belts, merging at the bottom-right of the layout via a SOUTH splitter chain.
- **Negotiated crossing map**: Rust `negotiate_lanes()` pre-computes crossings between all lane segments (including mergers), augmenting `_blocked_xs_at` so tap-offs know about future entity positions.
- **Fluid lanes**: Pipe trunks + pipe-to-ground tap-offs. Fluid lanes don't block belt tap-offs (pipes and belts don't conflict).

### Factorio belt lane rules

- **Sideloading**: Feeding a belt from the side only fills the NEAR lane (the lane closest to the feeder)
- **Sideloading onto underground input**: Only fills the FAR lane — must feed UG inputs straight (same direction), not from the side
- **Belt turns**: 90-degree turns preserve both lanes (with CW/CCW rotation)
- **Splitters**: Distribute items 50/50 between two output belts, preserving lane assignment per belt

---

## Spaghetti layout (parked)

Work is parked pending routing rework ([#62](https://github.com/storkme/fucktorio/issues/62)).
Kept here for context.

The layout pipeline: **place machine → assign inserters → route edges → repeat for next machine → place poles**.

### Incremental place-and-route (`src/routing/orchestrate.py`)

`build_layout_incremental()` places machines one at a time in dependency order. For each machine:
1. Try candidate positions (starting at spacing=2, corridor penalty rewards gap=1 for connected machines)
2. Assign inserters — `_get_sides()` returns all 12 border positions for 3x3 machines, shuffled by RNG
3. Route edges immediately — if routing fails, try the next candidate position
4. Adjacent machines (gap=1) get direct inserters via `_find_direct_gap()`, skipping belt routing entirely

External edges use belt stubs + A\* continuation routing instead of boundary conventions. `route_connections()` supports incremental calls via `existing_belt_dir_map`, `existing_group_networks`, `io_y_slots` parameters.

### Parallel random search (`src/search/layout_search.py`)

Single-pass evaluation of 60 random candidates in parallel (evolution was tried and abandoned — random search produces same quality, 5x faster). Three gene dimensions:

- **Placement order** — which machine to place first
- **Position seed** — RNG seed for candidate position shuffling
- **Side preferences** — inserter side priority per machine

`search_with_retries()` wraps this with up to 5 independent searches, returning the first zero-error layout or the best found. `SearchStats` dataclass logs per-attempt scores. Candidates scored by: validation errors + failed edges + belt count + area − direct insertions.

### Shared routing infrastructure (`src/routing/`)

- **Production graph** (`graph.py`): Converts `SolverResult` into a directed graph of `MachineNode`s connected by `FlowEdge`s
- **A\* router** (`router.py`): Item-aware lane-aware grid pathfinding — state is `(x, y, forced, lane)`. `other_item_tiles` parameter hard-blocks cross-item belt contamination; proximity penalty (+3.0) near foreign networks. Tracks belt lanes (left/right) through turns and sideloads. Underground belt support with perpendicular entry penalty. `_fix_belt_directions()` includes contamination guard. Enhanced retry loop (5 retries) with incoming contamination checks.
- **Inserter assignment** (`inserters.py`): Lane-aware inserter placement. `InserterAssignment` includes `approach_vec`, `target_lane`, `is_direct` fields
- **Common utilities** (`common.py`): Machine sizes, belt tier selection, direction constants, lane constants (`LANE_LEFT`/`LANE_RIGHT`), `inserter_target_lane()`
- **Routing result** (`router.py`): `RoutingResult` exposes `belt_dir_map` and `group_networks` for incremental routing

### The primary remaining problem

Independent edge routing: `route_connections()` routes each A\* edge independently, so earlier edges claim tiles that force later edges into bad paths (loops, wrong directions, contamination). This is the fundamental blocker — see [#62](https://github.com/storkme/fucktorio/issues/62) for the proposed fix (negotiated congestion routing).
