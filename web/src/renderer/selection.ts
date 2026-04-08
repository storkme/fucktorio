import { Container, Graphics } from "pixi.js";
import type { Viewport } from "pixi-viewport";
import type { PlacedEntity, LayoutResult } from "../engine";
import { TILE_PX } from "./entities";

const SEL_COLOR = 0x00e0a0;

export interface SelectionController {
  /** Release event listeners and destroy graphics. Call when layout changes. */
  destroy(): void;
  /** Clear the current selection. */
  clear(): void;
  /** Currently selected entities. */
  getSelected(): PlacedEntity[];
  /** Serialize selection + params + note to a JSON string. */
  buildJson(params: { item: string; rate: number } | null, note: string): string;
}

export function createSelectionController(
  canvas: HTMLCanvasElement,
  viewport: Viewport,
  entityLayer: Container,
  layout: LayoutResult,
  onSelectionChange: (entities: PlacedEntity[]) => void,
): SelectionController {
  // Tile lookup
  const tileMap = new Map<string, PlacedEntity>();
  for (const e of layout.entities) {
    tileMap.set(`${e.x ?? 0},${e.y ?? 0}`, e);
  }

  let dragStart: { sx: number; sy: number } | null = null;
  let isDragging = false;
  let selected: PlacedEntity[] = [];

  // Drag rect — shown while the mouse is held
  const dragRectG = new Graphics();
  entityLayer.addChild(dragRectG);

  // Persistent selection borders
  const borderG = new Graphics();
  entityLayer.addChild(borderG);

  function toWorld(clientX: number, clientY: number): { x: number; y: number } {
    const r = canvas.getBoundingClientRect();
    return viewport.toWorld(clientX - r.left, clientY - r.top);
  }

  function redrawDragRect(currClientX: number, currClientY: number): void {
    if (!dragStart) return;
    const sw = toWorld(dragStart.sx, dragStart.sy);
    const cw = toWorld(currClientX, currClientY);
    const minX = Math.min(sw.x, cw.x);
    const minY = Math.min(sw.y, cw.y);
    const w = Math.abs(cw.x - sw.x);
    const h = Math.abs(cw.y - sw.y);

    dragRectG.clear();
    dragRectG.rect(minX, minY, w, h).fill({ color: SEL_COLOR, alpha: 0.18 });
    dragRectG.setStrokeStyle({ width: 1, color: SEL_COLOR, alpha: 0.8 });
    dragRectG.rect(minX, minY, w, h).stroke();
  }

  function redrawBorders(entities: PlacedEntity[]): void {
    borderG.clear();
    if (entities.length === 0) return;
    borderG.setStrokeStyle({ width: 1.5, color: SEL_COLOR, alpha: 0.9 });
    for (const e of entities) {
      const px = (e.x ?? 0) * TILE_PX + 1;
      const py = (e.y ?? 0) * TILE_PX + 1;
      borderG.rect(px, py, TILE_PX - 2, TILE_PX - 2).stroke();
    }
  }

  function collectEntities(currClientX: number, currClientY: number): PlacedEntity[] {
    if (!dragStart) return [];
    const sw = toWorld(dragStart.sx, dragStart.sy);
    const cw = toWorld(currClientX, currClientY);
    const minTx = Math.min(Math.floor(sw.x / TILE_PX), Math.floor(cw.x / TILE_PX));
    const maxTx = Math.max(Math.floor(sw.x / TILE_PX), Math.floor(cw.x / TILE_PX));
    const minTy = Math.min(Math.floor(sw.y / TILE_PX), Math.floor(cw.y / TILE_PX));
    const maxTy = Math.max(Math.floor(sw.y / TILE_PX), Math.floor(cw.y / TILE_PX));
    const out: PlacedEntity[] = [];
    for (let tx = minTx; tx <= maxTx; tx++) {
      for (let ty = minTy; ty <= maxTy; ty++) {
        const e = tileMap.get(`${tx},${ty}`);
        if (e) out.push(e);
      }
    }
    return out;
  }

  const onDown = (e: PointerEvent) => {
    if (e.button !== 0) return;
    dragStart = { sx: e.clientX, sy: e.clientY };
    isDragging = false;
  };

  const onMove = (e: PointerEvent) => {
    if (!dragStart) return;
    const dx = e.clientX - dragStart.sx;
    const dy = e.clientY - dragStart.sy;
    if (!isDragging && dx * dx + dy * dy > 36) {
      isDragging = true;
    }
    if (isDragging) {
      redrawDragRect(e.clientX, e.clientY);
    }
  };

  const onUp = (e: PointerEvent) => {
    if (e.button !== 0) return;
    if (isDragging) {
      e.stopImmediatePropagation();
      dragRectG.clear();
      selected = collectEntities(e.clientX, e.clientY);
      redrawBorders(selected);
      onSelectionChange(selected);
    } else {
      // Plain click — clear selection (entity click events still fire normally)
      selected = [];
      borderG.clear();
      onSelectionChange([]);
    }
    dragStart = null;
    isDragging = false;
  };

  canvas.addEventListener("pointerdown", onDown, { capture: true });
  canvas.addEventListener("pointermove", onMove, { capture: true });
  canvas.addEventListener("pointerup", onUp, { capture: true });

  return {
    destroy() {
      canvas.removeEventListener("pointerdown", onDown, { capture: true });
      canvas.removeEventListener("pointermove", onMove, { capture: true });
      canvas.removeEventListener("pointerup", onUp, { capture: true });
      dragRectG.destroy();
      borderG.destroy();
    },
    clear() {
      selected = [];
      dragRectG.clear();
      borderG.clear();
      onSelectionChange([]);
    },
    getSelected() {
      return [...selected];
    },
    buildJson(params, note) {
      return JSON.stringify(
        {
          params,
          selected: selected.map((e) => ({
            x: e.x ?? 0,
            y: e.y ?? 0,
            name: e.name,
            direction: e.direction,
            carries: e.carries,
            recipe: e.recipe,
            rate: e.rate,
            io_type: e.io_type,
          })),
          note,
        },
        null,
        2,
      );
    },
  };
}
