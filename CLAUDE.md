# Fucktorio

Automated Factorio factory blueprint generator. Takes a target item + production rate, solves recipe dependencies, generates a spatial layout, and exports a Factorio-importable blueprint string.

## Quick start

```bash
# Requires: factorio-draftsman, pytest
pip install factorio-draftsman pytest

# Run tests
pytest tests/

# Generate a blueprint
python -m src.pipeline
```

## Architecture

Three-stage pipeline (`src/pipeline.py` orchestrates):

1. **Solver** (`src/solver/`) — Recursively resolves recipes via `draftsman.data`, calculates machine counts and flow rates. Returns `SolverResult`.
2. **Layout** (`src/spaghetti/`, `src/routing/`, `src/search/`) — Parallel random search with retries using incremental place-and-route. Each machine is placed one at a time, edges routed immediately, with retry on failure. Returns `LayoutResult`.
3. **Blueprint** (`src/blueprint/`) — Thin draftsman wrapper that converts `LayoutResult` to a base64 blueprint string.
4. **Validation** (`src/validate.py`) — Functional checks run after layout: pipe isolation, fluid port connectivity, inserter chains, power coverage.

## Key models (`src/models.py`)

- `ItemFlow` — item name, rate, fluid flag
- `MachineSpec` — machine type, recipe, count, inputs/outputs
- `SolverResult` — machines, external inputs/outputs, dependency order
- `PlacedEntity` — entity name, position, direction, recipe, carries (what item/fluid it transports)
- `LayoutResult` — entities, connections, dimensions

## Key source files

| File | Purpose |
|------|---------|
| `src/routing/router.py` | Item-aware A* pathfinding (state: x,y,forced,lane), `other_item_tiles` hard-blocks cross-item contamination, proximity penalty near foreign networks, underground belts, belt/pipe entity placement |
| `src/routing/inserters.py` | Lane-aware inserter assignment (`approach_vec`, `target_lane`, `is_direct` on `InserterAssignment`) |
| `src/routing/orchestrate.py` | Layout orchestration: batch (`build_layout`) and incremental (`build_layout_incremental`) builders, direct machine-to-machine insertion, stub placement on routing failure |
| `src/routing/graph.py` | Production graph construction from solver output |
| `src/routing/common.py` | Machine sizes, belt tier selection, direction constants, lane constants (`LANE_LEFT`/`LANE_RIGHT`), `inserter_target_lane()` |
| `src/routing/poles.py` | Power pole placement (greedy near-machine or grid fallback) |
| `src/search/layout_search.py` | Single-pass parallel random search (`evolutionary_layout`) + `search_with_retries()` wrapper (up to 5 independent attempts), `SearchStats` dataclass |
| `src/spaghetti/placer.py` | Incremental machine placement in dependency order |
| `src/spaghetti/layout.py` | Layout orchestrator — calls `search_with_retries()`, entry point for layout engine |
| `src/validate.py` | 16 validation checks (pipe isolation, belt loops, throughput, etc.) |
| `src/models.py` | Shared data models (ItemFlow, MachineSpec, SolverResult, PlacedEntity, LayoutResult) |

## Factorio game rules (constraints for the layout engine)

These are the physical rules the layout engine must satisfy:

- **Machines** craft recipes, need ingredients delivered and products extracted
- **Inserters** pick from one side, drop to the other. Regular inserters reach 1 tile; long-handed inserters reach 2 tiles
- **Belts** move items in a direction, connect when adjacent. Different tiers have different throughput limits (yellow: 15/s, red: 30/s, blue: 45/s)
- **Pipes** carry fluids, connect to any adjacent pipe (and merge — separate fluid networks must be physically isolated)
- **Fluid ports** on machines are at specific tile positions (queryable from `draftsman.data.entities`)
- **Entities** cannot overlap
- **Power** — machines need electricity; medium-electric-pole covers a 7x7 area

## Recipe complexity ladder

Tracks which recipes produce zero-error blueprints. Each tier represents increasing complexity. Moving up = real progress.

| Tier | Recipe | Complexity | Status |
|------|--------|-----------|--------|
| 1 | `iron-gear-wheel` | 1 recipe, 1 solid input | SOLVED -- zero errors consistently (belt-item-isolation fixed via item-aware A*) |
| 2 | `electronic-circuit` | 2 recipes, 2 solid inputs | Failing -- belt-flow-reachability (2-5 errors), contamination solved |
| 3 | `plastic-bar` | 1 recipe, 1 fluid + 1 solid input | Failing -- pipe isolation |
| 4 | `advanced-circuit` | 5+ recipes, mixed solid/fluid | Failing -- massive routing failures |
| 5 | `processing-unit` | Deep chain, multiple fluids | Not attempted |
| 6 | `rocket-control-unit` | Very deep chain | Not attempted |

