# Fucktorio

Automated Factorio factory blueprint generator. Takes a target item + production rate, solves recipe dependencies, generates a spatial layout, and exports a Factorio-importable blueprint string.

## Quick start

```bash
# Requires: uv (manages Python version + deps)
uv sync

# Run tests
uv run pytest tests/

# Generate a blueprint
uv run python -m src.pipeline
```

## Development conventions

- **Python version**: Pinned in `.python-version`. Use `uv` to manage the venv and run commands.
- **Running snippets**: Don't use inline `python -c` for multi-line exploratory code. Instead, write a script to `scripts/` (e.g. `scripts/debug_lanes.py`) and run it with `uv run python scripts/debug_lanes.py`. This makes it easier to iterate and review.
- **Running tests**: `uv run pytest tests/`.

## Architecture

Three-stage pipeline (`src/pipeline.py` orchestrates):

1. **Solver** (`src/solver/`) — Recursively resolves recipes via `draftsman.data`, calculates machine counts and flow rates. Returns `SolverResult`.
2. **Layout** — Two engines, both return `LayoutResult`:
   - **Bus** (`src/bus/`) — Deterministic row-based layout with main bus pattern. Currently the primary focus.
   - **Spaghetti** (`src/spaghetti/`, `src/routing/`, `src/search/`) — Random search with A* place-and-route. Currently parked pending routing rework ([#62](https://github.com/storkme/fucktorio/issues/62)).
3. **Blueprint** (`src/blueprint/`) — Thin draftsman wrapper that converts `LayoutResult` to a base64 blueprint string.
4. **Validation** (`src/validate.py`) — Functional checks run after layout: pipe isolation, fluid port connectivity, inserter chains, power coverage.
5. **Analysis** (`src/analysis/`) — Parses real Factorio blueprints into production graphs for studying community layouts.

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
| `src/search/layout_search.py` | Single-pass parallel random search (`random_search_layout`) + `search_with_retries()` wrapper (up to 5 independent attempts), `SearchStats` dataclass |
| `src/spaghetti/placer.py` | Incremental machine placement in dependency order |
| `src/spaghetti/layout.py` | Layout orchestrator — calls `search_with_retries()`, entry point for layout engine |
| `src/validate.py` | 18 validation checks (pipe isolation, belt loops, throughput, etc.) |
| `src/models.py` | Shared data models (ItemFlow, MachineSpec, SolverResult, PlacedEntity, LayoutResult) |
| `src/bus/layout.py` | Bus layout orchestrator — builds row-based layouts with main bus trunks |
| `src/bus/placer.py` | Row placement: groups machines by recipe, splits rows for throughput |
| `src/bus/templates.py` | Belt/inserter templates for bus rows (single-input, dual-input, lane-splitting) |
| `src/bus/bus_router.py` | Trunk routing, branch sideloading, underground belt pairs |
| `src/analysis/` | Blueprint analysis: classify, trace, infer items, build production graphs |

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

| Tier | Recipe | Complexity | Spaghetti | Bus |
|------|--------|-----------|-----------|-----|
| 1 | `iron-gear-wheel` | 1 recipe, 1 solid input | Inconsistent (xfail) — loops, contamination, unpaired UG | SOLVED |
| 2 | `electronic-circuit` | 2 recipes, 2 solid inputs | Failing — belt-flow-reachability | SOLVED (incl. from ores) |
| 3 | `plastic-bar` | 1 recipe, 1 fluid + 1 solid input | Failing — pipe isolation | Not attempted |
| 4 | `advanced-circuit` | 5+ recipes, mixed solid/fluid | Failing — massive routing failures | Not attempted |
| 5 | `processing-unit` | Deep chain, multiple fluids | Not attempted | Not attempted |
| 6 | `rocket-control-unit` | Very deep chain | Not attempted | Not attempted |

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

### The primary remaining problem (spaghetti)

Independent edge routing: `route_connections()` routes each A* edge independently, so earlier edges claim tiles that force later edges into bad paths (loops, wrong directions, contamination). This is the fundamental blocker — see [#62](https://github.com/storkme/fucktorio/issues/62) for the proposed fix (negotiated congestion routing). Spaghetti work is parked while bus layout is the focus.

## Bus layout engine (`src/bus/`)

Deterministic row-based layout using the main bus pattern. Machines grouped by recipe into rows, items transported on parallel trunk belts, with underground sideloading for branches.

### How it works

1. **Solver** produces `SolverResult` (same as spaghetti)
2. **Placer** (`placer.py`) groups machines by recipe into rows, splits rows when throughput exceeds belt capacity (lane-splitting for both lanes of a belt)
3. **Templates** (`templates.py`) stamp out belt/inserter patterns for each row type (single-input, dual-input, with optional lane splitting via sideload bridges)
4. **Bus router** (`bus_router.py`) places trunk belts, routes branches from trunks to rows via underground belt pairs
5. **Validation** runs the same checks as spaghetti (`validate.py`)

### Current status

Bus passes tiers 1-2 with zero validation errors (including electronic-circuit from raw ores with smelting). Next target: tier 3 (plastic-bar — first fluid recipe).

## Verification & validation

- `src/verify.py` — structural blueprint validation (overlap detection, unpaired undergrounds, ASCII map)
- `src/validate.py` — functional validation (pipe isolation, fluid connectivity, inserter chains, power coverage)
- `tests/` — pytest suite with `--viz` flag for HTML visualizations, deployed to GitHub Pages via CI

## Verification protocol for layout engine changes

Layout changes are easy to get wrong — errors can be masked by other changes, imports can shadow silently, and error counts can drop to zero for the wrong reason. Follow this protocol:

1. **Check the viz**: After any routing/placement change, generate `test_viz/iron-gear-wheel-10s.html` and visually inspect it. Zero validation errors mean nothing if the layout looks wrong.
   ```bash
   pytest tests/test_spaghetti.py::TestSpaghettiVisualization::test_viz_iron_gear_wheel --viz -x
   ```
2. **Verify the fix is running**: If you add new logic inside the incremental builder, confirm it actually executes (e.g. add a temporary `logger.info` or check that output changes). The `_evaluate` function catches ALL exceptions silently, so broken code just scores 10000 and gets ignored.
3. **Watch for import shadowing**: `build_layout_incremental()` is a long function. A `from .router import X` anywhere inside it makes `X` a local variable for the ENTIRE function, shadowing any module-level import of `X`. All imports from router should be at the top of the file.
4. **Don't trust error count drops alone**: If errors go from 5 to 0, ask WHY. Check that belt types are correct (should be yellow transport-belt for 10/s), check that the topology makes sense, check that the specific fix you intended is the reason errors dropped.
5. **One search attempt should take <2s** with the Rust A*. If it takes >10s, something is wrong (likely N×A* instead of multi-start, or an infinite loop).

## Visualizations

Test visualizations are deployed to GitHub Pages on main branch pushes:
https://storkme.github.io/fucktorio/
