import type { TraceEvent } from "../engine";

// ---------------------------------------------------------------------------
// Style injection (same pattern as sidebar.ts)
// ---------------------------------------------------------------------------
const STYLE_ID = "fucktorio-debug-panel-style";
const STYLE = `
.debug-panel { margin-top: 8px; font-family: monospace; }
.debug-panel details { margin-bottom: 6px; }
.debug-panel summary { cursor: pointer; color: #9cdcfe; font-size: 12px; padding: 3px 0; user-select: none; }
.debug-panel summary:hover { color: #b0dfff; }
.debug-panel .stat-row { display: flex; justify-content: space-between; padding: 2px 0; font-size: 11px; color: #ccc; }
.debug-panel .stat-label { color: #aaa; }
.debug-panel .stat-value { color: #e0e0e0; }
.debug-panel .timeline-bar { display: flex; height: 16px; border-radius: 3px; overflow: hidden; margin: 4px 0; }
.debug-panel .timeline-seg { min-width: 2px; }
.debug-panel .timeline-total { font-size: 11px; color: #aaa; margin-bottom: 4px; }
.debug-panel .machine-line { font-size: 11px; color: #ccc; padding: 1px 0 1px 12px; }
.debug-panel .lane-chip { display: inline-block; padding: 1px 4px; border-radius: 3px; background: #333; font-size: 10px; margin: 1px; color: #e0e0e0; }
.debug-panel .failure-item { color: #ff6666; font-size: 11px; padding: 1px 0 1px 12px; }
.debug-panel .skip-item { font-size: 11px; color: #aaa; padding: 1px 0 1px 12px; }
.debug-panel .cons-table { width: 100%; font-size: 11px; border-collapse: collapse; margin-top: 4px; }
.debug-panel .cons-table th { text-align: left; color: #9cdcfe; font-weight: normal; padding: 2px 4px; }
.debug-panel .cons-table td { padding: 2px 4px; color: #ccc; }
.debug-panel .amber { color: #ffaa44; }
.debug-panel .issue-line { font-size: 11px; padding: 1px 0 1px 12px; }
.debug-panel .issue-error { color: #f44; }
.debug-panel .issue-warn { color: #ffaa00; }
.debug-panel .ok { color: #6a6; }
`;

function injectStyle(): void {
  if (!document.getElementById(STYLE_ID)) {
    const el = document.createElement("style");
    el.id = STYLE_ID;
    el.textContent = STYLE;
    document.head.appendChild(el);
  }
}

// ---------------------------------------------------------------------------
// Event helpers
// ---------------------------------------------------------------------------

type EventOf<T extends string> = Extract<TraceEvent, { phase: T }>;

function findEvent<T extends string>(events: TraceEvent[], phase: T): EventOf<T> | undefined {
  return events.find(e => e.phase === phase) as EventOf<T> | undefined;
}

function filterEvents<T extends string>(events: TraceEvent[], phase: T): EventOf<T>[] {
  return events.filter(e => e.phase === phase) as EventOf<T>[];
}

// ---------------------------------------------------------------------------
// Section builders
// ---------------------------------------------------------------------------

function el(tag: string, cls?: string): HTMLElement {
  const e = document.createElement(tag) as HTMLElement;
  if (cls) e.className = cls;
  return e;
}

function div(cls?: string): HTMLDivElement {
  const e = document.createElement("div");
  if (cls) e.className = cls;
  return e;
}

function statRow(label: string, value: string): HTMLElement {
  const row = el("div", "stat-row");
  const lbl = el("span", "stat-label");
  lbl.textContent = label;
  const val = el("span", "stat-value");
  val.textContent = value;
  row.appendChild(lbl);
  row.appendChild(val);
  return row;
}

function section(title: string, open = false): { details: HTMLDetailsElement; body: HTMLDivElement } {
  const details = document.createElement("details");
  if (open) details.open = true;
  const summary = document.createElement("summary");
  summary.textContent = title;
  details.appendChild(summary);
  const body = div();
  details.appendChild(body);
  return { details, body };
}

