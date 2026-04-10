import { Application } from "pixi.js";
import { Viewport } from "pixi-viewport";

export const WORLD_SIZE = 3200;

export interface AppContext {
  app: Application;
  viewport: Viewport;
}

export async function createApp(container: HTMLElement): Promise<AppContext> {
  const app = new Application();
  await app.init({
    resizeTo: container,
    background: 0x1e1e1e,
    antialias: true,
  });

  container.appendChild(app.canvas);

  app.canvas.addEventListener("contextmenu", (e) => e.preventDefault());

  const viewport = new Viewport({
    screenWidth: container.clientWidth,
    screenHeight: container.clientHeight,
    worldWidth: WORLD_SIZE,
    worldHeight: WORLD_SIZE,
    events: app.renderer.events,
  });

  viewport.drag({ mouseButtons: "left" }).pinch().wheel().decelerate();

  app.stage.addChild(viewport);

  window.addEventListener("resize", () => {
    viewport.resize(container.clientWidth, container.clientHeight, WORLD_SIZE, WORLD_SIZE);
  });

  return { app, viewport };
}
