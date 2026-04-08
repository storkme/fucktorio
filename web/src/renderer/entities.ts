import { Assets, Container, Graphics, Sprite, Text, TextStyle, Texture } from "pixi.js";
import type { LayoutResult, PlacedEntity, EntityDirection } from "../engine";
import {
  buildBeltGraph,
  traceBeltNetwork,
  findAdjacentInserters,
  findAdjacentMachines,
  drawBeltNetworkOverlay,
  type BeltGraph,
} from "./beltGraph";

export const TILE_PX = 32;

// Colors sourced from src/visualize.py _MACHINE_COLORS / _INFRA_COLORS
const MACHINE_COLORS: Record<string, number> = {
  "assembling-machine-1": 0x5a6e82,
  "assembling-machine-2": 0x4a6278,
  "assembling-machine-3": 0x3a526a,
  "stone-furnace": 0x8a6040,
  "steel-furnace": 0x7a5030,
  "electric-furnace": 0x6a5a80,
  "chemical-plant": 0x3a7a50,
  "oil-refinery": 0x5a3a8a,
  centrifuge: 0x3a7a80,
  lab: 0x4a6a50,
  "rocket-silo": 0x4a4a6a,
  foundry: 0x8a6a30,
  "electromagnetic-plant": 0x2a5a9a,
  "cryogenic-plant": 0x4a7a8a,
  biochamber: 0x4a7a3a,
  biolab: 0x3a6a5a,
  recycler: 0x6a5a4a,
  crusher: 0x5a4a3a,
  beacon: 0x4a6080,
  "storage-tank": 0x4a6a5a,
  "big-electric-pole": 0x8b6914,
  substation: 0x6a6a8b,
  "electric-mining-drill": 0x7a6a30,
};
const DEFAULT_MACHINE_COLOR = 0x4a5a6a;

// Belt / underground belt / splitter tier colors: [base, chevron]
const BELT_COLORS: Record<string, [number, number]> = {
  "transport-belt": [0xa89030, 0xe0d070],
  "fast-transport-belt": [0xb03030, 0xff6060],
  "express-transport-belt": [0x3070b0, 0x70b0f0],
  "underground-belt": [0xa89030, 0xe0d070],
  "fast-underground-belt": [0xb03030, 0xff6060],
  "express-underground-belt": [0x3070b0, 0x70b0f0],
  splitter: [0xa89030, 0xe0d070],
  "fast-splitter": [0xb03030, 0xff6060],
  "express-splitter": [0x3070b0, 0x70b0f0],
};

const INSERTER_COLORS: Record<string, number> = {
  inserter: 0x6a8e3e,
  "fast-inserter": 0x4a90d0,
  "long-handed-inserter": 0xd04040,
};

const PIPE_COLOR = 0x4a7ab5;
const PIPE_TO_GROUND_COLOR = 0x3a6090;
const POLE_COLOR = 0xc0a030;
const POLE_BG = 0x2a2510;

// Item-to-color mapping for visual debugging

const ITEM_PALETTE: Record<string, number> = {
  "iron-plate": 0x9a9a9a,
  "copper-plate": 0xd07840,
  "iron-gear-wheel": 0x707070,
  "copper-cable": 0xe06020,
  "electronic-circuit": 0x50c050,
  "advanced-circuit": 0xc05050,
  "processing-unit": 0x5050c0,
  "plastic-bar": 0x8080a0,
  "steel-plate": 0x707888,
  "iron-ore": 0xa07070,
  "copper-ore": 0xd08060,
  "coal": 0x404040,
  "stone": 0xa09070,
  "sulfur": 0xc0c040,
  "crude-oil": 0x303050,
  "water": 0x4060b0,
  "petroleum-gas": 0xa0a060,
  "light-oil": 0xa0a0b0,
  "heavy-oil": 0x705040,
  "sulfuric-acid": 0xb0b030,
  "lubricant": 0x60b060,
};

function hslToHex(h: number, s: number, l: number): number {
  const a = s * Math.min(l, 1 - l);
  const f = (n: number) => {
    const k = (n + h * 12) % 12;
    return Math.round((l - a * Math.max(-1, Math.min(k - 3, 9 - k, 1))) * 255);
  };
  return (f(0) << 16) | (f(8) << 8) | f(4);
}

let itemColoringEnabled = true;
export function setItemColoring(enabled: boolean): void { itemColoringEnabled = enabled; }

let rateOverlayEnabled = false;
export function setRateOverlay(enabled: boolean): void { rateOverlayEnabled = enabled; }

/** Per-recipe input/output flows for machine overlays. */
interface RecipeFlows {
  inputs: { item: string; rate: number }[];
  outputs: { item: string; rate: number }[];
  machineCount: number;
}
let recipeFlowMap: Map<string, RecipeFlows> = new Map();

/** Look up per-machine flows for a recipe (returns undefined if not set). */
export function getRecipeFlows(recipe: string): RecipeFlows | undefined {
  return recipeFlowMap.get(recipe);
}

