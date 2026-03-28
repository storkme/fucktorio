"""Visual showcase: renders every entity type at multiple zoom levels.

Run directly to generate and open the showcase HTML:
    python -m src.showcase

Or via pytest:
    pytest tests/ --viz   (generates showcase alongside other viz files)
"""

from __future__ import annotations

import json
import os
import webbrowser
from pathlib import Path

from src.renderers import THEME_JS


def generate_showcase(output_path: str | None = None, open_browser: bool = True) -> str:
    """Generate a standalone HTML showcase of all entity renderings.

    Shows every entity type at multiple zoom levels, with a theme switcher
    between 'schematic' (current style) and 'factorio' (game-realistic style).
    """
    if output_path is None:
        output_path = "showcase.html"

    tiles = _build_showcase_tiles()

    html_content = _SHOWCASE_TEMPLATE.replace("__TILES_JSON__", json.dumps(tiles)).replace(
        "__THEME_JS__", THEME_JS
    )

    Path(output_path).write_text(html_content)
    print(f"  Showcase: {output_path}")

    if open_browser:
        webbrowser.open(f"file://{os.path.abspath(output_path)}")

    return output_path


def _build_showcase_tiles() -> list[dict]:
    """Build synthetic tile data for the showcase."""
    tiles: list[dict] = []

    def add(x, y, entity, direction=0, w=1, h=1, recipe="", color="", carries="", group=""):
        tiles.append({
            "x": x, "y": y, "w": w, "h": h,
            "entity": entity, "dir": direction,
            "recipe": recipe,
            "color": color or _default_color(entity, recipe),
            "carries": carries,
            "tooltip": f"{entity} dir={direction}",
            "group": group,
        })

    # --- Group 1: Transport belts (all tiers, all directions, with turns) ---
    gx, gy = 0, 0

    # Yellow belt - straight in all 4 directions
    add(gx, gy, "transport-belt", 0, group="Yellow Belt")
    add(gx+1, gy, "transport-belt", 4, group="Yellow Belt")
    add(gx+2, gy, "transport-belt", 8, group="Yellow Belt")
    add(gx+3, gy, "transport-belt", 12, group="Yellow Belt")

    # Yellow belt - L-turn (going east then south)
    add(gx+5, gy, "transport-belt", 4, group="Yellow Belt Turn")
    add(gx+6, gy, "transport-belt", 4, group="Yellow Belt Turn")
    add(gx+7, gy, "transport-belt", 8, group="Yellow Belt Turn")
    add(gx+7, gy+1, "transport-belt", 8, group="Yellow Belt Turn")
    add(gx+7, gy+2, "transport-belt", 8, group="Yellow Belt Turn")

    # Red belt - straight run
    add(gx, gy+2, "fast-transport-belt", 4, group="Red Belt")
    add(gx+1, gy+2, "fast-transport-belt", 4, group="Red Belt")
    add(gx+2, gy+2, "fast-transport-belt", 4, group="Red Belt")
    add(gx+3, gy+2, "fast-transport-belt", 4, group="Red Belt")

    # Blue belt - straight run
    add(gx, gy+4, "express-transport-belt", 4, group="Blue Belt")
    add(gx+1, gy+4, "express-transport-belt", 4, group="Blue Belt")
    add(gx+2, gy+4, "express-transport-belt", 4, group="Blue Belt")
    add(gx+3, gy+4, "express-transport-belt", 4, group="Blue Belt")

    # --- Group 2: Underground belts ---
    gx, gy = 0, 7
    add(gx, gy, "underground-belt", 4, group="Underground Belt")
    add(gx+3, gy, "underground-belt", 12, group="Underground Belt")

    # --- Group 3: Splitter ---
    gx, gy = 5, 7
    add(gx, gy, "splitter", 4, w=1, h=2, group="Splitter")
    add(gx+2, gy, "splitter", 0, w=2, h=1, group="Splitter")

    # --- Group 4: Inserters (all types, all directions) ---
    gx, gy = 0, 10
    add(gx, gy, "inserter", 0, group="Inserter")
    add(gx+1, gy, "inserter", 4, group="Inserter")
    add(gx+2, gy, "inserter", 8, group="Inserter")
    add(gx+3, gy, "inserter", 12, group="Inserter")

    add(gx+5, gy, "fast-inserter", 0, group="Fast Inserter")
    add(gx+6, gy, "fast-inserter", 4, group="Fast Inserter")
    add(gx+7, gy, "fast-inserter", 8, group="Fast Inserter")
    add(gx+8, gy, "fast-inserter", 12, group="Fast Inserter")

    add(gx, gy+2, "long-handed-inserter", 0, group="Long Inserter")
    add(gx+1, gy+2, "long-handed-inserter", 4, group="Long Inserter")
    add(gx+2, gy+2, "long-handed-inserter", 8, group="Long Inserter")
    add(gx+3, gy+2, "long-handed-inserter", 12, group="Long Inserter")

    # --- Group 5: Pipes (various connectivity patterns) ---
    gx, gy = 0, 14

    add(gx, gy, "pipe", group="Pipe")

    add(gx+2, gy, "pipe", group="Pipe Run")
    add(gx+3, gy, "pipe", group="Pipe Run")
    add(gx+4, gy, "pipe", group="Pipe Run")
    add(gx+5, gy, "pipe", group="Pipe Run")

    add(gx+7, gy, "pipe", group="Pipe T")
    add(gx+8, gy, "pipe", group="Pipe T")
    add(gx+9, gy, "pipe", group="Pipe T")
    add(gx+8, gy+1, "pipe", group="Pipe T")

    add(gx+11, gy, "pipe", group="Pipe Cross")
    add(gx+12, gy, "pipe", group="Pipe Cross")
    add(gx+13, gy, "pipe", group="Pipe Cross")
    add(gx+12, gy-1, "pipe", group="Pipe Cross")
    add(gx+12, gy+1, "pipe", group="Pipe Cross")

    add(gx, gy+2, "pipe-to-ground", 4, group="UG Pipe")
    add(gx+3, gy+2, "pipe-to-ground", 12, group="UG Pipe")

    # --- Group 6: Power poles ---
    gx, gy = 0, 18
    add(gx, gy, "medium-electric-pole", group="Power Pole")
    add(gx+4, gy, "medium-electric-pole", group="Power Pole")

    # --- Group 7: Assembling machines (3x3) ---
    gx, gy = 0, 21
    add(gx, gy, "assembling-machine-1", w=3, h=3, recipe="iron-gear-wheel",
        color="#4e79a7", group="Assembler 1")
    add(gx+4, gy, "assembling-machine-2", w=3, h=3, recipe="copper-cable",
        color="#f28e2b", group="Assembler 2")
    add(gx+8, gy, "assembling-machine-3", w=3, h=3, recipe="electronic-circuit",
        color="#e15759", group="Assembler 3")

    # --- Group 8: Chemical plant (3x3) ---
    gx, gy = 0, 25
    add(gx, gy, "chemical-plant", w=3, h=3, recipe="sulfuric-acid",
        color="#76b7b2", group="Chemical Plant")

    # --- Group 9: Oil refinery (5x5) ---
    gx, gy = 4, 25
    add(gx, gy, "oil-refinery", w=5, h=5, recipe="advanced-oil-processing",
        color="#59a14f", group="Oil Refinery")

    # --- Group 10: Mini factory vignette ---
    gx, gy = 0, 32
    for i in range(5):
        add(gx, gy+i, "transport-belt", 8, carries="iron-plate", group="Mini Factory")
    add(gx+1, gy+1, "inserter", 4, group="Mini Factory")
    add(gx+2, gy, "assembling-machine-2", w=3, h=3, recipe="iron-gear-wheel",
        color="#4e79a7", group="Mini Factory")
    add(gx+5, gy+1, "inserter", 4, group="Mini Factory")
    for i in range(5):
        add(gx+6, gy+i, "transport-belt", 8, carries="iron-gear-wheel", group="Mini Factory")
    add(gx+3, gy+4, "medium-electric-pole", group="Mini Factory")

    return tiles


