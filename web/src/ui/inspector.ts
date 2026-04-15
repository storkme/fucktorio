import type { PlacedEntity } from "../engine";
import { niceName, getRecipeFlows, isBeltEntity, type HighlightController } from "../renderer/entities";

export interface InspectorControls {
  onHover(entity: PlacedEntity | null): void;
  setHighlightController(ctrl: HighlightController | null): void;
  setTooltipOverride(text: string | null): void;
}

export function createInspector(container: HTMLElement): InspectorControls {
  const tooltip = document.createElement("div");
  tooltip.style.cssText = "position:fixed;background:#1e1e1e;color:#e0e0e0;border:1px solid #555;padding:4px 8px;font:12px monospace;pointer-events:none;border-radius:3px;display:none;z-index:1000;max-width:200px;line-height:1.5";
  document.body.appendChild(tooltip);

  document.addEventListener("mousemove", (e) => {
    tooltip.style.left = e.clientX + 14 + "px";
    tooltip.style.top = e.clientY - 10 + "px";
  });

  let highlightCtrl: HighlightController | null = null;
  let tooltipOverride: string | null = null;

  function iconTag(slug: string, size = 16): string {
    return `<img src="${import.meta.env.BASE_URL}icons/${slug}.png" width="${size}" height="${size}" style="vertical-align:middle;margin-right:3px;image-rendering:pixelated" onerror="this.style.display='none'">`;
  }

  function onHover(entity: PlacedEntity | null): void {
    if (tooltipOverride !== null) return;
    if (entity) {
      const dirArrow: Record<string, string> = { North: "\u2191", East: "\u2192", South: "\u2193", West: "\u2190" };
      let html = `${iconTag(entity.name)}<b>${niceName(entity.name)}</b>`;
      if (entity.direction && entity.name !== "pipe") html += `<br>${dirArrow[entity.direction] ?? ""} ${entity.direction}`;
      if (entity.carries) html += `<br>${iconTag(entity.carries)} ${niceName(entity.carries)}`;
      if (entity.rate != null) html += `<br><span style="color:#b5cea8">${entity.rate.toFixed(1)}/s</span>`;
      if (entity.io_type) html += `<br>io: ${entity.io_type}`;
      if (entity.recipe) {
        html += `<br>${iconTag(entity.recipe)} ${niceName(entity.recipe)}`;
        const flows = getRecipeFlows(entity.recipe);
        if (flows) {
          for (const inp of flows.inputs) html += `<br><span style="color:#aaa">\u25b6 ${iconTag(inp.item, 14)}${niceName(inp.item)} ${inp.rate.toFixed(1)}/s</span>`;
          for (const out of flows.outputs) html += `<br><span style="color:#aaa">\u25c0 ${iconTag(out.item, 14)}${niceName(out.item)} ${out.rate.toFixed(1)}/s</span>`;
        }
      }
      if (entity.segment_id) html += `<br><span style="color:#9cdcfe">${entity.segment_id}</span>`;
      html += `<br>pos: ${entity.x ?? 0}, ${entity.y ?? 0}`;
      tooltip.innerHTML = html;
      tooltip.style.display = "block";

      if (highlightCtrl) {
        if (isBeltEntity(entity.name)) {
          highlightCtrl.highlightBeltNetwork(entity);
        } else {
          highlightCtrl.highlightItem(highlightCtrl.chainKey(entity));
        }
      }
    } else {
      tooltip.style.display = "none";
      if (highlightCtrl) highlightCtrl.clearHighlight();
    }
  }

  void container;

  return {
    onHover,
    setHighlightController(ctrl: HighlightController | null): void {
      highlightCtrl = ctrl;
    },
    setTooltipOverride(text: string | null): void {
      tooltipOverride = text;
      if (text !== null) {
        tooltip.innerHTML = text;
        tooltip.style.display = "block";
      } else {
        tooltip.style.display = "none";
      }
    },
  };
}
