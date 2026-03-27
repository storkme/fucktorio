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
    "underground-belt": "#a89040",
    "inserter": "#6a8e3e",
    "pipe": "#4a7ab5",
    "pipe-to-ground": "#3a6090",
    "medium-electric-pole": "#8b6914",
    "splitter": "#c8b560",
}

_INFRA_LABELS = {
    "transport-belt": "Belt",
    "underground-belt": "UG Belt",
    "inserter": "Inserter",
    "pipe": "Pipe",
    "pipe-to-ground": "UG Pipe",
    "medium-electric-pole": "Pole",
    "splitter": "Splitter",
}

_3x3 = {
    "assembling-machine-1",
    "assembling-machine-2",
    "assembling-machine-3",
    "chemical-plant",
}
_5x5 = {"oil-refinery"}
_CRAFTING = _3x3 | _5x5


def visualize(
    bp_string: str,
    output_path: str | None = None,
    open_browser: bool = True,
    solver_result=None,
    production_graph=None,
) -> str:
    """Generate an HTML visualization of a blueprint string.

    Args:
        bp_string: Factorio blueprint string.
        output_path: Where to write the HTML file. Defaults to ./blueprint_viz.html.
        open_browser: Whether to open the file in the default browser.
        solver_result: Optional SolverResult for richer production info.
        production_graph: Optional ProductionGraph for flow diagram panel.

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
        if e.name in _CRAFTING and e.recipe and e.recipe not in recipe_colors:
            idx = len(recipe_order)
            recipe_colors[e.recipe] = _RECIPE_COLORS[idx % len(_RECIPE_COLORS)]
            recipe_order.append(e.recipe)

    # Recipe counts
    recipe_counts: Counter = Counter()
    for e in bp.entities:
        if e.name in _CRAFTING and e.recipe:
            recipe_counts[e.recipe] += 1

    # Entity counts
    entity_counts: Counter = Counter()
    for e in bp.entities:
        entity_counts[e.name] += 1

    # Build tile data: list of {x, y, w, h, color, entity, recipe, tooltip}
    tiles: list[dict] = []
    for e in bp.entities:
        tx = int(e.tile_position.x) if hasattr(e.tile_position, "x") else int(e.tile_position[0])
        ty = int(e.tile_position.y) if hasattr(e.tile_position, "y") else int(e.tile_position[1])

        if e.name in _CRAFTING:
            size = 5 if e.name in _5x5 else 3
            color = recipe_colors.get(e.recipe, "#888")
            tooltip = f"{e.name}\\n{e.recipe}"
            tiles.append(
                {
                    "x": tx,
                    "y": ty,
                    "w": size,
                    "h": size,
                    "color": color,
                    "entity": e.name,
                    "recipe": e.recipe or "",
                    "tooltip": tooltip,
                }
            )
        else:
            color = _INFRA_COLORS.get(e.name, "#666")
            label = _INFRA_LABELS.get(e.name, e.name)
            tooltip = label
            if e.name == "underground-belt":
                io = getattr(e, "io_type", None)
                if io:
                    tooltip += f" ({io})"
            tiles.append(
                {
                    "x": tx,
                    "y": ty,
                    "w": 1,
                    "h": 1,
                    "color": color,
                    "entity": e.name,
                    "recipe": "",
                    "tooltip": tooltip,
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

    html_content = _HTML_TEMPLATE.format(
        title=label,
        total_entities=total_entities,
        grid_w=grid_w,
        grid_h=grid_h,
        min_x=min_x,
        min_y=min_y,
        tiles_json=json.dumps(tiles),
        legend_recipes=legend_recipes,
        legend_infra=legend_infra,
        solver_info=solver_info or "null",
        graph_data=graph_data,
    )

    Path(output_path).write_text(html_content)
    print(f"  Visualization: {output_path}")

    if open_browser:
        webbrowser.open(f"file://{os.path.abspath(output_path)}")

    return output_path


_HTML_TEMPLATE = """\
<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>{title} — Fucktorio Visualizer</title>
<style>
  * {{ margin: 0; padding: 0; box-sizing: border-box; }}
  body {{
    font-family: 'Segoe UI', system-ui, -apple-system, sans-serif;
    background: #1a1a2e;
    color: #e0e0e0;
    display: flex;
    height: 100vh;
    overflow: hidden;
  }}

  /* Sidebar */
  #sidebar {{
    width: 320px;
    min-width: 320px;
    background: #16213e;
    border-right: 2px solid #0f3460;
    overflow-y: auto;
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }}
  #sidebar h1 {{
    font-size: 18px;
    color: #e94560;
    margin-bottom: 4px;
  }}
  #sidebar h2 {{
    font-size: 14px;
    color: #0f3460;
    background: #e94560;
    padding: 4px 8px;
    border-radius: 4px;
    margin-bottom: 8px;
    text-transform: uppercase;
    letter-spacing: 1px;
  }}
  .stat {{
    font-size: 13px;
    color: #a0a0b0;
    margin-bottom: 2px;
  }}
  .stat b {{ color: #e0e0e0; }}

  /* Legend */
  .legend-section {{ margin-bottom: 8px; }}
  .legend-item {{
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 6px;
    border-radius: 4px;
    cursor: pointer;
    font-size: 13px;
    transition: background 0.15s;
  }}
  .legend-item:hover {{ background: rgba(255,255,255,0.08); }}
  .legend-item.active {{ background: rgba(255,255,255,0.15); }}
  .legend-swatch {{
    width: 16px; height: 16px;
    border-radius: 3px;
    flex-shrink: 0;
    border: 1px solid rgba(255,255,255,0.2);
  }}
  .legend-label {{ flex: 1; }}
  .legend-count {{
    color: #888;
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }}

  /* Solver info */
  .solver-table {{
    width: 100%;
    border-collapse: collapse;
    font-size: 12px;
  }}
  .solver-table th {{
    text-align: left;
    padding: 4px 6px;
    border-bottom: 1px solid #0f3460;
    color: #888;
    font-weight: normal;
    text-transform: uppercase;
    font-size: 11px;
  }}
  .solver-table td {{
    padding: 4px 6px;
    border-bottom: 1px solid rgba(15,52,96,0.5);
  }}
  .solver-table tr:hover {{ background: rgba(255,255,255,0.05); }}
  .solver-recipe-swatch {{
    display: inline-block;
    width: 10px; height: 10px;
    border-radius: 2px;
    margin-right: 4px;
    vertical-align: middle;
  }}
  .fluid-tag {{
    font-size: 10px;
    background: #4a7ab5;
    color: white;
    padding: 1px 4px;
    border-radius: 2px;
    margin-left: 4px;
  }}

  /* Canvas area */
  #canvas-wrap {{
    flex: 1;
    position: relative;
    overflow: hidden;
    background: #0f0f23;
  }}
  canvas {{
    position: absolute;
    top: 0; left: 0;
    image-rendering: pixelated;
  }}

  /* Tooltip */
  #tooltip {{
    position: fixed;
    background: #16213e;
    border: 1px solid #0f3460;
    border-radius: 6px;
    padding: 8px 12px;
    font-size: 13px;
    pointer-events: none;
    display: none;
    z-index: 100;
    box-shadow: 0 4px 12px rgba(0,0,0,0.5);
    max-width: 250px;
  }}
  #tooltip .tt-entity {{ color: #e94560; font-weight: 600; }}
  #tooltip .tt-recipe {{ color: #76b7b2; }}
  #tooltip .tt-pos {{ color: #888; font-size: 11px; }}

  /* Tabs */
  #tab-bar {{
    display: flex;
    gap: 2px;
    padding: 8px 8px 0 8px;
    background: #0f0f23;
  }}
  .tab-btn {{
    background: #16213e;
    border: 1px solid #0f3460;
    border-bottom: none;
    color: #a0a0b0;
    padding: 8px 16px;
    border-radius: 6px 6px 0 0;
    cursor: pointer;
    font-size: 13px;
    font-family: inherit;
    transition: background 0.15s;
  }}
  .tab-btn:hover {{ background: #1a3a6e; color: #e0e0e0; }}
  .tab-btn.active {{ background: #0f0f23; color: #e94560; border-color: #e94560; }}
  .tab-panel {{ display: none; flex: 1; position: relative; overflow: hidden; background: #0f0f23; }}
  .tab-panel.active {{ display: block; }}

  /* Main content area */
  #main-area {{
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }}

  /* Controls */
  #controls {{
    position: absolute;
    bottom: 12px; right: 12px;
    display: flex;
    gap: 6px;
  }}
  #controls button {{
    background: #16213e;
    border: 1px solid #0f3460;
    color: #e0e0e0;
    width: 36px; height: 36px;
    border-radius: 6px;
    cursor: pointer;
    font-size: 18px;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: background 0.15s;
  }}
  #controls button:hover {{ background: #1a3a6e; }}

  /* Graph panel */
  #graph-panel canvas {{
    position: absolute;
    top: 0; left: 0;
  }}
