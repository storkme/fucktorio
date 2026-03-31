# Code Style and Conventions

## Python Style
- **Python 3.12+**, uses `from __future__ import annotations`
- **Dataclasses** for data models (not Pydantic)
- **Type hints** on function signatures (PEP 604 union syntax `X | Y`)
- **Docstrings** on classes and key functions (brief, one-line or short paragraph)
- **Line length**: 120 characters (ruff config)
- **Naming**: snake_case for functions/variables, PascalCase for classes, UPPER_SNAKE for constants
- **Private conventions**: underscore prefix for internal functions/constants (e.g. `_MACHINE_SIZE`, `_fix_belt_directions`)
- **Imports**: `from __future__ import annotations` at top, stdlib → third-party → local, relative imports within `src/`

## Ruff Configuration
- Target: py313
- Rules: E, F, I, UP, B, SIM
- isort with `src` as known first-party

## Testing
- pytest with custom fixtures in `conftest.py`
- Session-scoped fixtures for expensive operations (solver, layout)
- `--viz` flag for generating HTML visualizations
- Tests in `tests/` directory

## Project Patterns
- Incremental place-and-route: place one machine, route immediately, repeat
- Parallel random search over layout candidates (60 candidates)
- Validation errors as the primary quality metric
- `_evaluate` catches ALL exceptions silently — broken code scores 10000
