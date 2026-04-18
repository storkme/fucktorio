// Junction debugger modal — click a SAT zone, step through the
// junction solver's iterations. Reads cluster records produced by
// junctionTrace.ts.

import type { Viewport } from "pixi-viewport";
import type { BoundarySnapshot } from "../wasm-pkg/fucktorio_wasm.js";
import { TILE_PX } from "../renderer/entities";
import {
  formatFeeder,
  terminalIteration,
  type JunctionCluster,
  type JunctionIteration,
} from "./junctionTrace";
import "./junctionDebugger.css";

export interface JunctionDebuggerControls {
  open(cluster: JunctionCluster): void;
  close(): void;
  isOpen(): boolean;
}

export function createJunctionDebugger(
  container: HTMLElement,
  viewport: Viewport,
): JunctionDebuggerControls {
  // ----- DOM ---------------------------------------------------------------
  const backdrop = document.createElement("div");
  backdrop.className = "jd-backdrop";

  const modal = document.createElement("div");
  modal.className = "jd-modal";

  const titleBar = document.createElement("div");
  titleBar.className = "jd-titlebar";
  const title = document.createElement("span");
  title.className = "jd-title";
  const pill = document.createElement("span");
  pill.className = "jd-status-pill";
  const close = document.createElement("span");
  close.className = "jd-close";
  close.textContent = "\u00d7";
  titleBar.append(title, pill, close);

  const stepper = document.createElement("div");
  stepper.className = "jd-stepper";
  const prevBtn = document.createElement("button");
  prevBtn.className = "jd-step-btn";
  prevBtn.textContent = "\u25c0";
  prevBtn.title = "previous iteration (\u2190)";
  const stepLabel = document.createElement("span");
  stepLabel.className = "jd-step-label";
  const nextBtn = document.createElement("button");
  nextBtn.className = "jd-step-btn";
  nextBtn.textContent = "\u25b6";
  nextBtn.title = "next iteration (\u2192)";
  const terminalBtn = document.createElement("button");
  terminalBtn.className = "jd-step-btn jd-terminal-btn";
  terminalBtn.textContent = "\u21ba terminal";
  terminalBtn.title = "jump to default (terminal) iteration";
  stepper.append(prevBtn, stepLabel, nextBtn, terminalBtn);

  const body = document.createElement("div");
  body.className = "jd-body";
  const minimapWrap = document.createElement("div");
  minimapWrap.className = "jd-minimap-wrap";
  const minimap = document.createElement("div");
  minimap.className = "jd-minimap";
  const legend = buildLegend();
  minimapWrap.append(minimap, legend);
  const detail = document.createElement("div");
  detail.className = "jd-detail";
  body.append(minimapWrap, detail);

  const footer = document.createElement("div");
  footer.className = "jd-footer";
  footer.textContent = "Esc to close · \u2190/\u2192 step · Home/End first/last · click tile to pan";

  modal.append(titleBar, stepper, body, footer);
  container.append(backdrop, modal);

  // ----- Draggable titlebar ------------------------------------------------
  {
    let dragging = false;
    let offsetX = 0;
    let offsetY = 0;
    titleBar.addEventListener("pointerdown", (e) => {
      if ((e.target as HTMLElement) === close) return;
      dragging = true;
      const rect = modal.getBoundingClientRect();
      offsetX = e.clientX - rect.left;
      offsetY = e.clientY - rect.top;
      titleBar.setPointerCapture(e.pointerId);
      e.preventDefault();
    });
    titleBar.addEventListener("pointermove", (e) => {
      if (!dragging) return;
      modal.style.left = `${e.clientX - offsetX}px`;
      modal.style.top = `${e.clientY - offsetY}px`;
      modal.style.transform = "none";
    });
    titleBar.addEventListener("pointerup", () => {
      dragging = false;
    });
  }

  // ----- State -------------------------------------------------------------
  let currentCluster: JunctionCluster | null = null;
  let currentIter = 0;

  function open(cluster: JunctionCluster): void {
    currentCluster = cluster;
    currentIter = cluster.defaultIterIndex;
    backdrop.classList.add("jd-open");
    modal.classList.add("jd-open");
    render();
    panToCurrentBbox();
  }

  function closeModal(): void {
    currentCluster = null;
    backdrop.classList.remove("jd-open");
    modal.classList.remove("jd-open");
  }

  function isOpen(): boolean {
    return currentCluster !== null;
  }

  function setIter(i: number): void {
    if (!currentCluster) return;
    const clamped = Math.max(0, Math.min(currentCluster.iterations.length - 1, i));
    if (clamped === currentIter) return;
    currentIter = clamped;
    render();
    panToCurrentBbox();
  }

  function panToCurrentBbox(): void {
    if (!currentCluster) return;
    const it = currentCluster.iterations[currentIter];
    if (!it) return;
    const cx = (it.bbox.x + it.bbox.w / 2) * TILE_PX;
    const cy = (it.bbox.y + it.bbox.h / 2) * TILE_PX;
    viewport.moveCenter(cx, cy);
  }

  function render(): void {
    if (!currentCluster) return;
    const c = currentCluster;
    const it = c.iterations[currentIter];

    title.textContent = `Junction (${c.seed.x},${c.seed.y})`;
    pill.className = `jd-status-pill jd-${c.outcome.kind.toLowerCase()}`;
    pill.textContent = pillText(c);

    const n = c.iterations.length;
    stepLabel.textContent = `iter ${it ? it.iter : "-"} · ${currentIter + 1} / ${n}`;
    prevBtn.disabled = currentIter <= 0;
    nextBtn.disabled = currentIter >= n - 1;
    terminalBtn.disabled = currentIter === c.defaultIterIndex;

    renderMinimap(minimap, c, it);
    renderDetail(detail, c, it);
  }

  // ----- Wiring ------------------------------------------------------------
  close.addEventListener("click", closeModal);
  backdrop.addEventListener("click", (e) => {
    if (e.target === backdrop) closeModal();
  });
  prevBtn.addEventListener("click", () => setIter(currentIter - 1));
  nextBtn.addEventListener("click", () => setIter(currentIter + 1));
  terminalBtn.addEventListener("click", () => {
    if (currentCluster) setIter(currentCluster.defaultIterIndex);
  });

  // Capture-phase keyboard handler so we win the race against any
  // bubble-phase global shortcuts (e.g. stepThrough's ArrowLeft/Right).
  document.addEventListener(
    "keydown",
    (e) => {
      if (!isOpen()) return;
      const tag = (e.target as HTMLElement | null)?.tagName?.toUpperCase();
      if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;
      if (e.key === "Escape") {
        closeModal();
        e.stopImmediatePropagation();
        e.preventDefault();
      } else if (e.key === "ArrowLeft") {
        setIter(currentIter - 1);
        e.stopImmediatePropagation();
        e.preventDefault();
      } else if (e.key === "ArrowRight") {
        setIter(currentIter + 1);
        e.stopImmediatePropagation();
        e.preventDefault();
      } else if (e.key === "Home") {
        setIter(0);
        e.stopImmediatePropagation();
        e.preventDefault();
      } else if (e.key === "End" && currentCluster) {
        setIter(currentCluster.iterations.length - 1);
        e.stopImmediatePropagation();
        e.preventDefault();
      }
    },
    { capture: true },
  );

  // Click-in-minimap pan is wired inside renderMinimap (per-cell listener).
  void panFromCell; // silence until used

  function panFromCell(tx: number, ty: number): void {
    viewport.moveCenter((tx + 0.5) * TILE_PX, (ty + 0.5) * TILE_PX);
  }

  function attachCellPan(cell: HTMLDivElement, tx: number, ty: number): void {
    cell.addEventListener("click", () => panFromCell(tx, ty));
  }

  // Expose the attach helper to renderMinimap via closure.
  (minimap as unknown as { __attachPan: typeof attachCellPan }).__attachPan = attachCellPan;

  return { open, close: closeModal, isOpen };
}

