import { Container } from "pixi.js";
import { createApp, WORLD_SIZE } from "./renderer/app";
import { drawGrid } from "./renderer/grid";
import { drawGraph } from "./renderer/graph";
import { renderLayout, setItemColoring, setRateOverlay, itemColor, isBeltEntity, TILE_PX, type HighlightController } from "./renderer/entities";
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

  let highlightCtrl: HighlightController | null = null;

  function onSelect(entity: PlacedEntity | null): void {
    if (!entity) {
      infoPanel.style.display = "none";
      return;
    }
    const dirArrow: Record<string, string> = { North: "\u2191", East: "\u2192", South: "\u2193", West: "\u2190" };
    let html = `<div style="display:flex;justify-content:space-between;align-items:start"><b>${entity.name}</b><span style="cursor:pointer;color:#888;margin-left:8px" id="info-close">\u00d7</span></div>`;
    if (entity.recipe) html += `<div style="color:#dcdcaa">recipe: ${entity.recipe}</div>`;
    if (entity.rate != null) html += `<div style="color:#b5cea8">rate: ${entity.rate.toFixed(1)}/s</div>`;
    if (entity.carries) html += `<div style="color:#9cdcfe">carries: ${entity.carries}</div>`;
    if (entity.direction) html += `<div>${dirArrow[entity.direction] ?? ""} ${entity.direction}</div>`;
    html += `<div style="color:#888">pos: ${entity.x ?? 0}, ${entity.y ?? 0}</div>`;
    infoPanel.innerHTML = html;
    infoPanel.style.display = "block";
    const closeBtn = document.getElementById("info-close");
    if (closeBtn) closeBtn.addEventListener("click", () => { infoPanel.style.display = "none"; });
  }

  function onHover(entity: PlacedEntity | null): void {
    if (entity) {
      const dirArrow: Record<string, string> = { North: "\u2191", East: "\u2192", South: "\u2193", West: "\u2190" };
      let html = `<b>${entity.name}</b>`;
      if (entity.direction) html += `<br>${dirArrow[entity.direction] ?? ""} ${entity.direction}`;
      if (entity.carries) html += `<br>carries: ${entity.carries}`;
      if (entity.rate != null) html += `<br><span style="color:#b5cea8">${entity.rate.toFixed(1)}/s</span>`;
      if (entity.io_type) html += `<br>io: ${entity.io_type}`;
      if (entity.recipe) html += `<br>recipe: ${entity.recipe}`;
      if (entity.segment_id) html += `<br><span style="color:#9cdcfe">${entity.segment_id}</span>`;
      html += `<br>pos: ${entity.x ?? 0}, ${entity.y ?? 0}`;
      tooltip.innerHTML = html;
      tooltip.style.display = "block";

      // Belt entities → highlight the connected belt network (upstream dashed, downstream solid)
      // Everything else → highlight the item chain
      if (highlightCtrl) {
        if (isBeltEntity(entity.name)) {
          highlightCtrl.highlightBeltNetwork(entity);
        } else {
          highlightCtrl.highlightItem(highlightCtrl.chainKey(entity));
        }
      }
    } else {
      tooltip.style.display = "none";
      if (highlightCtrl) highlightCtrl.clearHighlight();
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

  // --- Rate overlay toggle ---
  const rateToggle = document.createElement("label");
  rateToggle.style.cssText = "position:absolute;top:34px;right:8px;background:rgba(0,0,0,0.6);color:#ccc;font:11px monospace;padding:4px 8px;border-radius:3px;cursor:pointer;z-index:10;display:flex;align-items:center;gap:5px;user-select:none";
  const rateCb = document.createElement("input");
  rateCb.type = "checkbox";
  rateCb.checked = false;
  rateCb.style.accentColor = "#569cd6";
  rateToggle.appendChild(rateCb);
  rateToggle.appendChild(document.createTextNode("Rates"));
  container.appendChild(rateToggle);

  // --- Item color legend (bottom-left) ---
  const legendEl = document.createElement("div");
  legendEl.style.cssText = "position:absolute;bottom:8px;left:8px;background:rgba(0,0,0,0.6);color:#ccc;font:11px monospace;padding:4px 8px;border-radius:3px;pointer-events:none;z-index:10;display:none;max-height:300px;overflow-y:auto";
  container.appendChild(legendEl);

  // --- Machine info panel (click) ---
  const infoPanel = document.createElement("div");
  infoPanel.style.cssText = "position:absolute;top:8px;left:8px;background:rgba(0,0,0,0.8);color:#e0e0e0;font:12px monospace;padding:8px 10px;border-radius:4px;border:1px solid #555;z-index:10;display:none;max-width:250px;line-height:1.5";
  container.appendChild(infoPanel);

  let lastLayout: LayoutResult | null = null;

  colorCb.addEventListener("change", () => {
    setItemColoring(colorCb.checked);
    if (lastLayout) renderLayoutOnCanvas(lastLayout);
  });

  rateCb.addEventListener("change", () => {
    setRateOverlay(rateCb.checked);
    if (lastLayout) renderLayoutOnCanvas(lastLayout);
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
    legendEl.style.display = "none";
    infoPanel.style.display = "none";
    if (!result) {
      viewport.moveCenter(WORLD_SIZE / 2, WORLD_SIZE / 2);
    }
  }

  function buildLegend(layout: LayoutResult): void {
    legendEl.innerHTML = "";
    const items = new Set<string>();
    for (const e of layout.entities) {
      if (e.carries) items.add(e.carries);
    }
    if (items.size === 0) {
      legendEl.style.display = "none";
      return;
    }
    const sorted = Array.from(items).sort();
    for (const item of sorted) {
      const row = document.createElement("div");
      row.style.cssText = "display:flex;align-items:center;gap:5px;padding:1px 0";
      const swatch = document.createElement("span");
      const color = itemColor(item);
      const hex = "#" + color.toString(16).padStart(6, "0");
      swatch.style.cssText = `display:inline-block;width:12px;height:12px;background:${hex};border-radius:2px;flex-shrink:0`;
      row.appendChild(swatch);
      const label = document.createElement("span");
      label.textContent = item;
      row.appendChild(label);
      legendEl.appendChild(row);
    }
    legendEl.style.display = "block";
  }

  function renderLayoutOnCanvas(layout: LayoutResult): void {
    lastLayout = layout;
    // Replace the DAG with the actual bus layout.
    drawGraph(viewport, null);
    highlightCtrl = renderLayout(layout, entityLayer, onHover, onSelect);
    buildLegend(layout);
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
