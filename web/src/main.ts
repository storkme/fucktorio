import { Container } from "pixi.js";
import type { Graphics } from "pixi.js";
import { createApp, WORLD_SIZE } from "./renderer/app";
import { drawGrid } from "./renderer/grid";
import { drawGraph } from "./renderer/graph";
import { initEntityIcons, renderLayout, setItemColoring, setRateOverlay, itemColor, TILE_PX } from "./renderer/entities";
import { createSelectionController, type SelectionController } from "./renderer/selection";
import { renderSidebar, type DisplayToggles } from "./ui/sidebar";
import { initCorpusPanel } from "./ui/corpus";
import { renderLanding } from "./ui/landing";
import {
  setupSnapshotDropZone,
  decodeSnapshot,
} from "./ui/snapshotLoader";
import { initEngine, getEngine } from "./engine";
import type { SolverResult, LayoutResult, PlacedEntity, ValidationIssue } from "./engine";
import { renderTraceOverlay, getTracePhases, eventsUpToPhase, type TraceEvent, type PhaseSnapshot } from "./renderer/traceOverlay";
import { renderValidationOverlay } from "./renderer/validationOverlay";
import { renderRegionOverlayDetailed, type RegionOverlayItem } from "./renderer/regionOverlay";
import * as debugState from "./state/debugState";
import { createOverlayPanel } from "./ui/overlayPanel";
import { createIssuesDialog } from "./ui/issuesDialog";
import { createInspector } from "./ui/inspector";
import { createSnapshotMode } from "./ui/snapshotMode";
import { createStepThrough } from "./ui/stepThrough";

const MACHINE_SLUGS = [
  "assembling-machine-1", "assembling-machine-2", "assembling-machine-3",
  "electric-furnace", "steel-furnace", "stone-furnace",
  "chemical-plant", "oil-refinery", "centrifuge", "lab", "rocket-silo",
  "foundry", "electromagnetic-plant", "cryogenic-plant", "biochamber", "biolab",
  "recycler", "crusher", "beacon", "storage-tank", "electric-mining-drill",
];

async function main(): Promise<void> {
  await initEngine();
  const engine = getEngine();
  await initEntityIcons(MACHINE_SLUGS);

  const appRoot = document.getElementById("app")!;
  const hash = window.location.hash;
  const params = new URLSearchParams(window.location.search);
  // Skip the landing page when the URL carries any generator state:
  // item/rate/machine/in/belt. Lets shared links (e.g. layout URLs
  // pasted into chat) open straight into the generator without the
  // extra click through the landing screen.
  const hasGeneratorParams =
    params.has("item") ||
    params.has("rate") ||
    params.has("machine") ||
    params.has("in") ||
    params.has("belt");
  const skipLanding =
    hash.startsWith("#/layout") ||
    params.has("generator") ||
    hasGeneratorParams;

  if (!skipLanding) {
    const landingHost = document.createElement("div");
    appRoot.appendChild(landingHost);

    renderLanding(landingHost, engine, {
      onOpenGenerator: () => {
        landingHost.remove();
        initGenerator(engine);
        window.history.replaceState({}, "", "#/layout");
      },
    });
    return;
  }

  initGenerator(engine);
}

