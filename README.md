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
  - **Recipe & entity database** (`draftsman.data.recipes`, `draftsman.data.entities`) — look up recipes, crafting speeds, machine types, and entity properties
  - **Blueprint construction** (`draftsman.blueprintable.Blueprint`, `draftsman.entity.new_entity`) — build blueprint objects and export the base64 strings that Factorio can import
  - **Blueprint parsing** (`draftsman.blueprintable.get_blueprintable_from_string`) — decode existing blueprints for visualization and verification

- **[pytest](https://docs.pytest.org/)** — Test runner. The `--viz` flag generates HTML visualizations of blueprint layouts, which are deployed to GitHub Pages via CI.

All layout logic (recipe solving, spatial placement, bus routing, power poles) and validation (overlap detection, underground belt pairing) is implemented in this project — draftsman provides the data and serialization, not the spatial reasoning.