// ---------------------------------------------------------------------------
// Phase color map for timeline
// ---------------------------------------------------------------------------
const PHASE_COLORS: Record<string, string> = {
  rows: "#4a9",
  lanes: "#49c",
  route: "#c84",
  validate: "#888",
  poles: "#cc4",
  solve: "#96a",
  merge: "#a86",
};

function phaseColor(name: string): string {
  for (const [key, color] of Object.entries(PHASE_COLORS)) {
    if (name.includes(key)) return color;
  }
  return "#69c";
}

// ---------------------------------------------------------------------------
// Section: Performance Timeline
// ---------------------------------------------------------------------------

function renderTimeline(body: HTMLDivElement, events: TraceEvent[]): void {
  const times = filterEvents(events, "PhaseTime");
  if (times.length === 0) {
    body.textContent = "No timing data";
    return;
  }

  const total = times.reduce((s, t) => s + (t.data as { duration_ms: number }).duration_ms, 0);
  const bar = el("div", "timeline-bar");

  for (const t of times) {
    const d = t.data as { phase: string; duration_ms: number };
    const pct = total > 0 ? (d.duration_ms / total) * 100 : 0;
    const seg = el("div", "timeline-seg");
    seg.style.width = `${pct}%`;
    seg.style.background = phaseColor(d.phase);
    seg.title = `${d.phase}: ${d.duration_ms}ms (${pct.toFixed(1)}%)`;
    bar.appendChild(seg);
  }
  body.appendChild(bar);

  const totalLine = el("div", "timeline-total");
  totalLine.textContent = `Total: ${total}ms`;
  body.appendChild(totalLine);
}

// ---------------------------------------------------------------------------
// Section: Solver Summary
// ---------------------------------------------------------------------------

function renderSolver(body: HTMLDivElement, events: TraceEvent[]): void {
  const evt = findEvent(events, "SolverCompleted");
  if (!evt) { body.textContent = "No solver data"; return; }
  const d = evt.data as {
    recipe_count: number; machine_count: number;
    external_input_count: number; external_output_count: number;
    machines: { recipe: string; machine: string; count: number; rate: number }[];
  };

  body.appendChild(statRow("Recipes", String(d.recipe_count)));
  body.appendChild(statRow("Machines", String(d.machine_count)));
  body.appendChild(statRow("External inputs", String(d.external_input_count)));
  body.appendChild(statRow("External outputs", String(d.external_output_count)));

  if (d.machines?.length) {
    for (const m of d.machines) {
      const line = el("div", "machine-line");
      line.textContent = `${m.count}\u00D7 \u2192 ${m.recipe} @ ${m.rate.toFixed(1)}/s`;
      body.appendChild(line);
    }
  }
}

// ---------------------------------------------------------------------------
// Section: Layout Stats
// ---------------------------------------------------------------------------

function renderLayoutStats(body: HTMLDivElement, events: TraceEvent[]): void {
  const lanes = findEvent(events, "LanesPlanned");
  const rows = findEvent(events, "RowsPlaced");
  const splits = filterEvents(events, "LaneSplit");

  if (!lanes && !rows) { body.textContent = "No layout data"; return; }

  if (lanes) {
    const d = lanes.data as { lanes: { is_fluid: boolean }[]; families: unknown[]; bus_width: number };
    const solids = d.lanes.filter(l => !l.is_fluid).length;
    const fluids = d.lanes.filter(l => l.is_fluid).length;
    body.appendChild(statRow("Bus width", `${d.bus_width} tiles`));
    body.appendChild(statRow("Lanes", `${solids} solid \u00B7 ${fluids} fluid`));
    body.appendChild(statRow("Balancer families", String(d.families.length)));
  }

  if (rows) {
    const d = rows.data as { rows: unknown[] };
    body.appendChild(statRow("Rows", String(d.rows.length)));
  }

  if (splits.length > 0) {
    for (const s of splits) {
      const d = s.data as { item: string; rate: number; n_splits: number };
      const line = el("div", "machine-line");
      line.textContent = `${d.item} ${d.rate.toFixed(1)}/s \u2192 ${d.n_splits} lanes`;
      body.appendChild(line);
    }
  }
}

