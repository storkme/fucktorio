import { Container } from "pixi.js";
import { createApp, WORLD_SIZE } from "./renderer/app";
import { drawGrid } from "./renderer/grid";
import { drawGraph } from "./renderer/graph";
import { renderLayout, setItemColoring, TILE_PX } from "./renderer/entities";
import { renderSidebar } from "./ui/sidebar";
import { initEngine, getEngine } from "./engine";
import type { SolverResult, LayoutResult, PlacedEntity } from "./engine";

async function main(): Promise<void> {
  await initEngine();
  const engine = getEngine();

  const container = document.getElementById("canvas-container");
  if (!container) throw new Error("Missing #canvas-container element");

  const { app, viewport } = await createApp(container);
  drawGrid(viewport);
  drawGraph(viewport, null);

  const entityLayer = new Container();
  viewport.addChild(entityLayer);
  viewport.moveCenter(WORLD_SIZE / 2, WORLD_SIZE / 2);

  // --- Hover tooltip ---
  const tooltip = document.createElement("div");
  tooltip.style.cssText = "position:fixed;background:#1e1e1e;color:#e0e0e0;border:1px solid #555;padding:4px 8px;font:12px monospace;pointer-events:none;border-radius:3px;display:none;z-index:1000;max-width:200px;line-height:1.5";
  document.body.appendChild(tooltip);

  document.addEventListener("mousemove", (e) => {
    tooltip.style.left = e.clientX + 14 + "px";
    tooltip.style.top = e.clientY - 10 + "px";
  });

  function onHover(entity: PlacedEntity | null): void {
    if (entity) {
      const dirArrow: Record<string, string> = { North: "\u2191", East: "\u2192", South: "\u2193", West: "\u2190" };
      let html = `<b>${entity.name}</b>`;
      if (entity.direction) html += `<br>${dirArrow[entity.direction] ?? ""} ${entity.direction}`;
      if (entity.carries) html += `<br>carries: ${entity.carries}`;
      if (entity.io_type) html += `<br>io: ${entity.io_type}`;
      if (entity.recipe) html += `<br>recipe: ${entity.recipe}`;
      if (entity.segment_id) html += `<br><span style="color:#9cdcfe">${entity.segment_id}</span>`;
      html += `<br>pos: ${entity.x ?? 0}, ${entity.y ?? 0}`;
      tooltip.innerHTML = html;
      tooltip.style.display = "block";
    } else {
      tooltip.style.display = "none";
    }
  }

  // --- Cursor tile position overlay ---
  const coordsEl = document.createElement("div");
  coordsEl.style.cssText = "position:absolute;bottom:8px;right:8px;background:rgba(0,0,0,0.6);color:#aaa;font:11px monospace;padding:3px 7px;border-radius:3px;pointer-events:none;z-index:10";
  coordsEl.textContent = "x:\u2013 y:\u2013";
  container.style.position = "relative";
  container.appendChild(coordsEl);

  // --- Item colour toggle (top-right of canvas) ---
  const colorToggle = document.createElement("label");
  colorToggle.style.cssText = "position:absolute;top:8px;right:8px;background:rgba(0,0,0,0.6);color:#ccc;font:11px monospace;padding:4px 8px;border-radius:3px;cursor:pointer;z-index:10;display:flex;align-items:center;gap:5px;user-select:none";
  const colorCb = document.createElement("input");
  colorCb.type = "checkbox";
  colorCb.checked = true;
  colorCb.style.accentColor = "#569cd6";
  colorToggle.appendChild(colorCb);
  colorToggle.appendChild(document.createTextNode("Item colours"));
  container.appendChild(colorToggle);
  let lastLayout: LayoutResult | null = null;

  colorCb.addEventListener("change", () => {
    setItemColoring(colorCb.checked);
    if (lastLayout) {
      entityLayer.removeChildren();
      renderLayout(lastLayout, entityLayer, onHover);
    }
  });

  app.canvas.addEventListener("pointermove", (e) => {
    const rect = app.canvas.getBoundingClientRect();
    const sx = e.clientX - rect.left;
    const sy = e.clientY - rect.top;
    const world = viewport.toWorld(sx, sy);
    const tx = Math.floor(world.x / TILE_PX);
    const ty = Math.floor(world.y / TILE_PX);
    coordsEl.textContent = `x:${tx} y:${ty}`;
  });

  function renderGraph(result: SolverResult | null): void {
    // Clear the entity layer when the DAG is redrawn — we've solved again.
    entityLayer.removeChildren();
    drawGraph(viewport, result);
    if (!result) {
      viewport.moveCenter(WORLD_SIZE / 2, WORLD_SIZE / 2);
    }
  }

  function renderLayoutOnCanvas(layout: LayoutResult): void {
    lastLayout = layout;
    // Replace the DAG with the actual bus layout.
    drawGraph(viewport, null);
    renderLayout(layout, entityLayer, onHover);
    const w = layout.width ?? 0;
    const h = layout.height ?? 0;
    if (w > 0 && h > 0) {
      const pxW = w * 32;
      const pxH = h * 32;
      viewport.fit(true, pxW * 1.1, pxH * 1.2);
      viewport.moveCenter(pxW / 2, pxH / 2);
    }
  }

  const sidebar = document.getElementById("sidebar");
  if (sidebar) {
    renderSidebar(sidebar, engine, {
      renderGraph,
      renderLayout: renderLayoutOnCanvas,
    });
  }
}

main().catch((err) => {
  console.error("Failed to initialize app:", err);
});
