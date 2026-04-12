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