/** Call before renderLayout to provide solver result data for machine overlays. */
export function setRecipeFlows(machines: { recipe: string; count: number; inputs: { item: string; rate: number }[]; outputs: { item: string; rate: number }[] }[]): void {
  recipeFlowMap = new Map();
  for (const m of machines) {
    // Solver rates are already per-machine — use as-is.
    recipeFlowMap.set(m.recipe, {
      inputs: m.inputs.map(f => ({ item: f.item, rate: f.rate })),
      outputs: m.outputs.map(f => ({ item: f.item, rate: f.rate })),
      machineCount: Math.ceil(m.count),
    });
  }
}

export function itemColor(item: string | undefined): number {
  if (!itemColoringEnabled) return 0x777777;
  if (!item) return 0x666666;
  if (item in ITEM_PALETTE) return ITEM_PALETTE[item];
  let h = 0;
  for (let i = 0; i < item.length; i++) h = (((h << 5) - h) + item.charCodeAt(i)) | 0;
  const hue = (Math.abs(h) % 30) * 12;
  return hslToHex(hue / 360, 0.55, 0.48);
}

// [width, height] in tiles for multi-tile entities
const MACHINE_SIZES: Record<string, [number, number]> = {
  "assembling-machine-1": [3, 3],
  "assembling-machine-2": [3, 3],
  "assembling-machine-3": [3, 3],
  "chemical-plant": [3, 3],
  "oil-refinery": [5, 5],
  "electric-furnace": [3, 3],
  "steel-furnace": [2, 2],
  "stone-furnace": [2, 2],
  centrifuge: [3, 3],
  lab: [3, 3],
  "rocket-silo": [9, 9],
  foundry: [5, 5],
  biochamber: [3, 3],
  biolab: [5, 5],
  "electromagnetic-plant": [4, 4],
  "cryogenic-plant": [5, 5],
  recycler: [2, 4],
  crusher: [2, 3],
  beacon: [3, 3],
  "storage-tank": [3, 3],
  "big-electric-pole": [2, 2],
  substation: [2, 2],
  "electric-mining-drill": [3, 3],
};

/** Convert a kebab-case slug to a display name: "assembling-machine-3" → "Assembling Machine 3" */
export function niceName(slug: string): string {
  return slug
    .split("-")
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join(" ");
}

// Entity-type sets derived from the lookup tables where possible
const MACHINE_ENTITIES = new Set(Object.keys(MACHINE_SIZES));
const INSERTER_ENTITIES = new Set(Object.keys(INSERTER_COLORS));
// Derived from BELT_COLORS keys by tier prefix
const BELT_ENTITIES = new Set(
  Object.keys(BELT_COLORS).filter((k) => !k.startsWith("underground") && !k.includes("splitter"))
);
const UG_BELT_ENTITIES = new Set(Object.keys(BELT_COLORS).filter((k) => k.startsWith("underground")));
const SPLITTER_ENTITIES = new Set(Object.keys(BELT_COLORS).filter((k) => k.includes("splitter")));
const PIPE_ENTITIES = new Set(["pipe", "pipe-to-ground"]);
const POLE_ENTITIES = new Set(["medium-electric-pole", "small-electric-pole"]);

// Direction helpers

function dirAngle(dir?: EntityDirection): number {
  switch (dir) {
    case "East": return Math.PI / 2;
    case "South": return Math.PI;
    case "West": return (3 * Math.PI) / 2;
    default: return 0;
  }
}

function dirVec(dir?: EntityDirection): [number, number] {
  switch (dir) {
    case "East": return [1, 0];
    case "South": return [0, 1];
    case "West": return [-1, 0];
    default: return [0, -1];
  }
}

/** Companion tile offset for a splitter's 2×1 footprint. */
function splitterCompanionOffset(dir?: EntityDirection): [number, number] {
  switch (dir) {
    case "South":
    case "North":
      return [1, 0]; // 2 wide horizontally
    case "East":
    case "West":
      return [0, 1]; // 2 tall vertically
    default:
      return [1, 0];
  }
}

/** Belt turn info: the perpendicular feed direction relative to our flow. */
interface BeltTurn {
  /** "cw" = feeder rotated clockwise from our direction (e.g. we go East, feeder comes from North). */
  turn: "cw" | "ccw";
}