</style>
</head>
<body>

<div id="sidebar">
  <div>
    <h1>{title}</h1>
    <div class="stat"><b>{total_entities}</b> entities &middot; <b>{grid_w}</b>&times;<b>{grid_h}</b> tiles</div>
  </div>

  <div class="legend-section">
    <h2>Recipes</h2>
    <div id="legend-recipes"></div>
  </div>

  <div class="legend-section">
    <h2>Infrastructure</h2>
    <div id="legend-infra"></div>
  </div>

  <div id="solver-section" style="display:none">
    <h2>Production</h2>
    <table class="solver-table">
      <thead><tr><th>Recipe</th><th>Count</th><th>Inputs</th></tr></thead>
      <tbody id="solver-body"></tbody>
    </table>
  </div>
</div>

<div id="main-area">
  <div id="tab-bar">
    <button class="tab-btn active" data-tab="layout-panel">Layout</button>
    <button class="tab-btn" data-tab="graph-panel" id="graph-tab" style="display:none">Flow Graph</button>
  </div>

  <div id="layout-panel" class="tab-panel active">
    <canvas id="grid"></canvas>
    <div id="controls">
      <button id="btn-fit" title="Fit to view">&#8982;</button>
      <button id="btn-zin" title="Zoom in">+</button>
      <button id="btn-zout" title="Zoom out">&minus;</button>
    </div>
  </div>

  <div id="graph-panel" class="tab-panel">
    <canvas id="graph-canvas"></canvas>
  </div>
