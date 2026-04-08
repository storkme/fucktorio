import { Container, Graphics, Text, TextStyle } from "pixi.js";
import { TILE_PX } from "./entities";
import type { TraceEvent } from "../engine";

export type { TraceEvent } from "../engine";

// Convenience narrowing aliases for event variants used in this file.
type RowsPlaced   = Extract<TraceEvent, { phase: "RowsPlaced" }>;
type LanesPlanned = Extract<TraceEvent, { phase: "LanesPlanned" }>;
export type PhaseSnapshot = Extract<TraceEvent, { phase: "PhaseSnapshot" }>;
export type PhaseComplete = Extract<TraceEvent, { phase: "PhaseComplete" }>;

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

  // --- Crossing zones (from CrossingZoneSolved) ---
  for (const evt of events) {
    if (evt.phase !== "CrossingZoneSolved") continue;
    const d = evt.data;
    const g = new Graphics();
    g.rect(d.x * TILE_PX, d.y * TILE_PX, d.width * TILE_PX, d.height * TILE_PX)
      .fill({ color: 0x44aaff, alpha: 0.08 })
      .stroke({ width: 1, color: 0x44aaff, alpha: 0.5 });
    g.eventMode = "static";
    g.on("pointerenter", () => onHover(`SAT zone: ${d.width}×${d.height} solved in ${d.solve_time_us}µs`));
    g.on("pointerleave", () => onHover(null));
    layer.addChild(g);
  }

  // --- Skipped crossing zones (from CrossingZoneSkipped) ---
  for (const evt of events) {
    if (evt.phase !== "CrossingZoneSkipped") continue;
    const d = evt.data;
    const g = new Graphics();
    g.rect(d.tap_x * TILE_PX, (d.tap_y - 1) * TILE_PX, TILE_PX * 2, TILE_PX * 3)
      .fill({ color: 0xff8844, alpha: 0.08 })
      .stroke({ width: 1, color: 0xff8844, alpha: 0.5 });
    g.eventMode = "static";
    g.on("pointerenter", () => onHover(`Skipped: ${d.tap_item} @ (${d.tap_x},${d.tap_y}) — ${d.reason}`));
    g.on("pointerleave", () => onHover(null));
    layer.addChild(g);
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

  // --- Phase summary label (top-left, above layout) ---
  const phaseNames = events.map(e => e.phase);
  const uniquePhases = [...new Set(phaseNames)];
  const summaryStyle = new TextStyle({ fontSize: 10, fill: "#aaa", fontFamily: "monospace" });
  const summaryText = new Text({ text: `Trace: ${uniquePhases.join(" → ")}`, style: summaryStyle });
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
