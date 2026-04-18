/**
 * Live streaming renderer.
 *
 * Consumes `TraceEvent`s as they arrive from the WASM worker during a layout
 * run. Currently wired up for `PhaseSnapshot` only: entities fade in
 * progressively as each pipeline phase completes (rows_placed → lanes_planned
 * → bus_routed → poles_placed), with a staggered NW→SE sweep within each
 * phase. Mirrors the phaseAnimation feel, but fed by the live stream instead
 * of post-hoc diffs.
 *
 * Per-event overlay effects (SAT probe pulses, ghost-path flashes, zone
 * highlights) were explored and deferred — creating one Pixi `Graphics` per
 * event saturates the render tree on big layouts. A follow-up can reintroduce
 * them using a single shared `Graphics` redrawn per frame.
 */

import type { Application, Container, Graphics } from "pixi.js";
import { Container as PixiContainer } from "pixi.js";
import type { LayoutResult, PlacedEntity, TraceEvent } from "../wasm-pkg/fucktorio_wasm";
import { renderLayout, type HighlightController } from "./entities";

const FADE_IN_MS = 180;
const ENTITY_STAGGER_DEFAULT_MS = 6;
const ENTITY_STAGGER_BUDGET_MS = 700;

function entityKey(e: PlacedEntity): string {
  return `${e.x ?? 0},${e.y ?? 0},${e.name},${e.recipe ?? ""}`;
}

interface EntityFade {
  graphics: Graphics[];
  revealStartMs: number;
}

export interface StreamingRendererHandle {
  onEvent(evt: TraceEvent): void;
  /** True if any PhaseSnapshot has been processed. Lets the caller decide
   *  whether to skip the Option-A phaseAnimation when the final layout lands. */
  hasCommittedEntities(): boolean;
  /** Stop all animations, unregister ticker. Graphics are left as-is. */
  cancel(): void;
  /** Snap every active fade to completion, then stop. */
  finish(): void;
  /** Access to the HighlightController from the most recent snapshot render. */
  getHighlightController(): HighlightController | null;
}

export function createStreamingRenderer(
  container: Container,
  app: Application,
  onHover?: (e: PlacedEntity | null) => void,
  onSelect?: (e: PlacedEntity | null) => void,
): StreamingRendererHandle {
  const entityLayer = new PixiContainer();
  container.addChild(entityLayer);

  const seenEntityKeys = new Set<string>();
  let entityFades: EntityFade[] = [];
  let entityPointer = 0;
  let latestController: HighlightController | null = null;
  let anySnapshot = false;
  let cancelled = false;

  const tick = (): void => {
    if (cancelled) return;
    const now = performance.now();

    // Entity fade-ins
    for (let i = entityPointer; i < entityFades.length; i++) {
      const f = entityFades[i];
      if (f.revealStartMs > now) break;
      const t = Math.min(1, (now - f.revealStartMs) / FADE_IN_MS);
      for (const g of f.graphics) g.alpha = t;
    }
    while (entityPointer < entityFades.length) {
      const f = entityFades[entityPointer];
      if (now - f.revealStartMs < FADE_IN_MS) break;
      for (const g of f.graphics) g.alpha = 1;
      entityPointer++;
    }
  };

  app.ticker.add(tick);

  function renderSnapshot(snapshot: {
    phase: string;
    entities: PlacedEntity[];
    width: number;
    height: number;
  }): void {
    // Finalise any in-progress fades before the re-render destroys their
    // Graphics. Keeps the visible state stable across the snapshot swap.
    for (const f of entityFades) for (const g of f.graphics) g.alpha = 1;
    entityFades = [];
    entityPointer = 0;

    const newEntityGraphics = new Map<string, Graphics[]>();
    const layoutForRender: LayoutResult = {
      entities: snapshot.entities,
      width: snapshot.width,
      height: snapshot.height,
    };
    latestController = renderLayout(
      layoutForRender,
      entityLayer,
      onHover,
      onSelect,
      (entity, gfx) => {
        newEntityGraphics.set(entityKey(entity), gfx);
      },
    );

    // Split into already-seen vs newly-introduced entities in this snapshot.
    const newEntities: PlacedEntity[] = [];
    for (const e of snapshot.entities) {
      const k = entityKey(e);
      const gfx = newEntityGraphics.get(k);
      if (!gfx) continue;
      if (seenEntityKeys.has(k)) {
        for (const g of gfx) g.alpha = 1;
      } else {
        for (const g of gfx) g.alpha = 0;
        newEntities.push(e);
      }
    }

    // NW→SE sweep: y ascending, then x.
    newEntities.sort((a, b) => {
      const dy = (a.y ?? 0) - (b.y ?? 0);
      if (dy !== 0) return dy;
      return (a.x ?? 0) - (b.x ?? 0);
    });

    const stagger =
      newEntities.length > 0
        ? Math.min(ENTITY_STAGGER_DEFAULT_MS, ENTITY_STAGGER_BUDGET_MS / newEntities.length)
        : ENTITY_STAGGER_DEFAULT_MS;
    const t0 = performance.now();
    newEntities.forEach((e, i) => {
      const gfx = newEntityGraphics.get(entityKey(e));
      if (!gfx) return;
      entityFades.push({ graphics: gfx, revealStartMs: t0 + i * stagger });
      seenEntityKeys.add(entityKey(e));
    });

    anySnapshot = true;
  }

  function onEvent(evt: TraceEvent): void {
    if (cancelled) return;
    if (evt.phase === "PhaseSnapshot") {
      renderSnapshot(evt.data);
    }
    // Other variants are ignored for v1 — see module header for rationale.
  }

  return {
    onEvent,
    hasCommittedEntities: () => anySnapshot,
    getHighlightController: () => latestController,
    cancel(): void {
      if (cancelled) return;
      cancelled = true;
      app.ticker.remove(tick);
    },
    finish(): void {
      if (cancelled) return;
      for (const f of entityFades) for (const g of f.graphics) g.alpha = 1;
      entityFades = [];
      cancelled = true;
      app.ticker.remove(tick);
    },
  };
}