</div>

<div id="tooltip">
  <div class="tt-entity"></div>
  <div class="tt-recipe"></div>
  <div class="tt-pos"></div>
</div>

<script>
const TILES = {tiles_json};
const LEGEND_RECIPES = {legend_recipes};
const LEGEND_INFRA = {legend_infra};
const SOLVER_INFO = {solver_info};
const GRAPH_DATA = {graph_data};
const MIN_X = {min_x};
const MIN_Y = {min_y};
const GRID_W = {grid_w};
const GRID_H = {grid_h};

// --- Tab switching ---
document.querySelectorAll('.tab-btn').forEach(btn => {{
  btn.addEventListener('click', () => {{
    document.querySelectorAll('.tab-btn').forEach(b => b.classList.remove('active'));
    document.querySelectorAll('.tab-panel').forEach(p => p.classList.remove('active'));
    btn.classList.add('active');
    document.getElementById(btn.dataset.tab).classList.add('active');
    if (btn.dataset.tab === 'graph-panel') renderGraph();
  }});
}});
if (GRAPH_DATA) document.getElementById('graph-tab').style.display = '';

// --- Canvas setup ---
const wrap = document.getElementById('layout-panel');
const canvas = document.getElementById('grid');
const ctx = canvas.getContext('2d');
const tooltip = document.getElementById('tooltip');

let scale = 1;
let offsetX = 0, offsetY = 0;
let dragging = false, dragStartX = 0, dragStartY = 0;
let highlightRecipe = null;

function resize() {{
  canvas.width = wrap.clientWidth;
  canvas.height = wrap.clientHeight;
  draw();
}}
window.addEventListener('resize', resize);

function fitToView() {{
  const pad = 40;
  const scaleX = (wrap.clientWidth - pad * 2) / GRID_W;
  const scaleY = (wrap.clientHeight - pad * 2) / GRID_H;
  scale = Math.min(scaleX, scaleY, 24);
  scale = Math.max(scale, 1);
  offsetX = (wrap.clientWidth - GRID_W * scale) / 2;
  offsetY = (wrap.clientHeight - GRID_H * scale) / 2;
  draw();
}}

