import { Container, Graphics, Text, TextStyle } from "pixi.js";
import { TILE_PX } from "./entities";
import type { TraceEvent } from "../engine";

export type { TraceEvent } from "../engine";

// Convenience narrowing aliases for event variants used in this file.
type RowsPlaced   = Extract<TraceEvent, { phase: "RowsPlaced" }>;
type LanesPlanned = Extract<TraceEvent, { phase: "LanesPlanned" }>;
export type PhaseSnapshot = Extract<TraceEvent, { phase: "PhaseSnapshot" }>;
export type PhaseComplete = Extract<TraceEvent, { phase: "PhaseComplete" }>;
type RouteFailureEvent = Extract<TraceEvent, { phase: "RouteFailure" }>;
type LaneConsolidatedEvent = Extract<TraceEvent, { phase: "LaneConsolidated" }>;
type RowSplitEvent = Extract<TraceEvent, { phase: "RowSplit" }>;
type LaneOrderOptimizedEvent = Extract<TraceEvent, { phase: "LaneOrderOptimized" }>;
type GhostSpecRoutedEvent = Extract<TraceEvent, { phase: "GhostSpecRouted" }>;
type GhostSpecFailedEvent = Extract<TraceEvent, { phase: "GhostSpecFailed" }>;
type GhostRoutingCompleteEvent = Extract<TraceEvent, { phase: "GhostRoutingComplete" }>;

/** Draw a dashed line segment on a Graphics context. */
function drawDashedLine(
  g: Graphics,
  x0: number, y0: number, x1: number, y1: number,
  dashLen: number, gapLen: number,
  opts: { width?: number; color?: number; alpha?: number },
): void {
  const dx = x1 - x0;
  const dy = y1 - y0;
  const dist = Math.sqrt(dx * dx + dy * dy);
  if (dist === 0) return;
  const ux = dx / dist;
  const uy = dy / dist;
  let drawn = 0;
  while (drawn < dist) {
    const segEnd = Math.min(drawn + dashLen, dist);
    g.moveTo(x0 + ux * drawn, y0 + uy * drawn)
      .lineTo(x0 + ux * segEnd, y0 + uy * segEnd)
      .stroke(opts);
    drawn = segEnd + gapLen;
  }
}

