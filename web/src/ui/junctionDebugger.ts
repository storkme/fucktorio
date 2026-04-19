// Junction debugger UI — two pieces, staged by how much context the
// user wants:
//
//   1. `jd-inline`  — small control anchored next to the selected SAT
//                     zone. Title, status pill, stepper, one-line
//                     summary, and a "details" button. Always shown
//                     while a zone is selected.
//
//   2. `jd-modal`   — large detail panel (Summary / Participating /
//                     Boundaries / SAT stats / Veto / Nearby). Opens
//                     only when the user hits the details button; has
//                     its own backdrop so clicks outside close the
//                     modal but keep the inline + selection.
//
// Consumer integration (main.ts):
//   - `onChange(state)` fires on open/iter-change/close so the PIXI
//     SAT overlay can update and the entity layer can be dimmed.
//
// Keyboard (active while inline OR modal is open):
//   Esc     — close modal if open, else deselect
//   ←/→     — step through every (iter, variant) slot linearly
//   w/s     — jump prev/next iter primary (skips variants)
//   a/d     — cycle variants within the current iter
//   Home/End— first/last slot
//   i       — toggle the details modal

import type { Viewport } from "pixi-viewport";
import type {
  BoundarySnapshot,
  TraceEvent,
} from "../wasm-pkg/fucktorio_wasm.js";
import { TILE_PX } from "../renderer/entities";
import {
  formatFeeder,
  ghostPathsNearBbox,
  terminalIteration,
  type JunctionCluster,
  type JunctionIteration,
} from "./junctionTrace";
import "./junctionDebugger.css";

export interface JunctionSelectionState {
  cluster: JunctionCluster;
  iter: JunctionIteration;
  trace: readonly TraceEvent[] | null;
}

export interface JunctionDebuggerOptions {
  onChange: (state: JunctionSelectionState | null) => void;
}

export interface JunctionDebuggerControls {
  open(cluster: JunctionCluster, trace?: readonly TraceEvent[]): void;
  close(): void;
  isOpen(): boolean;
}

