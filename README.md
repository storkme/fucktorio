# Fucktorio

Automated Factorio factory blueprint generator. Takes a target item + production rate, solves recipe dependencies, generates a spatial layout, and exports a Factorio-importable blueprint string.

## Quick start

```bash
pip install factorio-draftsman pytest

# Run tests
pytest tests/

# Generate a blueprint
python -m src.pipeline
```

## Dependencies

- **[factorio-draftsman](https://github.com/redruin1/factorio-draftsman)** — Provides the Factorio data layer and blueprint serialization. We use it for:
  - **Recipe & entity database** (`draftsman.data.recipes`, `draftsman.data.entities`) — look up recipes, crafting speeds, machine types, entity properties, and fluid port positions
  - **Blueprint construction** (`draftsman.blueprintable.Blueprint`, `draftsman.entity.new_entity`) — build blueprint objects and export the base64 strings that Factorio can import
  - **Blueprint parsing** (`draftsman.blueprintable.get_blueprintable_from_string`) — decode existing blueprints for visualization and verification

- **[pytest](https://docs.pytest.org/)** — Test runner. The `--viz` flag generates HTML visualizations of blueprint layouts, deployed to [GitHub Pages](https://storkme.github.io/fucktorio/).

All layout logic (recipe solving, spatial placement, routing, validation) is implemented in this project — draftsman provides the data and serialization, not the spatial reasoning.

## Architecture

1. **Solver** — recursively resolves recipe dependencies, calculates machine counts and item flow rates
2. **Layout** — positions machines and routes belts/pipes on a 2D tile grid
3. **Validation** — checks the layout actually works (pipe isolation, fluid connectivity, inserter chains, power coverage)
4. **Blueprint** — serializes the layout to a Factorio-importable base64 string

## Direction

The layout engine uses evolutionary search over machine placement, inserter sides, and routing order — no predefined patterns, no templates. Given only Factorio's game rules, it places machines and pathfinds belt/pipe routes between them using A* — a place-and-route approach analogous to PCB autorouting. The goal is to produce working but novel "spaghetti" factories.
