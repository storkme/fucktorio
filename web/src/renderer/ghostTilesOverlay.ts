// Global ghost-tile overlay. Highlights every tile that the ghost
// router's `GhostSpecRouted` events touched — effectively the pre-SAT
// bus-routing footprint. Useful as ambient context while debugging
// SAT: any tile lit up cyan is a tile the router laid a path through,
// so a SAT zone's crossings are legible against the surrounding
// routing.
//
// Drawn as one Graphics per layout (cheap — a filled square per
// unique tile). Toggleable via the overlay panel; Phase D wires the
// toggle state in main.ts.

import { Container, Graphics } from "pixi.js";
import { TILE_PX } from "./entities";
import type { TraceEvent } from "../wasm-pkg/fucktorio_wasm.js";

const GHOST_TILE_COLOR = 0x40d0e0;
const GHOST_TILE_ALPHA = 0.18;

/**
 * Render a ghost-tile layer from the layout trace. Returns null if
 * the trace has no ghost events (nothing to draw).
 */
export function renderGhostTilesOverlay(
  trace: readonly TraceEvent[] | undefined,
): Container | null {
  if (!trace || trace.length === 0) return null;
  const tiles = new Set<string>();
  for (const ev of trace) {
    if (ev.phase !== "GhostSpecRouted") continue;
    for (const [x, y] of ev.data.tiles) {
      tiles.add(`${x},${y}`);
    }
  }
  if (tiles.size === 0) return null;
  const layer = new Container();
  layer.label = "ghost-tiles-overlay";
  const g = new Graphics();
  for (const key of tiles) {
    const [xs, ys] = key.split(",");
    const x = Number(xs);
    const y = Number(ys);
    g.rect(x * TILE_PX, y * TILE_PX, TILE_PX, TILE_PX)
      .fill({ color: GHOST_TILE_COLOR, alpha: GHOST_TILE_ALPHA });
  }
  layer.addChild(g);
  return layer;
}