async function initGenerator(engine: ReturnType<typeof getEngine>): Promise<void> {
  const container = document.getElementById("canvas-container");
  if (!container) throw new Error("Missing #canvas-container element");

  const appRoot = document.getElementById("app")!;
  appRoot.style.display = "flex";
  const sidebar = document.getElementById("sidebar");
  if (sidebar) sidebar.style.display = "";
  container.style.display = "";

  const { app, viewport } = await createApp(container);
  drawGrid(viewport);
  drawGraph(viewport, null);

  debugState.create();

  // --- Modules ---
  const overlayControls = createOverlayPanel(container);
  const { debugCb, stepCb, valCb, regionsCb, soloRegionsCb, updateCoords } = overlayControls;

  const inspector = createInspector(container);

  const issuesDialog = createIssuesDialog(container, app, viewport);
  issuesDialog.setOnValClose(() => {
    valCb.checked = false;
    updateValidationOverlay();
  });

  setupSnapshotDropZone(container, (snap) => snapshotMode.load(snap));

  const entityLayer = new Container();
  viewport.addChild(entityLayer);
  viewport.moveCenter(WORLD_SIZE / 2, WORLD_SIZE / 2);

  // Click-to-inspect removed; pass no-op to renderLayout.
  const onSelect = (_entity: PlacedEntity | null): void => {};

  function onHover(entity: PlacedEntity | null): void {
    inspector.onHover(entity);
  }

  // --- Sidebar toggles (populated via onDisplayToggles callback) ---
  let colorCb: HTMLInputElement;
  let rateCb: HTMLInputElement;

  let soloRegionsActive = false;
  let soloSavedState: {
    colorChecked: boolean;
    rateChecked: boolean;
    valChecked: boolean;
    regionsChecked: boolean;
    entityAlpha: number;
  } | null = null;

  let traceOverlayLayer: Container | null = null;
  let snapshotActive = false;
  let prevSnapshotEntities: Set<string> | null = null;

  function entityKey(e: PlacedEntity): string {
    return `${e.x},${e.y},${e.name},${e.recipe ?? ""}`;
  }

  function getSnapshotForPhase(
    events: TraceEvent[],
    phaseIndex: number,
  ): { entities: PlacedEntity[]; width: number; height: number } | null {
    const phases = getTracePhases(events);
    if (phaseIndex < 0 || phaseIndex >= phases.length) return null;
    const phaseName = phases[phaseIndex].name;
    for (const evt of events) {
      if (evt.phase === "PhaseSnapshot" && (evt as PhaseSnapshot).data.phase === phaseName) {
        return (evt as PhaseSnapshot).data;
      }
    }
    return null;
  }

  const stepThrough = createStepThrough(container, {
    getLayout: () => lastLayout,
    isEnabled: () => debugCb.checked && stepCb.checked,
    onPhaseChange: () => updateTraceOverlay(),
    onJumpToFailure: (fromX, fromY) => {
      const targetX = fromX * TILE_PX + TILE_PX / 2;
      const targetY = fromY * TILE_PX + TILE_PX / 2;
      viewport.moveCenter(targetX, targetY);
      if (traceOverlayLayer) {
        const marker = traceOverlayLayer.children.find(c =>
          c.label === "RouteFailure" &&
          Math.abs(c.x - targetX) < 1 && Math.abs(c.y - targetY) < 1,
        );
        if (marker) {
          let pulses = 0;
          const interval = setInterval(() => {
            marker.alpha = marker.alpha < 0.5 ? 1.0 : 0.3;
            pulses++;
            if (pulses >= 6) {
              marker.alpha = 1.0;
              clearInterval(interval);
            }
          }, 100);
        }
      }
    },
  });

  function updateTraceOverlay(): void {
    if (traceOverlayLayer) {
      entityLayer.removeChild(traceOverlayLayer);
      traceOverlayLayer.destroy();
      traceOverlayLayer = null;
    }

    const phaseIndex = stepThrough.getPhaseIndex();
    const wantSnapshot = debugCb.checked && stepCb.checked && phaseIndex >= 0 && !!lastLayout?.trace;
    const snapshot = wantSnapshot
      ? getSnapshotForPhase(lastLayout!.trace as TraceEvent[], phaseIndex)
      : null;

    if (snapshot) {
      snapshotActive = true;
      const ctrl = renderLayout(
        { ...lastLayout!, entities: snapshot.entities, width: snapshot.width, height: snapshot.height },
        entityLayer, onHover, onSelect,
      );
      inspector.setHighlightController(ctrl);
      const newKeys = new Set(snapshot.entities.map(entityKey));
      const prev = prevSnapshotEntities;
      if (prev) {
        const added = snapshot.entities.filter(e => !prev.has(entityKey(e)));
        const addedPositions = new Set(added.map(e => `${e.x},${e.y}`));
        for (const child of entityLayer.children) {
          if (!("tint" in child) || addedPositions.size === 0) continue;
          const g = child as { x: number; y: number; tint: number };
          const tx = Math.round(g.x / TILE_PX);
          const ty = Math.round(g.y / TILE_PX);
          if (addedPositions.has(`${tx},${ty}`)) {
            g.tint = 0x44ff88;
            setTimeout(() => { g.tint = 0xffffff; }, 1000);
          }
        }
      }
      prevSnapshotEntities = newKeys;
    } else if (snapshotActive) {
      snapshotActive = false;
      prevSnapshotEntities = null;
      if (lastLayout) {
        const ctrl = renderLayout(lastLayout, entityLayer, onHover, onSelect);
        inspector.setHighlightController(ctrl);
      }
    }

    if (!debugCb.checked || !stepCb.checked || !lastLayout?.trace?.length) {
      stepThrough.update();
      return;
    }
    const events = phaseIndex < 0
      ? (lastLayout.trace as TraceEvent[])
      : eventsUpToPhase(lastLayout.trace as TraceEvent[], phaseIndex);
    traceOverlayLayer = renderTraceOverlay(
      events,
      lastLayout.width ?? 0,
      lastLayout.height ?? 0,
      entityLayer,
      (text) => {
        inspector.setTooltipOverride(text ? `<span style="color:#8af">TRACE</span> ${text}` : null);
      },
    );
    stepThrough.update();
  }

  let valOverlayLayer: Container | null = null;
  let valCircleMap: Map<string, Graphics[]> = new Map();
  let cachedValidationIssues: ValidationIssue[] | null = null;

  let regionOverlayLayer: Container | null = null;
  let regionHitTest: ((wx: number, wy: number) => RegionOverlayItem | null) | null = null;

  function panToTile(x: number, y: number): void {
    viewport.moveCenter(x * TILE_PX + TILE_PX / 2, y * TILE_PX + TILE_PX / 2);
  }

  function updateValidationOverlay(): void {
    if (valOverlayLayer) {
      entityLayer.removeChild(valOverlayLayer);
      valOverlayLayer.destroy();
      valOverlayLayer = null;
      valCircleMap = new Map();
    }
    issuesDialog.clearPulse();
    issuesDialog.setCircleMap(valCircleMap);

    if (lastLayout && !cachedValidationIssues) {
      try {
        cachedValidationIssues = engine.validateLayout(lastLayout, null);
      } catch {
        cachedValidationIssues = [];
      }
    }

    sidebarCtrl?.updateValidation(cachedValidationIssues ?? [], panToTile);

    if (!debugCb.checked || !valCb.checked || !lastLayout) {
      issuesDialog.populate(cachedValidationIssues ?? [], debugCb.checked, valCb.checked);
      return;
    }
    if (!cachedValidationIssues || cachedValidationIssues.length === 0) {
      issuesDialog.populate([], debugCb.checked, valCb.checked);
      return;
    }
    const result = renderValidationOverlay(
      cachedValidationIssues,
      entityLayer,
      (text) => {
        inspector.setTooltipOverride(text ? `<span style="color:#f44">VALIDATION</span> ${text}` : null);
      },
    );
    valOverlayLayer = result.layer;
    valCircleMap = result.circleMap;
    issuesDialog.setCircleMap(valCircleMap);
    issuesDialog.populate(cachedValidationIssues, debugCb.checked, valCb.checked);
  }

  function updateRegionOverlay(): void {
    if (regionOverlayLayer) {
      entityLayer.removeChild(regionOverlayLayer);
      regionOverlayLayer.destroy();
      regionOverlayLayer = null;
    }
    regionHitTest = null;
    if (!debugCb.checked || !regionsCb?.checked || !lastLayout) return;
    if (!lastLayout.regions || lastLayout.regions.length === 0) return;
    const detailed = renderRegionOverlayDetailed(lastLayout);
    regionOverlayLayer = detailed.layer;
    regionHitTest = detailed.hitTest;
    entityLayer.addChild(regionOverlayLayer);
  }

  // --- Item color legend (bottom-left) ---
  const legendEl = document.createElement("div");
  legendEl.style.cssText = "position:absolute;bottom:8px;left:8px;background:rgba(0,0,0,0.6);color:#ccc;font:11px monospace;padding:4px 8px;border-radius:3px;pointer-events:none;z-index:10;display:none;max-height:300px;overflow-y:auto";
  container.appendChild(legendEl);

  // --- Selection annotation bar ---
  const annotationBar = document.createElement("div");
  annotationBar.style.cssText = "position:absolute;bottom:34px;left:8px;background:rgba(0,0,0,0.8);color:#e0e0e0;font:11px monospace;padding:6px 8px;border-radius:3px;border:1px solid #00e0a0;z-index:10;display:none;min-width:200px";
  container.appendChild(annotationBar);

  const annotationCount = document.createElement("div");
  annotationCount.style.cssText = "color:#00e0a0;margin-bottom:4px";
  annotationBar.appendChild(annotationCount);

  const annotationNote = document.createElement("textarea");
  annotationNote.placeholder = "Add a note\u2026";
  annotationNote.rows = 2;
  annotationNote.style.cssText = "width:100%;box-sizing:border-box;background:#2a2a2a;color:#e0e0e0;border:1px solid #555;border-radius:2px;font:11px monospace;resize:vertical;margin-bottom:4px";
  annotationBar.appendChild(annotationNote);

  const annotationHint = document.createElement("div");
  annotationHint.style.cssText = "color:#777";
  annotationHint.textContent = "Ctrl+C to copy JSON";
  annotationBar.appendChild(annotationHint);

  let lastLayout: LayoutResult | null = null;
  let selectionCtrl: SelectionController | null = null;

  const snapshotMode = createSnapshotMode({
    sidebarEl: document.getElementById("sidebar"),
    getSidebarCtrl: () => sidebarCtrl,
    renderLayoutOnCanvas,
    setCachedValidationIssues: (issues) => { cachedValidationIssues = issues; },
    updateValidationOverlay,
    panToTile,
    onDebugEnable: () => overlayControls.setDebugEnabled(true),
    onValEnable: () => { valCb.checked = true; },
    onClear: () => {
      snapshotMode.clear();
      entityLayer.removeChildren();
      lastLayout = null;
      cachedValidationIssues = null;
      drawGraph(viewport, null);
      viewport.moveCenter(WORLD_SIZE / 2, WORLD_SIZE / 2);
      legendEl.style.display = "none";
      issuesDialog.setVisible(false);
      issuesDialog.populate([], false, false);
      sidebarCtrl?.updateValidation([], panToTile);
    },
  });

  function onSelectionChange(entities: PlacedEntity[]): void {
    if (entities.length === 0) {
      annotationBar.style.display = "none";
      annotationNote.value = "";
    } else {
      annotationCount.textContent = `${entities.length} entit${entities.length === 1 ? "y" : "ies"} selected`;
      annotationBar.style.display = "block";
    }
  }

  app.canvas.addEventListener("pointermove", (e) => {
    const rect = app.canvas.getBoundingClientRect();
    const sx = e.clientX - rect.left;
    const sy = e.clientY - rect.top;
    const world = viewport.toWorld(sx, sy);
    const tx = Math.floor(world.x / TILE_PX);
    const ty = Math.floor(world.y / TILE_PX);
    updateCoords(tx, ty);
  });

  // Click-to-pan for a SAT region
  app.canvas.addEventListener("pointerdown", (e) => {
    if (!regionHitTest || !regionsCb.checked) return;
    if (e.button !== 0 || e.shiftKey || e.altKey || e.ctrlKey || e.metaKey) return;
    const rect = app.canvas.getBoundingClientRect();
    const world = viewport.toWorld(e.clientX - rect.left, e.clientY - rect.top);
    const it = regionHitTest(world.x, world.y);
    if (it) {
      const cx = (it.region.x + it.region.width / 2) * TILE_PX;
      const cy = (it.region.y + it.region.height / 2) * TILE_PX;
      viewport.moveCenter(cx, cy);
    }
  });

  // Shift held → pause viewport drag so selection box works
  document.addEventListener("keydown", (e) => {
    if (e.key === "Shift") viewport.plugins.pause("drag");
  });
  document.addEventListener("keyup", (e) => {
    if (e.key === "Shift") viewport.plugins.resume("drag");
  });
  window.addEventListener("blur", () => viewport.plugins.resume("drag"));

  function renderGraph(result: SolverResult | null): void {
    entityLayer.removeChildren();
    drawGraph(viewport, result);
    legendEl.style.display = "none";
    if (!result) {
      viewport.moveCenter(WORLD_SIZE / 2, WORLD_SIZE / 2);
    }
  }

  function buildLegend(layout: LayoutResult): void {
    legendEl.innerHTML = "";
    const items = new Set<string>();
    for (const e of layout.entities) {
      if (e.carries) items.add(e.carries);
    }
    if (items.size === 0 || !colorCb.checked) {
      legendEl.style.display = "none";
      return;
    }
    const sorted = Array.from(items).sort();
    for (const item of sorted) {
      const row = document.createElement("div");
      row.style.cssText = "display:flex;align-items:center;gap:5px;padding:1px 0";
      const swatch = document.createElement("span");
      const color = itemColor(item);
      const hex = "#" + color.toString(16).padStart(6, "0");
      swatch.style.cssText = `display:inline-block;width:12px;height:12px;background:${hex};border-radius:2px;flex-shrink:0`;
      row.appendChild(swatch);
      const label = document.createElement("span");
      label.textContent = item;
      row.appendChild(label);
      legendEl.appendChild(row);
    }
    legendEl.style.display = "block";
  }

  function renderLayoutOnCanvas(layout: LayoutResult): void {
    lastLayout = layout;
    stepThrough.reset();
    snapshotActive = false;
    prevSnapshotEntities = null;
    if (selectionCtrl) { selectionCtrl.destroy(); selectionCtrl = null; }
    annotationBar.style.display = "none";
    annotationNote.value = "";
    cachedValidationIssues = null;
    drawGraph(viewport, null);
    const ctrl = renderLayout(layout, entityLayer, onHover, onSelect);
    inspector.setHighlightController(ctrl);
    selectionCtrl = createSelectionController(app.canvas, viewport, entityLayer, layout, onSelectionChange);
    buildLegend(layout);
    updateTraceOverlay();
    updateValidationOverlay();
    updateRegionOverlay();
    const w = layout.width ?? 0;
    const h = layout.height ?? 0;
    if (w > 0 && h > 0) {
      const pxW = w * 32;
      const pxH = h * 32;
      viewport.fit(true, pxW * 1.1, pxH * 1.2);
      viewport.moveCenter(pxW / 2, pxH / 2);
    }
    if (soloRegionsActive) {
      entityLayer.alpha = 0.12;
    }
  }

  // Ctrl+C / Ctrl+O keyboard shortcuts
  document.addEventListener("keydown", (e) => {
    if (!e.ctrlKey) return;
    if (e.key === "c") {
      if (!selectionCtrl || selectionCtrl.getSelected().length === 0) return;
      e.preventDefault();
      const params = sidebarCtrl?.getParams() ?? null;
      const json = selectionCtrl.buildJson(params, annotationNote.value.trim());
      navigator.clipboard.writeText(json).catch(() => undefined);
      annotationHint.textContent = "Copied!";
      setTimeout(() => { annotationHint.textContent = "Ctrl+C to copy JSON"; }, 2000);
    } else if (e.key === "o") {
      e.preventDefault();
      const input = document.createElement("input");
      input.type = "file";
      input.accept = ".fls";
      input.addEventListener("change", async () => {
        const file = input.files?.[0];
        if (!file) return;
        try {
          const text = await file.text();
          const snapshot = await decodeSnapshot(text);
          snapshotMode.load(snapshot);
        } catch (err) {
          alert(`Failed to load snapshot: ${err}`);
        }
      });
      input.click();
    }
  });

  const sidebarEl = document.getElementById("sidebar");
  let sidebarCtrl: ReturnType<typeof renderSidebar> | null = null;
  if (sidebarEl) {
    // ---- Tab bar ----
    const tabBar = document.createElement("div");
    tabBar.style.cssText = "display:flex;border-bottom:1px solid #2a2a2a;background:#141414;flex-shrink:0";

    function makeTab(label: string): HTMLButtonElement {
      const btn = document.createElement("button");
      btn.textContent = label;
      btn.style.cssText = "flex:1;padding:10px 4px;background:none;border:none;border-bottom:2px solid transparent;color:#777;font:12px 'JetBrains Mono','Consolas',monospace;cursor:pointer;letter-spacing:0.5px;transition:all 0.15s";
      return btn;
    }

    const tabGenerate = makeTab("Generate");
    const tabCorpus = makeTab("Corpus");
    tabBar.appendChild(tabGenerate);
    tabBar.appendChild(tabCorpus);

    const generatePanel = document.createElement("div");
    generatePanel.style.cssText = "flex:1;overflow:hidden;display:flex;flex-direction:column;";

    const corpusPanel = document.createElement("div");
    corpusPanel.style.cssText = "flex:1;overflow:hidden;display:none;flex-direction:column;";

    sidebarEl.style.cssText += ";display:flex;flex-direction:column;padding:0;overflow:hidden;";
    sidebarEl.appendChild(tabBar);
    sidebarEl.appendChild(generatePanel);
    sidebarEl.appendChild(corpusPanel);

    function switchTab(tab: "generate" | "corpus"): void {
      const isGenerate = tab === "generate";
      generatePanel.style.display = isGenerate ? "flex" : "none";
      corpusPanel.style.display = isGenerate ? "none" : "flex";
      tabGenerate.style.borderBottomColor = isGenerate ? "#569cd6" : "transparent";
      tabGenerate.style.color = isGenerate ? "#d4d4d4" : "#777";
      tabCorpus.style.borderBottomColor = isGenerate ? "transparent" : "#569cd6";
      tabCorpus.style.color = isGenerate ? "#777" : "#d4d4d4";
    }

    tabGenerate.onclick = () => switchTab("generate");
    tabCorpus.onclick = () => switchTab("corpus");
    switchTab("generate");

    sidebarCtrl = renderSidebar(generatePanel, engine, {
      renderGraph,
      renderLayout: renderLayoutOnCanvas,
    }, {
      getDebugMode: () => debugCb.checked,
      onDisplayToggles: (toggles: DisplayToggles) => {
        colorCb = toggles.colorCb;
        rateCb = toggles.rateCb;

        colorCb.addEventListener("change", () => {
          setItemColoring(colorCb.checked);
          if (!colorCb.checked) {
            legendEl.style.display = "none";
          } else if (lastLayout) {
            renderLayoutOnCanvas(lastLayout);
          }
        });
        rateCb.addEventListener("change", () => {
          setRateOverlay(rateCb.checked);
          if (lastLayout) renderLayoutOnCanvas(lastLayout);
        });
      },
    });

    // Wire overlay panel toggles
    debugCb.addEventListener("change", () => {
      stepThrough.reset();
      updateTraceOverlay();
      updateValidationOverlay();
      updateRegionOverlay();
    });
    stepCb.addEventListener("change", () => {
      stepThrough.reset();
      updateTraceOverlay();
    });
    valCb.addEventListener("change", updateValidationOverlay);
    regionsCb.addEventListener("change", updateRegionOverlay);

    soloRegionsCb.addEventListener("change", () => {
      if (soloRegionsCb.checked) {
        soloRegionsActive = true;
        soloSavedState = {
          colorChecked: colorCb.checked,
          rateChecked: rateCb.checked,
          valChecked: valCb.checked,
          regionsChecked: regionsCb.checked,
          entityAlpha: entityLayer.alpha,
        };

        if (!regionsCb.checked) {
          regionsCb.checked = true;
          updateRegionOverlay();
        }
        if (colorCb.checked) {
          colorCb.checked = false;
          setItemColoring(false);
          if (lastLayout) renderLayoutOnCanvas(lastLayout);
        }
        if (rateCb.checked) {
          rateCb.checked = false;
          setRateOverlay(false);
          if (lastLayout) renderLayoutOnCanvas(lastLayout);
        }
        if (valCb.checked) {
          valCb.checked = false;
          updateValidationOverlay();
        }

        entityLayer.alpha = 0.12;
        updateRegionOverlay();
      } else {
        soloRegionsActive = false;
        if (soloSavedState) {
          entityLayer.alpha = soloSavedState.entityAlpha;

          if (regionsCb.checked !== soloSavedState.regionsChecked) {
            regionsCb.checked = soloSavedState.regionsChecked;
            updateRegionOverlay();
          }
          if (valCb.checked !== soloSavedState.valChecked) {
            valCb.checked = soloSavedState.valChecked;
            updateValidationOverlay();
          }
          if (colorCb.checked !== soloSavedState.colorChecked) {
            colorCb.checked = soloSavedState.colorChecked;
            setItemColoring(colorCb.checked);
            if (lastLayout) renderLayoutOnCanvas(lastLayout);
          }
          if (rateCb.checked !== soloSavedState.rateChecked) {
            rateCb.checked = soloSavedState.rateChecked;
            setRateOverlay(rateCb.checked);
            if (lastLayout) renderLayoutOnCanvas(lastLayout);
          }

          soloSavedState = null;
        }
      }
    });

    initCorpusPanel(corpusPanel, renderLayoutOnCanvas);
  }
}

main().catch((err) => {
  console.error("Failed to initialize app:", err);
});
