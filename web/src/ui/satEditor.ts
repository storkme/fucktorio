// In-canvas SAT-zone editor (Phase F). Activated by clicking the
// pencil button in the inline junction debugger panel. Implements:
//
//   - Drag-to-paint belt runs with auto-UG when crossing forbidden
//     tiles. Tier 1 (synchronous, sub-ms) structural validation runs
//     on every commit.
//   - Tier 2 SAT-with-pins (debounced ~300ms) via engine.solveFixture,
//     surfacing a 🟢/🟡/🔴 indicator and rendering SAT's chosen
//     completion as a ghost-style overlay.
//   - "Accept ghost" promotes the solver's added entities into the
//     painted layer. Ctrl+Z restores prior state (single-level undo
//     in v1; persistence is in-memory).
//   - Export Fixture: extends `buildFixtureJson` with the painted
//     entities + computed `expected.max_cost`.
//
// Wired in `main.ts` via `JunctionDebuggerOptions.onEditRequested`.

import type { Container as PixiContainer } from "pixi.js";
import { Container } from "pixi.js";
import type { Viewport } from "pixi-viewport";
import type { Engine, PlacedEntity, EntityDirection } from "../engine";
import { renderLayout, TILE_PX } from "../renderer/entities";
import {
  BELT_UG_REACH,
  buildFixtureJson,
  type JunctionDebuggerControls,
  type JunctionSelectionState,
} from "./junctionDebugger";
import "./satEditor.css";

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

export interface SatEditorOptions {
  viewport: Viewport;
  /** Canvas element — pointer events are listened on it directly so
   * pixi-viewport's drag plugin doesn't swallow them first. */
  canvas: HTMLCanvasElement;
  engine: Engine;
  /** Junction debugger we attach the toolbar + status dot to. */
  jd: JunctionDebuggerControls;
  /** SAT zone overlay layer — kept on top of painted/ghost/preview so
   * the boundary chevrons + item icons stay visible above belts. */
  satZoneOverlayLayer: PixiContainer;
}

export interface SatEditorControls {
  /** Enter edit mode for the given selection. No-op when already active. */
  enter(state: JunctionSelectionState): void;
  /** Exit edit mode (button, Esc, or external trigger). */
  exit(): void;
  isActive(): boolean;
}

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

type Tool = "belt" | "ug-in" | "ug-out" | "erase";
type Status = "valid" | "solving" | "invalid" | "idle";

/** Painted entities always have integer coords + a direction — narrow
 * the optional fields on PlacedEntity to required inside the editor. */
type PaintedEntity = PlacedEntity & {
  x: number;
  y: number;
  direction: EntityDirection;
};

function asPainted(e: PlacedEntity): PaintedEntity | null {
  if (e.x === undefined || e.y === undefined || e.direction === undefined) return null;
  return e as PaintedEntity;
}

interface Bbox {
  x: number;
  y: number;
  w: number;
  h: number;
}

interface DragState {
  startX: number;
  startY: number;
  bendVerticalFirst: boolean;
}

interface BoundaryEntry {
  x: number;
  y: number;
  item: string;
  isInput: boolean;
}

interface ZoneCtx {
  bbox: Bbox;
  forbidden: Set<string>; // "x,y" keys
  beltTier: string;
  maxReach: number;
  items: string[]; // boundary items (for [/] cycling)
  boundaries: BoundaryEntry[];
  fixtureJson: string; // pre-built without painted/max_cost
  selection: JunctionSelectionState;
}

const DIR_DELTA: Record<EntityDirection, [number, number]> = {
  North: [0, -1],
  East: [1, 0],
  South: [0, 1],
  West: [-1, 0],
};

const ROTATE_CW: Record<EntityDirection, EntityDirection> = {
  North: "East",
  East: "South",
  South: "West",
  West: "North",
};

const BELT_NAMES: Record<string, string> = {
  "transport-belt": "transport-belt",
  "fast-transport-belt": "fast-transport-belt",
  "express-transport-belt": "express-transport-belt",
};

const UG_NAMES: Record<string, string> = {
  "transport-belt": "underground-belt",
  "fast-transport-belt": "fast-underground-belt",
  "express-transport-belt": "express-underground-belt",
};

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