// -----------------------------------------------------------------------
// Rendering helpers
// -----------------------------------------------------------------------

function pillText(cluster: JunctionCluster): string {
  switch (cluster.outcome.kind) {
    case "Solved":
      return `Solved · ${cluster.outcome.strategy} · ${cluster.outcome.regionTiles} tiles`;
    case "Capped":
      return `Capped · ${cluster.outcome.iters} iter · ${cluster.outcome.reason}`;
    case "Open":
      return "Open";
  }
}

function buildLegend(): HTMLDivElement {
  const l = document.createElement("div");
  l.className = "jd-legend";
  const rows: [string, string][] = [
    ["#2a2a2a", "bbox interior"],
    ["repeating-linear-gradient(45deg,#5a3a3a 0 3px,#2a1a1a 3px 6px)", "forbidden"],
    ["transparent", "in ◀ / out ▶ border"],
  ];
  for (const [bg, label] of rows) {
    const span = document.createElement("span");
    const sw = document.createElement("span");
    sw.className = "jd-legend-swatch";
    sw.style.background = bg;
    if (label.includes("border")) {
      sw.style.background = "#2a2a2a";
      sw.style.borderLeft = "2px solid #4f4";
      sw.style.borderRight = "2px solid #f44";
    }
    span.append(sw, document.createTextNode(label));
    l.appendChild(span);
  }
  return l;
}

