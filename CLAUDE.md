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
2. **Layout** (`src/spaghetti/`, `src/routing/`, `src/search/`) — Evolutionary search over machine placement, inserter sides, and edge routing order. Places machines on a grid, assigns inserters, then pathfinds belt/pipe routes between them using A*. Returns `LayoutResult`.
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
| `src/routing/router.py` | A* pathfinding, underground belts, belt/pipe entity placement |
| `src/routing/inserters.py` | Lane-aware inserter assignment between machines and belts |
| `src/routing/orchestrate.py` | Shared layout orchestration: inserters → routing → poles |
| `src/routing/graph.py` | Production graph construction from solver output |
| `src/routing/common.py` | Machine sizes, belt tier selection, direction constants |
| `src/routing/poles.py` | Power pole placement (greedy near-machine or grid fallback) |
| `src/search/layout_search.py` | Evolutionary search over placement parameters |
| `src/spaghetti/placer.py` | Incremental machine placement in dependency order |
| `src/spaghetti/layout.py` | Layout orchestrator (entry point for layout engine) |
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

## Layout engine

The layout pipeline: **place machines → assign inserters → route belts/pipes → place poles**.

### Evolutionary search (`src/search/layout_search.py`)

Generates a population of candidate layouts by varying three dimensions:
- **Machine positions** — incremental placement in dependency order, perturbed with Gaussian noise
- **Inserter side preferences** — which sides of each machine to place inserters on
- **Edge routing order** — order in which belt/pipe connections are routed (earlier routes get cleaner paths)

Each candidate is fully built (placement + routing + inserters + poles), validated, and scored. The best survive to the next generation. 30 candidates × 5 generations = 150 full layout evaluations.

### Shared infrastructure (`src/routing/`)

- **Production graph** (`graph.py`): Converts `SolverResult` into a directed graph of `MachineNode`s connected by `FlowEdge`s
- **A* router** (`router.py`): Grid pathfinding with underground belt support, belt tier selection, direction constraints
- **Inserter assignment** (`inserters.py`): Lane-aware inserter placement between machines and belts
- **Common utilities** (`common.py`): Machine sizes, belt tier selection, direction constants

### The fundamental problem

Placement and routing are treated as independent sequential steps, but they're deeply coupled. Placement ignores routing feasibility, then A* routing discovers the placement is bad. The evolutionary search partially addresses this by evaluating real routing outcomes, but it's still a brute-force exploration rather than a principled joint optimization.

## Verification & validation

- `src/verify.py` — structural blueprint validation (overlap detection, unpaired undergrounds, ASCII map)
- `src/validate.py` — functional validation (pipe isolation, fluid connectivity, inserter chains, power coverage)
- `tests/` — pytest suite with `--viz` flag for HTML visualizations, deployed to GitHub Pages via CI

## Visualizations

Test visualizations are deployed to GitHub Pages on main branch pushes:
https://storkme.github.io/fucktorio/
