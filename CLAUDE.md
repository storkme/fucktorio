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
2. **Layout** (`src/layout/`) — Converts solver output to positioned entities: assembly rows (`placer.py`, `templates.py`), main bus routing (`router.py`), and power poles (`poles.py`). Returns `LayoutResult`.
3. **Blueprint** (`src/blueprint/`) — Thin draftsman wrapper that converts `LayoutResult` to a base64 blueprint string.

## Key models (`src/models.py`)

- `ItemFlow` — item name, rate, fluid flag
- `MachineSpec` — machine type, recipe, count, inputs/outputs
- `SolverResult` — machines, external inputs/outputs, dependency order
- `PlacedEntity` — entity name, position, direction, recipe
- `LayoutResult` — entities, connections, dimensions

## Layout conventions

- Main bus runs vertically on the left (underground belts for solids, pipe-to-ground for fluids)
- Assembly rows stack horizontally to the right
- Templates: `single_input_row`, `dual_input_row`, `fluid_row`
- Machines: assembling-machine-3 (default), chemical-plant (fluids), oil-refinery
- Power: medium-electric-pole on a 7-tile grid

## Verification

`src/verify.py` — offline blueprint validation (overlap detection, unpaired undergrounds, ASCII map)
