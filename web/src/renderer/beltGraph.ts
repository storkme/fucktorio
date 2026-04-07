import { Graphics } from "pixi.js";
import type { PlacedEntity, EntityDirection, LayoutResult } from "../engine";

// Tile size in pixels — keep in sync with entities.ts TILE_PX
const TILE_PX = 32;

// Belt entity name sets (kept in sync with entities.ts)
const BELT_NAMES = new Set([
  "transport-belt",
  "fast-transport-belt",
  "express-transport-belt",
]);
const UG_NAMES = new Set([
  "underground-belt",
  "fast-underground-belt",
  "express-underground-belt",
]);
const SPLITTER_NAMES = new Set([
  "splitter",
  "fast-splitter",
  "express-splitter",
]);
const INSERTER_NAMES = new Set([
  "inserter",
  "fast-inserter",
  "long-handed-inserter",
]);
const MACHINE_NAMES = new Set([
  "assembling-machine-1",
  "assembling-machine-2",
  "assembling-machine-3",
  "chemical-plant",
  "oil-refinery",
  "electric-furnace",
  "steel-furnace",
  "stone-furnace",
  "centrifuge",
  "lab",
  "rocket-silo",
  "foundry",
  "biochamber",
  "biolab",
  "electromagnetic-plant",
  "cryogenic-plant",
  "recycler",
  "crusher",
  "beacon",
  "storage-tank",
  "big-electric-pole",
  "substation",
  "electric-mining-drill",
]);

// Chevron (highlight) colors per belt tier — matches BELT_COLORS[1] in entities.ts
const BELT_CHEVRON: Record<string, number> = {
  "transport-belt": 0xe0d070,
  "fast-transport-belt": 0xff6060,
  "express-transport-belt": 0x70b0f0,
  "underground-belt": 0xe0d070,
  "fast-underground-belt": 0xff6060,
  "express-underground-belt": 0x70b0f0,
  splitter: 0xe0d070,
  "fast-splitter": 0xff6060,
  "express-splitter": 0x70b0f0,
};

export type Lane = "left" | "right" | "both";

export interface BeltEdge {
  from: string;
  to: string;
  toLane: Lane;
  laneCross: boolean; // true for CW turns — lanes swap
  isSplitterOut: boolean;
}

export interface BeltGraph {
  nodes: Map<string, PlacedEntity>; // anchor key "x,y" → entity
  outEdges: Map<string, BeltEdge[]>; // forward adjacency
  inEdges: Map<string, BeltEdge[]>; // backward adjacency
  tileToAnchor: Map<string, string>; // every tile → anchor key (handles splitter second tile)
  entityMap: Map<string, PlacedEntity>; // all entities for inserter/machine lookup
}

function tk(x: number, y: number): string {
  return `${x},${y}`;
}

function dirVec(dir?: EntityDirection): [number, number] {
  switch (dir) {
    case "East":
      return [1, 0];
    case "South":
      return [0, 1];
    case "West":
      return [-1, 0];
    default:
      return [0, -1]; // North
  }
}

function addEdge(
  graph: BeltGraph,
  from: string,
  to: string,
  toLane: Lane,
  laneCross: boolean,
  isSplitterOut: boolean,
): void {
  const edge: BeltEdge = { from, to, toLane, laneCross, isSplitterOut };
  let out = graph.outEdges.get(from);
  if (!out) {
    out = [];
    graph.outEdges.set(from, out);
  }
  if (!out.some((e) => e.to === to)) out.push(edge);

  let inc = graph.inEdges.get(to);
  if (!inc) {
    inc = [];
    graph.inEdges.set(to, inc);
  }
  if (!inc.some((e) => e.from === from)) inc.push(edge);
}

const MAX_UG_DIST = 9;

