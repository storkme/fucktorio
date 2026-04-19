// Detailed overlay for the currently-selected SAT zone. Renders in
// world-space over the PIXI viewport so the user sees annotations at
// real scale on top of the belts/machines they're already looking at.
//
// Driven by the junction debugger's `onChange` callback — pass the
// selection state here and the layer redraws. Pass `null` to clear.
//
// Layers drawn (back to front):
//   1. Faint coloured tint inside the bbox
//   2. Dashed bbox outline
//   3. Diagonal hatching on each forbidden tile
//   4. Cyan ring at the cluster's seed tile
//   5. Per-boundary: 1/3-width edge bar (green IN / red OUT), plus a
//      centred item-icon sprite
//
// Item icons load via `Assets.load('/icons/<item>.png')` on first
// encounter and are cached by the asset system for all subsequent
// uses. An async-safe `renderId` guard discards late icon loads if
// the user stepped to a different iter before the sprite arrived.

import {
  Assets,
  Container,
  Graphics,
  Sprite,
  Text,
  TextStyle,
  type Texture,
} from "pixi.js";
import { TILE_PX } from "./entities";
import type { JunctionSelectionState } from "../ui/junctionDebugger";
import type { BoundarySnapshot } from "../wasm-pkg/fucktorio_wasm.js";

const BBOX_OUTLINE = 0xffd060;
const BBOX_FILL = 0xffd060;
const BBOX_FILL_ALPHA = 0.08;

const FORBIDDEN_HATCH = 0x8a4040;
const FORBIDDEN_HATCH_ALPHA = 0.55;

const SEED_COLOR = 0x40c0e0;

// Boundaries used to be green (IN) / red (OUT). We now rely on the
// chevron direction + edge position to communicate in/out implicitly,
// so the bar is just neutral grey.
const BOUNDARY_COLOR = 0x555555;
const BOUNDARY_ALPHA = 0.55;
const CHEVRON_COLOR = 0xffffff;
const CHEVRON_ALPHA = 0.85;
const INTERIOR_OUTLINE = 0xffffff;

const CHEVRON_STYLE = new TextStyle({
  fontFamily: "monospace",
  fontSize: 7,
  fontWeight: "700",
  fill: CHEVRON_COLOR,
});

// Shared item-icon cache. PIXI Assets already deduplicates by URL,
// but we also stash the promise so every boundary referencing the
// same item awaits the same resolution.
const iconCache = new Map<string, Promise<Texture | null>>();

function loadItemIcon(item: string): Promise<Texture | null> {
  let p = iconCache.get(item);
  if (!p) {
    const base = import.meta.env.BASE_URL;
    const url = `${base}icons/${item}.png`;
    p = Assets.load(url).catch(() => null) as Promise<Texture | null>;
    iconCache.set(item, p);
  }
  return p;
}

export interface SatZoneOverlayHandle {
  layer: Container;
  update(state: JunctionSelectionState | null): void;
  destroy(): void;
}

export function createSatZoneOverlay(): SatZoneOverlayHandle {
  const layer = new Container();
  layer.label = "sat-zone-overlay";
  // Async icon-load guard. Every update bumps this; stale icon
  // callbacks check it before adding themselves to the layer.
  let renderId = 0;

  function clear(): void {
    while (layer.children.length > 0) {
      const c = layer.children[0];
      layer.removeChild(c);
      c.destroy({ children: true });
    }
  }

  function update(state: JunctionSelectionState | null): void {
    renderId += 1;
    const myId = renderId;
    clear();
    if (!state) return;
    const { cluster, iter } = state;
    const b = iter.bbox;

    // Faint tint + outline. Drawn as two separate rects so the stroke
    // alignment doesn't chew into the fill area.
    const tint = new Graphics();
    tint
      .rect(b.x * TILE_PX, b.y * TILE_PX, b.w * TILE_PX, b.h * TILE_PX)
      .fill({ color: BBOX_FILL, alpha: BBOX_FILL_ALPHA });
    layer.addChild(tint);

    const outline = drawDashedRect(
      b.x * TILE_PX,
      b.y * TILE_PX,
      b.w * TILE_PX,
      b.h * TILE_PX,
      {
        dashLen: TILE_PX * 0.45,
        gapLen: TILE_PX * 0.25,
        width: 3,
        color: BBOX_OUTLINE,
        alpha: 0.95,
      },
    );
    layer.addChild(outline);

    // Forbidden-tile hatching. Each tile gets three parallel diagonals
    // contained to the tile footprint (bounded endpoints so we never
    // have to clip to the square).
    const hatch = new Graphics();
    hatch.setStrokeStyle({
      width: 1,
      color: FORBIDDEN_HATCH,
      alpha: FORBIDDEN_HATCH_ALPHA,
    });
    for (const [fx, fy] of iter.forbidden) {
      drawTileHatch(hatch, fx * TILE_PX, fy * TILE_PX, TILE_PX);
    }
    layer.addChild(hatch);

    // Seed ring.
    const seed = new Graphics();
    seed
      .circle(
        (cluster.seed.x + 0.5) * TILE_PX,
        (cluster.seed.y + 0.5) * TILE_PX,
        TILE_PX * 0.42,
      )
      .stroke({ width: 3, color: SEED_COLOR, alpha: 0.95 });
    layer.addChild(seed);

    // Boundaries: edge bars + item icons. Prefer SAT's own boundary
    // list (with interior-detection overrides) when available; fall
    // back to the raw growth-iter boundaries when SAT didn't run.
    const boundaries = iter.sat?.boundaries ?? iter.boundaries;
    for (const bd of boundaries) {
      drawBoundary(layer, bd, myId, () => renderId);
    }
  }

  function destroy(): void {
    clear();
    layer.destroy({ children: true });
  }

  return { layer, update, destroy };
}