// ---------------------------------------------------------------------------
// Section: Lane Ordering
// ---------------------------------------------------------------------------

function renderLaneOrder(body: HTMLDivElement, events: TraceEvent[]): void {
  const evt = findEvent(events, "LaneOrderOptimized");
  if (!evt) { body.textContent = "No lane order data"; return; }
  const d = evt.data as { ordering: string[]; crossing_score: number };

  body.appendChild(statRow("Crossing score", `${d.crossing_score} (lower = fewer UG hops)`));

  const chips = el("div");
  for (const item of d.ordering) {
    const chip = el("span", "lane-chip");
    chip.textContent = item;
    chips.appendChild(chip);
  }
  body.appendChild(chips);
}

// ---------------------------------------------------------------------------
// Section: A* Routing
// ---------------------------------------------------------------------------

function renderRouting(body: HTMLDivElement, events: TraceEvent[]): void {
  const neg = findEvent(events, "NegotiateComplete");
  const failures = filterEvents(events, "RouteFailure");

  if (neg) {
    const d = neg.data as { specs: number; iterations: number; duration_ms: number };
    body.appendChild(statRow("Specs", String(d.specs)));
    body.appendChild(statRow("Iterations", String(d.iterations)));
    body.appendChild(statRow("Duration", `${d.duration_ms}ms`));
  }

  if (failures.length === 0) {
    const ok = el("div", "ok");
    ok.textContent = "\u2713 all routes resolved";
    body.appendChild(ok);
  } else {
    const hdr = el("div", "failure-item");
    hdr.textContent = `\u26A0 ${failures.length} route failure${failures.length > 1 ? "s" : ""}:`;
    body.appendChild(hdr);
    for (const f of failures) {
      const d = f.data as { item: string; from_x: number; from_y: number; to_x: number; to_y: number; spec_key: string };
      const line = el("div", "failure-item");
      line.textContent = `  ${d.item}: (${d.from_x},${d.from_y})\u2192(${d.to_x},${d.to_y}) [${d.spec_key}]`;
      body.appendChild(line);
    }
  }
}

// ---------------------------------------------------------------------------
// Section: SAT Zones
// ---------------------------------------------------------------------------

function renderSatZones(body: HTMLDivElement, events: TraceEvent[]): void {
  const solved = filterEvents(events, "CrossingZoneSolved");
  const skipped = filterEvents(events, "CrossingZoneSkipped");
  const conflicts = filterEvents(events, "CrossingZoneConflict");

  if (solved.length === 0 && skipped.length === 0 && conflicts.length === 0) {
    body.textContent = "No SAT zone data";
    return;
  }

  const totalUs = solved.reduce((s, z) => s + (z.data as { solve_time_us: number }).solve_time_us, 0);

  body.appendChild(statRow("Zones solved", String(solved.length)));
  body.appendChild(statRow("Zones skipped", String(skipped.length)));
  body.appendChild(statRow("Conflicts", String(conflicts.length)));
  body.appendChild(statRow("Total solve time", `${totalUs.toLocaleString()}\u00B5s`));

  for (const s of skipped) {
    const d = s.data as { tap_item: string; tap_x: number; tap_y: number; reason: string };
    const line = el("div", "skip-item");
    line.textContent = `${d.tap_item} @ (${d.tap_x},${d.tap_y}): ${d.reason}`;
    body.appendChild(line);
  }
}

// ---------------------------------------------------------------------------
// Section: Lane Consolidation
// ---------------------------------------------------------------------------

