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
2. **Layout** — Converts solver output to positioned entities. Returns `LayoutResult`. Two layout engines:
   - `src/layout/` — **Main bus** layout (template-based, human-like factory pattern). Legacy, not actively developed.
   - `src/spaghetti/` — **Constraint-based** layout (planned). Place-and-route approach: position machines, then pathfind belt/pipe routes between them. No predefined pattern — produces novel, inhuman layouts.
3. **Blueprint** (`src/blueprint/`) — Thin draftsman wrapper that converts `LayoutResult` to a base64 blueprint string.
4. **Validation** (`src/validate.py`) — Functional checks run after layout: pipe isolation, fluid port connectivity, inserter chains, power coverage.

## Key models (`src/models.py`)

- `ItemFlow` — item name, rate, fluid flag
- `MachineSpec` — machine type, recipe, count, inputs/outputs
- `SolverResult` — machines, external inputs/outputs, dependency order
- `PlacedEntity` — entity name, position, direction, recipe, carries (what item/fluid it transports)
- `LayoutResult` — entities, connections, dimensions

## Factorio game rules (constraints for layout engines)

These are the physical rules any layout engine must satisfy:

- **Machines** craft recipes, need ingredients delivered and products extracted
- **Inserters** pick from one side, drop to the other. Regular inserters reach 1 tile; long-handed inserters reach 2 tiles
- **Belts** move items in a direction, connect when adjacent. Different tiers have different throughput limits (yellow: 15/s, red: 30/s, blue: 45/s)
- **Pipes** carry fluids, connect to any adjacent pipe (and merge — separate fluid networks must be physically isolated)
- **Fluid ports** on machines are at specific tile positions (queryable from `draftsman.data.entities`)
- **Entities** cannot overlap
- **Power** — machines need electricity; medium-electric-pole covers a 7x7 area

## Spaghetti builder approach (planned)

Goal: given a production graph from the solver, produce a working factory layout without any predefined pattern (no bus, no rows, no templates). Analogous to PCB place-and-route.

1. **Place** machines on the grid (e.g. force-directed, greedy, or random with retry)
2. **Route** belts/pipes between connected machines using grid pathfinding (BFS/A*)
3. **Validate** the result using `src/validate.py`
4. **Retry/adjust** if routing fails or validation finds issues

Key considerations:
- Belt throughput limits must be respected (don't overload a single belt)
- Inserter type selection (regular vs long-handed) based on layout geometry
- Pipe isolation (parallel pipes carrying different fluids will merge)
- Underground belts/pipes for crossing routes

## Verification & validation

- `src/verify.py` — structural blueprint validation (overlap detection, unpaired undergrounds, ASCII map)
- `src/validate.py` — functional validation (pipe isolation, fluid connectivity, inserter chains, power coverage)
- `tests/` — pytest suite with `--viz` flag for HTML visualizations, deployed to GitHub Pages via CI

## Visualizations

Test visualizations are deployed to GitHub Pages on main branch pushes:
https://storkme.github.io/fucktorio/