export function createJunctionDebugger(
  container: HTMLElement,
  viewport: Viewport,
  options: JunctionDebuggerOptions,
): JunctionDebuggerControls {
  // ----- Inline control (always visible while a zone is selected) ----------
  const inline = document.createElement("div");
  inline.className = "jd-inline";

  const inlineHead = document.createElement("div");
  inlineHead.className = "jd-inline-head";
  const title = document.createElement("span");
  title.className = "jd-title";
  const pill = document.createElement("span");
  pill.className = "jd-status-pill";
  const detailsBtn = document.createElement("button");
  detailsBtn.className = "jd-inline-btn jd-inline-details-btn";
  detailsBtn.textContent = "\u2139"; // information source glyph
  detailsBtn.title = "Show details (i)";
  const copyBtn = document.createElement("button");
  copyBtn.className = "jd-inline-btn jd-inline-copy-btn";
  copyBtn.textContent = "\u29c9"; // two joined squares — "copy" glyph
  copyBtn.title = "Copy debug dump to clipboard";
  const fixtureBtn = document.createElement("button");
  fixtureBtn.className = "jd-inline-btn jd-inline-fixture-btn";
  fixtureBtn.textContent = "\u26ab"; // black circle — "fixture" glyph (distinct from ⎘)
  fixtureBtn.title = "Copy as SAT-fixture JSON";
  const closeInline = document.createElement("span");
  closeInline.className = "jd-close";
  closeInline.textContent = "\u00d7";
  closeInline.title = "Deselect (Esc)";
  inlineHead.append(title, pill, copyBtn, fixtureBtn, detailsBtn, closeInline);

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
  terminalBtn.textContent = "\u21ba";
  terminalBtn.title = "jump to default (terminal) iteration";
  stepper.append(prevBtn, stepLabel, nextBtn, terminalBtn);

  const summaryLine = document.createElement("div");
  summaryLine.className = "jd-inline-summary";

  inline.append(inlineHead, stepper, summaryLine);
  container.append(inline);

  // ----- Modal (opt-in details) --------------------------------------------
  const modalBackdrop = document.createElement("div");
  modalBackdrop.className = "jd-modal-backdrop";

  const modal = document.createElement("div");
  modal.className = "jd-modal";

  const modalHead = document.createElement("div");
  modalHead.className = "jd-titlebar";
  const modalTitle = document.createElement("span");
  modalTitle.className = "jd-title";
  modalTitle.textContent = "Junction details";
  const modalPill = document.createElement("span");
  modalPill.className = "jd-status-pill";
  const closeModal = document.createElement("span");
  closeModal.className = "jd-close";
  closeModal.textContent = "\u00d7";
  closeModal.title = "Close details (Esc)";
  modalHead.append(modalTitle, modalPill, closeModal);

  const detail = document.createElement("div");
  detail.className = "jd-detail";

  const modalFooter = document.createElement("div");
  modalFooter.className = "jd-footer";
  modalFooter.textContent =
    "Esc close · \u2190/\u2192 step all · w/s iter · a/d variant · Home/End first/last";

  modal.append(modalHead, detail, modalFooter);
  container.append(modalBackdrop, modal);

  // ----- State -------------------------------------------------------------
  let currentCluster: JunctionCluster | null = null;
  let currentIter = 0;
  let currentTrace: readonly TraceEvent[] | null = null;
  let modalVisible = false;

  function open(cluster: JunctionCluster, trace?: readonly TraceEvent[]): void {
    currentCluster = cluster;
    currentTrace = trace ?? null;
    currentIter = cluster.defaultIterIndex;
    inline.classList.add("jd-open");
    render();
    updateInlinePosition();
    // Don't call panToCurrentBbox here — the user already picked where
    // they wanted to look by clicking the zone. Only pan if the bbox
    // is actually off-screen (see panIfOffscreen below).
    panIfOffscreen();
  }

  function closeAll(): void {
    if (!currentCluster) return;
    hideModal();
    currentCluster = null;
    currentTrace = null;
    inline.classList.remove("jd-open");
    options.onChange(null);
  }

  function showModal(): void {
    if (!currentCluster || modalVisible) return;
    modalVisible = true;
    modalBackdrop.classList.add("jd-open");
    modal.classList.add("jd-open");
    renderModal();
  }

  function hideModal(): void {
    if (!modalVisible) return;
    modalVisible = false;
    modalBackdrop.classList.remove("jd-open");
    modal.classList.remove("jd-open");
  }

  function toggleModal(): void {
    if (modalVisible) hideModal();
    else showModal();
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
    updateInlinePosition();
    panIfOffscreen();
  }

  /**
   * Only re-centre the viewport if the current bbox is entirely
   * outside the visible area. Lets the user keep their framing when
   * clicking a zone they're already looking at, while still bringing
   * off-screen zones into view (e.g. when stepping to a larger iter
   * whose growth pushed the bbox out of frame).
   */
  function panIfOffscreen(): void {
    if (!currentCluster) return;
    const it = currentCluster.iterations[currentIter];
    if (!it) return;
    const leftScreen = viewport.toScreen(it.bbox.x * TILE_PX, it.bbox.y * TILE_PX);
    const rightScreen = viewport.toScreen(
      (it.bbox.x + it.bbox.w) * TILE_PX,
      (it.bbox.y + it.bbox.h) * TILE_PX,
    );
    const rect = container.getBoundingClientRect();
    const offscreen =
      rightScreen.x < 0 ||
      leftScreen.x > rect.width ||
      rightScreen.y < 0 ||
      leftScreen.y > rect.height;
    if (!offscreen) return;
    const cx = (it.bbox.x + it.bbox.w / 2) * TILE_PX;
    const cy = (it.bbox.y + it.bbox.h / 2) * TILE_PX;
    viewport.moveCenter(cx, cy);
  }

  // ----- w/s iter · a/d variant navigation --------------------------------

  function iterGroups(): Map<number, number[]> {
    const groups = new Map<number, number[]>();
    const c = currentCluster;
    if (!c) return groups;
    for (let i = 0; i < c.iterations.length; i++) {
      const it = c.iterations[i];
      const list = groups.get(it.iter) ?? [];
      list.push(i);
      groups.set(it.iter, list);
    }
    return groups;
  }

  function jumpIter(delta: number): void {
    if (!currentCluster) return;
    const groups = iterGroups();
    const iters = Array.from(groups.keys()).sort((a, b) => a - b);
    const cur = currentCluster.iterations[currentIter].iter;
    const idx = iters.indexOf(cur);
    const next = iters[Math.max(0, Math.min(iters.length - 1, idx + delta))];
    const indices = groups.get(next) ?? [];
    const primaryIdx = indices.find(
      (i) => currentCluster!.iterations[i].variant === "",
    );
    setIter(primaryIdx ?? indices[0] ?? currentIter);
  }

  function cycleVariant(delta: number): void {
    if (!currentCluster) return;
    const groups = iterGroups();
    const cur = currentCluster.iterations[currentIter].iter;
    const indices = groups.get(cur) ?? [];
    if (indices.length <= 1) return;
    const pos = indices.indexOf(currentIter);
    const nextPos = (pos + delta + indices.length) % indices.length;
    setIter(indices[nextPos]);
  }

  // ----- Inline positioning (anchored to world-space bbox) ----------------

  /**
   * Anchor the inline panel just below the bbox, right-aligned with
   * the bbox's right edge. That is:
   *   - panel.right = bbox.right
   *   - panel.top   = bbox.bottom
   * If the panel would overflow the container bottom, flip above the
   * bbox (panel.bottom = bbox.top). If it would overflow the right
   * edge (because the bbox is near the right wall and the panel is
   * wider than the bbox), shift it leftward to stay visible.
   */
  function updateInlinePosition(): void {
    if (!currentCluster) return;
    const it = currentCluster.iterations[currentIter];
    if (!it) return;
    const rect = container.getBoundingClientRect();
    const w = inline.offsetWidth || 200;
    const h = inline.offsetHeight || 70;

    const bboxRight = (it.bbox.x + it.bbox.w) * TILE_PX;
    const bboxBottom = (it.bbox.y + it.bbox.h) * TILE_PX;
    const bboxTop = it.bbox.y * TILE_PX;
    const brScreen = viewport.toScreen(bboxRight, bboxBottom);
    const trScreen = viewport.toScreen(bboxRight, bboxTop);

    // Preferred: below the bbox, right-aligned with its right edge.
    let left = brScreen.x - w;
    let top = brScreen.y;

    // Flip above if we'd overflow the bottom.
    if (top + h > rect.height - 4) {
      top = trScreen.y - h;
    }
    // Clamp horizontally so the panel never escapes the container.
    left = Math.max(4, Math.min(left, rect.width - w - 4));
    top = Math.max(4, Math.min(top, rect.height - h - 4));

    inline.style.left = `${left}px`;
    inline.style.top = `${top}px`;
  }

  viewport.on("moved", updateInlinePosition);
  viewport.on("zoomed", updateInlinePosition);
  window.addEventListener("resize", updateInlinePosition);

  // ----- Render -----------------------------------------------------------

  function render(): void {
    if (!currentCluster) return;
    const c = currentCluster;
    const it = c.iterations[currentIter];

    title.textContent = `Junction (${c.seed.x},${c.seed.y})`;
    pill.className = `jd-status-pill jd-${c.outcome.kind.toLowerCase()}`;
    pill.textContent = outcomePill(c);

    const n = c.iterations.length;
    const variantSuffix = it && it.variant ? ` · ${it.variant}` : "";
    stepLabel.textContent = `iter ${it ? it.iter : "-"}${variantSuffix} · ${currentIter + 1}/${n}`;
    prevBtn.disabled = currentIter <= 0;
    nextBtn.disabled = currentIter >= n - 1;
    terminalBtn.disabled = currentIter === c.defaultIterIndex;

    summaryLine.innerHTML = "";
    for (const line of inlineSummaryLines(c, it)) {
      const row = document.createElement("div");
      row.className = `jd-inline-summary-row jd-inline-summary-row--${line.tone}`;
      row.textContent = line.text;
      summaryLine.appendChild(row);
    }

    if (modalVisible) renderModal();

    if (it) {
      options.onChange({ cluster: c, iter: it, trace: currentTrace });
    }
  }

  function renderModal(): void {
    if (!currentCluster) return;
    const c = currentCluster;
    const it = c.iterations[currentIter];
    modalPill.className = `jd-status-pill jd-${c.outcome.kind.toLowerCase()}`;
    modalPill.textContent = outcomePill(c);
    modalTitle.textContent = `Junction (${c.seed.x},${c.seed.y})`;
    renderDetail(detail, c, it);
  }

  // ----- Wiring ------------------------------------------------------------
  closeInline.addEventListener("click", closeAll);
  detailsBtn.addEventListener("click", toggleModal);
  copyBtn.addEventListener("click", copyDebugDump);
  fixtureBtn.addEventListener("click", () => {
    if (!currentCluster) return;
    const json = buildFixtureJson(
      currentCluster,
      currentCluster.iterations[currentIter],
      currentTrace,
    );
    flashAndCopy(fixtureBtn, json);
  });
  closeModal.addEventListener("click", hideModal);
  modalBackdrop.addEventListener("click", hideModal);

  function copyDebugDump(): void {
    if (!currentCluster) return;
    const dump = buildDebugDump(currentCluster, currentIter);
    const json = JSON.stringify(
      dump,
      (_k, v) => (typeof v === "bigint" ? String(v) : v),
      2,
    );
    // Try the modern clipboard API; fall back to a textarea for older
    // contexts (insecure http, etc.).
    const done = (ok: boolean) => {
      const original = copyBtn.textContent;
      copyBtn.textContent = ok ? "\u2713" : "!";
      copyBtn.classList.add("jd-inline-btn--flash");
      window.setTimeout(() => {
        copyBtn.textContent = original;
        copyBtn.classList.remove("jd-inline-btn--flash");
      }, 900);
    };
    if (navigator.clipboard?.writeText) {
      navigator.clipboard.writeText(json).then(
        () => done(true),
        () => done(false),
      );
    } else {
      const ta = document.createElement("textarea");
      ta.value = json;
      ta.style.position = "fixed";
      ta.style.opacity = "0";
      document.body.appendChild(ta);
      ta.select();
      try {
        document.execCommand("copy");
        done(true);
      } catch {
        done(false);
      }
      document.body.removeChild(ta);
    }
  }

  prevBtn.addEventListener("click", () => setIter(currentIter - 1));
  nextBtn.addEventListener("click", () => setIter(currentIter + 1));
  terminalBtn.addEventListener("click", () => {
    if (currentCluster) setIter(currentCluster.defaultIterIndex);
  });

  document.addEventListener(
    "keydown",
    (e) => {
      if (!isOpen()) return;
      const tag = (e.target as HTMLElement | null)?.tagName?.toUpperCase();
      if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;
      const key = e.key;
      const consume = () => {
        e.stopImmediatePropagation();
        e.preventDefault();
      };
      if (key === "Escape") {
        // Staged close: modal first, then the whole debugger.
        if (modalVisible) hideModal();
        else closeAll();
        consume();
      } else if (key === "ArrowLeft") {
        setIter(currentIter - 1);
        consume();
      } else if (key === "ArrowRight") {
        setIter(currentIter + 1);
        consume();
      } else if (key === "Home") {
        setIter(0);
        consume();
      } else if (key === "End" && currentCluster) {
        setIter(currentCluster.iterations.length - 1);
        consume();
      } else if (key === "w" || key === "W") {
        jumpIter(-1);
        consume();
      } else if (key === "s" || key === "S") {
        jumpIter(+1);
        consume();
      } else if (key === "a" || key === "A") {
        cycleVariant(-1);
        consume();
      } else if (key === "d" || key === "D") {
        cycleVariant(+1);
        consume();
      } else if (key === "i" || key === "I") {
        toggleModal();
        consume();
      }
    },
    { capture: true },
  );

  return { open, close: closeAll, isOpen };
}