export function buildBeltGraph(layout: LayoutResult): BeltGraph {
  const graph: BeltGraph = {
    nodes: new Map(),
    outEdges: new Map(),
    inEdges: new Map(),
    tileToAnchor: new Map(),
    entityMap: new Map(),
  };

  // Full entity map (all types) — used for UG exit scanning + inserter/machine lookup
  for (const e of layout.entities) {
    graph.entityMap.set(tk(e.x ?? 0, e.y ?? 0), e);
  }

  // Pass 1: populate belt nodes + tileToAnchor
  for (const e of layout.entities) {
    if (
      !BELT_NAMES.has(e.name) &&
      !UG_NAMES.has(e.name) &&
      !SPLITTER_NAMES.has(e.name)
    )
      continue;
    const x = e.x ?? 0;
    const y = e.y ?? 0;
    const k = tk(x, y);
    graph.nodes.set(k, e);
    graph.tileToAnchor.set(k, k);

    if (SPLITTER_NAMES.has(e.name)) {
      // Splitters occupy 2 tiles perpendicular to travel direction.
      // NS (North/South) → 2 tiles wide in X: second tile at (x+1, y)
      // EW (East/West)   → 2 tiles tall in Y: second tile at (x, y+1)
      const isNS = e.direction === "North" || e.direction === "South";
      const sx = x + (isNS ? 1 : 0);
      const sy = y + (isNS ? 0 : 1);
      graph.tileToAnchor.set(tk(sx, sy), k);
    }
  }

  // Pass 2: compute directed edges
  for (const [k, e] of graph.nodes) {
    const x = e.x ?? 0;
    const y = e.y ?? 0;
    const dir = e.direction ?? "North";
    const [dx, dy] = dirVec(dir);

    if (BELT_NAMES.has(e.name)) {
      // Forward edge: belt's output goes to the tile in front of it.
      const fAnchor = graph.tileToAnchor.get(tk(x + dx, y + dy));
      if (fAnchor !== undefined && fAnchor !== k) {
        const dest = graph.nodes.get(fAnchor)!;
        const [ddx, ddy] = dirVec(dest.direction);
        // Cross product of our dir × dest dir: > 0 = CW turn (lanes swap)
        const cross = dx * ddy - dy * ddx;
        addEdge(graph, k, fAnchor, "both", cross > 0, false);
      }
    } else if (UG_NAMES.has(e.name)) {
      if (e.io_type === "input") {
        // Scan forward for matching UG output (same name = same tier, same direction)
        for (let dist = 1; dist <= MAX_UG_DIST; dist++) {
          const te = graph.entityMap.get(tk(x + dx * dist, y + dy * dist));
          if (!te) continue;
          // Another input in same direction blocks the tunnel
          if (
            UG_NAMES.has(te.name) &&
            te.name === e.name &&
            te.io_type === "input" &&
            te.direction === dir
          )
            break;
          if (
            UG_NAMES.has(te.name) &&
            te.name === e.name &&
            te.io_type === "output" &&
            te.direction === dir
          ) {
            const tAnchor = graph.tileToAnchor.get(
              tk(te.x ?? 0, te.y ?? 0),
            );
            if (tAnchor !== undefined)
              addEdge(graph, k, tAnchor, "both", false, false);
            break;
          }
        }
      } else {
        // UG output: forward edge like a normal belt
        const fAnchor = graph.tileToAnchor.get(tk(x + dx, y + dy));
        if (fAnchor !== undefined && fAnchor !== k)
          addEdge(graph, k, fAnchor, "both", false, false);
      }
    } else if (SPLITTER_NAMES.has(e.name)) {
      // Splitter outputs: two tiles in the flow direction from the anchor tile and the second tile.
      const isNS = dir === "North" || dir === "South";
      const [sdx, sdy] = isNS ? [1, 0] : [0, 1];
      for (const [ox, oy] of [
        [x + dx, y + dy],
        [x + sdx + dx, y + sdy + dy],
      ] as [number, number][]) {
        const oAnchor = graph.tileToAnchor.get(tk(ox, oy));
        if (oAnchor !== undefined && oAnchor !== k)
          addEdge(graph, k, oAnchor, "both", false, true);
      }
    }
  }

  return graph;
}

export function traceBeltNetwork(
  startKey: string,
  graph: BeltGraph,
): { downstream: Set<string>; upstream: Set<string> } {
  const downstream = new Set<string>();
  const upstream = new Set<string>();

  // BFS forward (downstream)
  const fq: string[] = [startKey];
  downstream.add(startKey);
  while (fq.length > 0) {
    const cur = fq.shift()!;
    for (const edge of graph.outEdges.get(cur) ?? []) {
      if (!downstream.has(edge.to)) {
        downstream.add(edge.to);
        fq.push(edge.to);
      }
    }
  }

  // BFS backward (upstream)
  const bq: string[] = [startKey];
  upstream.add(startKey);
  while (bq.length > 0) {
    const cur = bq.shift()!;
    for (const edge of graph.inEdges.get(cur) ?? []) {
      if (!upstream.has(edge.from)) {
        upstream.add(edge.from);
        bq.push(edge.from);
      }
    }
  }

  return { downstream, upstream };
}

