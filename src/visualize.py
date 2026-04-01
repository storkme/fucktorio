"""Rich HTML visualization of blueprint layouts.

Generates a self-contained HTML file with:
  - Color-coded tile grid (each recipe gets a distinct color)
  - Hover tooltips showing entity details
  - Clickable legend with recipe highlighting
  - Production summary with dependency tree
  - Zoomable / scrollable map
"""

from __future__ import annotations

import html
import json
import math
import os
import webbrowser
from collections import Counter
from pathlib import Path

from draftsman.blueprintable import get_blueprintable_from_string

from src.renderers import THEME_JS

# Distinct, colorblind-friendly palette for recipes
_RECIPE_COLORS = [
    "#4e79a7",  # steel blue
    "#f28e2b",  # orange
    "#e15759",  # red
    "#76b7b2",  # teal
    "#59a14f",  # green
    "#edc948",  # yellow
    "#b07aa1",  # purple
    "#ff9da7",  # pink
    "#9c755f",  # brown
    "#bab0ac",  # grey
    "#86bcb6",  # light teal
    "#d37295",  # rose
    "#a0cbe8",  # light blue
    "#ffbe7d",  # light orange
    "#8cd17d",  # light green
]

# Fixed colors for infrastructure entities
_INFRA_COLORS = {
    "transport-belt": "#c8b560",
    "fast-transport-belt": "#e05050",
    "express-transport-belt": "#50a0e0",
    "underground-belt": "#a89040",
    "fast-underground-belt": "#e05050",
    "express-underground-belt": "#50a0e0",
    "inserter": "#6a8e3e",
    "fast-inserter": "#4a90d0",
    "long-handed-inserter": "#d04040",
    "pipe": "#4a7ab5",
    "pipe-to-ground": "#3a6090",
    "medium-electric-pole": "#8b6914",
    "splitter": "#c8b560",
    "fast-splitter": "#e05050",
    "express-splitter": "#50a0e0",
    "pump": "#4a7a6a",
}

_INFRA_LABELS = {
    "transport-belt": "Belt (yellow)",
    "fast-transport-belt": "Belt (red)",
    "express-transport-belt": "Belt (blue)",
    "underground-belt": "UG Belt",
    "fast-underground-belt": "UG Belt (red)",
    "express-underground-belt": "UG Belt (blue)",
    "inserter": "Inserter",
    "fast-inserter": "Fast Inserter",
    "long-handed-inserter": "Long Inserter",
    "pipe": "Pipe",
    "pipe-to-ground": "UG Pipe",
    "medium-electric-pole": "Pole",
    "splitter": "Splitter",
    "fast-splitter": "Splitter (red)",
    "express-splitter": "Splitter (blue)",
    "pump": "Pump",
}

# Multi-tile infra entities — (w, h) for entities that aren't 1x1
_INFRA_SIZES: dict[str, tuple[int, int]] = {
    "splitter": (2, 1),
    "fast-splitter": (2, 1),
    "express-splitter": (2, 1),
    "pump": (1, 2),
}

# Entity sizes — (w, h) for multi-tile entities that can hold recipes
_CRAFTING_SIZES: dict[str, tuple[int, int]] = {
    # Base game
    "assembling-machine-1": (3, 3),
    "assembling-machine-2": (3, 3),
    "assembling-machine-3": (3, 3),
    "chemical-plant": (3, 3),
    "oil-refinery": (5, 5),
    "stone-furnace": (2, 2),
    "steel-furnace": (2, 2),
    "electric-furnace": (3, 3),
    "centrifuge": (3, 3),
    "lab": (3, 3),
    # Space Age
    "foundry": (5, 5),
    "biochamber": (3, 3),
    "biolab": (5, 5),
    "electromagnetic-plant": (4, 4),
    "cryogenic-plant": (5, 5),
    "recycler": (2, 4),
    "crusher": (2, 3),
    "captive-biter-spawner": (5, 5),
    "rocket-silo": (9, 9),
}
_CRAFTING = set(_CRAFTING_SIZES)

# Non-crafting multi-tile entities (no recipe, fixed color + size)
_SUPPORT_SIZES: dict[str, tuple[int, int, str]] = {
    # name -> (w, h, color)
    "beacon": (3, 3, "#4a6080"),
    "storage-tank": (3, 3, "#4a6a5a"),
    "big-electric-pole": (2, 2, "#8b6914"),
    "substation": (2, 2, "#6a6a8b"),
    "electric-mining-drill": (3, 3, "#7a6a30"),
}