function draw() {{
  ctx.clearRect(0, 0, canvas.width, canvas.height);

  // Grid background
  ctx.fillStyle = '#0a0a1a';
  ctx.fillRect(0, 0, canvas.width, canvas.height);

  // Draw grid lines if zoomed in enough
  if (scale >= 8) {{
    ctx.strokeStyle = 'rgba(255,255,255,0.04)';
    ctx.lineWidth = 1;
    for (let x = 0; x <= GRID_W; x++) {{
      const px = offsetX + x * scale;
      ctx.beginPath(); ctx.moveTo(px, offsetY); ctx.lineTo(px, offsetY + GRID_H * scale); ctx.stroke();
    }}
    for (let y = 0; y <= GRID_H; y++) {{
      const py = offsetY + y * scale;
      ctx.beginPath(); ctx.moveTo(offsetX, py); ctx.lineTo(offsetX + GRID_W * scale, py); ctx.stroke();
    }}
  }}

  // Draw tiles
  for (const t of TILES) {{
    const px = offsetX + (t.x - MIN_X) * scale;
    const py = offsetY + (t.y - MIN_Y) * scale;
    const pw = t.w * scale;
    const ph = t.h * scale;

    let alpha = 1.0;
    if (highlightRecipe) {{
      if (t.recipe === highlightRecipe) {{
        alpha = 1.0;
      }} else if (t.recipe) {{
        alpha = 0.15;
      }} else {{
        alpha = 0.3;
      }}
    }}

    ctx.globalAlpha = alpha;
    ctx.fillStyle = t.color;
    ctx.fillRect(px, py, pw - (scale >= 4 ? 1 : 0), ph - (scale >= 4 ? 1 : 0));

    // Draw recipe label on machines when zoomed in
    if (t.recipe && scale >= 12) {{
      ctx.fillStyle = 'rgba(0,0,0,0.6)';
      ctx.font = `bold ${{Math.max(8, scale * 0.6)}}px sans-serif`;
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      ctx.fillText(t.recipe.substring(0, 6), px + pw / 2, py + ph / 2);
    }}
  }}
  ctx.globalAlpha = 1.0;
}}

// --- Pan & zoom ---
wrap.addEventListener('wheel', (e) => {{
  e.preventDefault();
  const rect = wrap.getBoundingClientRect();
  const mx = e.clientX - rect.left;
  const my = e.clientY - rect.top;

  const oldScale = scale;
  const factor = e.deltaY < 0 ? 1.15 : 1 / 1.15;
  scale = Math.min(Math.max(scale * factor, 0.5), 48);

  // Zoom toward mouse position
  offsetX = mx - (mx - offsetX) * (scale / oldScale);
  offsetY = my - (my - offsetY) * (scale / oldScale);
  draw();
}});

wrap.addEventListener('mousedown', (e) => {{
  dragging = true;
  dragStartX = e.clientX - offsetX;
  dragStartY = e.clientY - offsetY;
  wrap.style.cursor = 'grabbing';
}});
window.addEventListener('mousemove', (e) => {{
  if (dragging) {{
    offsetX = e.clientX - dragStartX;
    offsetY = e.clientY - dragStartY;
    draw();
  }}
  // Tooltip
  const rect = wrap.getBoundingClientRect();
  const mx = e.clientX - rect.left;
  const my = e.clientY - rect.top;
  const tileX = Math.floor((mx - offsetX) / scale) + MIN_X;
  const tileY = Math.floor((my - offsetY) / scale) + MIN_Y;

  let found = null;
  for (const t of TILES) {{
    if (tileX >= t.x && tileX < t.x + t.w && tileY >= t.y && tileY < t.y + t.h) {{
      found = t;
      break;
    }}
  }}

  if (found && mx >= 0 && my >= 0 && mx <= rect.width && my <= rect.height) {{
    tooltip.style.display = 'block';
    tooltip.style.left = (e.clientX + 12) + 'px';
    tooltip.style.top = (e.clientY + 12) + 'px';
    tooltip.querySelector('.tt-entity').textContent = found.entity;
    tooltip.querySelector('.tt-recipe').textContent = found.recipe || '';
    tooltip.querySelector('.tt-pos').textContent = `(${{tileX}}, ${{tileY}})`;
  }} else {{
    tooltip.style.display = 'none';
  }}
}});
window.addEventListener('mouseup', () => {{
  dragging = false;
  wrap.style.cursor = 'default';
}});