/** Detect if belt `e` is a 90° turn: exactly one perpendicular feeder, no straight feeder. */
function detectBeltTurn(e: PlacedEntity, tileMap: Map<string, PlacedEntity>): BeltTurn | null {
  const d = e.direction ?? "North";
  const [mdx, mdy] = dirVec(d);
  let hasStraightFeeder = false;
  let perpFeeder: BeltTurn | null = null;

  for (const [dx, dy] of [[0, -1], [1, 0], [0, 1], [-1, 0]] as [number, number][]) {
    const nx = (e.x ?? 0) + dx;
    const ny = (e.y ?? 0) + dy;
    const nb = tileMap.get(`${nx},${ny}`);
    if (!nb) continue;
    const feeds =
      BELT_ENTITIES.has(nb.name) ||
      (UG_BELT_ENTITIES.has(nb.name) && nb.io_type === "output") ||
      SPLITTER_ENTITIES.has(nb.name);
    if (!feeds) continue;
    const [ndx, ndy] = dirVec(nb.direction);
    // Does neighbour's flow point at us?
    // For splitters registered at companion tiles, use the actual tile
    // position (nx, ny) instead of the anchor position (nb.x, nb.y).
    const nbFlowX = SPLITTER_ENTITIES.has(nb.name) ? nx : (nb.x ?? 0);
    const nbFlowY = SPLITTER_ENTITIES.has(nb.name) ? ny : (nb.y ?? 0);
    if (nbFlowX + ndx !== (e.x ?? 0) || nbFlowY + ndy !== (e.y ?? 0)) continue;
    if (nb.direction === d) {
      hasStraightFeeder = true;
    } else {
      // Cross product of feeder flow × our flow: positive = cw, negative = ccw
      const cross = ndx * mdy - ndy * mdx;
      if (cross !== 0) perpFeeder = { turn: cross > 0 ? "cw" : "ccw" };
    }
  }
  return perpFeeder && !hasStraightFeeder ? perpFeeder : null;
}

/** Darken a packed 0xRRGGBB color by `factor` (0 = black, 1 = unchanged). */
function darken(color: number, factor: number): number {
  const r = Math.round(((color >> 16) & 0xff) * factor);
  const gr = Math.round(((color >> 8) & 0xff) * factor);
  const b = Math.round((color & 0xff) * factor);
  return (r << 16) | (gr << 8) | b;
}

// Draw functions

const TILE_RADIUS = 3;

/** Dark body shared by all belt tiers (like the Factorio belt frame). */
const BELT_BODY = 0x3a3a3a;
const BELT_BORDER = 0x555555;

function drawBelt(entity: PlacedEntity, turn: BeltTurn | null): Graphics {
  const g = new Graphics();
  const s = TILE_PX;
  const [, chev] = BELT_COLORS[entity.name] ?? [0xa89030, 0xe0d070];
  const laneColor = itemColor(entity.carries);

  if (turn) {
    drawBeltCorner(g, s, chev, entity.direction, turn, laneColor);
  } else {
    // Dark belt body with subtle border
    g.rect(0, 0, s, s).fill(BELT_BODY);
    g.setStrokeStyle({ width: 1, color: BELT_BORDER, alignment: 0 });
    g.rect(0, 0, s, s).stroke();

    const rg = new Graphics();
    rg.x = s / 2;
    rg.y = s / 2;
    rg.rotation = dirAngle(entity.direction);

    // Lane stripes: left lane = left half, right lane = right half
    rg.rect(-s / 2, -s / 2, s / 2 - 1, s).fill({ color: laneColor, alpha: 0.45 });
    rg.rect(1, -s / 2, s / 2 - 1, s).fill({ color: laneColor, alpha: 0.45 });
    // Dark center divider
    rg.rect(-1, -s / 2, 2, s).fill(0x0a0a0a);

    addBeltChevrons(rg, s, chev);
    g.addChild(rg);
  }

  return g;
}

