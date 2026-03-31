# Suggested Commands

## Running Tests
```bash
# Run all tests
pytest tests/

# Run with HTML visualizations
pytest tests/ --viz

# Run a specific test
pytest tests/test_spaghetti.py::TestSpaghettiVisualization::test_viz_iron_gear_wheel --viz -x
```

## Generate a Blueprint
```bash
python -m src.pipeline
```

## Building the Rust Native Module
```bash
# Requires maturin
maturin develop
```

## Linting & Formatting
```bash
# Lint
ruff check src/ tests/

# Format
ruff format src/ tests/

# Fix auto-fixable lint issues
ruff check --fix src/ tests/
```

## Utility Commands (Linux/WSL)
```bash
git status / git log / git diff
ls, cd, grep, find
```

## Visualization
- Test vizzes are generated in `test_viz/` when using `--viz`
- Deployed to GitHub Pages: https://storkme.github.io/fucktorio/
