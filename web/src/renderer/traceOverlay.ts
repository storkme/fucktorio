import { Container, Graphics, Text, TextStyle } from "pixi.js";
import { TILE_PX } from "./entities";
import type { PlacedEntity } from "../engine";

// --- Trace event types (mirrors Rust TraceEvent) ---
// These must match the #[serde(tag = "phase", content = "data")] format.

export interface RowsPlaced {
  phase: "RowsPlaced";
  data: { rows: RowInfo[] };
}
export interface RowSplit {
  phase: "RowSplit";
  data: { recipe: string; original_count: number; split_into: number; reason: string };
}
export interface LanesPlanned {
  phase: "LanesPlanned";
  data: { lanes: LaneInfo[]; families: FamilyInfo[]; bus_width: number };
}
export interface LaneSplit {
  phase: "LaneSplit";
  data: { item: string; rate: number; max_lane_cap: number; n_splits: number };
}
export interface LaneOrderOptimized {
  phase: "LaneOrderOptimized";
  data: { ordering: string[]; crossing_score: number };
}
export interface CrossingZoneSolved {
  phase: "CrossingZoneSolved";
  data: { x: number; y: number; width: number; height: number; solve_time_us: number };
}
export interface CrossingZoneSkipped {
  phase: "CrossingZoneSkipped";
  data: { tap_item: string; tap_x: number; tap_y: number; reason: string };
}
export interface BalancerStamped {
  phase: "BalancerStamped";
  data: { item: string; shape: [number, number]; y_start: number; y_end: number; template_found: boolean };
}
export interface LaneRouted {
  phase: "LaneRouted";
  data: { item: string; x: number; is_fluid: boolean; trunk_segments: number; tapoffs: number };
}
export interface TapoffRouted {
  phase: "TapoffRouted";
  data: { item: string; from_x: number; from_y: number; to_x: number; to_y: number; path_len: number };
}
export interface OutputMerged {
  phase: "OutputMerged";
  data: { item: string; rows: number[]; merge_y: number };
}
export interface MergerBlockPlaced {
  phase: "MergerBlockPlaced";
  data: { item: string; lanes: number; block_y: number; block_height: number };
}
export interface PolesPlaced {
  phase: "PolesPlaced";
  data: { count: number; strategy: string };
}
export interface PhaseComplete {
  phase: "PhaseComplete";
  data: { phase: string; entity_count: number };
}
export interface PhaseSnapshot {
  phase: "PhaseSnapshot";
  data: { phase: string; entities: PlacedEntity[]; width: number; height: number };
}

export type TraceEvent =
  | RowsPlaced | RowSplit | LanesPlanned | LaneSplit | LaneOrderOptimized
  | CrossingZoneSolved | CrossingZoneSkipped | BalancerStamped
  | LaneRouted | TapoffRouted | OutputMerged | MergerBlockPlaced | PolesPlaced
  | PhaseComplete | PhaseSnapshot;

interface RowInfo { index: number; recipe: string; machine: string; machine_count: number; y_start: number; y_end: number; row_kind: string; }
interface LaneInfo { item: string; x: number; rate: number; is_fluid: boolean; source_y: number; tap_off_ys: number[]; consumer_rows: number[]; producer_row: number | null; family_id: number | null; }
interface FamilyInfo { item: string; shape: [number, number]; lane_xs: number[]; balancer_y_start: number; balancer_y_end: number; total_rate: number; producer_rows: number[]; }

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