/** Draw a corner belt: dark body curving along the 90° path + 3 chevrons. */
function drawBeltCorner(
  g: Graphics,
  s: number,
  chev: number,
  direction: EntityDirection | undefined,
  turn: BeltTurn,
  laneColor: number,
): void {
  const cg = new Graphics();
  cg.x = s / 2;
  cg.y = s / 2;
  cg.rotation = dirAngle(direction);

  const r = s / 2;
  const sign = turn.turn === "cw" ? 1 : -1;
  const cornerX = sign * r;
  const cornerY = -r;

  // Quarter-disc body centred at the inside corner, radius s.
  // Covers the whole tile except the outer corner, which falls beyond the arc.
  const startA = turn.turn === "ccw" ? 0 : Math.PI / 2;
  const endA = turn.turn === "ccw" ? Math.PI / 2 : Math.PI;
  cg.moveTo(cornerX, cornerY)
    .arc(cornerX, cornerY, s, startA, endA, false)
    .closePath()
    .fill(BELT_BODY);
  cg.setStrokeStyle({ width: 1, color: BELT_BORDER, alignment: 0 });
  cg.moveTo(cornerX, cornerY)
    .arc(cornerX, cornerY, s, startA, endA, false)
    .closePath()
    .stroke();

  // Lane stripes: inner arc sector + outer annular sector, matching straight belt alpha.
  const rMid = s * 0.5;
  const rDiv = 1.5; // divider half-width px

  // Inner lane: pie sector from corner to rMid - divider
  cg.moveTo(cornerX, cornerY)
    .arc(cornerX, cornerY, rMid - rDiv, startA, endA, false)
    .closePath()
    .fill({ color: laneColor, alpha: 0.45 });

  // Outer lane: annular sector from rMid + divider to s
  const csA = Math.cos(startA), snA = Math.sin(startA);
  const csE = Math.cos(endA), snE = Math.sin(endA);
  cg.moveTo(cornerX + (rMid + rDiv) * csA, cornerY + (rMid + rDiv) * snA)
    .lineTo(cornerX + s * csA, cornerY + s * snA)
    .arc(cornerX, cornerY, s, startA, endA, false)
    .lineTo(cornerX + (rMid + rDiv) * csE, cornerY + (rMid + rDiv) * snE)
    .arc(cornerX, cornerY, rMid + rDiv, endA, startA, true)
    .closePath()
    .fill({ color: laneColor, alpha: 0.45 });

  // 2 chevrons projected onto the arc: tip sits on the centreline, arms splay
  // to outer/inner radii along an angular offset BEHIND the tip (opposite flow).
  const chSize = s * 0.22;
  const lineW = Math.max(1, s * 0.1);
  const chevR = s * 0.5;
  const dA = chSize / chevR;
  const rOuter = chevR + chSize;
  const rInner = chevR - chSize;
  cg.setStrokeStyle({ width: lineW, color: chev, cap: "round", join: "round" });
  const aStart = Math.PI / 2;
  const aEnd = turn.turn === "cw" ? Math.PI : 0;
  for (const frac of [0.6]) {
    const aTip = aStart + frac * (aEnd - aStart);
    const aBack = turn.turn === "cw" ? aTip - dA : aTip + dA;
    const tipX = cornerX + chevR * Math.cos(aTip);
    const tipY = cornerY + chevR * Math.sin(aTip);
    const outerX = cornerX + rOuter * Math.cos(aBack);
    const outerY = cornerY + rOuter * Math.sin(aBack);
    const innerX = cornerX + rInner * Math.cos(aBack);
    const innerY = cornerY + rInner * Math.sin(aBack);
    cg.moveTo(outerX, outerY).lineTo(tipX, tipY).lineTo(innerX, innerY).stroke();
  }

  g.addChild(cg);
}

/** 2 direction chevrons stacked along the flow axis (local -y). */
function addBeltChevrons(g: Graphics, s: number, chevColor: number): void {
  const chevSize = s * 0.22;
  g.setStrokeStyle({ width: Math.max(1, s * 0.1), color: chevColor, cap: "round", join: "round" });
  for (const oy of [-s * 0.22, s * 0.22]) {
    g.moveTo(-chevSize, oy + chevSize * 0.5)
      .lineTo(0, oy - chevSize * 0.5)
      .lineTo(chevSize, oy + chevSize * 0.5)
      .stroke();
  }
}

function drawUndergroundBelt(entity: PlacedEntity): Graphics {
  const g = new Graphics();
  const s = TILE_PX;
  const [base, chev] = BELT_COLORS[entity.name] ?? [0xa89030, 0xe0d070];
  const isInput = entity.io_type === "input";
  const half = s / 2;

  // In local coords (rotation=0 = North): flow direction = -y.
  // Input UG:  items flow in (-y), tunnel goes in flow direction. Open mouth at +y.
  // Output UG: items emerge in flow direction (-y), tunnel comes from +y. Open mouth at -y.
  const tunnelY = isInput ? -half : 0;   // underground half (very dark)
  const surfaceY = isInput ? 0 : -half;  // open/surface half (belt-coloured)

  g.rect(0, 0, s, s).fill(BELT_BODY);
  g.setStrokeStyle({ width: 1, color: BELT_BORDER, alignment: 0 });
  g.rect(0, 0, s, s).stroke();

  const m = new Graphics();
  m.x = half;
  m.y = half;
  m.rotation = dirAngle(entity.direction);

  // Underground half — near-black to read as "buried"
  m.rect(-half, tunnelY, s, half).fill(0x181818);

  // Surface half — item colour so it reads as a belt connection
  m.rect(-half, surfaceY, s, half).fill({ color: itemColor(entity.carries), alpha: 0.7 });

  // Dividing line between the two halves
  m.setStrokeStyle({ width: 1, color: 0x505050 });
  m.moveTo(-half, 0).lineTo(half, 0).stroke();

  // Filled triangle arrow pointing in flow direction (-y), centered on tile.
  // Large enough to be unambiguous at 32 px.
  const arrW = s * 0.52;
  const arrH = s * 0.36;
  m.moveTo(0, -arrH / 2)
    .lineTo( arrW / 2,  arrH / 2)
    .lineTo(-arrW / 2,  arrH / 2)
    .closePath()
    .fill(chev);

  // Bright edge stripe at the open mouth to mark where surface belt connects
  const stripeH = Math.max(2, s * 0.08);
  const stripeY = isInput ? half - stripeH : -half;
  m.rect(-half, stripeY, s, stripeH).fill(base);

  g.addChild(m);
  return g;
}