// -----------------------------------------------------------------------
// Summary helpers
// -----------------------------------------------------------------------

/**
 * Compact debug dump for sharing a specific cluster+iter with the
 * investigator. The focus is **boundaries** — that's the primary
 * signal for understanding why a solve succeeded or failed. Everything
 * else (full tile lists, proposed entities, SAT clauses, nearby
 * stamped summary) is trimmed away.
 *
 * Per-iter we keep: bbox, variant, stripped boundaries, attempts,
 * veto, and a tiny SAT summary (satisfied / vars / clauses / time).
 * Per-boundary we keep: x, y, direction, item, is_input, interior,
 * plus a one-line feeder stringification when present.
 */
function buildDebugDump(cluster: JunctionCluster, currentIter: number): unknown {
  const stripFeeder = (f: { entity_name: string; entity_x: number; entity_y: number; direction: string } | undefined) =>
    f ? `${f.entity_name}@(${f.entity_x},${f.entity_y}) ${f.direction}` : undefined;
  const stripBoundary = (b: {
    x: number;
    y: number;
    direction: string;
    item: string;
    is_input: boolean;
    interior: boolean;
    spec_key: string;
    external_feeder?: { entity_name: string; entity_x: number; entity_y: number; direction: string };
  }): Record<string, unknown> => {
    const out: Record<string, unknown> = {
      x: b.x,
      y: b.y,
      dir: b.direction,
      item: b.item,
      in: b.is_input,
    };
    if (b.interior) out.interior = true;
    if (b.spec_key) out.spec = b.spec_key;
    const f = stripFeeder(b.external_feeder);
    if (f) out.feeder = f;
    return out;
  };
  const iterations = cluster.iterations.map((it, idx) => {
    const bs = it.sat?.boundaries ?? it.boundaries;
    const row: Record<string, unknown> = {
      idx,
      iter: it.iter,
      bbox: it.bbox,
      boundaries: bs.map(stripBoundary),
      attempts: it.attempts.map((a) => ({
        strategy: a.strategy,
        outcome: a.outcome,
        ...(a.detail ? { detail: a.detail } : {}),
      })),
    };
    if (it.variant) row.variant = it.variant;
    if (it.veto) row.veto = it.veto;
    if (it.sat) {
      row.sat = {
        satisfied: it.sat.satisfied,
        vars: it.sat.variables,
        clauses: it.sat.clauses,
        solveUs: it.sat.solve_time_us,
      };
    }
    return row;
  });
  return {
    url: window.location.href,
    ts: new Date().toISOString(),
    currentIterIndex: currentIter,
    seed: cluster.seed,
    outcome: cluster.outcome,
    participating: cluster.participating.map((p) => ({
      key: p.key,
      item: p.item,
      start: [p.initial_tile_x, p.initial_tile_y],
    })),
    iterations,
  };
}

