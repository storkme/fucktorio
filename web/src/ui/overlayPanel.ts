import * as debugState from "../state/debugState";
import "./overlayPanel.css";

export interface OverlayPanelControls {
  updateCoords(x: number, y: number): void;
  /** Check the master Debug toggle and reveal its sub-panel. */
  setDebugEnabled(on: boolean): void;
  debugCb: HTMLInputElement;
  stepCb: HTMLInputElement;
  valCb: HTMLInputElement;
  regionsCb: HTMLInputElement;
  soloRegionsCb: HTMLInputElement;
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

  const coordsEl = document.createElement("div");
  coordsEl.className = "overlay-coords";
  coordsEl.textContent = "x:\u2013 y:\u2013";
  panel.appendChild(coordsEl);

  const state = debugState.get();
  const debugCb = makeToggle(panel, "Debug", state.master);

  const subPanel = document.createElement("div");
  subPanel.className = "overlay-sub-panel";
  subPanel.style.display = state.master ? "flex" : "none";

  const stepCb = makeToggle(subPanel, "Step-through", state.stepThrough);
  const valCb = makeToggle(subPanel, "Validation", state.validation);
  const regionsCb = makeToggle(subPanel, "SAT Zones", state.satZones);
  const soloRegionsCb = makeToggle(subPanel, "Solo regions", state.soloRegions);
  panel.appendChild(subPanel);

  container.appendChild(panel);

  debugCb.addEventListener("change", () => {
    subPanel.style.display = debugCb.checked ? "flex" : "none";
    debugState.set({ master: debugCb.checked });
  });

  return {
    updateCoords(x: number, y: number): void {
      coordsEl.textContent = `x:${x} y:${y}`;
    },
    setDebugEnabled(on: boolean): void {
      debugCb.checked = on;
      subPanel.style.display = on ? "flex" : "none";
      debugState.set({ master: on });
    },
    debugCb,
    stepCb,
    valCb,
    regionsCb,
    soloRegionsCb,
  };
}