function drawSplitter(entity: PlacedEntity): Graphics {
  const g = new Graphics();
  const [base, chev] = BELT_COLORS[entity.name] ?? [0xa89030, 0xe0d070];
  // Splitters are 2 tiles wide perpendicular to travel direction
  const isNS = entity.direction === "North" || entity.direction === "South";
  const pw = isNS ? TILE_PX * 2 - 1 : TILE_PX - 1;
  const ph = isNS ? TILE_PX - 1 : TILE_PX * 2 - 1;
  const cell = isNS ? pw / 2 : ph / 2;
  const barThick = Math.max(2, Math.min(pw, ph) * 0.18);

  g.roundRect(0, 0, pw, ph, TILE_RADIUS).fill(base);
  // Item tint overlay
  g.roundRect(0, 0, pw, ph, TILE_RADIUS).fill({ color: itemColor(entity.carries), alpha: 0.3 });
  if (isNS) {
    g.rect(cell - barThick / 2, 0, barThick, ph).fill(darken(base, 0.5));
  } else {
    g.rect(0, cell - barThick / 2, pw, barThick).fill(darken(base, 0.5));
  }

  const angle = dirAngle(entity.direction);
  const chevSize = cell * 0.25;
  const lineW = Math.max(1, cell * 0.12);
  for (let half = 0; half < 2; half++) {
    const hcx = isNS ? cell * half + cell / 2 : pw / 2;
    const hcy = isNS ? ph / 2 : cell * half + cell / 2;
    const cg = new Graphics();
    cg.x = hcx;
    cg.y = hcy;
    cg.rotation = angle;
    cg.setStrokeStyle({ width: lineW, color: chev, cap: "round" });
    cg.moveTo(-chevSize, chevSize * 0.5).lineTo(0, -chevSize * 0.5).lineTo(chevSize, chevSize * 0.5).stroke();
    g.addChild(cg);
  }

  return g;
}

function drawInserter(entity: PlacedEntity): Graphics {
  const g = new Graphics();
  const s = TILE_PX - 1;
  const armColor = entity.carries ? itemColor(entity.carries) : (INSERTER_COLORS[entity.name] ?? 0x6a8e3e);

  g.roundRect(0, 0, s, s, TILE_RADIUS).fill(0x2a3a2a);

  const armG = new Graphics();
  armG.x = s / 2;
  armG.y = s / 2;
  armG.rotation = dirAngle(entity.direction);

  armG.circle(0, s * 0.2, s * 0.15).fill(0x444444);

  const armW = Math.max(1.5, s * 0.12);
  armG.setStrokeStyle({ width: armW, color: armColor, cap: "round" });
  armG.moveTo(0, s * 0.2).lineTo(0, -s * 0.35).stroke();

  const clawY = -s * 0.35;
  const clawW = s * 0.18;
  armG.moveTo(-clawW, clawY - clawW * 0.6)
    .lineTo(0, clawY)
    .lineTo(clawW, clawY - clawW * 0.6)
    .stroke();

  g.addChild(armG);
  return g;
}

function drawPipe(entity: PlacedEntity): Graphics {
  const g = new Graphics();
  const s = TILE_PX - 1;
  const isGround = entity.name === "pipe-to-ground";
  const pipeColor = isGround ? PIPE_TO_GROUND_COLOR : PIPE_COLOR;

  g.roundRect(0, 0, s, s, TILE_RADIUS).fill(0x1a2a3a);

  const cx = s / 2;
  const cy = s / 2;
  const pipeWidth = Math.max(2, s * 0.4);

  g.setStrokeStyle({ width: pipeWidth, color: pipeColor, cap: "round" });
  if (isGround) {
    // Single stub toward the underground entry/exit direction
    const [dx, dy] = dirVec(entity.direction);
    g.moveTo(cx, cy).lineTo(cx + dx * s / 2, cy + dy * s / 2).stroke();
    g.circle(cx, cy, pipeWidth * 0.4).fill(pipeColor);
    g.circle(cx, cy, pipeWidth * 0.25).fill(0x0a1520);
  } else {
    // Four stubs so adjacent pipes visually connect without neighbor lookup
    for (const [dx, dy] of [[0, -1], [1, 0], [0, 1], [-1, 0]] as [number, number][]) {
      g.moveTo(cx, cy).lineTo(cx + dx * s / 2, cy + dy * s / 2).stroke();
    }
    g.circle(cx, cy, pipeWidth * 0.4).fill(pipeColor);
  }

  return g;
}