function renderMinimap(
  el: HTMLDivElement,
  cluster: JunctionCluster,
  it: JunctionIteration | undefined,
): void {
  el.innerHTML = "";
  if (!it) return;

  const margin = 1; // 1-tile pad on each side
  const gridW = it.bbox.w + margin * 2;
  const gridH = it.bbox.h + margin * 2;
  const origin = { x: it.bbox.x - margin, y: it.bbox.y - margin };

  // Cell size: clamp 12..24 based on the larger dimension
  const maxDim = Math.max(gridW, gridH);
  const cellPx = Math.max(12, Math.min(24, Math.floor(280 / Math.max(maxDim, 1))));

  el.style.setProperty("--cell", `${cellPx}px`);
  el.style.gridTemplateColumns = `repeat(${gridW}, var(--cell))`;

  const tilesIn = new Set(it.tiles.map(([x, y]) => `${x},${y}`));
  const forbiddenIn = new Set(it.forbidden.map(([x, y]) => `${x},${y}`));
  const boundaryByTile = new Map<string, BoundarySnapshot>();
  for (const b of it.boundaries) boundaryByTile.set(`${b.x},${b.y}`, b);

  const attach = (el as unknown as { __attachPan?: (c: HTMLDivElement, tx: number, ty: number) => void })
    .__attachPan;

  for (let gy = 0; gy < gridH; gy++) {
    for (let gx = 0; gx < gridW; gx++) {
      const tx = origin.x + gx;
      const ty = origin.y + gy;
      const key = `${tx},${ty}`;
      const cell = document.createElement("div");
      cell.className = "jd-cell";
      cell.dataset.tx = String(tx);
      cell.dataset.ty = String(ty);

      const inBbox = tilesIn.has(key);
      if (inBbox) {
        cell.classList.add("jd-cell--interior");
      } else {
        cell.classList.add("jd-cell--margin");
      }

      if (forbiddenIn.has(key)) cell.classList.add("jd-cell--forbidden");

      if (tx === cluster.seed.x && ty === cluster.seed.y) {
        cell.classList.add("jd-cell--seed");
      }

      if (it.veto && it.veto.break_tile_x === tx && it.veto.break_tile_y === ty) {
        cell.classList.add("jd-cell--break");
        const mark = document.createElement("span");
        mark.className = "jd-cell__glyph";
        mark.textContent = "!";
        mark.style.color = "#f6f";
        cell.appendChild(mark);
      }

      const b = boundaryByTile.get(key);
      if (b) {
        cell.classList.add(b.is_input ? "jd-cell--boundary-in" : "jd-cell--boundary-out");
        if (b.interior) cell.classList.add("jd-cell--interior-bdry");
        const glyph = document.createElement("span");
        glyph.className = "jd-cell__glyph";
        glyph.textContent = dirGlyph(b.direction);
        cell.appendChild(glyph);
        const label = document.createElement("span");
        label.className = "jd-cell__label";
        label.textContent = itemAbbr(b.item);
        cell.appendChild(label);
        cell.title = `${b.is_input ? "IN" : "OUT"} (${b.x},${b.y}) ${b.direction} ${b.item}${
          b.spec_key ? ` · ${b.spec_key}` : ""
        }${b.interior ? " · interior" : ""}${
          b.external_feeder ? ` · feeder ${formatFeeder(b.external_feeder)}` : ""
        }`;
      } else if (inBbox) {
        cell.title = `(${tx},${ty})`;
      }

      if (attach) attach(cell, tx, ty);
      el.appendChild(cell);
    }
  }
}