// --- Legend ---
function buildLegend() {{
  const recipesEl = document.getElementById('legend-recipes');
  for (const item of LEGEND_RECIPES) {{
    const div = document.createElement('div');
    div.className = 'legend-item';
    div.innerHTML = `<div class="legend-swatch" style="background:${{item.color}}"></div>` +
      `<span class="legend-label">${{item.recipe}}</span>` +
      `<span class="legend-count">&times;${{item.count}}</span>`;
    div.addEventListener('click', () => {{
      if (highlightRecipe === item.recipe) {{
        highlightRecipe = null;
        div.classList.remove('active');
      }} else {{
        highlightRecipe = item.recipe;
        recipesEl.querySelectorAll('.legend-item').forEach(el => el.classList.remove('active'));
        div.classList.add('active');
      }}
      draw();
    }});
    recipesEl.appendChild(div);
  }}

  const infraEl = document.getElementById('legend-infra');
  for (const item of LEGEND_INFRA) {{
    const div = document.createElement('div');
    div.className = 'legend-item';
    div.innerHTML = `<div class="legend-swatch" style="background:${{item.color}}"></div>` +
      `<span class="legend-label">${{item.label}}</span>` +
      `<span class="legend-count">&times;${{item.count}}</span>`;
    infraEl.appendChild(div);
  }}
}}

// --- Solver info table ---
function buildSolverTable() {{
  if (!SOLVER_INFO) return;
  document.getElementById('solver-section').style.display = 'block';
  const tbody = document.getElementById('solver-body');
  for (const row of SOLVER_INFO) {{
    // Find color for this recipe
    const legendItem = LEGEND_RECIPES.find(l => l.recipe === row.recipe);
    const color = legendItem ? legendItem.color : '#888';
    const tr = document.createElement('tr');
    const inputStr = row.inputs.map(i =>
      i.item + ' ' + i.rate + (i.fluid ? '<span class="fluid-tag">fluid</span>' : '')
    ).join('<br>');
    tr.innerHTML = `<td><span class="solver-recipe-swatch" style="background:${{color}}"></span>${{row.recipe}}</td>` +
      `<td>${{row.placed}}</td>` +
      `<td>${{inputStr}}</td>`;
    tbody.appendChild(tr);
  }}
}}

// --- Controls ---
document.getElementById('btn-fit').addEventListener('click', fitToView);
document.getElementById('btn-zin').addEventListener('click', () => {{
  scale = Math.min(scale * 1.5, 48);
  draw();
}});
document.getElementById('btn-zout').addEventListener('click', () => {{
  scale = Math.max(scale / 1.5, 0.5);
  draw();
}});