function drawPole(): Graphics {
  const g = new Graphics();
  const s = TILE_PX - 1;

  g.roundRect(0, 0, s, s, TILE_RADIUS).fill(POLE_BG);

  const cx = s / 2;
  const cy = s / 2;
  const armLen = s * 0.38;
  const armW = Math.max(1.5, s * 0.2);

  g.rect(cx - armW / 2, cy - armLen, armW, armLen * 2).fill(POLE_COLOR);
  g.rect(cx - armLen, cy - armW / 2, armLen * 2, armW).fill(POLE_COLOR);
  g.circle(cx, cy, armW * 0.6).fill(0xe0c040);

  return g;
}

function drawMachine(entity: PlacedEntity): Graphics {
  const g = new Graphics();
  const [tw, th] = MACHINE_SIZES[entity.name] ?? [1, 1];
  const pw = tw * TILE_PX - 1;
  const ph = th * TILE_PX - 1;

  // Dotted outline showing the tile footprint
  g.setStrokeStyle({ width: 1, color: 0xaaaaaa, alpha: 0.4 });
  // Pixi v8 doesn't support native dash — draw a dotted rect manually
  const dash = 3;
  const gap = 3;
  for (const [x0, y0, x1, y1] of [
    [0, 0, pw, 0], [pw, 0, pw, ph], [pw, ph, 0, ph], [0, ph, 0, 0],
  ] as [number, number, number, number][]) {
    const len = Math.sqrt((x1 - x0) ** 2 + (y1 - y0) ** 2);
    const dx = (x1 - x0) / len;
    const dy = (y1 - y0) / len;
    let d = 0;
    while (d < len) {
      const end = Math.min(d + dash, len);
      g.moveTo(x0 + dx * d, y0 + dy * d).lineTo(x0 + dx * end, y0 + dy * end).stroke();
      d = end + gap;
    }
  }

  // Machine sprite — scaled 1.5x, centred on the tile footprint.
  // Entity-frame sprites are designed so (0,0) = top-left of footprint at 1x.
  // To scale 1.5x around the footprint centre, offset by -0.25 * footprint size.
  const spriteScale = 1.5;
  const frameTexture = Assets.get<Texture>(`/entity-frames/${entity.name}.png`);
  if (frameTexture) {
    const sprite = new Sprite(frameTexture);
    const baseScale = TILE_PX / ENTITY_FRAME_TILE_PX;
    sprite.scale.set(baseScale * spriteScale);
    // At 1x the sprite fills [0..pw, 0..ph]. At 1.5x it fills [0..pw*1.5, 0..ph*1.5].
    // Shift so the centre stays at (pw/2, ph/2).
    sprite.x = -pw * (spriteScale - 1) / 2;
    sprite.y = -ph * (spriteScale - 1) / 2;
    g.addChild(sprite);
  } else {
    const iconTexture = Assets.get<Texture>(`/icons/${entity.name}.png`);
    if (iconTexture) {
      const sprite = new Sprite(iconTexture);
      const iconSize = Math.min(pw, ph) * 0.8 * spriteScale;
      sprite.width = iconSize;
      sprite.height = iconSize;
      sprite.x = (pw - iconSize) / 2;
      sprite.y = (ph - iconSize) / 2;
      g.addChild(sprite);
    } else {
      const color = MACHINE_COLORS[entity.name] ?? DEFAULT_MACHINE_COLOR;
      g.roundRect(2, 2, pw - 4, ph - 4, 3).fill({ color, alpha: 0.5 });
    }
  }

  // Overlay panel: recipe name, rate, inputs/outputs
  if (entity.recipe) {
    const panelW = pw - 2;
    const flows = recipeFlowMap.get(entity.recipe);
    const inputCount = flows ? flows.inputs.length : 0;
    const outputCount = flows ? flows.outputs.length : 0;
    const flowLines = inputCount + outputCount;
    const lineH = 12;
    const headerH = 16;
    const panelH = headerH + flowLines * lineH + 6;
    const panelX = 1;
    const panelY = ph - panelH;

    // Semi-transparent dark background
    g.roundRect(panelX, panelY, panelW, panelH, 3)
      .fill({ color: 0x000000, alpha: 0.75 });

    let cy = panelY + 3;
    const iconSz = 14;

    const dropShadow = { color: 0x000000, alpha: 1, blur: 2, distance: 0 };

    // Header: recipe icon + nice name + rate — centred
    const recipeIcon = Assets.get<Texture>(`/icons/${entity.recipe}.png`);
    const label = niceName(entity.recipe);
    const rateStr = entity.rate != null ? ` ${entity.rate.toFixed(1)}/s` : "";
    const headerStyle = new TextStyle({
      fontSize: 9, fill: 0xffffff, fontWeight: "bold",
      align: "center", dropShadow,
    });
    const headerText = new Text({ text: label + rateStr, style: headerStyle });
    const totalHeaderW = (recipeIcon ? iconSz + 2 : 0) + headerText.width;
    const headerStartX = panelX + (panelW - totalHeaderW) / 2;

    if (recipeIcon) {
      const icoSprite = new Sprite(recipeIcon);
      icoSprite.width = iconSz;
      icoSprite.height = iconSz;
      icoSprite.x = headerStartX;
      icoSprite.y = cy;
      g.addChild(icoSprite);
      headerText.x = headerStartX + iconSz + 2;
    } else {
      headerText.x = headerStartX;
    }
    headerText.y = cy;
    g.addChild(headerText);
    cy += headerH;

    // Flow rows: icon + item name + rate — centred
    const flowStyle = new TextStyle({ fontSize: 8, fill: 0xcccccc, dropShadow });
    const renderFlow = (item: string, rate: number, prefix: string) => {
      const fIcon = Assets.get<Texture>(`/icons/${item}.png`);
      const fText = new Text({ text: `${prefix}${niceName(item)} ${rate.toFixed(1)}/s`, style: flowStyle });
      const fIconSz = 10;
      const rowW = (fIcon ? fIconSz + 2 : 0) + fText.width;
      const rx = panelX + (panelW - rowW) / 2;
      if (fIcon) {
        const fs = new Sprite(fIcon);
        fs.width = fIconSz; fs.height = fIconSz;
        fs.x = rx; fs.y = cy + 1;
        g.addChild(fs);
        fText.x = rx + fIconSz + 2;
      } else {
        fText.x = rx;
      }
      fText.y = cy;
      g.addChild(fText);
      cy += lineH;
    };

    if (flows) {
      for (const inp of flows.inputs) renderFlow(inp.item, inp.rate, "\u25b6 ");
      for (const out of flows.outputs) renderFlow(out.item, out.rate, "\u25c0 ");
    }
  }

  return g;
}

