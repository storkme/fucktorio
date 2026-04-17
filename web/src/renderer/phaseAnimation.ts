/**
 * Progressive phase-reveal animation.
 *
 * Renders a layout but starts every entity at alpha=0, then fades them in
 * phase-by-phase (rows_placed → lanes_planned → bus_routed → poles_placed →
 * "final" pseudo-phase for output-merger stragglers). Within each phase,
 * entities reveal staggered in NW→SE order (row-by-row top-to-bottom,
 * left-to-right within each row).
 *
 * Driven by `app.ticker`. Cancellable mid-flight via the returned handle.
 * Callers that don't want animation (corpus, parsed blueprints) just use
 * `renderLayout` directly.
 */

import type { Application, Container, Graphics } from "pixi.js";
import type { LayoutResult, PlacedEntity } from "../wasm-pkg/fucktorio_wasm";
import { renderLayout, type HighlightController } from "./entities";

const TOTAL_BUDGET_MS = 2200;
const PHASE_PAUSE_MS = 180;
const FADE_MS = 200;
const DEFAULT_STAGGER_MS = 8;
const PHASE_ORDER = ["rows_placed", "lanes_planned", "bus_routed", "poles_placed"] as const;

type PhaseName = (typeof PHASE_ORDER)[number] | "final";

export interface PhaseAnimationHandle {
  cancel(): void;
  finish(): void;
  isDone(): boolean;
}

interface ScheduledReveal {
  graphics: Graphics[];
  revealStartMs: number;
}

function entityKey(e: PlacedEntity): string {
  return `${e.x ?? 0},${e.y ?? 0},${e.name},${e.recipe ?? ""}`;
}

interface PhaseSnapshotData {
  phase: string;
  entities: PlacedEntity[];
  width: number;
  height: number;
}

function extractPhaseSnapshots(
  layout: LayoutResult,
): Map<string, PhaseSnapshotData> {
  const byPhase = new Map<string, PhaseSnapshotData>();
  const trace = layout.trace;
  if (!Array.isArray(trace)) return byPhase;
  for (const evt of trace) {
    const anyEvt = evt as { phase?: string; data?: PhaseSnapshotData };
    if (anyEvt.phase === "PhaseSnapshot" && anyEvt.data) {
      byPhase.set(anyEvt.data.phase, anyEvt.data);
    }
  }
  return byPhase;
}

/**
 * Bucket entities into ordered phases using snapshot diffs.
 * Each phase's bucket contains entities that were NEW in that snapshot
 * (relative to the previous one). A synthetic "final" bucket catches
 * anything in layout.entities not seen in any snapshot.
 */
function bucketByPhase(
  layout: LayoutResult,
): Array<{ phase: PhaseName; entities: PlacedEntity[] }> {
  const snapshots = extractPhaseSnapshots(layout);
  const buckets: Array<{ phase: PhaseName; entities: PlacedEntity[] }> = [];
  const seen = new Set<string>();

  for (const phase of PHASE_ORDER) {
    const snap = snapshots.get(phase);
    if (!snap) {
      buckets.push({ phase, entities: [] });
      continue;
    }
    const added: PlacedEntity[] = [];
    for (const e of snap.entities) {
      const k = entityKey(e);
      if (!seen.has(k)) {
        seen.add(k);
        added.push(e);
      }
    }
    buckets.push({ phase, entities: added });
  }

  // Synthetic "final" phase for entities that never appeared in a snapshot
  // (e.g. output-merger stragglers emitted after poles_placed).
  const stragglers: PlacedEntity[] = [];
  for (const e of layout.entities) {
    const k = entityKey(e);
    if (!seen.has(k)) {
      seen.add(k);
      stragglers.push(e);
    }
  }
  buckets.push({ phase: "final", entities: stragglers });

  // NW→SE sort within each phase: y ascending (north→south), then x
  // ascending (west→east). Gives a row-by-row top-to-bottom sweep.
  for (const b of buckets) {
    b.entities.sort((a, c) => {
      const ay = a.y ?? 0;
      const cy = c.y ?? 0;
      if (ay !== cy) return ay - cy;
      return (a.x ?? 0) - (c.x ?? 0);
    });
  }

  return buckets;
}