// --- Production Graph ---
let graphRendered = false;
function renderGraph() {{
  if (graphRendered || !GRAPH_DATA) return;
  graphRendered = true;

  const panel = document.getElementById('graph-panel');
  const gc = document.getElementById('graph-canvas');
  const gctx = gc.getContext('2d');

  gc.width = panel.clientWidth || 800;
  gc.height = panel.clientHeight || 600;

  const nodes = GRAPH_DATA.nodes;
  const edges = GRAPH_DATA.edges;

  // Group nodes by recipe for layered layout
  const recipeGroups = {{}};
  nodes.forEach(n => {{
    if (!recipeGroups[n.recipe]) recipeGroups[n.recipe] = [];
    recipeGroups[n.recipe].push(n);
  }});
  const recipes = Object.keys(recipeGroups);

  // Collect unique items from external inputs
  const extInputItems = [...new Set(edges.filter(e => e.from === null).map(e => e.item))];

  // Assign layer (x) based on dependency depth via BFS from external inputs
  const recipeLayers = {{}};
  const recipeInputs = {{}};
  recipes.forEach(r => {{ recipeInputs[r] = []; }});
  edges.forEach(e => {{
    if (e.from !== null && e.to !== null) {{
      const fromRecipe = nodes.find(n => n.id === e.from)?.recipe;
      const toRecipe = nodes.find(n => n.id === e.to)?.recipe;
      if (fromRecipe && toRecipe && fromRecipe !== toRecipe) {{
        if (!recipeInputs[toRecipe].includes(fromRecipe))
          recipeInputs[toRecipe].push(fromRecipe);
      }}
    }}
  }});

  // Topological layering
  const assigned = new Set();
  let layer = 0;
  // Start with recipes that have no recipe inputs (only external)
  let current = recipes.filter(r => recipeInputs[r].length === 0);
  while (current.length > 0) {{
    current.forEach(r => {{ recipeLayers[r] = layer; assigned.add(r); }});
    layer++;
    current = recipes.filter(r => !assigned.has(r) && recipeInputs[r].every(dep => assigned.has(dep)));
    if (current.length === 0 && assigned.size < recipes.length) {{
      // Cycle or orphan — assign remaining
      recipes.filter(r => !assigned.has(r)).forEach(r => {{ recipeLayers[r] = layer; assigned.add(r); }});
    }}
  }}
  const maxLayer = Math.max(...Object.values(recipeLayers), 0);
  const totalLayers = maxLayer + 1;

  // Position nodes
  const nodeRadius = 20;
  const layerWidth = gc.width / (totalLayers + 2);  // +2 for external input/output columns
  const nodePositions = {{}};

  // External input positions (leftmost column)
  const extPositions = {{}};
  extInputItems.forEach((item, i) => {{
    const y = (gc.height / (extInputItems.length + 1)) * (i + 1);
    extPositions[item] = {{ x: layerWidth * 0.5, y }};
  }});

  // Machine node positions (grouped by recipe, spread vertically)
  recipes.forEach(recipe => {{
    const group = recipeGroups[recipe];
    const lx = layerWidth * (recipeLayers[recipe] + 1.5);
    group.forEach((n, i) => {{
      const y = (gc.height / (group.length + 1)) * (i + 1);
      nodePositions[n.id] = {{ x: lx, y, color: n.color, recipe: n.recipe, entity: n.entity, instance: n.instance }};
    }});
  }});

  // Draw edges
  gctx.lineWidth = 1.5;
  edges.forEach(e => {{
    let from, to;
    if (e.from === null) {{
      from = extPositions[e.item];
      if (!from) return;
    }} else {{
      from = nodePositions[e.from];
    }}
    if (e.to === null) return;  // skip external outputs for now
    to = nodePositions[e.to];
    if (!from || !to) return;

    gctx.strokeStyle = e.is_fluid ? '#4a7ab5' : '#888';
    gctx.beginPath();
    // Curved edge
    const cx = (from.x + to.x) / 2;
    gctx.moveTo(from.x, from.y);
    gctx.quadraticCurveTo(cx, from.y, to.x, to.y);
    gctx.stroke();

    // Edge label
    const mx = (from.x + to.x) / 2;
    const my = (from.y + to.y) / 2 - 6;
    gctx.fillStyle = '#888';
    gctx.font = '10px sans-serif';
    gctx.textAlign = 'center';
    const rateStr = e.rate < 1 ? e.rate.toFixed(2) : e.rate.toFixed(1);
    gctx.fillText(`${{e.item}} ${{rateStr}}/s`, mx, my);
  }});

  // Draw external input nodes
  extInputItems.forEach(item => {{
    const pos = extPositions[item];
    gctx.fillStyle = '#2a6e2a';
    gctx.beginPath();
    gctx.arc(pos.x, pos.y, 14, 0, Math.PI * 2);
    gctx.fill();
    gctx.strokeStyle = '#4a9e4a';
    gctx.lineWidth = 2;
    gctx.stroke();
    gctx.fillStyle = '#e0e0e0';
    gctx.font = '10px sans-serif';
    gctx.textAlign = 'center';
    gctx.textBaseline = 'middle';
    // Truncate long names
    const label = item.length > 12 ? item.substring(0, 10) + '..' : item;
    gctx.fillText(label, pos.x, pos.y);
  }});

  // Draw machine nodes
  Object.values(nodePositions).forEach(pos => {{
    gctx.fillStyle = pos.color;
    gctx.beginPath();
    gctx.arc(pos.x, pos.y, nodeRadius, 0, Math.PI * 2);
    gctx.fill();
    gctx.strokeStyle = 'rgba(255,255,255,0.3)';
    gctx.lineWidth = 2;
    gctx.stroke();

    // Label
    gctx.fillStyle = '#fff';
    gctx.font = 'bold 10px sans-serif';
    gctx.textAlign = 'center';
    gctx.textBaseline = 'middle';
    const label = pos.recipe.length > 12 ? pos.recipe.substring(0, 10) + '..' : pos.recipe;
    gctx.fillText(label, pos.x, pos.y - 4);
    gctx.font = '9px sans-serif';
    gctx.fillStyle = '#ccc';
    gctx.fillText(`#${{pos.instance}}`, pos.x, pos.y + 8);
  }});
}}

// --- Init ---
buildLegend();
buildSolverTable();
resize();
fitToView();
</script>
</body>
</html>
"""