export function renderTraceOverlay(
  events: TraceEvent[],
  layoutWidth: number,
  layoutHeight: number,
  container: Container,
  onHover: (text: string | null) => void,
): Container {
  const layer = new Container();

  // --- Lane columns (from LanesPlanned) ---
  const lanesEvent = events.find((e): e is LanesPlanned => e.phase === "LanesPlanned");
  if (lanesEvent) {
    for (const lane of lanesEvent.data.lanes) {
      const g = new Graphics();
      const lx = lane.x * TILE_PX;
      g.rect(lx, 0, TILE_PX, layoutHeight * TILE_PX)
        .fill({ color: lane.is_fluid ? 0x44aaff : 0x44ff88, alpha: 0.04 });
      g.eventMode = "static";
      g.on("pointerenter", () => onHover(`Lane: ${lane.item} @ x=${lane.x} (${lane.rate.toFixed(1)}/s${lane.is_fluid ? " fluid" : ""})`));
      g.on("pointerleave", () => onHover(null));
      layer.addChild(g);
    }
  }

  // --- Row boundaries (from RowsPlaced) ---
  const rowsEvent = events.find((e): e is RowsPlaced => e.phase === "RowsPlaced");
  if (rowsEvent) {
    for (const row of rowsEvent.data.rows) {
      const g = new Graphics();
      const ry = row.y_end * TILE_PX;
      g.moveTo(0, ry)
        .lineTo(layoutWidth * TILE_PX, ry)
        .stroke({ width: 1, color: 0x6a8a5a, alpha: 0.3 });
      g.eventMode = "static";
      g.on("pointerenter", () => onHover(`Row ${row.index}: ${row.recipe} (${row.machine_count}× ${row.machine})`));
      g.on("pointerleave", () => onHover(null));
      layer.addChild(g);
    }
  }

  // --- Balancer blocks (from BalancerStamped) ---
  for (const evt of events) {
    if (evt.phase !== "BalancerStamped") continue;
    const d = evt.data;
    const height = (d.y_end - d.y_start) * TILE_PX;
    if (height <= 0) continue;
    const g = new Graphics();
    g.rect(0, d.y_start * TILE_PX, layoutWidth * TILE_PX, height)
      .fill({ color: 0xaa44ff, alpha: 0.05 })
      .stroke({ width: 1, color: 0xaa44ff, alpha: 0.4 });
    g.eventMode = "static";
    g.on("pointerenter", () => onHover(`Balancer: ${d.item} ${d.shape[0]}→${d.shape[1]} (template: ${d.template_found})`));
    g.on("pointerleave", () => onHover(null));
    layer.addChild(g);
  }

  // --- Tap-off paths (from TapoffRouted) ---
  for (const evt of events) {
    if (evt.phase !== "TapoffRouted") continue;
    const d = evt.data;
    const g = new Graphics();
    g.moveTo(d.from_x * TILE_PX + TILE_PX / 2, d.from_y * TILE_PX + TILE_PX / 2)
      .lineTo(d.to_x * TILE_PX + TILE_PX / 2, d.to_y * TILE_PX + TILE_PX / 2)
      .stroke({ width: 2, color: 0x88ff44, alpha: 0.5 });
    g.eventMode = "static";
    g.on("pointerenter", () => onHover(`Tap-off: ${d.item} (${d.from_x},${d.from_y})→(${d.to_x},${d.to_y}) len=${d.path_len}`));
    g.on("pointerleave", () => onHover(null));
    layer.addChild(g);
  }

  // --- Merger blocks (from MergerBlockPlaced) ---
  for (const evt of events) {
    if (evt.phase !== "MergerBlockPlaced") continue;
    const d = evt.data;
    const g = new Graphics();
    g.rect(0, d.block_y * TILE_PX, layoutWidth * TILE_PX, d.block_height * TILE_PX)
      .fill({ color: 0xffcc44, alpha: 0.05 })
      .stroke({ width: 1, color: 0xffcc44, alpha: 0.4 });
    g.eventMode = "static";
    g.on("pointerenter", () => onHover(`Merger: ${d.item} (${d.lanes} lanes, y=${d.block_y}..${d.block_y + d.block_height})`));
    g.on("pointerleave", () => onHover(null));
    layer.addChild(g);
  }

  // --- Route failures (from RouteFailure) ---
  for (const evt of events) {
    if (evt.phase !== "RouteFailure") continue;
    const d = (evt as RouteFailureEvent).data;
    const cx = d.from_x * TILE_PX + TILE_PX / 2;
    const cy = d.from_y * TILE_PX + TILE_PX / 2;
    const halfSpan = 3;
    const g = new Graphics();
    g.label = "RouteFailure";
    // Red ✕ cross at source tile
    g.moveTo(cx - halfSpan, cy - halfSpan)
      .lineTo(cx + halfSpan, cy + halfSpan)
      .stroke({ width: 2, color: 0xff3333 });
    g.moveTo(cx + halfSpan, cy - halfSpan)
      .lineTo(cx - halfSpan, cy + halfSpan)
      .stroke({ width: 2, color: 0xff3333 });
    // Dashed red line from source to target
    drawDashedLine(g, cx, cy, d.to_x * TILE_PX + TILE_PX / 2, d.to_y * TILE_PX + TILE_PX / 2,
      6, 4, { width: 1, color: 0xff3333, alpha: 0.6 });
    g.eventMode = "static";
    g.on("pointerenter", () => onHover(`Route failed: ${d.item} (${d.from_x},${d.from_y})\u2192(${d.to_x},${d.to_y}) [${d.spec_key}]`));
    g.on("pointerleave", () => onHover(null));
    layer.addChild(g);
  }

  // --- Lane consolidation badges (from LaneConsolidated) ---
  const badgeStyle = new TextStyle({ fontSize: 10, fill: "#ffaa44", fontFamily: "monospace", fontWeight: "bold" });
  for (const evt of events) {
    if (evt.phase !== "LaneConsolidated") continue;
    const d = (evt as LaneConsolidatedEvent).data;
    let laneX = -1;
    if (lanesEvent) {
      const match = lanesEvent.data.lanes.find(l => l.item === d.item);
      if (match) laneX = match.x;
    }
    if (laneX < 0) continue;
    const badge = new Text({ text: `\u00F7${d.n_trunk_lanes}`, style: badgeStyle });
    badge.x = laneX * TILE_PX + TILE_PX / 2 - badge.width / 2;
    badge.y = 2;
    badge.eventMode = "static";
    badge.on("pointerenter", () => onHover(`${d.item}: ${d.consumer_count} consumers share ${d.n_trunk_lanes} lane(s) @ ${d.rate_per_lane.toFixed(1)}/s each`));
    badge.on("pointerleave", () => onHover(null));
    layer.addChild(badge);
  }

  // --- Row split indicators (from RowSplit) ---
  const splitStyle = new TextStyle({ fontSize: 10, fill: "#ffcc44", fontFamily: "monospace", fontWeight: "bold" });
  for (const evt of events) {
    if (evt.phase !== "RowSplit") continue;
    const d = (evt as RowSplitEvent).data;
    let splitY = -1;
    if (rowsEvent) {
      const matching = rowsEvent.data.rows.filter(r => r.recipe === d.recipe);
      if (matching.length > 0) {
        splitY = matching.reduce((a, b) => a.y_end > b.y_end ? a : b).y_end;
      }
    }
    if (splitY < 0) continue;
    const label = new Text({ text: `\u2295${d.split_into}`, style: splitStyle });
    label.x = 4;
    label.y = splitY * TILE_PX - label.height - 1;
    label.eventMode = "static";
    label.on("pointerenter", () => onHover(`${d.recipe}: split ${d.original_count}\u2192${d.split_into} rows \u2014 ${d.reason}`));
    label.on("pointerleave", () => onHover(null));
    layer.addChild(label);
  }

  const summaryStyle = new TextStyle({ fontSize: 10, fill: "#aaa", fontFamily: "monospace" });

  // --- Ghost routing paths (from GhostSpecRouted) ---
  const ghostPalette = [
    0x569cd6, 0xd0a040, 0x6ac080, 0xc06090, 0x70b0e0,
    0xb080d0, 0xe07050, 0x60c0c0, 0xd0d060, 0x80b060,
  ];
  let ghostPathIdx = 0;
  for (const evt of events) {
    if (evt.phase !== "GhostSpecRouted") continue;
    const d = (evt as GhostSpecRoutedEvent).data;
    const color = ghostPalette[ghostPathIdx % ghostPalette.length];
    ghostPathIdx++;
    const g = new Graphics();
    // Draw path polyline through tile centers
    if (d.tiles && d.tiles.length > 1) {
      g.setStrokeStyle({ width: 3, color, alpha: 0.7 });
      g.moveTo(d.tiles[0][0] * TILE_PX + TILE_PX / 2, d.tiles[0][1] * TILE_PX + TILE_PX / 2);
      for (let i = 1; i < d.tiles.length; i++) {
        g.lineTo(d.tiles[i][0] * TILE_PX + TILE_PX / 2, d.tiles[i][1] * TILE_PX + TILE_PX / 2);
      }
      g.stroke();
    }
    // Crossing tiles as yellow diamonds
    if (d.crossing_tiles) {
      for (const [cx, cy] of d.crossing_tiles) {
        const px = cx * TILE_PX + TILE_PX / 2;
        const py = cy * TILE_PX + TILE_PX / 2;
        const ds = TILE_PX * 0.25;
        g.moveTo(px, py - ds).lineTo(px + ds, py).lineTo(px, py + ds).lineTo(px - ds, py).closePath()
          .fill({ color: 0xffdd00, alpha: 0.85 });
      }
    }
    g.eventMode = "static";
    g.on("pointerenter", () => onHover(`Ghost path: ${d.spec_key} len=${d.path_len} crossings=${d.crossings} turns=${d.turns}`));
    g.on("pointerleave", () => onHover(null));
    layer.addChild(g);
  }

  // --- Ghost spec failures (from GhostSpecFailed) ---
  for (const evt of events) {
    if (evt.phase !== "GhostSpecFailed") continue;
    const d = (evt as GhostSpecFailedEvent).data;
    const cx = d.from_x * TILE_PX + TILE_PX / 2;
    const cy = d.from_y * TILE_PX + TILE_PX / 2;
    const halfSpan = 4;
    const g = new Graphics();
    g.label = "RouteFailure";
    g.moveTo(cx - halfSpan, cy - halfSpan).lineTo(cx + halfSpan, cy + halfSpan).stroke({ width: 2, color: 0xff3333 });
    g.moveTo(cx + halfSpan, cy - halfSpan).lineTo(cx - halfSpan, cy + halfSpan).stroke({ width: 2, color: 0xff3333 });
    drawDashedLine(g, cx, cy, d.to_x * TILE_PX + TILE_PX / 2, d.to_y * TILE_PX + TILE_PX / 2,
      6, 4, { width: 1, color: 0xff3333, alpha: 0.6 });
    g.eventMode = "static";
    g.on("pointerenter", () => onHover(`Ghost failed: ${d.spec_key} (${d.from_x},${d.from_y})→(${d.to_x},${d.to_y})`));
    g.on("pointerleave", () => onHover(null));
    layer.addChild(g);
  }

  // --- Ghost cluster zones (from GhostClusterSolved / GhostClusterFailed) ---
  for (const evt of events) {
    if (evt.phase !== "GhostClusterSolved" && evt.phase !== "GhostClusterFailed") continue;
    const isFailed = evt.phase === "GhostClusterFailed";
    const solved = isFailed ? null : (evt as Extract<TraceEvent, { phase: "GhostClusterSolved" }>).data;
    const failed = isFailed ? (evt as Extract<TraceEvent, { phase: "GhostClusterFailed" }>).data : null;
    const base = solved ?? failed!;
    const color = isFailed ? 0xff4444 : 0x44aaff;
    const g = new Graphics();
    g.rect(base.zone_x * TILE_PX, base.zone_y * TILE_PX, base.zone_w * TILE_PX, base.zone_h * TILE_PX)
      .fill({ color, alpha: isFailed ? 0.15 : 0.08 })
      .stroke({ width: isFailed ? 2 : 1, color, alpha: isFailed ? 0.9 : 0.6 });
    g.eventMode = "static";
    const solveInfo = solved ? ` vars=${solved.variables} clauses=${solved.clauses} ${(solved.solve_time_us / 1000).toFixed(1)}ms` : "";
    g.on("pointerenter", () => onHover(`Cluster #${base.cluster_id}: ${base.zone_w}x${base.zone_h} @ (${base.zone_x},${base.zone_y}) ${base.boundary_count} ports${solveInfo}${isFailed ? " FAILED" : ""}`));
    g.on("pointerleave", () => onHover(null));
    layer.addChild(g);
  }

  // --- Ghost routing summary (from GhostRoutingComplete) ---
  const ghostComplete = events.find((e): e is GhostRoutingCompleteEvent => e.phase === "GhostRoutingComplete");
  if (ghostComplete) {
    const d = ghostComplete.data;
    const info = `Ghost: ${d.entity_count} entities, ${d.cluster_count} clusters (max ${d.max_cluster_tiles} tiles), ${d.unroutable_count} unroutable`;
    const summaryGhost = new Text({ text: info, style: summaryStyle });
    summaryGhost.x = 4;
    summaryGhost.y = -28;
    layer.addChild(summaryGhost);
  }
  const phaseNames = events.map(e => e.phase);
  const uniquePhases = [...new Set(phaseNames)];
  const laneOrder = events.find((e): e is LaneOrderOptimizedEvent => e.phase === "LaneOrderOptimized");
  const crossingSuffix = laneOrder ? ` | lane order: ${laneOrder.data.crossing_score} crossings` : "";
  const summaryText = new Text({ text: `Trace: ${uniquePhases.join(" → ")}${crossingSuffix}`, style: summaryStyle });
  summaryText.x = 4;
  summaryText.y = -16;
  layer.addChild(summaryText);

  container.addChild(layer);
  return layer;
}