function renderConsolidation(body: HTMLDivElement, events: TraceEvent[]): void {
  const conss = filterEvents(events, "LaneConsolidated");
  if (conss.length === 0) return;

  const table = el("table", "cons-table");
  const thead = el("tr");
  for (const h of ["Item", "Consumers", "Lanes", "Rate/lane"]) {
    const th = el("th");
    th.textContent = h;
    thead.appendChild(th);
  }
  table.appendChild(thead);

  for (const c of conss) {
    const d = c.data as { item: string; consumer_count: number; n_trunk_lanes: number; rate_per_lane: number };
    const tr = el("tr");
    const tdItem = el("td"); tdItem.textContent = d.item;
    const tdCons = el("td"); tdCons.textContent = String(d.consumer_count);
    const tdLanes = el("td"); tdLanes.textContent = String(d.n_trunk_lanes);
    const tdRate = el("td");
    const nearCapacity = d.rate_per_lane >= 13.5;
    tdRate.textContent = `${d.rate_per_lane.toFixed(1)}/s`;
    if (nearCapacity) tdRate.className = "amber";
    tr.appendChild(tdItem);
    tr.appendChild(tdCons);
    tr.appendChild(tdLanes);
    tr.appendChild(tdRate);
    table.appendChild(tr);
  }
  body.appendChild(table);
}

// ---------------------------------------------------------------------------
// Section: Power
// ---------------------------------------------------------------------------

function renderPower(body: HTMLDivElement, events: TraceEvent[]): void {
  const evt = findEvent(events, "PolesPlaced");
  if (!evt) { body.textContent = "No power data"; return; }
  const d = evt.data as { count: number; strategy: string };
  body.appendChild(statRow("Poles", String(d.count)));
  body.appendChild(statRow("Strategy", d.strategy));
}

// ---------------------------------------------------------------------------
// Section: Validation Summary
// ---------------------------------------------------------------------------

function renderValidation(body: HTMLDivElement, events: TraceEvent[]): void {
  const evt = findEvent(events, "ValidationCompleted");
  if (!evt) { body.textContent = "No validation data"; return; }
  const d = evt.data as { error_count: number; warning_count: number; issues: { severity: string; category: string; message: string }[] };

  body.appendChild(statRow("Errors", String(d.error_count)));
  body.appendChild(statRow("Warnings", String(d.warning_count)));

  for (const issue of d.issues) {
    const line = el("div", "issue-line");
    const cls = issue.severity === "Error" ? "issue-error" : "issue-warn";
    line.classList.add(cls);
    line.textContent = `[${issue.severity.toLowerCase()}] ${issue.category}: ${issue.message}`;
    body.appendChild(line);
  }
}

// ---------------------------------------------------------------------------
// Main export
// ---------------------------------------------------------------------------

export function renderDebugPanel(events: TraceEvent[]): HTMLElement {
  injectStyle();
  const root = div("debug-panel");

  // Performance Timeline
  const timeline = section("Performance", true);
  renderTimeline(timeline.body, events);
  root.appendChild(timeline.details);

  // Solver Summary
  const solver = section("Solver");
  renderSolver(solver.body, events);
  root.appendChild(solver.details);

  // Layout Stats
  const layoutStats = section("Layout Stats");
  renderLayoutStats(layoutStats.body, events);
  root.appendChild(layoutStats.details);

  // Lane Ordering
  const laneOrder = section("Lane Ordering");
  renderLaneOrder(laneOrder.body, events);
  root.appendChild(laneOrder.details);

  // A* Routing
  const routing = section("A* Routing", true);
  renderRouting(routing.body, events);
  root.appendChild(routing.details);

  // SAT Zones
  const satZones = section("SAT Zones");
  renderSatZones(satZones.body, events);
  root.appendChild(satZones.details);

  // Lane Consolidation (only if events exist)
  const consolidation = section("Lane Consolidation");
  renderConsolidation(consolidation.body, events);
  if (consolidation.body.hasChildNodes()) {
    root.appendChild(consolidation.details);
  }

  // Power
  const power = section("Power");
  renderPower(power.body, events);
  root.appendChild(power.details);

  // Validation Summary
  const validation = section("Validation");
  renderValidation(validation.body, events);
  root.appendChild(validation.details);

  return root;
}