### Known failure modes

1. **Belt-flow-reachability** (current #1 blocker): Continuation routing connects tiles topologically but belt directions point the wrong way for input edges. The belt path exists physically but items can't flow upstream. Fix needed at `route_connections` level.
2. **Belt cross-item contamination**: SOLVED for tier 1 via `other_item_tiles` hard-blocking in A*. Proximity penalty (+3.0) and contamination guard in `_fix_belt_directions()` prevent residual cases.
3. **Belt loops**: A* routing accidentally creates cycles in the belt network.
4. **Disconnected networks**: Output belts from different machines don't merge into a connected trunk.
5. **Throughput violations**: Too many inserters feeding the same belt lane.
6. **Failed routing edges**: A* can't find any path from source to destination.
7. **Pipe merge contamination**: Adjacent pipes carrying different fluids merge (fluids connect to all adjacent pipes).

## Layout engine

The layout pipeline: **place machine → assign inserters → route edges → repeat for next machine → place poles**.

### Incremental place-and-route (`src/routing/orchestrate.py`)

`build_layout_incremental()` places machines one at a time in dependency order. For each machine:
1. Try candidate positions (starting at spacing=2, corridor penalty rewards gap=1 for connected machines)
2. Assign inserters — `_get_sides()` returns all 12 border positions for 3x3 machines, shuffled by RNG
3. Route edges immediately — if routing fails, try the next candidate position
4. Adjacent machines (gap=1) get direct inserters via `_find_direct_gap()`, skipping belt routing entirely

External edges use belt stubs + A* continuation routing instead of boundary conventions. `route_connections()` supports incremental calls via `existing_belt_dir_map`, `existing_group_networks`, `io_y_slots` parameters.

### Parallel random search (`src/search/layout_search.py`)

Single-pass evaluation of 60 random candidates in parallel (evolution was tried and abandoned -- random search produces same quality, 5x faster). Three gene dimensions:
- **Placement order** — which machine to place first
- **Position seed** — RNG seed for candidate position shuffling
- **Side preferences** — inserter side priority per machine

`search_with_retries()` wraps this with up to 5 independent searches, returning the first zero-error layout or the best found. `SearchStats` dataclass logs per-attempt scores. Candidates scored by validation errors + failed edges + belt count + area - direct insertions.

### Shared infrastructure (`src/routing/`)

- **Production graph** (`graph.py`): Converts `SolverResult` into a directed graph of `MachineNode`s connected by `FlowEdge`s
- **A* router** (`router.py`): Item-aware lane-aware grid pathfinding — state is `(x, y, forced, lane)`. `other_item_tiles` parameter hard-blocks cross-item belt contamination; proximity penalty (+3.0) near foreign networks. Tracks belt lanes (left/right) through turns and sideloads. Underground belt support with perpendicular entry penalty. `_fix_belt_directions()` includes contamination guard. Enhanced retry loop (5 retries) with incoming contamination checks
- **Inserter assignment** (`inserters.py`): Lane-aware inserter placement. `InserterAssignment` includes `approach_vec`, `target_lane`, `is_direct` fields
- **Common utilities** (`common.py`): Machine sizes, belt tier selection, direction constants, lane constants (`LANE_LEFT`/`LANE_RIGHT`), `inserter_target_lane()`
- **Routing result** (`router.py`): `RoutingResult` exposes `belt_dir_map` and `group_networks` for incremental routing

### The primary remaining problem

Belt-flow-reachability: continuation routing connects source to destination topologically, but belt directions can point the wrong way for input edges (items can't flow upstream). This produces 2-5 errors per layout on tier 1 and is the main blocker for consistent zero-error output. The fix needs to happen at the `route_connections()` level. When continuation routing fails entirely, a belt stub is still placed so inserters have a target.

Belt-item-isolation is SOLVED via `other_item_tiles` hard-blocking in A* (was the previous primary blocker).

## Verification & validation

- `src/verify.py` — structural blueprint validation (overlap detection, unpaired undergrounds, ASCII map)
- `src/validate.py` — functional validation (pipe isolation, fluid connectivity, inserter chains, power coverage)
- `tests/` — pytest suite with `--viz` flag for HTML visualizations, deployed to GitHub Pages via CI

## Visualizations

Test visualizations are deployed to GitHub Pages on main branch pushes:
https://storkme.github.io/fucktorio/