function drawGenericEntity(): Graphics {
  const g = new Graphics();
  const s = TILE_PX - 1;
  g.rect(0, 0, s, s).fill(0x4a5a6a);
  g.setStrokeStyle({ width: 1, color: 0x000000, alpha: 0.4 });
  g.rect(0, 0, s, s).stroke();
  return g;
}

export function isBeltEntity(name: string): boolean {
  return BELT_ENTITIES.has(name) || UG_BELT_ENTITIES.has(name) || SPLITTER_ENTITIES.has(name);
}

// Pixel-per-tile used when generating entity-frame PNGs (see scripts/extract_entity_frames.py)
const ENTITY_FRAME_TILE_PX = 64;

export async function initEntityIcons(slugs: string[]): Promise<void> {
  const urls = [
    ...slugs.map((s) => `/icons/${s}.png`),
    ...slugs.map((s) => `/entity-frames/${s}.png`),
  ];
  // Load individually and ignore 404s — not all slugs have sprites yet
  await Promise.allSettled(urls.map((url) => Assets.load(url)));
}

// Chain highlight controller returned by renderLayout

export interface HighlightController {
  /** Highlight all entities that carry the given item; dim everything else. */
  highlightItem(item: string | null): void;
  /** Highlight the connected belt network (upstream dashed, downstream solid). */
  highlightBeltNetwork(entity: PlacedEntity | null): void;
  /** Reset all entities to full opacity and remove any overlay. */
  clearHighlight(): void;
  /** Get the item chain key for an entity (its `carries` value, or recipe for machines). */
  chainKey(entity: PlacedEntity): string | null;
}

// Main renderer

