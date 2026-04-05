import { createApp, WORLD_SIZE } from "./renderer/app";
import { drawGrid } from "./renderer/grid";
import { drawGraph } from "./renderer/graph";
import { renderSidebar } from "./ui/sidebar";
import { initEngine, getEngine } from "./engine";
import type { SolverResult } from "./engine";

async function main(): Promise<void> {
  await initEngine();
  const engine = getEngine();

  const container = document.getElementById("canvas-container");
  if (!container) {
    throw new Error("Missing #canvas-container element");
  }

  const { viewport } = await createApp(container);
  drawGrid(viewport);
  drawGraph(viewport, null);

  viewport.moveCenter(WORLD_SIZE / 2, WORLD_SIZE / 2);

  function renderGraph(result: SolverResult | null): void {
    drawGraph(viewport, result);
    if (!result) {
      viewport.moveCenter(WORLD_SIZE / 2, WORLD_SIZE / 2);
    }
  }

  const sidebar = document.getElementById("sidebar");
  if (sidebar) {
    renderSidebar(sidebar, engine, renderGraph);
  }
}

main().catch((err) => {
  console.error("Failed to initialize app:", err);
});