def _default_color(entity: str, recipe: str) -> str:
    """Get the default schematic color for an entity."""
    infra = {
        "transport-belt": "#c8b560",
        "fast-transport-belt": "#e05050",
        "express-transport-belt": "#50a0e0",
        "underground-belt": "#a89040",
        "inserter": "#6a8e3e",
        "fast-inserter": "#4a90d0",
        "long-handed-inserter": "#d04040",
        "pipe": "#4a7ab5",
        "pipe-to-ground": "#3a6090",
        "medium-electric-pole": "#8b6914",
        "splitter": "#c8b560",
    }
    return infra.get(entity, "#888")


# ---------------------------------------------------------------------------
# HTML template — self-contained showcase with theme switching
# Theme rendering code is injected via __THEME_JS__ from src/renderers.py
# ---------------------------------------------------------------------------

_SHOWCASE_TEMPLATE = r"""<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Fucktorio — Visual Showcase</title>
<style>
  * { margin: 0; padding: 0; box-sizing: border-box; }
  body {
    font-family: 'Segoe UI', system-ui, -apple-system, sans-serif;
    background: #1a1a2e;
    color: #e0e0e0;
    display: flex;
    flex-direction: column;
    height: 100vh;
    overflow: hidden;
  }
  #toolbar {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 10px 20px;
    background: #16213e;
    border-bottom: 2px solid #0f3460;
    flex-shrink: 0;
  }
  #toolbar h1 { font-size: 16px; color: #e94560; margin-right: 16px; }
  .toolbar-group { display: flex; align-items: center; gap: 8px; }
  .toolbar-group label { font-size: 13px; color: #a0a0b0; }
  .theme-btn {
    background: #0f0f23; border: 1px solid #0f3460; color: #a0a0b0;
    padding: 6px 14px; border-radius: 4px; cursor: pointer;
    font-size: 13px; font-family: inherit; transition: all 0.15s;
  }
  .theme-btn:hover { background: #1a3a6e; color: #e0e0e0; }
  .theme-btn.active { background: #e94560; color: white; border-color: #e94560; }
  .zoom-btn {
    background: #0f0f23; border: 1px solid #0f3460; color: #e0e0e0;
    width: 32px; height: 32px; border-radius: 4px; cursor: pointer;
    font-size: 16px; display: flex; align-items: center; justify-content: center;
  }
  .zoom-btn:hover { background: #1a3a6e; }
  #zoom-label { font-size: 13px; color: #a0a0b0; min-width: 50px; text-align: center; }
  #canvas-wrap { flex: 1; position: relative; overflow: hidden; background: #0f0f23; }
  canvas { position: absolute; top: 0; left: 0; image-rendering: pixelated; }
  #tooltip {
    position: fixed; background: #16213e; border: 1px solid #0f3460;
    border-radius: 6px; padding: 8px 12px; font-size: 13px;
    pointer-events: none; display: none; z-index: 100;
    box-shadow: 0 4px 12px rgba(0,0,0,0.5); max-width: 300px;
  }
  #tooltip .tt-entity { color: #e94560; font-weight: 600; }
  #tooltip .tt-group { color: #76b7b2; font-size: 12px; }
  #tooltip .tt-pos { color: #888; font-size: 11px; }
</style>
</head>
<body>

<div id="toolbar">
  <h1>Visual Showcase</h1>
  <div class="toolbar-group">
    <label>Theme:</label>
    <button class="theme-btn active" data-theme="schematic">Schematic</button>
    <button class="theme-btn" data-theme="factorio">Factorio</button>
  </div>
  <div class="toolbar-group">
    <label>Zoom:</label>
    <button class="zoom-btn" id="btn-zout">&minus;</button>
    <span id="zoom-label">24px</span>
    <button class="zoom-btn" id="btn-zin">+</button>
    <button class="zoom-btn" id="btn-fit" title="Fit to view">&#8982;</button>
  </div>
</div>

<div id="canvas-wrap">
  <canvas id="grid"></canvas>
</div>

<div id="tooltip">
  <div class="tt-entity"></div>
  <div class="tt-group"></div>
  <div class="tt-pos"></div>
</div>

<script>
const TILES = __TILES_JSON__;

let currentTheme = 'schematic';

document.querySelectorAll('.theme-btn').forEach(btn => {
  btn.addEventListener('click', () => {
    document.querySelectorAll('.theme-btn').forEach(b => b.classList.remove('active'));
    btn.classList.add('active');
    currentTheme = btn.dataset.theme;
    draw();
  });
});

const wrap = document.getElementById('canvas-wrap');
const canvas = document.getElementById('grid');
const ctx = canvas.getContext('2d');
const tooltip = document.getElementById('tooltip');

const MIN_X = Math.min(...TILES.map(t => t.x));
const MIN_Y = Math.min(...TILES.map(t => t.y));
const MAX_X = Math.max(...TILES.map(t => t.x + t.w));
const MAX_Y = Math.max(...TILES.map(t => t.y + t.h));
const GRID_W = MAX_X - MIN_X;
const GRID_H = MAX_Y - MIN_Y;

let scale = 24;
let offsetX = 0, offsetY = 0;
let dragging = false, dragStartX = 0, dragStartY = 0;

function resize() {
  canvas.width = wrap.clientWidth;
  canvas.height = wrap.clientHeight;
  draw();
}
window.addEventListener('resize', resize);

function fitToView() {
  const pad = 60;
  const scaleX = (wrap.clientWidth - pad * 2) / GRID_W;
  const scaleY = (wrap.clientHeight - pad * 2) / GRID_H;
  scale = Math.min(scaleX, scaleY, 48);
  scale = Math.max(scale, 4);
  offsetX = (wrap.clientWidth - GRID_W * scale) / 2;
  offsetY = (wrap.clientHeight - GRID_H * scale) / 2;
  updateZoomLabel();
  draw();
}

function updateZoomLabel() {
  document.getElementById('zoom-label').textContent = Math.round(scale) + 'px';
}

// Build adjacency lookup for pipe connectivity (data-specific)
const tileMap = {};
for (const t of TILES) {
  for (let dx = 0; dx < t.w; dx++) {
    for (let dy = 0; dy < t.h; dy++) {
      const key = (t.x + dx) + ',' + (t.y + dy);
      tileMap[key] = t;
    }
  }
}

// --- Shared theme rendering code (injected from src/renderers.py) ---
__THEME_JS__

function draw() {
  const theme = getTheme();
  ctx.clearRect(0, 0, canvas.width, canvas.height);

  ctx.fillStyle = theme.background;
  ctx.fillRect(0, 0, canvas.width, canvas.height);

  if (scale >= 6) {
    ctx.strokeStyle = theme.gridLine;
    ctx.lineWidth = 1;
    for (let x = 0; x <= GRID_W; x++) {
      const px = offsetX + x * scale;
      ctx.beginPath(); ctx.moveTo(px, offsetY); ctx.lineTo(px, offsetY + GRID_H * scale); ctx.stroke();
    }
    for (let y = 0; y <= GRID_H; y++) {
      const py = offsetY + y * scale;
      ctx.beginPath(); ctx.moveTo(offsetX, py); ctx.lineTo(offsetX + GRID_W * scale, py); ctx.stroke();
    }
  }

  // Group labels
  if (scale >= 6) {
    const groups = {};
    for (const t of TILES) {
      if (t.group && !groups[t.group]) {
        groups[t.group] = { x: t.x, y: t.y };
      }
      if (t.group && groups[t.group]) {
        groups[t.group].x = Math.min(groups[t.group].x, t.x);
        groups[t.group].y = Math.min(groups[t.group].y, t.y);
      }
    }
    ctx.fillStyle = 'rgba(200,200,200,0.25)';
    ctx.font = `${Math.max(9, scale * 0.4)}px sans-serif`;
    ctx.textAlign = 'left';
    ctx.textBaseline = 'bottom';
    for (const [name, pos] of Object.entries(groups)) {
      const px = offsetX + (pos.x - MIN_X) * scale;
      const py = offsetY + (pos.y - MIN_Y) * scale - 4;
      ctx.fillText(name, px, py);
    }
  }

  const machines = [];
  const infra = [];
  for (const t of TILES) {
    if (t.w > 1 || t.h > 1) machines.push(t);
    else infra.push(t);
  }

  for (const t of machines) {
    const px = offsetX + (t.x - MIN_X) * scale;
    const py = offsetY + (t.y - MIN_Y) * scale;
    const pw = t.w * scale;
    const ph = t.h * scale;
    if (t.entity === 'splitter') {
      theme.drawSplitter(ctx, px, py, pw, ph, t);
    } else {
      theme.drawMachine(ctx, px, py, pw, ph, t);
    }
  }

  for (const t of infra) {
    const px = offsetX + (t.x - MIN_X) * scale;
    const py = offsetY + (t.y - MIN_Y) * scale;
    const s = scale;
    if (isBelt(t.entity)) {
      theme.drawBelt(ctx, px, py, s, t);
    } else if (isPipe(t.entity)) {
      theme.drawPipe(ctx, px, py, s, t);
    } else if (isInserter(t.entity)) {
      theme.drawInserter(ctx, px, py, s, t);
    } else if (t.entity === 'medium-electric-pole') {
      theme.drawPole(ctx, px, py, s, t);
    } else {
      const gap = scale >= 4 ? 1 : 0;
      ctx.fillStyle = t.color;
      ctx.fillRect(px, py, s - gap, s - gap);
    }
  }
}

// --- Pan & zoom ---
wrap.addEventListener('wheel', (e) => {
  e.preventDefault();
  const rect = wrap.getBoundingClientRect();
  const mx = e.clientX - rect.left;
  const my = e.clientY - rect.top;
  const oldScale = scale;
  const factor = e.deltaY < 0 ? 1.15 : 1 / 1.15;
  scale = Math.min(Math.max(scale * factor, 2), 64);
  offsetX = mx - (mx - offsetX) * (scale / oldScale);
  offsetY = my - (my - offsetY) * (scale / oldScale);
  updateZoomLabel();
  draw();
});

wrap.addEventListener('mousedown', (e) => {
  dragging = true;
  dragStartX = e.clientX - offsetX;
  dragStartY = e.clientY - offsetY;
  wrap.style.cursor = 'grabbing';
});
window.addEventListener('mousemove', (e) => {
  if (dragging) {
    offsetX = e.clientX - dragStartX;
    offsetY = e.clientY - dragStartY;
    draw();
  }
  const rect = wrap.getBoundingClientRect();
  const mx = e.clientX - rect.left;
  const my = e.clientY - rect.top;
  const tileX = Math.floor((mx - offsetX) / scale) + MIN_X;
  const tileY = Math.floor((my - offsetY) / scale) + MIN_Y;

  let found = null;
  for (const t of TILES) {
    if (tileX >= t.x && tileX < t.x + t.w && tileY >= t.y && tileY < t.y + t.h) {
      found = t;
      break;
    }
  }

  if (found && mx >= 0 && my >= 0 && mx <= rect.width && my <= rect.height) {
    tooltip.style.display = 'block';
    tooltip.style.left = (e.clientX + 12) + 'px';
    tooltip.style.top = (e.clientY + 12) + 'px';
    tooltip.querySelector('.tt-entity').textContent = found.entity;
    tooltip.querySelector('.tt-group').textContent = found.group || '';
    tooltip.querySelector('.tt-pos').textContent = `(${tileX}, ${tileY}) dir=${found.dir}`;
  } else {
    tooltip.style.display = 'none';
  }
});
window.addEventListener('mouseup', () => {
  dragging = false;
  wrap.style.cursor = 'default';
});

document.getElementById('btn-fit').addEventListener('click', fitToView);
document.getElementById('btn-zin').addEventListener('click', () => {
  scale = Math.min(scale * 1.5, 64);
  updateZoomLabel();
  draw();
});
document.getElementById('btn-zout').addEventListener('click', () => {
  scale = Math.max(scale / 1.5, 2);
  updateZoomLabel();
  draw();
});

resize();
fitToView();
</script>
</body>
</html>"""


if __name__ == "__main__":
    generate_showcase()
