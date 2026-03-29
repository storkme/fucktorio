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
2. **Layout** — Converts solver output to positioned entities. Returns `LayoutResult`. Three layout engines:
   - `src/layout/` — **Main bus** layout (template-based, human-like factory pattern). Legacy, not actively developed.
   - `src/spaghetti/` — **Constraint-based** layout. Place-and-route approach: position machines on a grid, then pathfind belt/pipe routes between them using A*.
   - `src/ml/` — **ML-optimized** layout (scipy L-BFGS-B placement optimization + shared routing). Uses differentiable loss functions to optimize machine placement, then routes with the same A* infrastructure.
   Both spaghetti and ML engines share routing infrastructure (currently in `src/spaghetti/`, planned move to `src/routing/`).
3. **Blueprint** (`src/blueprint/`) — Thin draftsman wrapper that converts `LayoutResult` to a base64 blueprint string.
4. **Validation** (`src/validate.py`) — Functional checks run after layout: pipe isolation, fluid port connectivity, inserter chains, power coverage.

## Key models (`src/models.py`)

- `ItemFlow` — item name, rate, fluid flag
- `MachineSpec` — machine type, recipe, count, inputs/outputs
- `SolverResult` — machines, external inputs/outputs, dependency order
- `PlacedEntity` — entity name, position, direction, recipe, carries (what item/fluid it transports)
- `LayoutResult` — entities, connections, dimensions

## Key source files

| File | Lines | Purpose |
|------|-------|---------|
| `src/spaghetti/router.py` | ~830 | A* pathfinding, underground belts, belt/pipe entity placement |
| `src/spaghetti/inserters.py` | ~240 | Lane-aware inserter assignment between machines and belts |
| `src/spaghetti/layout.py` | ~230 | Spaghetti orchestrator with retry strategies |
| `src/spaghetti/placer.py` | ~60 | Grid-based machine placement |
| `src/spaghetti/graph.py` | ~175 | Production graph construction from solver output |
| `src/ml/placer.py` | ~120 | Scipy-optimized machine placement |
| `src/ml/loss.py` | ~150 | Differentiable placement loss functions |
| `src/ml/layout.py` | ~220 | ML orchestrator (mirrors spaghetti structure) |
| `src/validate.py` | ~2000 | 16 validation checks (pipe isolation, belt loops, throughput, etc.) |
| `src/models.py` | ~80 | Shared data models (ItemFlow, MachineSpec, SolverResult, PlacedEntity, LayoutResult) |

## Factorio game rules (constraints for layout engines)

These are the physical rules any layout engine must satisfy:

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
| 1 | `iron-gear-wheel` | 1 recipe, 1 solid input | Working |
| 2 | `electronic-circuit` | 2 recipes, 2 solid inputs | Failing -- cross-contamination, belt loops |
| 3 | `plastic-bar` | 1 recipe, 1 fluid + 1 solid input | Failing -- pipe isolation |
| 4 | `advanced-circuit` | 5+ recipes, mixed solid/fluid | Failing -- massive routing failures |
| 5 | `processing-unit` | Deep chain, multiple fluids | Not attempted |
| 6 | `rocket-control-unit` | Very deep chain | Not attempted |

### Known failure modes

1. **Belt cross-item contamination**: Belt carrying item A feeds into a tile carrying item B. Happens when routes for different items cross.
2. **Belt loops**: A* routing accidentally creates cycles in the belt network.
3. **Disconnected networks**: Output belts from different machines don't merge into a connected trunk.
4. **Throughput violations**: Too many inserters feeding the same belt lane.
5. **Failed routing edges**: A* can't find any path from source to destination.
6. **Pipe merge contamination**: Adjacent pipes carrying different fluids merge (fluids connect to all adjacent pipes).

## Layout engines

Both layout engines share the same pipeline: place machines -> assign inserters -> route belts/pipes -> place poles. They differ only in machine placement strategy.

### Shared infrastructure (`src/routing/` planned, currently in `src/spaghetti/`)

- **Production graph** (`graph.py`): Converts `SolverResult` into a directed graph of `MachineNode`s connected by `FlowEdge`s
- **A* router** (`router.py`): Grid pathfinding with underground belt support, belt tier selection, direction constraints
- **Inserter assignment** (`inserters.py`): Lane-aware inserter placement between machines and belts, supports top/bottom and left/right strategies
- **Common utilities** (`common.py`): Machine sizes, belt tier selection, direction constants

### Spaghetti engine (`src/spaghetti/`)

- **Placer**: Simple grid layout -- `cols = ceil(sqrt(n))`, fixed spacing between machines
- **Retry**: Escalating strategy -- varies spacing (4, 6, 8, 10) and inserter side strategy (top/bottom vs left/right)
- **Status**: Works for tier 1 (iron-gear-wheel). Fails on multi-recipe chains due to grid placement creating unavoidable crossings.

### ML engine (`src/ml/`)

- **Placer**: Scipy L-BFGS-B optimization with differentiable loss (overlap penalty, edge distance, compactness, alignment)
- **Retry**: Increases min_gap parameter on failure
- **Loss functions** (`loss.py`): overlap_penalty, edge_distance, compactness, alignment -- combined with configurable weights
- **Status**: Similar results to spaghetti -- placement optimization doesn't overcome fundamental routing conflicts.

### The fundamental problem

Placement and routing are treated as independent sequential steps, but they're deeply coupled. Grid/optimized placement ignores routing feasibility, then A* routing discovers the placement is bad. Fixing this requires constraint-aware placement that considers routing during machine positioning.

## Verification & validation

- `src/verify.py` — structural blueprint validation (overlap detection, unpaired undergrounds, ASCII map)
- `src/validate.py` — functional validation (pipe isolation, fluid connectivity, inserter chains, power coverage)
- `tests/` — pytest suite with `--viz` flag for HTML visualizations, deployed to GitHub Pages via CI

## Visualizations

Test visualizations are deployed to GitHub Pages on main branch pushes:
https://storkme.github.io/fucktorio/