function outcomePill(cluster: JunctionCluster): string {
  switch (cluster.outcome.kind) {
    case "Solved":
      return `Solved · ${cluster.outcome.regionTiles}t`;
    case "Capped":
      return `Capped · ${cluster.outcome.iters} iter`;
    case "Open":
      return "Open";
  }
}

interface SummaryLine {
  text: string;
  tone: "ok" | "warn" | "fail" | "dim";
}

/**
 * Single most-informative line about this particular iter. Prefers
 * the per-iter "why didn't it work" signal (veto / last failed
 * attempt) over the cluster-level outcome so stepping actually
 * surfaces new info. Falls back to the cluster outcome when the iter
 * has nothing specific to say.
 */
function inlineSummaryLines(
  cluster: JunctionCluster,
  it: JunctionIteration | undefined,
): SummaryLine[] {
  if (it) {
    if (it.veto) {
      return [{
        text: `veto · ${truncate(it.veto.broken_segment, 22)} @ (${it.veto.break_tile_x},${it.veto.break_tile_y})`,
        tone: "warn",
      }];
    }
    const lastWin = it.attempts.find((a) => a.outcome === "Solved");
    if (lastWin) {
      return [{ text: `${lastWin.strategy} ok · ${lastWin.elapsedUs}µs`, tone: "ok" }];
    }
    const lastFail = [...it.attempts].reverse().find((a) => a.outcome !== "Solved");
    if (lastFail) {
      const detail = lastFail.detail ? ` · ${truncate(lastFail.detail, 28)}` : "";
      return [{
        text: `${lastFail.strategy} → ${truncate(lastFail.outcome, 12)}${detail}`,
        tone: "fail",
      }];
    }
  }
  switch (cluster.outcome.kind) {
    case "Solved":
      return [{ text: `solved @ iter ${cluster.outcome.growthIter}`, tone: "ok" }];
    case "Capped":
      return [{ text: `cap: ${truncate(cluster.outcome.reason, 32)}`, tone: "fail" }];
    case "Open":
      return [{ text: "open — never terminated", tone: "warn" }];
  }
}