export function renderLayout(
  layout: LayoutResult,
  container: Container,
  onHover?: (entity: PlacedEntity | null) => void,
  onSelect?: (entity: PlacedEntity | null) => void,
): HighlightController {
  container.removeChildren();

  // Build tile map for belt turn detection.
  // Splitters occupy 2 tiles (perpendicular to facing), so register both.
  const tileMap = new Map<string, PlacedEntity>();
  for (const e of layout.entities) {
    tileMap.set(`${e.x ?? 0},${e.y ?? 0}`, e);
    if (SPLITTER_ENTITIES.has(e.name)) {
      const [dx, dy] = splitterCompanionOffset(e.direction);
      tileMap.set(`${(e.x ?? 0) + dx},${(e.y ?? 0) + dy}`, e);
    }
  }

  // Index: item name → list of Graphics in that chain
  const itemIndex = new Map<string, Graphics[]>();
  const allGraphics: Graphics[] = [];
  // Maps each Graphics object back to its tile key "x,y" for belt network lookup
  const graphicsKeyMap = new Map<Graphics, string>();

  for (const entity of layout.entities) {
    let g: Graphics;

    if (BELT_ENTITIES.has(entity.name)) {
      g = drawBelt(entity, detectBeltTurn(entity, tileMap));
    } else if (UG_BELT_ENTITIES.has(entity.name)) {
      g = drawUndergroundBelt(entity);
    } else if (SPLITTER_ENTITIES.has(entity.name)) {
      g = drawSplitter(entity);
    } else if (INSERTER_ENTITIES.has(entity.name)) {
      g = drawInserter(entity);
    } else if (PIPE_ENTITIES.has(entity.name)) {
      g = drawPipe(entity);
    } else if (POLE_ENTITIES.has(entity.name)) {
      g = drawPole();
    } else if (MACHINE_ENTITIES.has(entity.name)) {
      g = drawMachine(entity);
    } else {
      g = drawGenericEntity();
    }

    g.x = (entity.x ?? 0) * TILE_PX;
    g.y = (entity.y ?? 0) * TILE_PX;

    // Rate overlay: small text label on belt/inserter entities with a rate
    if (rateOverlayEnabled && entity.rate != null) {
      const rateText = new Text({
        text: entity.rate.toFixed(1),
        style: { fontSize: 8, fill: 0xffffff, align: "center" },
      });
      rateText.x = TILE_PX / 2 - rateText.width / 2;
      rateText.y = TILE_PX / 2 - rateText.height / 2;
      g.addChild(rateText);
    }

    // Make every entity interactive for hover + click
    g.eventMode = "static";
    g.cursor = "pointer";
    if (onHover) {
      g.on("pointerenter", () => onHover(entity));
      g.on("pointerleave", () => onHover(null));
    }
    if (onSelect) {
      g.on("click", () => onSelect(entity));
    }

    // Register in item chain index
    const key = chainKey(entity);
    if (key) {
      if (!itemIndex.has(key)) itemIndex.set(key, []);
      itemIndex.get(key)!.push(g);
    }
    graphicsKeyMap.set(g, `${entity.x ?? 0},${entity.y ?? 0}`);
    allGraphics.push(g);

    container.addChild(g);
  }

  // Build belt connectivity graph (once per layout)
  const beltGraph: BeltGraph = buildBeltGraph(layout);

  // Overlay Graphics for belt network highlight — created/destroyed on hover
  let overlayGraphics: Graphics | null = null;

  // Draw crossing zone overlays
  if (layout.regions?.length) {
    for (const region of layout.regions) {
      if (region.kind !== "crossing_zone") continue;
      const zg = new Graphics();
      const rx = region.x * TILE_PX;
      const ry = region.y * TILE_PX;
      const rw = region.width * TILE_PX;
      const rh = region.height * TILE_PX;
      zg.rect(rx, ry, rw, rh)
        .stroke({ width: 1, color: 0x44aaff, alpha: 0.4 })
        .fill({ color: 0x44aaff, alpha: 0.06 });
      zg.eventMode = "static";
      zg.hitArea = { contains: (x: number, y: number) =>
        x >= rx && x < rx + rw && y >= ry && y < ry + rh };
      zg.on("pointerenter", () => onHover?.(null));
      container.addChild(zg);
    }
  }

  function clearHighlightInternal(): void {
    if (overlayGraphics) {
      container.removeChild(overlayGraphics);
      overlayGraphics.destroy();
      overlayGraphics = null;
    }
    for (const g of allGraphics) g.alpha = 1;
  }

  return {
    highlightItem(item: string | null): void {
      clearHighlightInternal();
      if (!item) return;
      const highlighted = itemIndex.get(item);
      if (!highlighted || highlighted.length === 0) return;
      const highlightSet = new Set(highlighted);
      for (const g of allGraphics) {
        g.alpha = highlightSet.has(g) ? 1 : 0.15;
      }
    },

    highlightBeltNetwork(entity: PlacedEntity | null): void {
      clearHighlightInternal();
      if (!entity) return;

      const startKey = `${entity.x ?? 0},${entity.y ?? 0}`;
      // Resolve to anchor (handles clicking on splitter's second tile)
      const anchor = beltGraph.tileToAnchor.get(startKey) ?? startKey;
      if (!beltGraph.nodes.has(anchor)) return;

      const { downstream, upstream } = traceBeltNetwork(anchor, beltGraph);
      const allBelt = new Set([...downstream, ...upstream]);
      const inserters = findAdjacentInserters(allBelt, beltGraph.entityMap);
      const machines = findAdjacentMachines(inserters, beltGraph.entityMap);

      for (const g of allGraphics) {
        const k = graphicsKeyMap.get(g);
        if (!k) {
          g.alpha = 0.15;
          continue;
        }
        if (allBelt.has(k)) {
          g.alpha = 0.5;
        } else if (inserters.has(k)) {
          g.alpha = 0.9;
        } else if (machines.has(k)) {
          g.alpha = 0.75;
        } else {
          g.alpha = 0.15;
        }
      }

      overlayGraphics = new Graphics();
      drawBeltNetworkOverlay(overlayGraphics, downstream, upstream, anchor, beltGraph);
      container.addChild(overlayGraphics);
    },

    clearHighlight(): void {
      clearHighlightInternal();
    },

    chainKey,
  };
}

/** Get the item chain key for an entity. */
function chainKey(entity: PlacedEntity): string | null {
  // Belts, underground belts, splitters, inserters, pipes — use carries
  if (entity.carries) return entity.carries;
  // Machines — use recipe as chain key (connects to the items it produces)
  if (entity.recipe) return entity.recipe;
  return null;
}
