import { Container } from "pixi.js";
import type { Graphics } from "pixi.js";
import { createApp, WORLD_SIZE } from "./renderer/app";
import { drawGrid } from "./renderer/grid";
import { drawGraph } from "./renderer/graph";
import { initEntityIcons, renderLayout, setItemColoring, setRateOverlay, itemColor, isBeltEntity, niceName, getRecipeFlows, TILE_PX, type HighlightController } from "./renderer/entities";
import { createSelectionController, type SelectionController } from "./renderer/selection";
import { renderSidebar, type DisplayToggles } from "./ui/sidebar";
import { initCorpusPanel } from "./ui/corpus";
import { renderLanding } from "./ui/landing";
import {
  setupSnapshotDropZone,
  showSnapshotBanner,
  decodeSnapshot,
  type LayoutSnapshot,
  type BannerCallbacks,
} from "./ui/snapshotLoader";
import { initEngine, getEngine } from "./engine";
import type { SolverResult, LayoutResult, PlacedEntity, ValidationIssue } from "./engine";
import { renderTraceOverlay, renderGhostRoutingOverlay, getTracePhases, eventsUpToPhase, type TraceEvent, type PhaseSnapshot } from "./renderer/traceOverlay";
import { renderValidationOverlay, VALIDATION_CIRCLE_ALPHA } from "./renderer/validationOverlay";
import { renderRegionOverlayDetailed, type RegionOverlayItem } from "./renderer/regionOverlay";
import { classLabel } from "./renderer/regionClassify";

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

  // Route: #/layout goes straight to the generator; anything else shows
  // the landing page.  Also support the legacy ?generator=true param.
  const appRoot = document.getElementById("app")!;
  const hash = window.location.hash;
  const params = new URLSearchParams(window.location.search);
  const skipLanding = hash.startsWith("#/layout") || params.has("generator");

  if (!skipLanding) {
    const landingHost = document.createElement("div");
    appRoot.appendChild(landingHost);

    renderLanding(landingHost, engine, {
      onOpenGenerator: () => {
        landingHost.remove();
        initGenerator(engine);
        // Persist layout route so refresh stays on the generator
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

  // Show sidebar + canvas (may be hidden behind landing page).
  // Explicitly set flex to ensure the two-column layout activates even if
  // the landing page previously hid #app or the HTML has a different default.
  const appRoot = document.getElementById("app")!;
  appRoot.style.display = "flex";
  const sidebar = document.getElementById("sidebar");
  if (sidebar) sidebar.style.display = "";
  container.style.display = "";

  const { app, viewport } = await createApp(container);
  drawGrid(viewport);
  drawGraph(viewport, null);

  // --- Snapshot drag-drop ---
  setupSnapshotDropZone(container, (snap) => loadSnapshot(snap));

  const entityLayer = new Container();
  viewport.addChild(entityLayer);
  viewport.moveCenter(WORLD_SIZE / 2, WORLD_SIZE / 2);

  // --- Hover tooltip ---
  const tooltip = document.createElement("div");
  tooltip.style.cssText = "position:fixed;background:#1e1e1e;color:#e0e0e0;border:1px solid #555;padding:4px 8px;font:12px monospace;pointer-events:none;border-radius:3px;display:none;z-index:1000;max-width:200px;line-height:1.5";
  document.body.appendChild(tooltip);

  document.addEventListener("mousemove", (e) => {
    tooltip.style.left = e.clientX + 14 + "px";
    tooltip.style.top = e.clientY - 10 + "px";
  });

  let highlightCtrl: HighlightController | null = null;

  /** Inline <img> tag for an item/entity icon */
  function iconTag(slug: string, size = 16): string {
    return `<img src="${import.meta.env.BASE_URL}icons/${slug}.png" width="${size}" height="${size}" style="vertical-align:middle;margin-right:3px;image-rendering:pixelated" onerror="this.style.display='none'">`;
  }

  function onSelect(entity: PlacedEntity | null): void {
    if (!entity) {
      infoPanel.style.display = "none";
      return;
    }
    const dirArrow: Record<string, string> = { North: "\u2191", East: "\u2192", South: "\u2193", West: "\u2190" };
    let html = `<div style="display:flex;justify-content:space-between;align-items:start">${iconTag(entity.name)}<b>${niceName(entity.name)}</b><span style="cursor:pointer;color:#888;margin-left:8px" id="info-close">\u00d7</span></div>`;
    if (entity.recipe) html += `<div style="color:#dcdcaa">${iconTag(entity.recipe)} ${niceName(entity.recipe)}</div>`;
    if (entity.rate != null) html += `<div style="color:#b5cea8">rate: ${entity.rate.toFixed(1)}/s</div>`;
    if (entity.carries) html += `<div style="color:#9cdcfe">${iconTag(entity.carries)} ${niceName(entity.carries)}</div>`;
    // Regular pipes are directionless (connect to all 4 neighbours) — the direction
    // field on them is meaningless data. Skip showing it to avoid confusion.
    if (entity.direction && entity.name !== "pipe") html += `<div>${dirArrow[entity.direction] ?? ""} ${entity.direction}</div>`;
    html += `<div style="color:#888">pos: ${entity.x ?? 0}, ${entity.y ?? 0}</div>`;
    infoPanel.innerHTML = html;
    infoPanel.style.display = "block";
    const closeBtn = document.getElementById("info-close");
    if (closeBtn) closeBtn.addEventListener("click", () => { infoPanel.style.display = "none"; });
  }

  function onHover(entity: PlacedEntity | null): void {
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

      // Belt entities → highlight the connected belt network (upstream dashed, downstream solid)
      // Everything else → highlight the item chain
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

  // --- Cursor tile position overlay ---
  // --- Canvas overlay controls (bottom-right, pinned) ---
  const overlayPanel = document.createElement("div");
  overlayPanel.style.cssText = "position:absolute;bottom:8px;right:8px;background:rgba(0,0,0,0.6);color:#aaa;font:11px monospace;padding:4px 8px;border-radius:3px;z-index:10;display:flex;flex-direction:column;gap:2px;user-select:none";
  container.style.position = "relative";

  const coordsEl = document.createElement("div");
  coordsEl.style.cssText = "color:#aaa;font:11px monospace;pointer-events:none";
  coordsEl.textContent = "x:\u2013 y:\u2013";
  overlayPanel.appendChild(coordsEl);

  function makeOverlayToggle(label: string, checked = false): HTMLInputElement {
    const cb = document.createElement("input");
    cb.type = "checkbox";
    cb.checked = checked;
    cb.style.cssText = "accent-color:#569cd6;width:12px;height:12px;margin:0;vertical-align:middle";
    const lbl = document.createElement("label");
    lbl.style.cssText = "display:flex;align-items:center;gap:4px;cursor:pointer;font-size:10px;color:#888";
    lbl.appendChild(cb);
    lbl.appendChild(document.createTextNode(label));
    overlayPanel.appendChild(lbl);
    return cb;
  }

  const debugCb = makeOverlayToggle("Debug");
  const valCb = makeOverlayToggle("Validation");
  const regionsCb = makeOverlayToggle("SAT Zones");
  const soloRegionsCb = makeOverlayToggle("Solo regions");
  const ghostCb = makeOverlayToggle("Ghost routes");
  const ghostModeCb = makeOverlayToggle("Ghost mode");

  // Initialize ghost mode from URL state. Auto-enables SAT zone overlay
  // when ghost is on so the user immediately sees the junction rectangles.
  {
    const initialGhost = new URLSearchParams(window.location.search).get("ghost") === "1";
    if (initialGhost) {
      ghostModeCb.checked = true;
      regionsCb.checked = true;
    }
  }

  container.appendChild(overlayPanel);

  // Sidebar toggles — populated via onDisplayToggles callback.
  let colorCb: HTMLInputElement;
  let rateCb: HTMLInputElement;

  // Solo-regions flag: true whenever solo mode is active (persists across re-renders)
  let soloRegionsActive = false;
  // Solo-regions state: saved toggle states before solo mode was enabled
  let soloSavedState: {
    colorChecked: boolean;
    rateChecked: boolean;
    valChecked: boolean;
    regionsChecked: boolean;
    entityAlpha: number;
  } | null = null;

  let traceOverlayLayer: Container | null = null;
  let tracePhaseIndex = -1; // -1 = show all phases
  let ghostOverlayLayer: Container | null = null;
  let ghostEntityAlphaSaved: number | null = null; // saved entity alpha before ghost mode
  let snapshotActive = false; // true when entities are from a PhaseSnapshot
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

  // --- Step-through controls ---
  const stepBar = document.createElement("div");
  stepBar.style.cssText = "position:absolute;top:60px;right:70px;background:rgba(0,0,0,0.6);color:#ccc;font:11px monospace;padding:2px 6px;border-radius:3px;z-index:10;display:none;align-items:center;gap:4px;user-select:none";
  const prevBtn = document.createElement("button");
  prevBtn.textContent = "\u25C0";
  prevBtn.style.cssText = "background:none;border:1px solid #555;color:#ccc;cursor:pointer;padding:0 4px;border-radius:2px;font-size:10px";
  const phaseLabel = document.createElement("span");
  phaseLabel.textContent = "all";
  phaseLabel.style.minWidth = "80px";
  phaseLabel.style.textAlign = "center";
  const nextBtn = document.createElement("button");
  nextBtn.textContent = "\u25B6";
  nextBtn.style.cssText = "background:none;border:1px solid #555;color:#ccc;cursor:pointer;padding:0 4px;border-radius:2px;font-size:10px";
  const failBtn = document.createElement("button");
  failBtn.style.cssText = "background:#442;color:#f66;border:1px solid #833;cursor:pointer;padding:0 6px;border-radius:2px;font-size:10px;display:none";
  stepBar.appendChild(prevBtn);
  stepBar.appendChild(phaseLabel);
  stepBar.appendChild(nextBtn);
  stepBar.appendChild(failBtn);
  container.appendChild(stepBar);

  function updateStepControls(): void {
    if (!debugCb.checked || !lastLayout?.trace?.length) {
      stepBar.style.display = "none";
      failBtn.style.display = "none";
      return;
    }
    const trace = lastLayout.trace as TraceEvent[];
    const phases = getTracePhases(trace);
    if (phases.length === 0) {
      stepBar.style.display = "none";
      failBtn.style.display = "none";
      return;
    }
    stepBar.style.display = "flex";

    // Compute cumulative PhaseTime durations
    const timeEvents = trace.filter(e => e.phase === "PhaseTime") as Extract<TraceEvent, { phase: "PhaseTime" }>[];
    // PhaseComplete events carry entity_count
    const completeEvents = trace.filter(e => e.phase === "PhaseComplete") as Extract<TraceEvent, { phase: "PhaseComplete" }>[];

    if (tracePhaseIndex < 0) {
      const totalMs = timeEvents.reduce((s, t) => s + t.data.duration_ms, 0);
      const totalEntities = completeEvents.length > 0 ? completeEvents[completeEvents.length - 1].data.entity_count : 0;
      phaseLabel.textContent = `all (${phases.length}) — ${totalEntities} entities, ${totalMs}ms`;
    } else {
      // Sum PhaseTime events up to and including the current phase's PhaseComplete index
      const phaseEndIdx = phases[tracePhaseIndex].eventIndex;
      let elapsedMs = 0;
      for (const t of timeEvents) {
        const tIdx = trace.indexOf(t as TraceEvent);
        if (tIdx <= phaseEndIdx) elapsedMs += t.data.duration_ms;
      }
      const entityCount = completeEvents.find(c => c.data.phase === phases[tracePhaseIndex].name)?.data.entity_count ?? 0;
      phaseLabel.textContent = `${phases[tracePhaseIndex].name} — ${entityCount} entities, ${elapsedMs}ms`;
    }
    prevBtn.disabled = tracePhaseIndex <= 0 && tracePhaseIndex !== -1;
    nextBtn.disabled = tracePhaseIndex >= phases.length - 1;

    // Failure badge
    const failCount = trace.filter(e => e.phase === "RouteFailure").length;
    if (failCount > 0) {
      failBtn.textContent = `\u26A0 ${failCount}`;
      failBtn.style.display = "inline-block";
    } else {
      failBtn.style.display = "none";
    }
  }

  function updateTraceOverlay(): void {
    if (traceOverlayLayer) {
      entityLayer.removeChild(traceOverlayLayer);
      traceOverlayLayer.destroy();
      traceOverlayLayer = null;
    }

    // Step-through: re-render entities from snapshot for the selected phase.
    const wantSnapshot = debugCb.checked && tracePhaseIndex >= 0 && !!lastLayout?.trace;
    const snapshot = wantSnapshot
      ? getSnapshotForPhase(lastLayout!.trace as TraceEvent[], tracePhaseIndex)
      : null;

    if (snapshot) {
      snapshotActive = true;
      highlightCtrl = renderLayout(
        { ...lastLayout!, entities: snapshot.entities, width: snapshot.width, height: snapshot.height },
        entityLayer, onHover, onSelect,
      );
      // Entity delta highlight: tint newly added entities green for 1 second
      const newKeys = new Set(snapshot.entities.map(entityKey));
      const prev = prevSnapshotEntities;
      if (prev) {
        const added = snapshot.entities.filter(e => !prev.has(entityKey(e)));
        const addedPositions = new Set(added.map(e => `${e.x},${e.y}`));
        for (const child of entityLayer.children) {
          // Skip overlay containers and non-graphics children
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
      // Was showing a snapshot — restore full entity rendering.
      snapshotActive = false;
      prevSnapshotEntities = null;
      if (lastLayout) {
        highlightCtrl = renderLayout(lastLayout, entityLayer, onHover, onSelect);
      }
    }

    if (!debugCb.checked || !lastLayout?.trace?.length) {
      updateStepControls();
      return;
    }
    const events = tracePhaseIndex < 0
      ? (lastLayout.trace as TraceEvent[])
      : eventsUpToPhase(lastLayout.trace as TraceEvent[], tracePhaseIndex);
    traceOverlayLayer = renderTraceOverlay(
      events,
      lastLayout.width ?? 0,
      lastLayout.height ?? 0,
      entityLayer,
      (text) => {
        if (text) {
          tooltip.innerHTML = `<span style="color:#8af">TRACE</span> ${text}`;
          tooltip.style.display = "block";
        } else {
          tooltip.style.display = "none";
        }
      },
    );
    updateStepControls();
  }

  prevBtn.addEventListener("click", () => {
    const phases = getTracePhases((lastLayout?.trace ?? []) as TraceEvent[]);
    if (tracePhaseIndex === -1) tracePhaseIndex = phases.length - 1;
    else if (tracePhaseIndex > 0) tracePhaseIndex--;
    updateTraceOverlay();
  });
  nextBtn.addEventListener("click", () => {
    const phases = getTracePhases((lastLayout?.trace ?? []) as TraceEvent[]);
    if (tracePhaseIndex < phases.length - 1) tracePhaseIndex++;
    updateTraceOverlay();
  });

  // --- Jump to first route failure ---
  function jumpToFirstFailure(): void {
    if (!lastLayout?.trace) return;
    const failures = (lastLayout.trace as TraceEvent[]).filter(e => e.phase === "RouteFailure") as Extract<TraceEvent, { phase: "RouteFailure" }>[];
    if (failures.length === 0) return;
    const first = failures[0].data;
    const targetX = first.from_x * TILE_PX + TILE_PX / 2;
    const targetY = first.from_y * TILE_PX + TILE_PX / 2;
    viewport.moveCenter(targetX, targetY);
    // Pulse the first RouteFailure marker in the overlay
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
  }

  failBtn.addEventListener("click", jumpToFirstFailure);

  // --- Keyboard shortcuts for step-through ---
  document.addEventListener("keydown", (e) => {
    // Don't fire when typing in inputs
    const tag = (e.target as HTMLElement)?.tagName;
    if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;
    // Only when step bar is visible
    if (stepBar.style.display === "none") return;

    if (e.key === "ArrowLeft") {
      e.preventDefault();
      prevBtn.click();
    } else if (e.key === "ArrowRight") {
      e.preventDefault();
      nextBtn.click();
    } else if (e.key === "f") {
      e.preventDefault();
      jumpToFirstFailure();
    }
  });

  let valOverlayLayer: Container | null = null;
  let valCircleMap: Map<string, Graphics[]> = new Map();
  let cachedValidationIssues: ValidationIssue[] | null = null;

  let regionOverlayLayer: Container | null = null;
  let regionOverlayItems: RegionOverlayItem[] = [];
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
    clearPulse();

    // Ensure issues are computed whenever we have a layout (not just when overlay is on).
    if (lastLayout && !cachedValidationIssues) {
      try {
        cachedValidationIssues = engine.validateLayout(lastLayout, null);
      } catch {
        cachedValidationIssues = [];
      }
    }

    // Always update the sidebar validation list regardless of valCb state.
    sidebarCtrl?.updateValidation(cachedValidationIssues ?? [], panToTile);

    if (!valCb.checked || !lastLayout) {
      populateIssuesPanel(cachedValidationIssues ?? []);
      return;
    }
    if (!cachedValidationIssues || cachedValidationIssues.length === 0) {
      populateIssuesPanel([]);
      return;
    }
    const result = renderValidationOverlay(
      cachedValidationIssues,
      entityLayer,
      (text) => {
        if (text) {
          tooltip.innerHTML = `<span style="color:#f44">VALIDATION</span> ${text}`;
          tooltip.style.display = "block";
        } else {
          tooltip.style.display = "none";
        }
      },
    );
    valOverlayLayer = result.layer;
    valCircleMap = result.circleMap;
    populateIssuesPanel(cachedValidationIssues);
  }

  function updateRegionOverlay(): void {
    if (regionOverlayLayer) {
      entityLayer.removeChild(regionOverlayLayer);
      regionOverlayLayer.destroy();
      regionOverlayLayer = null;
    }
    regionOverlayItems = [];
    regionHitTest = null;
    updateRegionHud();
    if (!regionsCb?.checked || !lastLayout) return;
    if (!lastLayout.regions || lastLayout.regions.length === 0) return;
    const detailed = renderRegionOverlayDetailed(lastLayout);
    regionOverlayLayer = detailed.layer;
    regionOverlayItems = detailed.items;
    regionHitTest = detailed.hitTest;
    entityLayer.addChild(regionOverlayLayer);
    updateRegionHud();
  }

  function updateGhostOverlay(): void {
    if (ghostOverlayLayer) {
      viewport.removeChild(ghostOverlayLayer);
      ghostOverlayLayer.destroy();
      ghostOverlayLayer = null;
    }

    if (!ghostCb?.checked) {
      // Restore entity layer alpha when ghost mode is off
      if (ghostEntityAlphaSaved !== null) {
        entityLayer.alpha = ghostEntityAlphaSaved;
        ghostEntityAlphaSaved = null;
      }
      return;
    }

    if (!lastLayout?.trace?.length) return;
    const events = lastLayout.trace as TraceEvent[];
    const hasGhostEvents = events.some(e =>
      e.phase === "GhostSpecRouted" ||
      e.phase === "GhostSpecFailed" ||
      e.phase === "GhostClusterSolved" ||
      e.phase === "GhostClusterFailed" ||
      e.phase === "GhostRoutingComplete",
    );
    if (!hasGhostEvents) return;

    // Save and hide entity layer
    if (ghostEntityAlphaSaved === null) {
      ghostEntityAlphaSaved = entityLayer.alpha;
    }
    entityLayer.alpha = 0;

    ghostOverlayLayer = renderGhostRoutingOverlay(
      events,
      lastLayout.width ?? 0,
      lastLayout.height ?? 0,
      viewport,
      (text) => {
        if (text) {
          tooltip.innerHTML = `<span style="color:#8af">GHOST</span> ${text}`;
          tooltip.style.display = "block";
        } else {
          tooltip.style.display = "none";
        }
      },
    );
  }

  // --- Item color legend (bottom-left) ---
  const legendEl = document.createElement("div");
  legendEl.style.cssText = "position:absolute;bottom:8px;left:8px;background:rgba(0,0,0,0.6);color:#ccc;font:11px monospace;padding:4px 8px;border-radius:3px;pointer-events:none;z-index:10;display:none;max-height:300px;overflow-y:auto";
  container.appendChild(legendEl);

  // --- Ghost mode HUD (top-center) ---
  // Compact stats block showing region counts by kind and by class.
  // Only visible when ghost mode is on.
  const ghostHud = document.createElement("div");
  ghostHud.style.cssText = "position:absolute;top:8px;left:50%;transform:translateX(-50%);background:rgba(10,10,15,0.85);color:#ccc;font:11px monospace;padding:6px 10px;border-radius:4px;border:1px solid #333;z-index:10;display:none;white-space:nowrap;pointer-events:none;box-shadow:0 2px 8px rgba(0,0,0,0.5)";
  container.appendChild(ghostHud);

  // --- Region detail panel (bottom-right) ---
  // Shown on hover over a region when ghost mode is on. Displays the
  // engine-assigned kind, classifier output, dimensions, item breakdown,
  // and SAT stats if applicable.
  const regionDetail = document.createElement("div");
  regionDetail.style.cssText = "position:absolute;bottom:8px;right:8px;background:rgba(10,10,15,0.9);color:#ddd;font:11px monospace;padding:8px 12px;border-radius:4px;border:1px solid #444;z-index:10;display:none;max-width:360px;pointer-events:none;box-shadow:0 4px 12px rgba(0,0,0,0.6);line-height:1.5";
  container.appendChild(regionDetail);

  function updateRegionHud(): void {
    if (!ghostModeCb?.checked) {
      ghostHud.style.display = "none";
      return;
    }
    const items = regionOverlayItems;
    if (items.length === 0) {
      ghostHud.textContent = "ghost: 0 regions";
      ghostHud.style.display = "block";
      return;
    }

    const kindCounts = new Map<string, number>();
    const classCounts = new Map<string, number>();
    for (const it of items) {
      kindCounts.set(it.region.kind, (kindCounts.get(it.region.kind) ?? 0) + 1);
      const c = it.classification.cls;
      classCounts.set(c, (classCounts.get(c) ?? 0) + 1);
    }

    // Overlap pair count — bbox-overlap test, same as report_zone_overlaps.
    let overlapPairs = 0;
    for (let i = 0; i < items.length; i++) {
      const a = items[i].region;
      for (let j = i + 1; j < items.length; j++) {
        const b = items[j].region;
        const xOverlap = a.x < b.x + b.width && b.x < a.x + a.width;
        const yOverlap = a.y < b.y + b.height && b.y < a.y + a.height;
        if (xOverlap && yOverlap) overlapPairs++;
      }
    }

    const fmtMap = (m: Map<string, number>, fmt: (k: string) => string) =>
      [...m.entries()]
        .sort((a, b) => b[1] - a[1])
        .map(([k, v]) => `${fmt(k)} ${v}`)
        .join("  ");

    const kindStr = fmtMap(kindCounts, k => k.replace("_template", "").replace("ghost_", ""));
    const classStr = fmtMap(classCounts, k => classLabel(k as Parameters<typeof classLabel>[0]));
    const overlapStr = overlapPairs > 0 ? `  |  ⚠ ${overlapPairs} overlap` : "";

    ghostHud.innerHTML = `<span style="color:#8bf">ghost</span> ${items.length} regions  |  kind: ${kindStr}  |  class: ${classStr}${overlapStr}`;
    ghostHud.style.display = "block";
  }

  function showRegionDetail(item: RegionOverlayItem | null): void {
    if (!item) {
      regionDetail.style.display = "none";
      return;
    }
    const r = item.region;
    const c = item.classification;
    const satStats = r.solve_time_us > 0
      ? `<br><span style="color:#999">SAT: ${r.variables} vars, ${r.clauses} clauses, ${r.solve_time_us}µs</span>`
      : "";
    const portStr = r.ports && r.ports.length > 0 ? `${r.ports.length} ports` : "no ports";
    const itemList = [...c.items.values()]
      .map(ip => `${ip.name} (${ip.axis}, ${ip.inputs.length}in/${ip.outputs.length}out)`)
      .join("<br>&nbsp;&nbsp;");

    regionDetail.innerHTML = `
      <div style="color:#8bf;margin-bottom:4px"><b>${r.kind}</b> → ${classLabel(c.cls)}</div>
      <div style="color:#aaa">(${r.x}, ${r.y})  ${r.width}×${r.height}  ${portStr}</div>
      <div style="margin-top:6px;color:#ddd">${c.summary}</div>
      ${itemList ? `<div style="margin-top:6px;color:#bbb"><b>items:</b><br>&nbsp;&nbsp;${itemList}</div>` : ""}
      ${satStats}
    `;
    regionDetail.style.display = "block";
  }

  let pinnedRow: HTMLDivElement | null = null;

  function unpinRow(): void {
    if (pinnedRow) {
      pinnedRow.style.background = "";
      pinnedRow = null;
    }
    clearPulse();
  }

  // Escape unpins; clicking outside the issues panel unpins.
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") unpinRow();
  });
  document.addEventListener("pointerdown", (e) => {
    if (pinnedRow && !issuesPanel.contains(e.target as Node)) unpinRow();
  });

  // Shift held → pause viewport drag so selection box works.
  document.addEventListener("keydown", (e) => {
    if (e.key === "Shift") viewport.plugins.pause("drag");
  });
  document.addEventListener("keyup", (e) => {
    if (e.key === "Shift") viewport.plugins.resume("drag");
  });
  // Also handle shift release when window loses focus.
  window.addEventListener("blur", () => viewport.plugins.resume("drag"));

  // --- Validation issues floating dialog ---
  const issuesPanel = document.createElement("div");
  issuesPanel.style.cssText = "position:absolute;top:8px;right:8px;background:#1a1a1a;color:#e0e0e0;font:11px monospace;border-radius:4px;border:1px solid #333;z-index:10;display:none;max-width:360px;max-height:calc(100% - 24px);overflow:hidden;box-shadow:0 4px 20px rgba(0,0,0,0.5);flex-direction:column";
  container.appendChild(issuesPanel);

  // Title bar for dragging
  const issuesTitleBar = document.createElement("div");
  issuesTitleBar.style.cssText = "display:flex;align-items:center;justify-content:space-between;padding:6px 10px;background:#222;border-bottom:1px solid #333;cursor:move;user-select:none;flex-shrink:0";
  const issuesTitleText = document.createElement("span");
  issuesTitleText.style.cssText = "font-size:11px;font-weight:600;color:#888;text-transform:uppercase;letter-spacing:0.8px";
  issuesTitleText.textContent = "Validation";
  issuesTitleBar.appendChild(issuesTitleText);
  const issuesCountBadge = document.createElement("span");
  issuesCountBadge.style.cssText = "font-size:10px;color:#f66;background:rgba(255,68,68,0.12);padding:1px 6px;border-radius:3px;margin-left:8px";
  issuesTitleBar.appendChild(issuesCountBadge);
  const issuesCloseBtn = document.createElement("span");
  issuesCloseBtn.style.cssText = "cursor:pointer;color:#666;font-size:14px;line-height:1;padding:0 2px";
  issuesCloseBtn.textContent = "\u00d7";
  issuesCloseBtn.addEventListener("click", () => {
    valCb.checked = false;
    updateValidationOverlay();
  });
  issuesTitleBar.appendChild(issuesCloseBtn);
  issuesPanel.appendChild(issuesTitleBar);

  const issuesBody = document.createElement("div");
  issuesBody.style.cssText = "overflow-y:auto;max-height:calc(100% - 32px);padding:4px 8px;line-height:1.4";
  issuesPanel.appendChild(issuesBody);

  // Make the dialog draggable
  {
    let dragging = false;
    let offsetX = 0;
    let offsetY = 0;
    issuesTitleBar.addEventListener("pointerdown", (e) => {
      if ((e.target as HTMLElement) === issuesCloseBtn) return;
      dragging = true;
      const rect = issuesPanel.getBoundingClientRect();
      const containerRect = container.getBoundingClientRect();
      offsetX = e.clientX - rect.left + containerRect.left;
      offsetY = e.clientY - rect.top + containerRect.top;
      issuesTitleBar.setPointerCapture(e.pointerId);
      e.preventDefault();
    });
    issuesTitleBar.addEventListener("pointermove", (e) => {
      if (!dragging) return;
      const x = e.clientX - offsetX;
      const y = e.clientY - offsetY;
      issuesPanel.style.left = `${x}px`;
      issuesPanel.style.top = `${y}px`;
      issuesPanel.style.right = "auto";
    });
    issuesTitleBar.addEventListener("pointerup", () => { dragging = false; });
  }

  // Pulse state: tracks the markers being pulsed and the Pixi ticker callback.
  let activePulse: { markers: Graphics[]; tickerFn: () => void } | null = null;

  function clearPulse(): void {
    if (activePulse) {
      for (const m of activePulse.markers) m.alpha = VALIDATION_CIRCLE_ALPHA;
      app.ticker.remove(activePulse.tickerFn);
      activePulse = null;
    }
  }

  function pulseCircle(key: string): void {
    clearPulse();
    const markers = valCircleMap.get(key);
    if (!markers || markers.length === 0) return;
    // Toggle alpha every ~150ms using the Pixi ticker so the pulse is synced
    // with the render loop rather than an independent setInterval.
    let elapsed = 0;
    let on = true;
    const tickerFn = (): void => {
      elapsed += app.ticker.deltaMS;
      if (elapsed >= 150) {
        elapsed -= 150;
        on = !on;
        const alpha = on ? 1.0 : 0.35;
        for (const m of markers) m.alpha = alpha;
      }
    };
    app.ticker.add(tickerFn);
    activePulse = { markers, tickerFn };
  }

  function populateIssuesPanel(issues: ValidationIssue[]): void {
    issuesBody.innerHTML = "";
    pinnedRow = null;
    clearPulse();
    if (!valCb.checked || issues.length === 0) {
      issuesPanel.style.display = "none";
      return;
    }
    issuesPanel.style.display = "flex";
    const errors = issues.filter(i => i.severity === "Error").length;
    const warns = issues.length - errors;
    issuesCountBadge.textContent = errors > 0 ? `${errors} error${errors > 1 ? "s" : ""}` : `${warns} warning${warns > 1 ? "s" : ""}`;
    issuesCountBadge.style.color = errors > 0 ? "#f66" : "#fa0";
    issuesCountBadge.style.background = errors > 0 ? "rgba(255,68,68,0.12)" : "rgba(255,170,0,0.12)";
    for (const issue of issues) {
      const row = document.createElement("div");
      row.style.cssText = "padding:3px 0;border-bottom:1px solid #333;cursor:default;display:flex;align-items:baseline;gap:6px;user-select:text";
      if (issue.x == null || issue.y == null) {
        row.style.opacity = "0.6";
      }
      const dot = document.createElement("span");
      dot.style.cssText = `display:inline-block;width:8px;height:8px;border-radius:50%;flex-shrink:0;background:${issue.severity === "Error" ? "#f44" : "#fa0"}`;
      row.appendChild(dot);
      const cat = document.createElement("span");
      cat.style.cssText = `color:${issue.severity === "Error" ? "#f66" : "#fa0"};flex-shrink:0`;
      cat.textContent = issue.category;
      row.appendChild(cat);
      const msg = document.createElement("span");
      msg.style.cssText = "color:#ccc";
      msg.textContent = issue.message;
      row.appendChild(msg);
      if (issue.x != null && issue.y != null) {
        row.style.cursor = "pointer";
        const key = `${issue.x},${issue.y}`;
        row.addEventListener("mouseenter", () => {
          if (pinnedRow === row) return;
          viewport.moveCenter(issue.x! * TILE_PX + TILE_PX / 2, issue.y! * TILE_PX + TILE_PX / 2);
          pulseCircle(key);
        });
        row.addEventListener("mouseleave", () => {
          if (pinnedRow === row) return;
          clearPulse();
        });
        row.addEventListener("click", (e) => {
          e.stopPropagation();
          if (pinnedRow === row) {
            unpinRow();
          } else {
            unpinRow();
            pinnedRow = row;
            row.style.background = "rgba(255,255,255,0.08)";
            viewport.moveCenter(issue.x! * TILE_PX + TILE_PX / 2, issue.y! * TILE_PX + TILE_PX / 2);
            pulseCircle(key);
          }
        });
      }
      issuesBody.appendChild(row);
    }
  }

  // --- Machine info panel (click) ---
  const infoPanel = document.createElement("div");
  infoPanel.style.cssText = "position:absolute;top:8px;left:8px;background:rgba(0,0,0,0.8);color:#e0e0e0;font:12px monospace;padding:8px 10px;border-radius:4px;border:1px solid #555;z-index:10;display:none;max-width:250px;line-height:1.5";
  container.appendChild(infoPanel);

  // --- Selection annotation bar ---
  const annotationBar = document.createElement("div");
  annotationBar.style.cssText = "position:absolute;bottom:34px;left:8px;background:rgba(0,0,0,0.8);color:#e0e0e0;font:11px monospace;padding:6px 8px;border-radius:3px;border:1px solid #00e0a0;z-index:10;display:none;min-width:200px";
  container.appendChild(annotationBar);

  const annotationCount = document.createElement("div");
  annotationCount.style.cssText = "color:#00e0a0;margin-bottom:4px";
  annotationBar.appendChild(annotationCount);

  const annotationNote = document.createElement("textarea");
  annotationNote.placeholder = "Add a note…";
  annotationNote.rows = 2;
  annotationNote.style.cssText = "width:100%;box-sizing:border-box;background:#2a2a2a;color:#e0e0e0;border:1px solid #555;border-radius:2px;font:11px monospace;resize:vertical;margin-bottom:4px";
  annotationBar.appendChild(annotationNote);

  const annotationHint = document.createElement("div");
  annotationHint.style.cssText = "color:#777";
  annotationHint.textContent = "Ctrl+C to copy JSON";
  annotationBar.appendChild(annotationHint);

  let lastLayout: LayoutResult | null = null;
  let selectionCtrl: SelectionController | null = null;
  let activeBanner: HTMLDivElement | null = null;

  function clearSnapshotBanner(): void {
    if (activeBanner) {
      activeBanner.remove();
      activeBanner = null;
    }
    // Re-enable sidebar
    const sidebarEl = document.getElementById("sidebar");
    sidebarEl?.querySelectorAll("input,select,button").forEach((el) => {
      (el as HTMLInputElement).disabled = false;
    });
  }

  function loadSnapshot(snapshot: LayoutSnapshot): void {
    // Build a LayoutResult from snapshot data (inject trace events)
    const layout: LayoutResult = {
      ...snapshot.layout,
      trace: snapshot.trace.events as LayoutResult["trace"],
    } as LayoutResult;

    // Auto-enable debug + validation if there are issues or trace events
    if (snapshot.trace.events.length > 0) {
      debugCb.checked = true;
    }
    if (snapshot.validation.issues.length > 0) {
      valCb.checked = true;
    }

    renderLayoutOnCanvas(layout);

    // Override cached issues with the snapshot's pre-computed validation
    // (renderLayoutOnCanvas resets cachedValidationIssues, so set it after).
    if (snapshot.validation.issues.length > 0) {
      cachedValidationIssues = snapshot.validation.issues as unknown as ValidationIssue[];
      // Re-render the overlay and sidebar list with the snapshot issues.
      updateValidationOverlay();
    }

    // Pre-fill sidebar form with snapshot params. Skip auto-solve —
    // otherwise the debounced runSolve fires ~150ms later and wipes the
    // rendered entities when it redraws the DAG.
    sidebarCtrl?.setParams(snapshot.params, { skipAutoSolve: true });

    // Show banner
    clearSnapshotBanner();
    const bannerCallbacks: BannerCallbacks = {
      onClear: () => {
        clearSnapshotBanner();
        entityLayer.removeChildren();
        lastLayout = null;
        cachedValidationIssues = null;
        drawGraph(viewport, null);
        viewport.moveCenter(WORLD_SIZE / 2, WORLD_SIZE / 2);
        legendEl.style.display = "none";
        infoPanel.style.display = "none";
        issuesPanel.style.display = "none";
        populateIssuesPanel([]);
        sidebarCtrl?.updateValidation([], panToTile);
      },
    };
    const sidebarEl = document.getElementById("sidebar");
    if (sidebarEl) {
      activeBanner = showSnapshotBanner(sidebarEl, snapshot, bannerCallbacks);
      // Disable recipe/layout controls in snapshot mode, but keep display
      // toggles (SAT Zones, Solo regions, etc.) fully interactive.
      sidebarEl.querySelectorAll("input,select,button").forEach((el) => {
        if (el.closest("[data-snapshot-keep]")) return;
        (el as HTMLInputElement).disabled = true;
      });
    }
  }

  function onSelectionChange(entities: PlacedEntity[]): void {
    if (entities.length === 0) {
      annotationBar.style.display = "none";
      annotationNote.value = "";
    } else {
      annotationCount.textContent = `${entities.length} entit${entities.length === 1 ? "y" : "ies"} selected`;
      annotationBar.style.display = "block";
    }
  }

  let hoveredRegion: RegionOverlayItem | null = null;
  app.canvas.addEventListener("pointermove", (e) => {
    const rect = app.canvas.getBoundingClientRect();
    const sx = e.clientX - rect.left;
    const sy = e.clientY - rect.top;
    const world = viewport.toWorld(sx, sy);
    const tx = Math.floor(world.x / TILE_PX);
    const ty = Math.floor(world.y / TILE_PX);
    coordsEl.textContent = `x:${tx} y:${ty}`;

    // Region hover — only when ghost mode + region overlay are active.
    if (regionHitTest && ghostModeCb.checked && regionsCb.checked) {
      const it = regionHitTest(world.x, world.y);
      if (it !== hoveredRegion) {
        hoveredRegion = it;
        showRegionDetail(it);
      }
    } else if (hoveredRegion) {
      hoveredRegion = null;
      showRegionDetail(null);
    }
  });

  // Click-to-pan for a region.
  app.canvas.addEventListener("pointerdown", (e) => {
    if (!regionHitTest || !ghostModeCb.checked || !regionsCb.checked) return;
    // Only the primary button, without modifiers.
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

  function renderGraph(result: SolverResult | null): void {
    // Clear the entity layer when the DAG is redrawn — we've solved again.
    entityLayer.removeChildren();
    drawGraph(viewport, result);
    legendEl.style.display = "none";
    infoPanel.style.display = "none";
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
    tracePhaseIndex = -1;
    snapshotActive = false;
    prevSnapshotEntities = null;
    // Destroy previous selection controller (new layout = new tile map)
    if (selectionCtrl) { selectionCtrl.destroy(); selectionCtrl = null; }
    annotationBar.style.display = "none";
    annotationNote.value = "";
    // Clear cached validation state for new layout
    cachedValidationIssues = null;
    // Replace the DAG with the actual bus layout.
    drawGraph(viewport, null);
    highlightCtrl = renderLayout(layout, entityLayer, onHover, onSelect);
    selectionCtrl = createSelectionController(app.canvas, viewport, entityLayer, layout, onSelectionChange);
    buildLegend(layout);
    updateTraceOverlay();
    updateValidationOverlay();
    updateRegionOverlay();
    // Reset ghost overlay alpha tracking when a new layout is loaded
    ghostEntityAlphaSaved = null;
    updateGhostOverlay();
    const w = layout.width ?? 0;
    const h = layout.height ?? 0;
    if (w > 0 && h > 0) {
      const pxW = w * 32;
      const pxH = h * 32;
      viewport.fit(true, pxW * 1.1, pxH * 1.2);
      viewport.moveCenter(pxW / 2, pxH / 2);
    }
    // Re-apply solo-regions dimming after entity layer rebuild
    if (soloRegionsActive) {
      entityLayer.alpha = 0.12;
    }
  }

  // Ctrl+C: copy selection JSON when entities are selected
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
      // Ctrl+O: open snapshot file picker
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
          loadSnapshot(snapshot);
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

    // ---- Panels ----
    const generatePanel = document.createElement("div");
    generatePanel.style.cssText = "flex:1;overflow:hidden;display:flex;flex-direction:column;";

    const corpusPanel = document.createElement("div");
    corpusPanel.style.cssText = "flex:1;overflow:hidden;display:none;flex-direction:column;";

    // Make sidebar a flex column
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
      getGhostMode: () => ghostModeCb.checked,
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

    // Wire overlay panel toggles (created above, independent of sidebar)
    debugCb.addEventListener("change", () => {
      tracePhaseIndex = -1;
      updateTraceOverlay();
    });
    valCb.addEventListener("change", updateValidationOverlay);
    regionsCb.addEventListener("change", updateRegionOverlay);
    ghostCb.addEventListener("change", updateGhostOverlay);
    ghostModeCb.addEventListener("change", () => {
      // Toggling ghost mode re-runs layout with the alternate router.
      // Also auto-enables the region overlay so the user sees junctions.
      if (ghostModeCb.checked && !regionsCb.checked) {
        regionsCb.checked = true;
      }
      updateRegionHud();
      showRegionDetail(null);
      // Re-run the layout by clicking the solve button's layout generation.
      // The sidebar reads getGhostMode() on each layoutBtn click.
      (generatePanel.querySelector(".sb-btn-primary") as HTMLButtonElement | null)?.click();
    });

    soloRegionsCb.addEventListener("change", () => {
      if (soloRegionsCb.checked) {
        soloRegionsActive = true;
        // Entering solo mode: save current state
        soloSavedState = {
          colorChecked: colorCb.checked,
          rateChecked: rateCb.checked,
          valChecked: valCb.checked,
          regionsChecked: regionsCb.checked,
          entityAlpha: entityLayer.alpha,
        };

        // Turn on SAT zones
        if (!regionsCb.checked) {
          regionsCb.checked = true;
          updateRegionOverlay();
        }

        // Hide item colours
        if (colorCb.checked) {
          colorCb.checked = false;
          setItemColoring(false);
          if (lastLayout) renderLayoutOnCanvas(lastLayout);
        }

        // Hide rate labels
        if (rateCb.checked) {
          rateCb.checked = false;
          setRateOverlay(false);
          if (lastLayout) renderLayoutOnCanvas(lastLayout);
        }

        // Hide validation overlay
        if (valCb.checked) {
          valCb.checked = false;
          updateValidationOverlay();
        }

        // Dim entity layer
        entityLayer.alpha = 0.12;

        // Ensure region overlay is on top after re-render
        updateRegionOverlay();
      } else {
        soloRegionsActive = false;
        // Exiting solo mode: restore previous state
        if (soloSavedState) {
          entityLayer.alpha = soloSavedState.entityAlpha;

          // Restore regions checkbox
          if (regionsCb.checked !== soloSavedState.regionsChecked) {
            regionsCb.checked = soloSavedState.regionsChecked;
            updateRegionOverlay();
          }

          // Restore validation
          if (valCb.checked !== soloSavedState.valChecked) {
            valCb.checked = soloSavedState.valChecked;
            updateValidationOverlay();
          }

          // Restore item colours
          if (colorCb.checked !== soloSavedState.colorChecked) {
            colorCb.checked = soloSavedState.colorChecked;
            setItemColoring(colorCb.checked);
            if (lastLayout) renderLayoutOnCanvas(lastLayout);
          }

          // Restore rate labels
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
