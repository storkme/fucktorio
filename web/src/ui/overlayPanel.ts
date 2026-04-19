import * as debugState from "../state/debugState";
import "./overlayPanel.css";

export interface OverlayPanelControls {
  /** Check the master Debug toggle and reveal its sub-panel. */
  setDebugEnabled(on: boolean): void;
  debugCb: HTMLInputElement;
  colorCb: HTMLInputElement;
  stepCb: HTMLInputElement;
  valCb: HTMLInputElement;
  regionsCb: HTMLInputElement;
  soloRegionsCb: HTMLInputElement;
  ghostTilesCb: HTMLInputElement;
}

function makeToggle(parent: HTMLElement, label: string, checked = false): HTMLInputElement {
  const cb = document.createElement("input");
  cb.type = "checkbox";
  cb.checked = checked;
  const wrap = document.createElement("div");
  wrap.className = "overlay-toggle";
  const lbl = document.createElement("label");
  lbl.appendChild(cb);
  lbl.appendChild(document.createTextNode(label));
  wrap.appendChild(lbl);
  parent.appendChild(wrap);
  return cb;
}

export function createOverlayPanel(container: HTMLElement): OverlayPanelControls {
  container.style.position = "relative";

  const panel = document.createElement("div");
  panel.className = "overlay-panel";

  const state = debugState.get();
  const debugCb = makeToggle(panel, "Debug", state.master);
  const colorCb = makeToggle(panel, "Item colours", false);

  const subPanel = document.createElement("div");
  subPanel.className = "overlay-sub-panel";
  subPanel.style.display = state.master ? "flex" : "none";

  const stepCb = makeToggle(subPanel, "Step-through", state.stepThrough);
  const valCb = makeToggle(subPanel, "Validation", state.validation);
  const regionsCb = makeToggle(subPanel, "SAT Zones", state.satZones);
  const ghostTilesCb = makeToggle(subPanel, "Ghost tiles", state.ghostTiles);
  const soloRegionsCb = makeToggle(subPanel, "Solo regions", state.soloRegions);
  panel.appendChild(subPanel);

  container.appendChild(panel);

  debugCb.addEventListener("change", () => {
    subPanel.style.display = debugCb.checked ? "flex" : "none";
    debugState.set({ master: debugCb.checked });
  });

  // Persist the SAT Zones + Ghost tiles toggles alongside the master
  // Debug flag so a mid-session reload keeps those overlays visible.
  // Only user-initiated flips (change events) write to storage;
  // programmatic overrides like solo-mode save/restore deliberately
  // don't fire a change event and so won't persist.
  regionsCb.addEventListener("change", () => {
    debugState.set({ satZones: regionsCb.checked });
  });
  ghostTilesCb.addEventListener("change", () => {
    debugState.set({ ghostTiles: ghostTilesCb.checked });
  });

  return {
    setDebugEnabled(on: boolean): void {
      debugCb.checked = on;
      subPanel.style.display = on ? "flex" : "none";
      debugState.set({ master: on });
    },
    debugCb,
    colorCb,
    stepCb,
    valCb,
    regionsCb,
    soloRegionsCb,
    ghostTilesCb,
  };
}
