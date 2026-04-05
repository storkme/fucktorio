import { createApp, WORLD_SIZE } from "./renderer/app";
import { drawGrid } from "./renderer/grid";
import { renderSidebar } from "./ui/sidebar";
import { initEngine, getEngine } from "./engine";

async function main(): Promise<void> {
  await initEngine();
  const engine = getEngine();

  const sidebar = document.getElementById("sidebar");
  if (sidebar) {
    renderSidebar(sidebar, engine);
  }

  const container = document.getElementById("canvas-container");
  if (!container) {
    throw new Error("Missing #canvas-container element");
  }

  const { viewport } = await createApp(container);
  drawGrid(viewport);

  viewport.moveCenter(WORLD_SIZE / 2, WORLD_SIZE / 2);
}

main().catch((err) => {
  console.error("Failed to initialize app:", err);
});
