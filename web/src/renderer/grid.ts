import { Graphics } from "pixi.js";
import type { Viewport } from "pixi-viewport";

const TILE_SIZE = 32;
const GRID_TILES = 100;
const MINOR_COLOR = 0x333333;
const MAJOR_COLOR = 0x555555;

export function drawGrid(viewport: Viewport): Graphics {
  const g = new Graphics();
  const size = GRID_TILES * TILE_SIZE;

  for (let i = 0; i <= GRID_TILES; i++) {
    const pos = i * TILE_SIZE;
    const isMajor = i % 10 === 0;
    const color = isMajor ? MAJOR_COLOR : MINOR_COLOR;
    const width = isMajor ? 1.5 : 1;

    // Vertical line
    g.moveTo(pos, 0).lineTo(pos, size).stroke({ width, color });
    // Horizontal line
    g.moveTo(0, pos).lineTo(size, pos).stroke({ width, color });
  }

  viewport.addChild(g);
  return g;
}