export function renderLayoutPhaseAnimated(
  layout: LayoutResult,
  container: Container,
  onHover: ((entity: PlacedEntity | null) => void) | undefined,
  onSelect: ((entity: PlacedEntity | null) => void) | undefined,
  app: Application,
): { controller: HighlightController; handle: PhaseAnimationHandle } {
  const entityGraphics = new Map<string, Graphics[]>();

  const controller = renderLayout(layout, container, onHover, onSelect, (entity, gfx) => {
    entityGraphics.set(entityKey(entity), gfx);
  });

  // Any container child not owned by an entity is an ambient decoration
  // (currently: underground-belt tunnel stripes, drawn before the main
  // entity loop in renderLayout). Fade those in at the start of the
  // bus_routed phase so they don't float visible over hidden belts.
  const entityOwned = new Set<Graphics>();
  for (const arr of entityGraphics.values()) {
    for (const g of arr) entityOwned.add(g);
  }
  const ambientGraphics: Graphics[] = [];
  for (const child of container.children) {
    const g = child as Graphics;
    if (!entityOwned.has(g)) ambientGraphics.push(g);
  }

  // Start everything invisible.
  for (const g of ambientGraphics) g.alpha = 0;
  for (const arr of entityGraphics.values()) {
    for (const g of arr) g.alpha = 0;
  }

  const buckets = bucketByPhase(layout);
  const nonEmptyCount = buckets.reduce((n, b) => n + (b.entities.length > 0 ? 1 : 0), 0);

  // Trivial case: nothing to animate. Snap everything and bail early.
  if (nonEmptyCount === 0) {
    for (const g of ambientGraphics) g.alpha = 1;
    for (const arr of entityGraphics.values()) for (const g of arr) g.alpha = 1;
    return {
      controller,
      handle: { cancel: () => {}, finish: () => {}, isDone: () => true },
    };
  }

  // Budget per phase. Single shared budget so short phases don't waste time;
  // stagger shrinks adaptively for large buckets so we never exceed the cap.
  const budgetForStaggers = Math.max(0, TOTAL_BUDGET_MS - PHASE_PAUSE_MS * nonEmptyCount);
  const perPhaseBudget = budgetForStaggers / nonEmptyCount;

  const scheduled: ScheduledReveal[] = [];
  const phaseStartByName = new Map<PhaseName, number>();
  let cursorMs = 0;

  for (const bucket of buckets) {
    if (bucket.entities.length === 0) continue;
    phaseStartByName.set(bucket.phase, cursorMs);
    const stagger = Math.min(DEFAULT_STAGGER_MS, perPhaseBudget / bucket.entities.length);
    bucket.entities.forEach((e, i) => {
      const gfx = entityGraphics.get(entityKey(e));
      if (!gfx || gfx.length === 0) return; // entity with no matching graphic — skip silently
      scheduled.push({ graphics: gfx, revealStartMs: cursorMs + i * stagger });
    });
    const lastEntityStart = (bucket.entities.length - 1) * stagger;
    cursorMs += lastEntityStart + FADE_MS + PHASE_PAUSE_MS;
  }

  // Ambient graphics fade in with the bus_routed phase (where belts first
  // appear). Fall back to phase 0 if bus_routed is empty.
  if (ambientGraphics.length > 0) {
    const ambientStart =
      phaseStartByName.get("bus_routed") ??
      phaseStartByName.get("rows_placed") ??
      0;
    for (const g of ambientGraphics) {
      scheduled.push({ graphics: [g], revealStartMs: ambientStart });
    }
  }

  scheduled.sort((a, b) => a.revealStartMs - b.revealStartMs);

  // --- Ticker loop ---
  const startWallMs = performance.now();
  let pointer = 0;
  let cancelled = false;
  let done = scheduled.length === 0;

  const tick = (): void => {
    if (cancelled || done) return;
    const elapsed = performance.now() - startWallMs;

    // Update alpha on every item currently in its fade window.
    for (let i = pointer; i < scheduled.length; i++) {
      const item = scheduled[i];
      if (item.revealStartMs > elapsed) break;
      const t = Math.min(1, (elapsed - item.revealStartMs) / FADE_MS);
      for (const g of item.graphics) g.alpha = t;
    }

    // Advance the pointer past items that have fully faded in.
    while (pointer < scheduled.length) {
      const item = scheduled[pointer];
      if (elapsed - item.revealStartMs < FADE_MS) break;
      for (const g of item.graphics) g.alpha = 1;
      pointer++;
    }

    if (pointer >= scheduled.length) {
      done = true;
      app.ticker.remove(tick);
    }
  };

  app.ticker.add(tick);

  const handle: PhaseAnimationHandle = {
    cancel() {
      if (cancelled || done) return;
      cancelled = true;
      app.ticker.remove(tick);
    },
    finish() {
      if (cancelled || done) return;
      for (const item of scheduled) {
        for (const g of item.graphics) g.alpha = 1;
      }
      done = true;
      app.ticker.remove(tick);
    },
    isDone() {
      return done || cancelled;
    },
  };

  return { controller, handle };
}