function dirGlyph(dir: string): string {
  switch (dir) {
    case "North": return "\u2191";
    case "East": return "\u2192";
    case "South": return "\u2193";
    case "West": return "\u2190";
    default: return "?";
  }
}

function itemAbbr(item: string): string {
  // Use the first alpha chars of each hyphen-segment, max 2 chars
  const parts = item.split("-");
  if (parts.length === 1) return parts[0].slice(0, 2);
  return (parts[0][0] ?? "") + (parts[1][0] ?? "");
}

function renderDetail(
  el: HTMLDivElement,
  cluster: JunctionCluster,
  it: JunctionIteration | undefined,
): void {
  el.innerHTML = "";
  el.appendChild(renderSummary(cluster));
  el.appendChild(renderParticipating(cluster, it));
  if (it) {
    el.appendChild(renderBoundaries(it));
    el.appendChild(renderAttempts(it));
    el.appendChild(renderSat(it));
    if (it.veto) el.appendChild(renderVeto(it));
  }
  if (cluster.nearbyStamped.length > 0) {
    el.appendChild(renderNearby(cluster));
  }
}

function section(title: string, open: boolean = true): { details: HTMLDetailsElement; bodyEl: HTMLDivElement } {
  const details = document.createElement("details");
  if (open) details.open = true;
  const summary = document.createElement("summary");
  summary.textContent = title;
  const bodyEl = document.createElement("div");
  bodyEl.className = "jd-sec-body";
  details.append(summary, bodyEl);
  return { details, bodyEl };
}

function renderSummary(cluster: JunctionCluster): HTMLDetailsElement {
  const { details, bodyEl } = section("Summary");
  const grid = document.createElement("div");
  grid.className = "jd-kv-grid";
  const kv: [string, string][] = [
    ["seed", `(${cluster.seed.x}, ${cluster.seed.y})`],
    ["iterations", String(cluster.iterations.length)],
    ["outcome", cluster.outcome.kind],
  ];
  if (cluster.outcome.kind === "Solved") {
    kv.push(
      ["strategy", cluster.outcome.strategy],
      ["solved at iter", String(cluster.outcome.growthIter)],
      ["region tiles", String(cluster.outcome.regionTiles)],
    );
  } else if (cluster.outcome.kind === "Capped") {
    kv.push(
      ["iters attempted", String(cluster.outcome.iters)],
      ["region tiles", String(cluster.outcome.regionTiles)],
      ["reason", cluster.outcome.reason],
    );
  }
  for (const [k, v] of kv) {
    const ks = document.createElement("span");
    ks.textContent = k;
    const vs = document.createElement("span");
    vs.textContent = v;
    grid.append(ks, vs);
  }
  bodyEl.appendChild(grid);
  return details;
}

function renderParticipating(
  cluster: JunctionCluster,
  it: JunctionIteration | undefined,
): HTMLDetailsElement {
  const { details, bodyEl } = section("Participating specs");
  if (cluster.participating.length === 0) {
    const row = document.createElement("div");
    row.className = "jd-row jd-row--dim";
    row.textContent = "(none reported)";
    bodyEl.appendChild(row);
    return details;
  }
  const aliveThisIter = new Set(it?.participating ?? []);
  for (const p of cluster.participating) {
    const row = document.createElement("div");
    row.className = "jd-row";
    if (it && !aliveThisIter.has(p.key)) row.classList.add("jd-spec-drop");
    row.textContent = `${p.key} · ${p.item} · start=(${p.initial_tile_x},${p.initial_tile_y}) · path_len=${p.path_len} · frontier=[${p.initial_start}..${p.initial_end}]`;
    bodyEl.appendChild(row);
  }
  if (it && it.encountered.length > 0) {
    const eRow = document.createElement("div");
    eRow.className = "jd-row jd-row--dim";
    eRow.textContent = `encountered (non-participating): ${it.encountered.join(", ")}`;
    bodyEl.appendChild(eRow);
  }
  return details;
}