export function findAdjacentInserters(
  beltKeys: Set<string>,
  entityMap: Map<string, PlacedEntity>,
): Set<string> {
  const inserters = new Set<string>();
  const offsets: [number, number][] = [
    [0, -1],
    [1, 0],
    [0, 1],
    [-1, 0],
  ];
  for (const k of beltKeys) {
    const [xs, ys] = k.split(",").map(Number);
    for (const [ox, oy] of offsets) {
      const nk = tk(xs + ox, ys + oy);
      const ne = entityMap.get(nk);
      if (ne && INSERTER_NAMES.has(ne.name)) inserters.add(nk);
    }
  }
  return inserters;
}

export function findAdjacentMachines(
  inserterKeys: Set<string>,
  entityMap: Map<string, PlacedEntity>,
): Set<string> {
  const machines = new Set<string>();
  const offsets: [number, number][] = [
    [0, -1],
    [1, 0],
    [0, 1],
    [-1, 0],
  ];
  for (const k of inserterKeys) {
    const [xs, ys] = k.split(",").map(Number);
    for (const [ox, oy] of offsets) {
      const nk = tk(xs + ox, ys + oy);
      const ne = entityMap.get(nk);
      if (ne && MACHINE_NAMES.has(ne.name)) machines.add(nk);
    }
  }
  return machines;
}

// Corner arc parameters lookup: key = "inDir_outDir"
// Arc center offsets (as pixel coords relative to tile origin px,py), startAngle, endAngle,
// anticlockwise flag — all in canvas coordinates (y increases downward).
interface CornerArcDef {
  cx: (px: number, py: number, s: number) => number;
  cy: (px: number, py: number, s: number) => number;
  startAngle: number;
  endAngle: number;
  anticlockwise: boolean;
}

const CORNER_ARC_TABLE: Record<string, CornerArcDef> = {
  // CW turns (anticlockwise=false)
  East_South:  { cx: (px) => px,   cy: (_px, py, s) => py + s, startAngle: -Math.PI / 2,     endAngle: 0,              anticlockwise: false },
  South_West:  { cx: (px) => px,   cy: (_px, py)    => py,     startAngle: 0,                 endAngle: Math.PI / 2,    anticlockwise: false },
  West_North:  { cx: (px, _py, s) => px + s, cy: (_px, py) => py, startAngle: Math.PI / 2,  endAngle: Math.PI,         anticlockwise: false },
  North_East:  { cx: (px, _py, s) => px + s, cy: (_px, py, s) => py + s, startAngle: Math.PI, endAngle: 3 * Math.PI / 2, anticlockwise: false },
  // CCW turns (anticlockwise=true)
  East_North:  { cx: (px) => px,   cy: (_px, py)    => py,     startAngle: Math.PI / 2,       endAngle: 0,              anticlockwise: true  },
  North_West:  { cx: (px) => px,   cy: (_px, py, s) => py + s, startAngle: 0,                 endAngle: -Math.PI / 2,   anticlockwise: true  },
  West_South:  { cx: (px, _py, s) => px + s, cy: (_px, py, s) => py + s, startAngle: -Math.PI / 2, endAngle: -Math.PI, anticlockwise: true  },
  South_East:  { cx: (px, _py, s) => px + s, cy: (_px, py) => py, startAngle: Math.PI,       endAngle: Math.PI / 2,    anticlockwise: true  },
};

/** Detect if belt at key k is a 90° corner; returns {inDir, outDir} or null. */
function getCornerTurn(
  k: string,
  graph: BeltGraph,
): { inDir: string; outDir: string } | null {
  const e = graph.nodes.get(k);
  if (!e || !BELT_NAMES.has(e.name)) return null;
  const outDir = e.direction ?? "North";
  for (const edge of graph.inEdges.get(k) ?? []) {
    const src = graph.nodes.get(edge.from);
    if (!src) continue;
    const inDir = src.direction ?? "North";
    if (`${inDir}_${outDir}` in CORNER_ARC_TABLE) return { inDir, outDir };
  }
  return null;
}

function drawDashedLine(
  g: Graphics,
  x1: number,
  y1: number,
  x2: number,
  y2: number,
  dashLen: number,
  gapLen: number,
): void {
  const dx = x2 - x1;
  const dy = y2 - y1;
  const totalLen = Math.sqrt(dx * dx + dy * dy);
  if (totalLen === 0) return;
  const ux = dx / totalLen;
  const uy = dy / totalLen;
  let pos = 0;
  let drawing = true;
  while (pos < totalLen) {
    const segLen = Math.min(drawing ? dashLen : gapLen, totalLen - pos);
    if (drawing) {
      g.moveTo(x1 + ux * pos, y1 + uy * pos)
        .lineTo(x1 + ux * (pos + segLen), y1 + uy * (pos + segLen))
        .stroke();
    }
    pos += segLen;
    drawing = !drawing;
  }
}