interface DashStyle {
  dashLen: number;
  gapLen: number;
  width: number;
  color: number;
  alpha: number;
}

/**
 * Dashed rect outline. PIXI Graphics has no native dash support, so
 * we emit individual short strokes along each edge. Dashes restart at
 * each corner so the pattern always reads cleanly.
 */
function drawDashedRect(
  x: number,
  y: number,
  w: number,
  h: number,
  style: DashStyle,
): Graphics {
  const g = new Graphics();
  g.setStrokeStyle({ width: style.width, color: style.color, alpha: style.alpha });
  const segments: [number, number, number, number][] = [
    [x, y, x + w, y],         // top
    [x + w, y, x + w, y + h], // right
    [x + w, y + h, x, y + h], // bottom
    [x, y + h, x, y],         // left
  ];
  for (const [x0, y0, x1, y1] of segments) {
    const dx = x1 - x0;
    const dy = y1 - y0;
    const len = Math.hypot(dx, dy);
    const ux = dx / len;
    const uy = dy / len;
    const step = style.dashLen + style.gapLen;
    for (let d = 0; d < len; d += step) {
      const d2 = Math.min(d + style.dashLen, len);
      g.moveTo(x0 + ux * d, y0 + uy * d)
        .lineTo(x0 + ux * d2, y0 + uy * d2)
        .stroke();
    }
  }
  return g;
}

/**
 * Three parallel diagonals bounded to the tile. Avoids a real hatch
 * mask — the endpoints are chosen to stay inside the square.
 */
function drawTileHatch(g: Graphics, x: number, y: number, size: number): void {
  const t = size;
  g.moveTo(x, y + t).lineTo(x + t, y).stroke();
  g.moveTo(x + t / 3, y + t).lineTo(x + t, y + (2 * t) / 3).stroke();
  g.moveTo(x, y + (2 * t) / 3).lineTo(x + (2 * t) / 3, y).stroke();
}

/**
 * The edge of the boundary's tile where flow physically crosses.
 * - IN: items enter from the side OPPOSITE the flow direction
 * - OUT: items leave through the side MATCHING the flow direction
 */
function boundaryEdge(
  isInput: boolean,
  dir: string,
): "top" | "right" | "bottom" | "left" {
  const m: Record<string, "top" | "right" | "bottom" | "left"> = isInput
    ? { North: "bottom", East: "left", South: "top", West: "right" }
    : { North: "top", East: "right", South: "bottom", West: "left" };
  return m[dir] ?? "top";
}

function drawBoundary(
  layer: Container,
  b: BoundarySnapshot,
  myId: number,
  currentId: () => number,
): void {
  const tileX = b.x * TILE_PX;
  const tileY = b.y * TILE_PX;
  const barThick = TILE_PX / 3;
  const edge = boundaryEdge(b.is_input, b.direction);
  const isHorizontalBar = edge === "top" || edge === "bottom";

  let bx = tileX;
  let by = tileY;
  let bw = TILE_PX;
  let bh = TILE_PX;
  if (edge === "top") {
    bh = barThick;
  } else if (edge === "bottom") {
    by = tileY + TILE_PX - barThick;
    bh = barThick;
  } else if (edge === "left") {
    bw = barThick;
  } else {
    // right
    bx = tileX + TILE_PX - barThick;
    bw = barThick;
  }

  const g = new Graphics();
  g.rect(bx, by, bw, bh).fill({ color: BOUNDARY_COLOR, alpha: BOUNDARY_ALPHA });
  if (b.interior) {
    g.setStrokeStyle({ width: 1, color: INTERIOR_OUTLINE, alpha: 0.5 });
    g.rect(bx, by, bw, bh).stroke();
  }
  layer.addChild(g);

  // Flow-direction chevrons: two per bar, flanking the icon. The
  // chevron glyph inherits the flow direction (↑ ↓ ← →) and edge
  // position implies IN vs OUT — no colour coding needed.
  const glyph = dirArrow(b.direction);
  const [c1x, c1y, c2x, c2y] = isHorizontalBar
    ? [bx + bw / 6, by + bh / 2, bx + (bw * 5) / 6, by + bh / 2]
    : [bx + bw / 2, by + bh / 6, bx + bw / 2, by + (bh * 5) / 6];
  const ch1 = new Text({ text: glyph, style: CHEVRON_STYLE });
  ch1.anchor.set(0.5);
  ch1.x = c1x;
  ch1.y = c1y;
  ch1.alpha = CHEVRON_ALPHA;
  layer.addChild(ch1);
  const ch2 = new Text({ text: glyph, style: CHEVRON_STYLE });
  ch2.anchor.set(0.5);
  ch2.x = c2x;
  ch2.y = c2y;
  ch2.alpha = CHEVRON_ALPHA;
  layer.addChild(ch2);

  // Item icon centred on the bar. Sized to fit comfortably inside
  // the bar thickness so nothing spills outside the tile.
  const cx = bx + bw / 2;
  const cy = by + bh / 2;
  loadItemIcon(b.item).then((tex) => {
    if (myId !== currentId()) return;
    if (!tex) return;
    const sp = new Sprite(tex);
    sp.anchor.set(0.5);
    sp.x = cx;
    sp.y = cy;
    const target = barThick * 0.95; // stay inside the bar
    sp.width = target;
    sp.height = target;
    layer.addChild(sp);
  });
}

function dirArrow(dir: string): string {
  switch (dir) {
    case "North": return "\u25b2"; // ▲
    case "East":  return "\u25b6"; // ▶
    case "South": return "\u25bc"; // ▼
    case "West":  return "\u25c0"; // ◀
    default:      return "?";
  }
}