def _serialize_lane_rates(lane_rates: dict | None) -> str:
    """Serialize lane_rates to JSON with string keys for JS consumption."""
    if not lane_rates:
        return "null"
    return json.dumps({f"{x},{y}": rates for (x, y), rates in lane_rates.items()})


def visualize(
    bp_string: str,
    output_path: str | None = None,
    open_browser: bool = True,
    solver_result=None,
    production_graph=None,
    validation_issues=None,
    layout_result=None,
    lane_rates: dict | None = None,
) -> str:
    """Generate an HTML visualization of a blueprint string.

    Args:
        bp_string: Factorio blueprint string.
        output_path: Where to write the HTML file. Defaults to ./blueprint_viz.html.
        open_browser: Whether to open the file in the default browser.
        solver_result: Optional SolverResult for richer production info.
        production_graph: Optional ProductionGraph for flow diagram panel.
        validation_issues: Optional list of ValidationIssue for error display.

    Returns:
        Path to the generated HTML file.
    """
    import warnings

    from draftsman.warning import DraftsmanWarning

    with warnings.catch_warnings(record=True):
        warnings.simplefilter("always", DraftsmanWarning)
        bp = get_blueprintable_from_string(bp_string)

    if output_path is None:
        output_path = "blueprint_viz.html"

    # --- Collect data ---
    # Assign colors to recipes
    recipe_colors: dict[str, str] = {}
    recipe_order: list[str] = []
    for e in bp.entities:
        recipe = getattr(e, "recipe", None)
        if e.name in _CRAFTING and recipe and recipe not in recipe_colors:
            idx = len(recipe_order)
            recipe_colors[recipe] = _RECIPE_COLORS[idx % len(_RECIPE_COLORS)]
            recipe_order.append(recipe)

    # Recipe counts
    recipe_counts: Counter = Counter()
    for e in bp.entities:
        recipe = getattr(e, "recipe", None)
        if e.name in _CRAFTING and recipe:
            recipe_counts[recipe] += 1

    # Entity counts
    entity_counts: Counter = Counter()
    for e in bp.entities:
        entity_counts[e.name] += 1

    # Build carries lookup from layout_result (draftsman entities don't preserve carries)
    carries_lookup: dict[tuple[int, int], str] = {}
    if layout_result is not None:
        for ent in layout_result.entities:
            if ent.carries:
                carries_lookup[(ent.x, ent.y)] = ent.carries

    # Build tile data: list of {x, y, w, h, color, entity, recipe, tooltip, direction}
    tiles: list[dict] = []
    for e in bp.entities:
        tx = int(e.tile_position.x) if hasattr(e.tile_position, "x") else int(e.tile_position[0])
        ty = int(e.tile_position.y) if hasattr(e.tile_position, "y") else int(e.tile_position[1])

        # Get direction (0=N, 4=E, 8=S, 12=W)
        direction = int(getattr(e, "direction", 0) or 0)
        carries = carries_lookup.get((tx, ty)) or getattr(e, "carries", None) or ""

        if e.name in _SUPPORT_SIZES:
            sw, sh, scolor = _SUPPORT_SIZES[e.name]
            tiles.append(
                {
                    "x": tx,
                    "y": ty,
                    "w": sw,
                    "h": sh,
                    "color": scolor,
                    "entity": e.name,
                    "recipe": "",
                    "tooltip": e.name,
                    "dir": direction,
                    "carries": carries,
                }
            )
        elif e.name in _CRAFTING_SIZES:
            cw, ch = _CRAFTING_SIZES[e.name]
            recipe = getattr(e, "recipe", None)
            color = recipe_colors.get(recipe, "#888")
            tooltip = f"{e.name}\\n{recipe}"
            tiles.append(
                {
                    "x": tx,
                    "y": ty,
                    "w": cw,
                    "h": ch,
                    "color": color,
                    "entity": e.name,
                    "recipe": recipe or "",
                    "tooltip": tooltip,
                    "dir": direction,
                    "carries": carries,
                }
            )
        else:
            color = _INFRA_COLORS.get(e.name, "#666")
            label = _INFRA_LABELS.get(e.name, e.name)
            tooltip = label
            if carries:
                tooltip += f" [{carries}]"
            io_type = ""
            if e.name in ("underground-belt", "fast-underground-belt", "express-underground-belt"):
                io_type = getattr(e, "io_type", None) or ""
                if io_type:
                    tooltip += f" ({io_type})"
            iw, ih = _INFRA_SIZES.get(e.name, (1, 1))
            # Splitters/pumps rotate: swap w/h for E/W directions
            if iw != ih and direction in (4, 12):
                iw, ih = ih, iw
            tiles.append(
                {
                    "x": tx,
                    "y": ty,
                    "w": iw,
                    "h": ih,
                    "color": color,
                    "entity": e.name,
                    "recipe": "",
                    "tooltip": tooltip,
                    "dir": direction,
                    "carries": carries,
                    "ioType": io_type,
                }
            )

    # Bounding box
    if not tiles:
        min_x = min_y = max_x = max_y = 0
    else:
        min_x = min(t["x"] for t in tiles)
        min_y = min(t["y"] for t in tiles)
        max_x = max(t["x"] + t["w"] for t in tiles)
        max_y = max(t["y"] + t["h"] for t in tiles)

    grid_w = max_x - min_x
    grid_h = max_y - min_y

    # Build solver info if available
    solver_info = ""
    if solver_result:
        recipe_to_spec = {m.recipe: m for m in solver_result.machines}
        rows = []
        for recipe in recipe_order:
            spec = recipe_to_spec.get(recipe)
            if spec:
                rows.append(
                    {
                        "recipe": recipe,
                        "entity": spec.entity,
                        "count": f"{spec.count:.1f}",
                        "placed": str(math.ceil(spec.count)),
                        "inputs": [
                            {"item": f.item, "rate": f"{f.rate * spec.count:.1f}/s", "fluid": f.is_fluid}
                            for f in spec.inputs
                        ],
                        "outputs": [
                            {"item": f.item, "rate": f"{f.rate * spec.count:.1f}/s", "fluid": f.is_fluid}
                            for f in spec.outputs
                        ],
                    }
                )
        solver_info = json.dumps(rows)

    # Build legend data
    legend_recipes = json.dumps(
        [{"recipe": r, "color": recipe_colors[r], "count": recipe_counts[r]} for r in recipe_order]
    )
    legend_infra = json.dumps(
        [
            {
                "entity": name,
                "label": _INFRA_LABELS.get(name, name),
                "color": _INFRA_COLORS.get(name, "#666"),
                "count": entity_counts[name],
            }
            for name in sorted(entity_counts.keys())
            if name not in _CRAFTING
        ]
    )

    label = html.escape(bp.label or "Blueprint")
    total_entities = len(bp.entities)

    # Build production graph data if available
    graph_data = "null"
    if production_graph is not None:
        graph_nodes = []
        for node in production_graph.nodes:
            graph_nodes.append(
                {
                    "id": node.id,
                    "recipe": node.spec.recipe,
                    "entity": node.spec.entity,
                    "instance": node.instance,
                    "color": recipe_colors.get(node.spec.recipe, "#888"),
                }
            )
        graph_edges = []
        for edge in production_graph.edges:
            graph_edges.append(
                {
                    "item": edge.item,
                    "rate": round(edge.rate, 2),
                    "is_fluid": edge.is_fluid,
                    "from": edge.from_node,
                    "to": edge.to_node,
                }
            )
        graph_data = json.dumps({"nodes": graph_nodes, "edges": graph_edges})

    # Build validation issues data
    validation_data = "null"
    if validation_issues:
        validation_data = json.dumps(
            [
                {
                    "severity": i.severity,
                    "category": i.category,
                    "message": i.message,
                    "x": i.x,
                    "y": i.y,
                }
                for i in validation_issues
            ]
        )

    _template_path = Path(__file__).parent / "templates" / "visualize.html"
    html_content = _template_path.read_text()
    replacements = {
        "__TITLE__": html.escape(label),
        "__TOTAL_ENTITIES__": str(total_entities),
        "__GRID_W__": str(grid_w),
        "__GRID_H__": str(grid_h),
        "__MIN_X__": str(min_x),
        "__MIN_Y__": str(min_y),
        "__TILES_JSON__": json.dumps(tiles),
        "__LEGEND_RECIPES__": legend_recipes,
        "__LEGEND_INFRA__": legend_infra,
        "__SOLVER_INFO__": solver_info or "null",
        "__GRAPH_DATA__": graph_data,
        "__VALIDATION_DATA__": validation_data,
        "__LANE_RATES__": _serialize_lane_rates(lane_rates),
        "/* __THEME_JS__ */": THEME_JS,
    }
    for token, value in replacements.items():
        html_content = html_content.replace(token, value)

    Path(output_path).write_text(html_content)
    print(f"  Visualization: {output_path}")

    if open_browser:
        webbrowser.open(f"file://{os.path.abspath(output_path)}")

    return output_path