function drawDashedArc(
  g: Graphics,
  cx: number, cy: number, r: number,
  startAngle: number, endAngle: number,
  anticlockwise: boolean,
  dashAngleRad: number, gapAngleRad: number,
): void {
  let span = anticlockwise ? startAngle - endAngle : endAngle - startAngle;
  if (span < 0) span += 2 * Math.PI;
  let pos = 0;
  let drawing = true;
  while (pos < span) {
    const segSpan = Math.min(drawing ? dashAngleRad : gapAngleRad, span - pos);
    if (drawing) {
      const segStart = anticlockwise ? startAngle - pos : startAngle + pos;
      const segEnd = anticlockwise ? segStart - segSpan : segStart + segSpan;
      const sx = cx + r * Math.cos(segStart);
      const sy = cy + r * Math.sin(segStart);
      g.moveTo(sx, sy).arc(cx, cy, r, segStart, segEnd, anticlockwise).stroke();
    }
    pos += segSpan;
    drawing = !drawing;
  }
}

export function drawBeltNetworkOverlay(
  g: Graphics,
  downstream: Set<string>,
  upstream: Set<string>,
  startKey: string,
  graph: BeltGraph,
): void {
  const s = TILE_PX;
  const r = s / 2; // corner arc radius (entry/exit at edge centres)

  // Upstream (subtle dashed) — drawn first so downstream renders on top
  for (const k of upstream) {
    if (downstream.has(k)) continue;
    const e = graph.nodes.get(k);
    if (!e) continue;
    const px = (e.x ?? 0) * s;
    const py = (e.y ?? 0) * s;
    const chev = BELT_CHEVRON[e.name] ?? 0xe0d070;
    const [dx, dy] = dirVec(e.direction);
    const cx = px + s / 2;
    const cy = py + s / 2;

    g.rect(px, py, s, s).fill({ color: chev, alpha: 0.05 });
    g.setStrokeStyle({ width: 1.5, color: chev, alpha: 0.28, cap: "round" });

    const corner = getCornerTurn(k, graph);
    if (corner) {
      const def = CORNER_ARC_TABLE[`${corner.inDir}_${corner.outDir}`];
      const acx = def.cx(px, py, s);
      const acy = def.cy(px, py, s);
      drawDashedArc(g, acx, acy, r, def.startAngle, def.endAngle, def.anticlockwise, 5 / r, 3 / r);
    } else {
      drawDashedLine(g, cx - dx * s * 0.45, cy - dy * s * 0.45, cx + dx * s * 0.45, cy + dy * s * 0.45, 5, 3);
    }
  }

  // Downstream (solid, brighter)
  for (const k of downstream) {
    const e = graph.nodes.get(k);
    if (!e) continue;
    const px = (e.x ?? 0) * s;
    const py = (e.y ?? 0) * s;
    const chev = BELT_CHEVRON[e.name] ?? 0xe0d070;
    const [dx, dy] = dirVec(e.direction);
    const cx = px + s / 2;
    const cy = py + s / 2;

    g.rect(px, py, s, s).fill({ color: chev, alpha: 0.2 });
    g.setStrokeStyle({ width: 2, color: chev, alpha: 0.85, cap: "round" });

    const corner = getCornerTurn(k, graph);
    if (corner) {
      const def = CORNER_ARC_TABLE[`${corner.inDir}_${corner.outDir}`];
      const acx = def.cx(px, py, s);
      const acy = def.cy(px, py, s);
      const sx = acx + r * Math.cos(def.startAngle);
      const sy = acy + r * Math.sin(def.startAngle);
      g.moveTo(sx, sy).arc(acx, acy, r, def.startAngle, def.endAngle, def.anticlockwise).stroke();
    } else {
      g.moveTo(cx - dx * s * 0.45, cy - dy * s * 0.45)
        .lineTo(cx + dx * s * 0.45, cy + dy * s * 0.45)
        .stroke();
    }
  }

  // White border around the hovered tile
  const he = graph.nodes.get(startKey);
  if (he) {
    const px = (he.x ?? 0) * s;
    const py = (he.y ?? 0) * s;
    g.setStrokeStyle({ width: 2, color: 0xffffff, alpha: 0.8 });
    g.rect(px + 1, py + 1, s - 2, s - 2).stroke();
  }
}
