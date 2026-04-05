import { Container } from "pixi.js";
import { createApp, WORLD_SIZE } from "./renderer/app";
import { drawGrid } from "./renderer/grid";
import { drawGraph } from "./renderer/graph";
import { renderLayout } from "./renderer/entities";
import { renderSidebar } from "./ui/sidebar";
import { initEngine, getEngine } from "./engine";
import type { SolverResult, LayoutResult } from "./engine";

async function main(): Promise<void> {
  await initEngine();
  const engine = getEngine();

  const container = document.getElementById("canvas-container");
  if (!container) throw new Error("Missing #canvas-container element");

  const { viewport } = await createApp(container);
  drawGrid(viewport);
  drawGraph(viewport, null);

  const entityLayer = new Container();
  viewport.addChild(entityLayer);
  viewport.moveCenter(WORLD_SIZE / 2, WORLD_SIZE / 2);

  function renderGraph(result: SolverResult | null): void {
    // Clear the entity layer when the DAG is redrawn — we've solved again.
    entityLayer.removeChildren();
    drawGraph(viewport, result);
    if (!result) {
      viewport.moveCenter(WORLD_SIZE / 2, WORLD_SIZE / 2);
    }
  }

  function renderLayoutOnCanvas(layout: LayoutResult): void {
    // Replace the DAG with the actual bus layout.
    drawGraph(viewport, null);
    renderLayout(layout, entityLayer);
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