// ─── Ghost routing overlay ──────────────────────────────────────────────────

type GhostClusterSolvedEvent = Extract<TraceEvent, { phase: "GhostClusterSolved" }>;
type GhostClusterFailedEvent = Extract<TraceEvent, { phase: "GhostClusterFailed" }>;

const GHOST_PALETTE = [
  0x569cd6, 0xd0a040, 0x6ac080, 0xc06090, 0x70b0e0,
  0xb080d0, 0xe07050, 0x60c0c0, 0xd0d060, 0x80b060,
  0xe0a060, 0x60a0d0, 0xd060a0, 0xa0d060, 0x9060d0,
];

/** Deterministic color assignment by item name. */
function ghostColorForItem(item: string, colorMap: Map<string, number>): number {
  if (colorMap.has(item)) return colorMap.get(item)!;
  const color = GHOST_PALETTE[colorMap.size % GHOST_PALETTE.length];
  colorMap.set(item, color);
  return color;
}

/**
 * Render a standalone ghost routing overlay.
 * Shows paths colored by item, crossing heatmap, cluster zones, and failures.
 * Call with `parent` = the viewport or entity layer to receive the new Container.
 */
export function renderGhostRoutingOverlay(
  events: TraceEvent[],
  layoutWidth: number,
  layoutHeight: number,
  parent: Container,
  onHover: (text: string | null) => void,
): Container {
  const layer = new Container();

  // Build item→color map by order of first appearance
  const colorMap = new Map<string, number>();

  // Collect all GhostSpecRouted events and extract item name from spec_key
  const routedEvents = events.filter((e): e is GhostSpecRoutedEvent => e.phase === "GhostSpecRouted");
  const failedEvents = events.filter((e): e is GhostSpecFailedEvent => e.phase === "GhostSpecFailed");

  // Extract item name from spec_key (format: "item-name:from_x,from_y->to_x,to_y" or similar)
  function itemFromSpecKey(specKey: string): string {
    // spec_key is typically "item-name:..." or just "item-name"
    const colonIdx = specKey.indexOf(":");
    return colonIdx >= 0 ? specKey.slice(0, colonIdx) : specKey;
  }

  // Pre-populate color map in order of appearance
  for (const evt of routedEvents) {
    const item = itemFromSpecKey(evt.data.spec_key);
    ghostColorForItem(item, colorMap);
  }
  for (const evt of failedEvents) {
    const item = itemFromSpecKey(evt.data.spec_key);
    ghostColorForItem(item, colorMap);
  }

  // --- Crossing heatmap ---
  // Map "x,y" -> set of distinct item names traversing that tile
  const tileItems = new Map<string, Set<string>>();
  for (const evt of routedEvents) {
    const item = itemFromSpecKey(evt.data.spec_key);
    if (evt.data.tiles) {
      for (const [tx, ty] of evt.data.tiles) {
        const key = `${tx},${ty}`;
        if (!tileItems.has(key)) tileItems.set(key, new Set());
        tileItems.get(key)!.add(item);
      }
    }
  }

  // Draw crossing hotspots (tiles with 2+ distinct items)
  const heatG = new Graphics();
  for (const [key, items] of tileItems) {
    if (items.size < 2) continue;
    const [tx, ty] = key.split(",").map(Number);
    const alpha = Math.min(0.3 + items.size * 0.12, 0.9);
    heatG.rect(tx * TILE_PX, ty * TILE_PX, TILE_PX, TILE_PX)
      .fill({ color: 0xff6600, alpha });
  }
  layer.addChild(heatG);

  // --- Axis occupancy (Phase-1 instrumentation) ---
  // Red square = same-axis conflict (>=2 N/S OR >=2 E/W specs at this tile).
  // Blue dot   = perpendicular crossing (>=1 N/S AND >=1 E/W spec at this tile).
  // A tile can be both — render both layers; the hover hit-rect covers either.
  const axisEvent = events.find(
    (e): e is Extract<TraceEvent, { phase: "GhostAxisOccupancy" }> =>
      e.phase === "GhostAxisOccupancy",
  );
  let axisSummary = "";
  if (axisEvent) {
    const axisG = new Graphics();
    const dotG = new Graphics();
    for (const tile of axisEvent.data.tiles) {
      const sameAxis = tile.vert_count >= 2 || tile.horiz_count >= 2;
      const perp = tile.vert_count >= 1 && tile.horiz_count >= 1;
      if (sameAxis) {
        const maxSame = Math.max(tile.vert_count, tile.horiz_count);
        const alpha = Math.min(0.25 + maxSame * 0.15, 0.85);
        axisG.rect(tile.x * TILE_PX, tile.y * TILE_PX, TILE_PX, TILE_PX)
          .fill({ color: 0xff2222, alpha });
      }
      if (perp) {
        dotG.circle(tile.x * TILE_PX + TILE_PX / 2, tile.y * TILE_PX + TILE_PX / 2, TILE_PX * 0.22)
          .fill({ color: 0x3399ff, alpha: 0.9 })
          .stroke({ width: 1, color: 0x0a2a55, alpha: 0.9 });
      }
    }
    // Hit-rect layer (separate so dots and squares don't fight for hover hits).
    const hitLayer = new Container();
    for (const tile of axisEvent.data.tiles) {
      const hit = new Graphics();
      hit.rect(tile.x * TILE_PX, tile.y * TILE_PX, TILE_PX, TILE_PX)
        .fill({ color: 0xffffff, alpha: 0.001 });
      hit.eventMode = "static";
      const tags =
        (tile.vert_count >= 2 || tile.horiz_count >= 2 ? " [same-axis]" : "") +
        (tile.vert_count >= 1 && tile.horiz_count >= 1 ? " [perpendicular]" : "");
      const label = `Axis (${tile.x},${tile.y}): V=${tile.vert_count} H=${tile.horiz_count}${tags}`;
      hit.on("pointerenter", () => onHover(label));
      hit.on("pointerleave", () => onHover(null));
      hitLayer.addChild(hit);
    }
    layer.addChild(axisG);
    layer.addChild(dotG);
    layer.addChild(hitLayer);

    axisSummary = ` | axis: ${axisEvent.data.same_axis_conflict_count} same, ${axisEvent.data.perpendicular_crossing_count} perp`;
  }

  // --- Cluster zones ---
  for (const evt of events) {
    if (evt.phase !== "GhostClusterSolved" && evt.phase !== "GhostClusterFailed") continue;
    const isFailed = evt.phase === "GhostClusterFailed";
    const d = isFailed
      ? (evt as GhostClusterFailedEvent).data
      : (evt as GhostClusterSolvedEvent).data;
    const solvedData = isFailed ? null : (evt as GhostClusterSolvedEvent).data;
    const color = isFailed ? 0xff4444 : 0x44aaff;
    const zg = new Graphics();
    zg.rect(d.zone_x * TILE_PX, d.zone_y * TILE_PX, d.zone_w * TILE_PX, d.zone_h * TILE_PX)
      .fill({ color, alpha: isFailed ? 0.18 : 0.1 })
      .stroke({ width: isFailed ? 2 : 1, color, alpha: isFailed ? 0.95 : 0.65 });
    zg.eventMode = "static";
    const solveInfo = solvedData
      ? ` vars=${solvedData.variables} clauses=${solvedData.clauses} ${(solvedData.solve_time_us / 1000).toFixed(1)}ms`
      : "";
    const label = `Cluster #${d.cluster_id}: ${d.zone_w}x${d.zone_h} @ (${d.zone_x},${d.zone_y}) ${d.boundary_count} ports${solveInfo}${isFailed ? " FAILED" : ""}`;
    zg.on("pointerenter", () => onHover(label));
    zg.on("pointerleave", () => onHover(null));
    layer.addChild(zg);

    // Zone dimension label
    const zoneStyle = new TextStyle({ fontSize: 9, fill: isFailed ? "#ff8888" : "#88ccff", fontFamily: "monospace" });
    const zoneText = new Text({ text: `#${d.cluster_id} ${d.zone_w}×${d.zone_h}`, style: zoneStyle });
    zoneText.x = d.zone_x * TILE_PX + 2;
    zoneText.y = d.zone_y * TILE_PX + 2;
    layer.addChild(zoneText);
  }

  // --- Routed paths ---
  for (const evt of routedEvents) {
    const d = evt.data;
    const item = itemFromSpecKey(d.spec_key);
    const color = ghostColorForItem(item, colorMap);

    const pg = new Graphics();
    if (d.tiles && d.tiles.length > 1) {
      pg.setStrokeStyle({ width: 3, color, alpha: 0.8 });
      pg.moveTo(d.tiles[0][0] * TILE_PX + TILE_PX / 2, d.tiles[0][1] * TILE_PX + TILE_PX / 2);
      for (let i = 1; i < d.tiles.length; i++) {
        pg.lineTo(d.tiles[i][0] * TILE_PX + TILE_PX / 2, d.tiles[i][1] * TILE_PX + TILE_PX / 2);
      }
      pg.stroke();
    }
    // Start dot
    if (d.tiles && d.tiles.length > 0) {
      const [sx, sy] = d.tiles[0];
      pg.circle(sx * TILE_PX + TILE_PX / 2, sy * TILE_PX + TILE_PX / 2, TILE_PX * 0.25)
        .fill({ color, alpha: 0.9 });
    }
    pg.eventMode = "static";
    pg.on("pointerenter", () => onHover(`Ghost path: ${d.spec_key} len=${d.path_len} crossings=${d.crossings} turns=${d.turns}`));
    pg.on("pointerleave", () => onHover(null));
    layer.addChild(pg);
  }

  // --- Failed specs ---
  for (const evt of failedEvents) {
    const d = evt.data;
    const fg = new Graphics();
    const cx = d.from_x * TILE_PX + TILE_PX / 2;
    const cy = d.from_y * TILE_PX + TILE_PX / 2;
    const hs = TILE_PX * 0.3;
    fg.moveTo(cx - hs, cy - hs).lineTo(cx + hs, cy + hs).stroke({ width: 2, color: 0xff3333 });
    fg.moveTo(cx + hs, cy - hs).lineTo(cx - hs, cy + hs).stroke({ width: 2, color: 0xff3333 });
    drawDashedLine(fg, cx, cy, d.to_x * TILE_PX + TILE_PX / 2, d.to_y * TILE_PX + TILE_PX / 2,
      6, 4, { width: 1.5, color: 0xff3333, alpha: 0.7 });
    fg.eventMode = "static";
    fg.on("pointerenter", () => onHover(`Ghost failed: ${d.spec_key} (${d.from_x},${d.from_y})\u2192(${d.to_x},${d.to_y})`));
    fg.on("pointerleave", () => onHover(null));
    layer.addChild(fg);
  }

  // --- Item color legend ---
  let legendY = 8;
  for (const [item, color] of colorMap) {
    const hexStr = `#${color.toString(16).padStart(6, "0")}`;
    const swatch = new Graphics();
    swatch.rect(8, legendY, 12, 10).fill({ color });
    layer.addChild(swatch);
    const lbl = new Text({ text: item, style: new TextStyle({ fontSize: 10, fill: hexStr, fontFamily: "monospace" }) });
    lbl.x = 24;
    lbl.y = legendY;
    layer.addChild(lbl);
    legendY += 13;
  }

  // --- Summary label ---
  const ghostComplete = events.find((e): e is GhostRoutingCompleteEvent => e.phase === "GhostRoutingComplete");
  const crossingTileCount = [...tileItems.values()].filter(s => s.size >= 2).length;
  const clusterCount = events.filter(e => e.phase === "GhostClusterSolved" || e.phase === "GhostClusterFailed").length;
  const summaryText = (ghostComplete
    ? `Ghost: ${ghostComplete.data.entity_count} specs, ${crossingTileCount} crossing tiles, ${ghostComplete.data.cluster_count} clusters`
    : `Ghost: ${routedEvents.length} routed, ${failedEvents.length} failed, ${crossingTileCount} crossing tiles, ${clusterCount} clusters`)
    + axisSummary;

  const summaryStyle = new TextStyle({ fontSize: 11, fill: "#ffffff", fontFamily: "monospace", fontWeight: "bold" });
  const summaryLabel = new Text({ text: summaryText, style: summaryStyle });
  summaryLabel.x = 4;
  summaryLabel.y = 4;

  // Background pill for the summary
  const summaryBg = new Graphics();
  summaryBg.rect(0, 0, summaryLabel.width + 12, summaryLabel.height + 6)
    .fill({ color: 0x000000, alpha: 0.65 });
  summaryBg.y = 0;

  // We'll position these relative to top-left of layout area; place at y offset from legend
  summaryBg.x = 4;
  summaryBg.y = legendY + 4;
  summaryLabel.x = 10;
  summaryLabel.y = legendY + 7;

  layer.addChild(summaryBg);
  layer.addChild(summaryLabel);

  // Unused params (kept for API symmetry with renderTraceOverlay)
  void layoutWidth;
  void layoutHeight;

  parent.addChild(layer);
  return layer;
}

// ─── Phase utilities ────────────────────────────────────────────────────────

/** Get phase boundaries from trace events. Returns phase names and the event index where each starts. */
export function getTracePhases(events: TraceEvent[]): { name: string; eventIndex: number }[] {
  const phases: { name: string; eventIndex: number }[] = [];
  for (let i = 0; i < events.length; i++) {
    const evt = events[i];
    if (evt.phase === "PhaseComplete") {
      phases.push({ name: (evt as { phase: "PhaseComplete"; data: { phase: string; entity_count: number } }).data.phase, eventIndex: i });
    }
  }
  return phases;
}

/** Get events up to and including a given phase index. */
export function eventsUpToPhase(events: TraceEvent[], phaseIndex: number): TraceEvent[] {
  const phases = getTracePhases(events);
  if (phaseIndex < 0 || phaseIndex >= phases.length) return events;
  const endIdx = phases[phaseIndex].eventIndex + 1;
  return events.slice(0, endIdx);
}