function renderBoundaries(it: JunctionIteration): HTMLDetailsElement {
  const { details, bodyEl } = section("Boundaries");
  if (it.boundaries.length === 0) {
    const row = document.createElement("div");
    row.className = "jd-row jd-row--dim";
    row.textContent = "(none)";
    bodyEl.appendChild(row);
    return details;
  }
  for (const b of it.boundaries) {
    const row = document.createElement("div");
    row.className = "jd-row";
    const tag = b.is_input ? "IN " : "OUT";
    const interior = b.interior ? " (interior)" : "";
    const feeder = b.external_feeder ? ` ← ${formatFeeder(b.external_feeder)}` : "";
    row.style.color = b.is_input ? "#9f9" : "#f99";
    row.textContent = `${tag} (${b.x},${b.y}) ${dirGlyph(b.direction)} ${b.direction} · ${b.item}${interior} · ${b.spec_key}${feeder}`;
    bodyEl.appendChild(row);
  }
  return details;
}

function renderAttempts(it: JunctionIteration): HTMLDetailsElement {
  const { details, bodyEl } = section("Strategy attempts");
  if (it.attempts.length === 0) {
    const row = document.createElement("div");
    row.className = "jd-row jd-row--dim";
    row.textContent = "(no attempts recorded)";
    bodyEl.appendChild(row);
    return details;
  }
  for (const a of it.attempts) {
    const row = document.createElement("div");
    row.className = "jd-row";
    const failed = a.outcome !== "Solved";
    row.classList.add(failed ? "jd-row--fail" : "jd-row--pass");
    const detail = a.detail ? `  ${a.detail}` : "";
    row.textContent = `${a.strategy} → ${a.outcome}${detail}  · ${a.elapsedUs}µs`;
    bodyEl.appendChild(row);
  }
  return details;
}

function renderSat(it: JunctionIteration): HTMLDetailsElement {
  const { details, bodyEl } = section("SAT", Boolean(it.sat));
  if (!it.sat) {
    const row = document.createElement("div");
    row.className = "jd-row jd-row--dim";
    row.textContent = "(SAT not invoked this iteration)";
    bodyEl.appendChild(row);
    return details;
  }
  const s = it.sat;
  const grid = document.createElement("div");
  grid.className = "jd-kv-grid";
  const kv: [string, string][] = [
    ["satisfied", String(s.satisfied)],
    ["zone", `(${s.zone_x},${s.zone_y}) ${s.zone_w}×${s.zone_h}`],
    ["belt tier", s.belt_tier],
    ["max reach", String(s.max_reach)],
    ["vars", String(s.variables)],
    ["clauses", String(s.clauses)],
    ["solve time", `${s.solve_time_us}µs`],
    ["entities placed", String(s.entities_raw)],
    ["forced empty", String(s.forced_empty.length)],
    ["boundaries", String(s.boundaries.length)],
  ];
  for (const [k, v] of kv) {
    const ks = document.createElement("span");
    ks.textContent = k;
    const vs = document.createElement("span");
    vs.textContent = v;
    grid.append(ks, vs);
  }
  bodyEl.appendChild(grid);
  return details;
}

function renderVeto(it: JunctionIteration): HTMLDetailsElement {
  const { details, bodyEl } = section("Walker veto");
  if (!it.veto) return details;
  const v = it.veto;
  const grid = document.createElement("div");
  grid.className = "jd-kv-grid";
  const kv: [string, string][] = [
    ["strategy", v.strategy],
    ["broken segment", v.broken_segment],
    ["break tile", `(${v.break_tile_x},${v.break_tile_y})`],
    ["break count", String(v.break_count)],
  ];
  for (const [k, val] of kv) {
    const ks = document.createElement("span");
    ks.textContent = k;
    const vs = document.createElement("span");
    vs.textContent = val;
    grid.append(ks, vs);
  }
  bodyEl.appendChild(grid);
  return details;
}

function renderNearby(cluster: JunctionCluster): HTMLDetailsElement {
  const { details, bodyEl } = section("Nearby stamped", false);
  for (const n of cluster.nearbyStamped) {
    const row = document.createElement("div");
    row.className = "jd-row";
    const carries = n.carries ? ` carries=${n.carries}` : "";
    const seg = n.segment_id ? ` · seg=${n.segment_id}` : "";
    row.textContent = `(${n.x},${n.y}) ${n.name} ${n.direction}${carries}${seg}${n.feeds_seed_area ? "  ⚠ feeds seed" : ""}`;
    bodyEl.appendChild(row);
  }
  return details;
}

// Provide a typed getter for terminalIteration's return when needed elsewhere.
void terminalIteration;