function truncate(s: string, n: number): string {
  return s.length <= n ? s : `${s.slice(0, n - 1)}\u2026`;
}

// -----------------------------------------------------------------------
// Detail-panel rendering (only drawn when the modal is open)
// -----------------------------------------------------------------------

function effectiveBoundaries(it: JunctionIteration): BoundarySnapshot[] {
  return it.sat?.boundaries ?? it.boundaries;
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

function section(
  title: string,
  open: boolean = true,
): { details: HTMLDetailsElement; bodyEl: HTMLDivElement } {
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
  const boundaries = effectiveBoundaries(it);
  const titleSuffix = it.sat ? " (as fed to SAT)" : " (spec perimeter)";
  const { details, bodyEl } = section(`Boundaries${titleSuffix}`);
  if (boundaries.length === 0) {
    const row = document.createElement("div");
    row.className = "jd-row jd-row--dim";
    row.textContent = "(none)";
    bodyEl.appendChild(row);
    return details;
  }
  for (const b of boundaries) {
    const row = document.createElement("div");
    row.className = "jd-row";
    const tag = b.is_input ? "IN " : "OUT";
    const interior = b.interior ? " (interior)" : "";
    const feeder = b.external_feeder ? ` ← ${formatFeeder(b.external_feeder)}` : "";
    row.style.color = b.is_input ? "#9f9" : "#f99";
    const specTag = b.spec_key ? ` · ${b.spec_key}` : "";
    row.textContent = `${tag} (${b.x},${b.y}) ${dirGlyph(b.direction)} ${b.direction} · ${b.item}${interior}${specTag}${feeder}`;
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

void terminalIteration;

// -----------------------------------------------------------------------
// Copy-as-fixture helpers
// -----------------------------------------------------------------------

/** UG max reach by belt tier — yellow 4, red 6, blue 8. */
const BELT_UG_REACH: Record<string, number> = {
  "transport-belt": 4,
  "fast-transport-belt": 6,
  "express-transport-belt": 8,
};

/**
 * Build a fixture JSON string for the given cluster + iteration. The
 * caller is responsible for copying it to the clipboard via `flashAndCopy`.
 */
function buildFixtureJson(
  cluster: JunctionCluster,
  it: JunctionIteration | undefined,
  trace: readonly TraceEvent[] | null,
): string {
  const seed = cluster.seed;
  const bbox = it?.bbox ?? { x: seed.x, y: seed.y, w: 1, h: 1 };
  const iterIndex = it?.iter ?? 0;

  // Derive belt tier and max_reach from SAT invocation data when
  // available; fall back to sensible defaults.
  const beltTier: string = it?.sat?.belt_tier ?? "transport-belt";
  const maxReach: number = it?.sat?.max_reach ?? (BELT_UG_REACH[beltTier] ?? 4);

  // Map effective boundaries to the fixture schema.
  const boundaries = effectiveBoundaries(it ?? {
    iter: 0,
    variant: "",
    bbox,
    tiles: [],
    forbidden: [],
    boundaries: [],
    participating: [],
    encountered: [],
    attempts: [],
    sat: null,
    veto: null,
  }).map((b) => ({
    x: b.x,
    y: b.y,
    dir: b.direction as string,
    item: b.item,
    in: b.is_input,
    ...(b.interior ? { interior: true } : {}),
  }));

  // Ghost paths near the bbox (informational context).
  const ghostPaths = (it && trace)
    ? ghostPathsNearBbox(trace, bbox, 2).map((p) => ({
        item: p.item,
        spec_key: p.specKey,
        tiles: p.tiles,
      }))
    : [];

  const fixture: Record<string, unknown> = {
    version: 1,
    name: `fixture_${seed.x}_${seed.y}_iter${iterIndex}`,
    notes: "",
    source_url: window.location.href,
    seed: [seed.x, seed.y],
    bbox: { x: bbox.x, y: bbox.y, w: bbox.w, h: bbox.h },
    forbidden: it?.forbidden ?? [],
    belt_tier: beltTier,
    max_reach: maxReach,
    boundaries,
    expected: { mode: "solve" },
    ...(ghostPaths.length > 0 ? { context: { ghost_paths: ghostPaths } } : {}),
  };

  return JSON.stringify(fixture, null, 2);
}

/**
 * Write `text` to the clipboard and briefly flash `btn` green to signal
 * success. Falls back to `prompt()` when the Clipboard API is unavailable
 * (non-HTTPS, browser policy, etc.).
 */
function flashAndCopy(btn: HTMLButtonElement, text: string): void {
  const original = btn.textContent ?? "";
  const succeed = () => {
    btn.textContent = "✓";
    btn.style.color = "#9f9";
    setTimeout(() => {
      btn.textContent = original;
      btn.style.color = "";
    }, 1200);
  };
  const fallback = () => {
    prompt("Copy fixture JSON (Ctrl+A, Ctrl+C):", text);
  };
  if (navigator.clipboard?.writeText) {
    navigator.clipboard.writeText(text).then(succeed, fallback);
  } else {
    fallback();
  }
}