export function createSatEditor(opts: SatEditorOptions): SatEditorControls {
  const { viewport, canvas, engine, jd, satZoneOverlayLayer } = opts;

  // ----- PIXI layers (added on enter, removed on exit) ---------------------
  // All three layers are inserted *below* the SAT zone overlay so the
  // boundary chevrons + item icons stay visible on top of painted
  // belts.
  let paintedLayer: PixiContainer | null = null;
  let ghostLayer: PixiContainer | null = null;
  let previewLayer: PixiContainer | null = null;

  // ----- DOM (mounted in inline panel on enter) ----------------------------
  let toolbarEl: HTMLDivElement | null = null;
  let statusDot: HTMLSpanElement | null = null;

  // ----- State -------------------------------------------------------------
  let active = false;
  let zone: ZoneCtx | null = null;
  let painted: PaintedEntity[] = [];
  let undoStack: PaintedEntity[][] = [];
  let redoStack: PaintedEntity[][] = [];
  let ghost: PaintedEntity[] = []; // last SAT completion delta
  let tool: Tool = "belt";
  let brushDir: EntityDirection = "East";
  let itemIndex = 0;
  let drag: DragState | null = null;
  let status: Status = "idle";
  let solverEpoch = 0;
  let solveTimer: number | null = null;
  let lastSatCost: number | null = null;

  // ----- Helpers -----------------------------------------------------------

  function key(x: number, y: number): string {
    return `${x},${y}`;
  }

  function inBbox(x: number, y: number): boolean {
    if (!zone) return false;
    const b = zone.bbox;
    return x >= b.x && x < b.x + b.w && y >= b.y && y < b.y + b.h;
  }

  function paintedAt(x: number, y: number): PlacedEntity | undefined {
    return painted.find((e) => e.x === x && e.y === y);
  }

  function currentItem(): string | null {
    if (!zone || zone.items.length === 0) return null;
    const idx = Math.max(0, Math.min(itemIndex, zone.items.length - 1));
    return zone.items[idx] ?? null;
  }

  function pushUndo(): void {
    undoStack.push(painted.map((e) => ({ ...e })));
    if (undoStack.length > 64) undoStack.shift();
    redoStack.length = 0;
  }

  function setPainted(next: PaintedEntity[], snapshot: boolean): void {
    if (snapshot) pushUndo();
    painted = next;
    rerender();
    scheduleValidate();
  }

  // ----- PIXI render -------------------------------------------------------

  function rerender(): void {
    if (paintedLayer) {
      renderLayout({ entities: painted, width: 0, height: 0 }, paintedLayer);
    }
    if (ghostLayer) {
      renderLayout({ entities: ghost, width: 0, height: 0 }, ghostLayer);
    }
  }

  function rerenderPreview(entities: PaintedEntity[], invalid: boolean): void {
    if (!previewLayer) return;
    renderLayout({ entities, width: 0, height: 0 }, previewLayer);
    previewLayer.alpha = invalid ? 0.5 : 0.45;
    previewLayer.tint = invalid ? 0xff5555 : 0xffffff;
  }

  function clearPreview(): void {
    if (!previewLayer) return;
    previewLayer.removeChildren();
    previewLayer.tint = 0xffffff;
  }

  // ----- Validation --------------------------------------------------------

  function setStatus(s: Status, reason = ""): void {
    status = s;
    if (!statusDot) return;
    statusDot.classList.remove("ok", "solving", "invalid", "idle");
    // Map status → CSS class.
    statusDot.classList.add(s === "valid" ? "ok" : s);
    const symbol =
      s === "valid" ? "\u25CF" : s === "solving" ? "\u25D4" : s === "invalid" ? "\u25CF" : "\u25CB";
    statusDot.textContent = symbol;
    let label = "";
    if (s === "valid") {
      label = lastSatCost !== null
        ? `valid · cost ${lastSatCost} / yours ${userCost(painted)}`
        : "valid";
    } else if (s === "solving") {
      label = "solving…";
    } else if (s === "invalid") {
      label = "invalid";
    } else {
      label = "no edits yet";
    }
    statusDot.title = reason ? `${label}\n${reason}` : label;
    updateButtonsForStatus();
  }

  function userCost(ents: PaintedEntity[]): number {
    let c = 0;
    for (const e of ents) {
      if (BELT_NAMES[e.name] === e.name) c += 1;
      else if (UG_NAMES[zone?.beltTier ?? "transport-belt"] === e.name) c += 5;
    }
    return c;
  }

  /** Cheap structural checks. Returns null on success or a reason string. */
  function tier1(): string | null {
    if (!zone) return "no zone";
    const seenTiles = new Set<string>();
    for (const e of painted) {
      const k = key(e.x, e.y);
      if (seenTiles.has(k)) return `duplicate entity at (${e.x},${e.y})`;
      seenTiles.add(k);
      if (!inBbox(e.x, e.y)) return `entity at (${e.x},${e.y}) outside bbox`;
      if (zone.forbidden.has(k)) return `entity at (${e.x},${e.y}) on forbidden tile`;
    }

    // UG pair check: every UG-in must have a matching UG-out within
    // max_reach in the facing direction with no other UG of the same
    // item between.
    for (const e of painted) {
      if (e.io_type !== "input") continue;
      const [dx, dy] = DIR_DELTA[e.direction];
      let found = false;
      for (let i = 1; i <= zone.maxReach + 1; i++) {
        const tx = e.x + dx * i;
        const ty = e.y + dy * i;
        const at = paintedAt(tx, ty);
        if (!at) continue;
        if (
          at.io_type === "output" &&
          at.direction === e.direction &&
          at.carries === e.carries
        ) {
          found = true;
          break;
        }
        // Any other UG of the same item in between blocks the pair.
        if (at.io_type === "input" && at.carries === e.carries) {
          return `UG-in at (${e.x},${e.y}) blocked by another UG-in at (${tx},${ty})`;
        }
      }
      if (!found) {
        return `UG-in at (${e.x},${e.y}) has no matching UG-out within reach ${zone.maxReach}`;
      }
    }
    return null;
  }

  // ----- Debounced SAT-with-pins ------------------------------------------

  function scheduleValidate(): void {
    if (!active || !zone) return;
    const reason = tier1();
    if (reason) {
      ghost = [];
      lastSatCost = null;
      rerender();
      setStatus("invalid", reason);
      return;
    }
    setStatus("solving");
    if (solveTimer !== null) {
      window.clearTimeout(solveTimer);
    }
    const epoch = ++solverEpoch;
    solveTimer = window.setTimeout(() => {
      runSat(epoch);
    }, 300);
  }

  async function runSat(epoch: number): Promise<void> {
    if (!zone) return;
    try {
      const result = await engine.solveFixture(zone.fixtureJson, painted);
      if (epoch !== solverEpoch || !active) return; // superseded
      if (!result) {
        ghost = [];
        lastSatCost = null;
        rerender();
        setStatus("invalid", "SAT cannot complete this layout");
        return;
      }
      // Compute ghost = solver entities \ painted.
      const paintedKeys = new Set(painted.map((e) => key(e.x, e.y)));
      const narrowed: PaintedEntity[] = [];
      for (const e of result.entities) {
        const p = asPainted(e);
        if (p && !paintedKeys.has(key(p.x, p.y))) narrowed.push(p);
      }
      ghost = narrowed;
      lastSatCost = result.cost;
      rerender();
      setStatus("valid");
    } catch (err) {
      if (epoch !== solverEpoch) return;
      ghost = [];
      rerender();
      setStatus("invalid", `solver error: ${err instanceof Error ? err.message : String(err)}`);
    }
  }

  // ----- Drag-to-paint -----------------------------------------------------

  function tileFromEvent(ev: PointerEvent): { x: number; y: number } | null {
    const rect = canvas.getBoundingClientRect();
    const world = viewport.toWorld(ev.clientX - rect.left, ev.clientY - rect.top);
    return { x: Math.floor(world.x / TILE_PX), y: Math.floor(world.y / TILE_PX) };
  }

  function lShape(
    a: { x: number; y: number },
    b: { x: number; y: number },
    verticalFirst: boolean,
  ): { x: number; y: number }[] {
    const path: { x: number; y: number }[] = [];
    const stepX = b.x === a.x ? 0 : b.x > a.x ? 1 : -1;
    const stepY = b.y === a.y ? 0 : b.y > a.y ? 1 : -1;
    if (verticalFirst) {
      for (let y = a.y; y !== b.y + stepY && stepY !== 0; y += stepY) path.push({ x: a.x, y });
      if (stepY === 0) path.push({ x: a.x, y: a.y });
      for (let x = a.x + stepX; stepX !== 0 && x !== b.x + stepX; x += stepX) {
        path.push({ x, y: b.y });
      }
    } else {
      for (let x = a.x; x !== b.x + stepX && stepX !== 0; x += stepX) path.push({ x, y: a.y });
      if (stepX === 0) path.push({ x: a.x, y: a.y });
      for (let y = a.y + stepY; stepY !== 0 && y !== b.y + stepY; y += stepY) {
        path.push({ x: b.x, y });
      }
    }
    // Dedupe consecutive duplicates (e.g. when stepX==0).
    const out: { x: number; y: number }[] = [];
    for (const p of path) {
      const last = out[out.length - 1];
      if (!last || last.x !== p.x || last.y !== p.y) out.push(p);
    }
    return out;
  }

  function dirBetween(
    a: { x: number; y: number },
    b: { x: number; y: number },
  ): EntityDirection | null {
    if (a.x === b.x && a.y === b.y - 1) return "South";
    if (a.x === b.x && a.y === b.y + 1) return "North";
    if (a.y === b.y && a.x === b.x - 1) return "East";
    if (a.y === b.y && a.x === b.x + 1) return "West";
    return null;
  }

  /**
   * Plan a belt + auto-UG run along the L-shape path. Returns null if
   * the plan is invalid (out of bounds, drag starts/ends in obstacle,
   * UG run too long for max_reach).
   */
  function planRun(path: { x: number; y: number }[]): PaintedEntity[] | null {
    if (!zone || path.length === 0) return null;
    for (const p of path) {
      if (!inBbox(p.x, p.y)) return null;
    }
    const isFree = (p: { x: number; y: number }) => !zone!.forbidden.has(key(p.x, p.y));
    const first = path[0];
    const last = path[path.length - 1];
    if (!first || !last) return null;
    if (!isFree(first) || !isFree(last)) return null;

    const entities: PaintedEntity[] = [];
    const item = currentItem();

    let i = 0;
    while (i < path.length) {
      const cur = path[i];
      const next = path[i + 1] ?? null;
      // Direction at this tile: prefer next-tile direction; for last
      // tile use prev-tile direction (already-placed previous belt
      // points at us, so we extend that flow).
      const dir = next
        ? dirBetween(cur, next)
        : i > 0
          ? dirBetween(path[i - 1], cur)
          : brushDir;
      if (!dir) return null;

      // Check whether the run from i+1 hits an obstacle.
      let j = i + 1;
      while (j < path.length && isFree(path[j])) j++;
      // path[i] is free, path[i+1..j-1] are free, path[j] is obstacle (or end).
      if (j === path.length) {
        // Rest of the path is free; just place a belt here and advance.
        entities.push(makeBelt(cur, dir, item));
        i++;
        continue;
      }
      // Find the end of the obstacle run.
      let k = j;
      while (k < path.length && !isFree(path[k])) k++;
      // path[j..k-1] are obstacle. Need a free tile at path[k] to land
      // the UG-out — refuse otherwise.
      if (k === path.length) return null;

      // Reach distance is from path[j-1] to path[k]. Must be ≤ max_reach + 1
      // (so the obstacle run length + 2 endpoints ≤ max_reach + 2 tiles
      // apart). The Factorio rule: distance between UG-in and UG-out
      // tiles is at most `max_reach`. (max_reach 4 means yellow can
      // span 4 tiles between, so 5 tiles apart in axis terms.)
      const ugIn = path[j - 1];
      const ugOut = path[k];
      const dist = Math.abs(ugOut.x - ugIn.x) + Math.abs(ugOut.y - ugIn.y);
      if (dist > zone.maxReach + 1) return null;

      // Place belts up to (but not including) the UG-in tile.
      for (let p = i; p < j - 1; p++) {
        const here = path[p];
        const dirHere = dirBetween(here, path[p + 1]);
        if (!dirHere) return null;
        entities.push(makeBelt(here, dirHere, item));
      }
      // UG-in at path[j-1], facing dir.
      const ugInDir = dirBetween(ugIn, ugOut);
      if (!ugInDir) return null;
      entities.push(makeUg(ugIn, ugInDir, "input", item));
      entities.push(makeUg(ugOut, ugInDir, "output", item));
      i = k + 1;
      // After the UG-out we need to also place a belt at ugOut+1 if the
      // path continues — but ugOut itself is already an entity. The
      // outer loop will pick up at i=k+1 and continue from there.
      // The belt at path[k] is already placed via makeUg as ugOut.
    }

    return entities;
  }

  function makeBelt(
    p: { x: number; y: number },
    dir: EntityDirection,
    item: string | null,
  ): PaintedEntity {
    const name = BELT_NAMES[zone?.beltTier ?? "transport-belt"] ?? "transport-belt";
    return {
      name,
      x: p.x,
      y: p.y,
      direction: dir,
      carries: item ?? undefined,
    } as PaintedEntity;
  }

  function makeUg(
    p: { x: number; y: number },
    dir: EntityDirection,
    io: "input" | "output",
    item: string | null,
  ): PaintedEntity {
    const name = UG_NAMES[zone?.beltTier ?? "transport-belt"] ?? "underground-belt";
    return {
      name,
      x: p.x,
      y: p.y,
      direction: dir,
      io_type: io,
      carries: item ?? undefined,
    } as PaintedEntity;
  }

  function commitRun(plan: PaintedEntity[]): void {
    // Replace any existing painted entities at the same tiles.
    const planKeys = new Set(plan.map((e) => key(e.x, e.y)));
    const next = painted.filter((e) => !planKeys.has(key(e.x, e.y))).concat(plan);
    setPainted(next, true);
  }

  function eraseTile(p: { x: number; y: number }): void {
    if (!zone) return;
    if (!inBbox(p.x, p.y)) return;
    if (!painted.some((e) => e.x === p.x && e.y === p.y)) return;
    const next = painted.filter((e) => !(e.x === p.x && e.y === p.y));
    setPainted(next, true);
  }

  // ----- Pointer wiring ----------------------------------------------------

  /** When the user clicks/starts a drag on an input boundary tile,
   * auto-pick that boundary's item so subsequent belts carry it. */
  function autoPickItemForTile(t: { x: number; y: number }): void {
    if (!zone) return;
    const b = zone.boundaries.find((b) => b.x === t.x && b.y === t.y && b.isInput);
    if (!b) return;
    const idx = zone.items.indexOf(b.item);
    if (idx >= 0 && idx !== itemIndex) {
      itemIndex = idx;
      renderToolbar();
    }
  }

  function onPointerDown(ev: PointerEvent): void {
    if (!active || !zone) return;
    if (ev.button !== 0) return;
    const t = tileFromEvent(ev);
    if (!t || !inBbox(t.x, t.y)) return;
    autoPickItemForTile(t);
    if (tool === "erase") {
      eraseTile(t);
      ev.stopPropagation();
      ev.preventDefault();
      return;
    }
    if (tool === "ug-in" || tool === "ug-out") {
      // Single-tile placement for UG-in/out — useful when planning a
      // hand-placed spine. Drag UG-in alone is rarely what users mean.
      const e =
        tool === "ug-in"
          ? makeUg(t, brushDir, "input", currentItem())
          : makeUg(t, brushDir, "output", currentItem());
      const next = painted.filter((p) => !(p.x === t.x && p.y === t.y)).concat(e);
      setPainted(next, true);
      ev.stopPropagation();
      ev.preventDefault();
      return;
    }
    // Belt: start a drag and immediately preview the single-tile run
    // (so the user sees something before they move).
    drag = { startX: t.x, startY: t.y, bendVerticalFirst: false };
    const path = lShape({ x: t.x, y: t.y }, t, false);
    const plan = planRun(path);
    rerenderPreview(plan ?? [], plan === null);
    ev.stopPropagation();
    ev.preventDefault();
  }

  function onPointerMove(ev: PointerEvent): void {
    if (!active || !drag || !zone) return;
    const t = tileFromEvent(ev);
    if (!t) return;
    const path = lShape({ x: drag.startX, y: drag.startY }, t, drag.bendVerticalFirst);
    const plan = planRun(path);
    rerenderPreview(plan ?? [], plan === null);
  }

  function onPointerUp(ev: PointerEvent): void {
    if (!active || !drag || !zone) {
      drag = null;
      clearPreview();
      return;
    }
    const t = tileFromEvent(ev);
    if (!t) {
      drag = null;
      clearPreview();
      return;
    }
    const path = lShape({ x: drag.startX, y: drag.startY }, t, drag.bendVerticalFirst);
    const plan = planRun(path);
    drag = null;
    clearPreview();
    if (!plan) {
      // Flash the status indicator red briefly.
      setStatus("invalid", "drag rejected: out of bounds, on obstacle, or UG too long");
      return;
    }
    commitRun(plan);
    ev.stopPropagation();
    ev.preventDefault();
  }

  // ----- Hotkeys -----------------------------------------------------------

  function onKeyDown(ev: KeyboardEvent): void {
    if (!active) return;
    const tag = (ev.target as HTMLElement | null)?.tagName?.toUpperCase();
    if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;
    const consume = () => {
      ev.stopImmediatePropagation();
      ev.preventDefault();
    };
    if (ev.key === "Escape") {
      exit();
      consume();
      return;
    }
    if (ev.key === "1") { setTool("belt"); consume(); return; }
    if (ev.key === "2") { setTool("ug-in"); consume(); return; }
    if (ev.key === "3") { setTool("ug-out"); consume(); return; }
    if (ev.key === "0") { setTool("erase"); consume(); return; }
    if (ev.key === "r" || ev.key === "R") {
      if (drag) {
        drag.bendVerticalFirst = !drag.bendVerticalFirst;
      } else {
        brushDir = ROTATE_CW[brushDir];
        renderToolbar();
      }
      consume();
      return;
    }
    if (ev.key === "[" && zone) {
      itemIndex = (itemIndex - 1 + zone.items.length) % zone.items.length;
      renderToolbar();
      consume();
      return;
    }
    if (ev.key === "]" && zone) {
      itemIndex = (itemIndex + 1) % zone.items.length;
      renderToolbar();
      consume();
      return;
    }
    if ((ev.key === "Enter" || ev.key === "a" || ev.key === "A") && status === "valid" && ghost.length > 0) {
      acceptGhost();
      consume();
      return;
    }
    if ((ev.ctrlKey || ev.metaKey) && (ev.key === "z" || ev.key === "Z")) {
      if (ev.shiftKey) redo();
      else undo();
      consume();
      return;
    }
  }

  function setTool(t: Tool): void {
    tool = t;
    renderToolbar();
  }

  function undo(): void {
    if (undoStack.length === 0) return;
    redoStack.push(painted);
    painted = undoStack.pop()!;
    rerender();
    scheduleValidate();
  }

  function redo(): void {
    if (redoStack.length === 0) return;
    undoStack.push(painted);
    painted = redoStack.pop()!;
    rerender();
    scheduleValidate();
  }

  function acceptGhost(): void {
    if (ghost.length === 0) return;
    const next = painted.concat(ghost.map((e) => ({ ...e })));
    ghost = [];
    setPainted(next, true);
  }

  // ----- Toolbar rendering -------------------------------------------------

  function renderToolbar(): void {
    if (!toolbarEl) return;
    toolbarEl.innerHTML = "";

    const tools: [Tool, string, string][] = [
      ["belt", "B", "Belt (1)"],
      ["ug-in", "↧", "UG-in (2)"],
      ["ug-out", "↥", "UG-out (3)"],
      ["erase", "✕", "Erase (0)"],
    ];
    for (const [t, glyph, title] of tools) {
      const b = document.createElement("button");
      b.className = "se-tool" + (tool === t ? " se-tool-active" : "");
      b.textContent = glyph;
      b.title = title;
      b.addEventListener("click", () => setTool(t));
      toolbarEl.appendChild(b);
    }

    const dirBtn = document.createElement("button");
    dirBtn.className = "se-dir";
    const dirGlyph: Record<EntityDirection, string> = {
      North: "↑",
      East: "→",
      South: "↓",
      West: "←",
    };
    dirBtn.textContent = dirGlyph[brushDir];
    dirBtn.title = "Brush direction (R rotates)";
    dirBtn.addEventListener("click", () => {
      brushDir = ROTATE_CW[brushDir];
      renderToolbar();
    });
    toolbarEl.appendChild(dirBtn);

    if (zone && zone.items.length > 1) {
      const itemSel = document.createElement("select");
      itemSel.className = "se-item";
      for (const [idx, it] of zone.items.entries()) {
        const o = document.createElement("option");
        o.value = String(idx);
        o.textContent = it;
        if (idx === itemIndex) o.selected = true;
        itemSel.appendChild(o);
      }
      itemSel.addEventListener("change", () => {
        itemIndex = Number(itemSel.value) | 0;
      });
      toolbarEl.appendChild(itemSel);
    } else if (zone && zone.items.length === 1) {
      const lbl = document.createElement("span");
      lbl.className = "se-item-label";
      lbl.textContent = zone.items[0];
      toolbarEl.appendChild(lbl);
    }

    // Spacer
    const spacer = document.createElement("span");
    spacer.style.flex = "1";
    toolbarEl.appendChild(spacer);

    const acceptBtn = document.createElement("button");
    acceptBtn.className = "se-accept";
    acceptBtn.textContent = "Accept";
    acceptBtn.title = "Promote ghost into painted layer (Enter)";
    acceptBtn.addEventListener("click", acceptGhost);
    acceptBtn.disabled = !(status === "valid" && ghost.length > 0);
    toolbarEl.appendChild(acceptBtn);

    const revertBtn = document.createElement("button");
    revertBtn.className = "se-revert";
    revertBtn.textContent = "Revert";
    revertBtn.title = "Discard all painted edits";
    revertBtn.addEventListener("click", () => {
      setPainted([], true);
    });
    toolbarEl.appendChild(revertBtn);

    const exportBtn = document.createElement("button");
    exportBtn.className = "se-export";
    exportBtn.textContent = "Export";
    exportBtn.title = "Save fixture JSON (clipboard + download)";
    exportBtn.addEventListener("click", exportFixture);
    exportBtn.disabled = status !== "valid";
    toolbarEl.appendChild(exportBtn);

    const doneBtn = document.createElement("button");
    doneBtn.className = "se-done";
    doneBtn.textContent = "Done";
    doneBtn.title = "Exit edit mode (Esc)";
    doneBtn.addEventListener("click", exit);
    toolbarEl.appendChild(doneBtn);
  }

  function updateButtonsForStatus(): void {
    if (!toolbarEl) return;
    const accept = toolbarEl.querySelector(".se-accept") as HTMLButtonElement | null;
    if (accept) accept.disabled = !(status === "valid" && ghost.length > 0);
    const exp = toolbarEl.querySelector(".se-export") as HTMLButtonElement | null;
    if (exp) exp.disabled = status !== "valid";
  }

  // ----- Export fixture ----------------------------------------------------

  function exportFixture(): void {
    if (!zone || status !== "valid") return;
    const cost = lastSatCost ?? userCost(painted);
    const json = buildFixtureJson(zone.selection.cluster, zone.selection.iter, zone.selection.trace, {
      maxCost: cost,
      paintedEntities: painted as unknown[],
    });
    const name = (zone.selection.cluster.seed
      ? `fixture_${zone.selection.cluster.seed.x}_${zone.selection.cluster.seed.y}_painted`
      : "fixture_painted") + ".json";
    void navigator.clipboard?.writeText(json).catch(() => {});
    const blob = new Blob([json], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = name;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  }

  // ----- Enter / exit ------------------------------------------------------

  function enter(state: JunctionSelectionState): void {
    if (active) exit();
    const it = state.iter;
    const beltTier = (it.sat?.belt_tier ?? "transport-belt") as string;
    const maxReach = (it.sat?.max_reach as number | undefined) ?? (BELT_UG_REACH[beltTier] ?? 4);
    const rawBoundaries = it.sat?.boundaries ?? it.boundaries;
    const items = Array.from(new Set(rawBoundaries.map((b) => b.item)));
    const boundaries: BoundaryEntry[] = rawBoundaries.map((b) => ({
      x: b.x,
      y: b.y,
      item: b.item,
      isInput: b.is_input,
    }));
    zone = {
      bbox: { x: it.bbox.x, y: it.bbox.y, w: it.bbox.w, h: it.bbox.h },
      forbidden: new Set((it.forbidden ?? []).map((p) => `${p[0]},${p[1]}`)),
      beltTier,
      maxReach,
      items,
      boundaries,
      fixtureJson: buildFixtureJson(state.cluster, state.iter, state.trace),
      selection: state,
    };
    painted = [];
    undoStack = [];
    redoStack = [];
    ghost = [];
    tool = "belt";
    brushDir = "East";
    itemIndex = 0;
    drag = null;
    lastSatCost = null;

    // Mount PIXI layers below the SAT zone overlay so the boundary
    // chevrons + item icons stay visible on top of painted belts.
    // Order, top → bottom: satZoneOverlay, previewLayer, ghostLayer,
    // paintedLayer, entityLayer.
    paintedLayer = new Container();
    ghostLayer = new Container();
    ghostLayer.alpha = 0.55;
    previewLayer = new Container();
    viewport.addChild(paintedLayer);
    viewport.addChild(ghostLayer);
    viewport.addChild(previewLayer);
    // Bring the SAT overlay back to the top.
    viewport.setChildIndex(satZoneOverlayLayer, viewport.children.length - 1);

    // Mount toolbar in inline panel.
    toolbarEl = document.createElement("div");
    toolbarEl.className = "se-toolbar";
    jd.inlineEl.appendChild(toolbarEl);

    statusDot = document.createElement("span");
    statusDot.className = "se-status";
    // Insert before the close button to keep the title left-anchored.
    jd.inlineEl.querySelector(".jd-inline-head")?.appendChild(statusDot);

    jd.setEditMode(true);

    // Pointer events on the canvas DOM (not via viewport.on) so the
    // pixi-viewport drag plugin doesn't claim the event first. The
    // drag plugin is paused for the duration of edit mode anyway —
    // belt of dust on pixi-viewport's "input is captured" semantics.
    viewport.plugins.pause("drag");
    canvas.addEventListener("pointerdown", onPointerDown, { capture: true });
    canvas.addEventListener("pointerup", onPointerUp, { capture: true });
    canvas.addEventListener("pointermove", onPointerMove, { capture: true });
    document.addEventListener("keydown", onKeyDown, { capture: true });

    active = true;
    renderToolbar();
    setStatus("idle");
  }

  function exit(): void {
    if (!active) return;
    active = false;
    if (solveTimer !== null) {
      window.clearTimeout(solveTimer);
      solveTimer = null;
    }
    solverEpoch++;
    if (paintedLayer) {
      viewport.removeChild(paintedLayer);
      paintedLayer.destroy({ children: true });
      paintedLayer = null;
    }
    if (ghostLayer) {
      viewport.removeChild(ghostLayer);
      ghostLayer.destroy({ children: true });
      ghostLayer = null;
    }
    if (previewLayer) {
      viewport.removeChild(previewLayer);
      previewLayer.destroy({ children: true });
      previewLayer = null;
    }
    if (toolbarEl) {
      toolbarEl.remove();
      toolbarEl = null;
    }
    if (statusDot) {
      statusDot.remove();
      statusDot = null;
    }
    canvas.removeEventListener("pointerdown", onPointerDown, { capture: true });
    canvas.removeEventListener("pointerup", onPointerUp, { capture: true });
    canvas.removeEventListener("pointermove", onPointerMove, { capture: true });
    document.removeEventListener("keydown", onKeyDown, { capture: true });
    viewport.plugins.resume("drag");
    jd.setEditMode(false);
    zone = null;
    painted = [];
    undoStack = [];
    redoStack = [];
    ghost = [];
  }

  function isActive(): boolean {
    return active;
  }

  return { enter, exit, isActive };
}
