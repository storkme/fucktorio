import { Container } from "pixi.js";
import type { Graphics } from "pixi.js";
import { createApp, WORLD_SIZE } from "./renderer/app";
import { drawGrid } from "./renderer/grid";
import { drawGraph } from "./renderer/graph";
import { initEntityIcons, renderLayout, setItemColoring, setRateOverlay, itemColor, isBeltEntity, niceName, getRecipeFlows, TILE_PX, type HighlightController } from "./renderer/entities";
import { createSelectionController, type SelectionController } from "./renderer/selection";
import { renderSidebar } from "./ui/sidebar";
import { initCorpusPanel } from "./ui/corpus";
import {
  setupSnapshotDropZone,
  showSnapshotBanner,
  decodeSnapshot,
  type LayoutSnapshot,
  type BannerCallbacks,
} from "./ui/snapshotLoader";
import { initEngine, getEngine } from "./engine";
import type { SolverResult, LayoutResult, PlacedEntity, ValidationIssue } from "./engine";
import { renderTraceOverlay, getTracePhases, eventsUpToPhase, type TraceEvent, type PhaseSnapshot } from "./renderer/traceOverlay";
import { renderValidationOverlay, VALIDATION_CIRCLE_ALPHA } from "./renderer/validationOverlay";

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

  const container = document.getElementById("canvas-container");
  if (!container) throw new Error("Missing #canvas-container element");

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
    return `<img src="/icons/${slug}.png" width="${size}" height="${size}" style="vertical-align:middle;margin-right:3px;image-rendering:pixelated" onerror="this.style.display='none'">`;
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
    if (entity.direction) html += `<div>${dirArrow[entity.direction] ?? ""} ${entity.direction}</div>`;
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
      if (entity.direction) html += `<br>${dirArrow[entity.direction] ?? ""} ${entity.direction}`;
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
  const coordsEl = document.createElement("div");
  coordsEl.style.cssText = "position:absolute;bottom:8px;right:8px;background:rgba(0,0,0,0.6);color:#aaa;font:11px monospace;padding:3px 7px;border-radius:3px;pointer-events:none;z-index:10";
  coordsEl.textContent = "x:\u2013 y:\u2013";
  container.style.position = "relative";
  container.appendChild(coordsEl);

  // --- Item colour toggle (top-right of canvas) ---
  const colorToggle = document.createElement("label");
  colorToggle.style.cssText = "position:absolute;top:8px;right:8px;background:rgba(0,0,0,0.6);color:#ccc;font:11px monospace;padding:4px 8px;border-radius:3px;cursor:pointer;z-index:10;display:flex;align-items:center;gap:5px;user-select:none";
  const colorCb = document.createElement("input");
  colorCb.type = "checkbox";
  colorCb.checked = true;
  colorCb.style.accentColor = "#569cd6";
  colorToggle.appendChild(colorCb);
  colorToggle.appendChild(document.createTextNode("Item colours"));
  container.appendChild(colorToggle);

  // --- Rate overlay toggle ---
  const rateToggle = document.createElement("label");
  rateToggle.style.cssText = "position:absolute;top:34px;right:8px;background:rgba(0,0,0,0.6);color:#ccc;font:11px monospace;padding:4px 8px;border-radius:3px;cursor:pointer;z-index:10;display:flex;align-items:center;gap:5px;user-select:none";
  const rateCb = document.createElement("input");
  rateCb.type = "checkbox";
  rateCb.checked = false;
  rateCb.style.accentColor = "#569cd6";
  rateToggle.appendChild(rateCb);
  rateToggle.appendChild(document.createTextNode("Rates"));
  container.appendChild(rateToggle);

  // --- Debug trace toggle ---
  const debugToggle = document.createElement("label");
  debugToggle.style.cssText = "position:absolute;top:60px;right:8px;background:rgba(0,0,0,0.6);color:#ccc;font:11px monospace;padding:4px 8px;border-radius:3px;cursor:pointer;z-index:10;display:flex;align-items:center;gap:5px;user-select:none";
  const debugCb = document.createElement("input");
  debugCb.type = "checkbox";
  debugCb.checked = false;
  debugCb.style.accentColor = "#569cd6";
  debugToggle.appendChild(debugCb);
  debugToggle.appendChild(document.createTextNode("Debug"));
  container.appendChild(debugToggle);

  let traceOverlayLayer: Container | null = null;
  let tracePhaseIndex = -1; // -1 = show all phases
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

  debugCb.addEventListener("change", () => {
    tracePhaseIndex = -1;
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

  // --- Validation overlay toggle ---
  const valToggle = document.createElement("label");
  valToggle.style.cssText = "position:absolute;top:86px;right:8px;background:rgba(0,0,0,0.6);color:#ccc;font:11px monospace;padding:4px 8px;border-radius:3px;cursor:pointer;z-index:10;display:flex;align-items:center;gap:5px;user-select:none";
  const valCb = document.createElement("input");
  valCb.type = "checkbox";
  valCb.checked = false;
  valCb.style.accentColor = "#569cd6";
  valToggle.appendChild(valCb);
  valToggle.appendChild(document.createTextNode("Validation"));
  container.appendChild(valToggle);

  let valOverlayLayer: Container | null = null;
  let valCircleMap: Map<string, Graphics[]> = new Map();
  let cachedValidationIssues: ValidationIssue[] | null = null;

  function updateValidationOverlay(): void {
    if (valOverlayLayer) {
      entityLayer.removeChild(valOverlayLayer);
      valOverlayLayer.destroy();
      valOverlayLayer = null;
      valCircleMap = new Map();
    }
    clearPulse();
    if (!valCb.checked || !lastLayout) {
      populateIssuesPanel(cachedValidationIssues ?? []);
      return;
    }
    if (!cachedValidationIssues) {
      try {
        cachedValidationIssues = engine.validateLayout(lastLayout, null);
      } catch {
        cachedValidationIssues = [];
      }
    }
    if (cachedValidationIssues.length === 0) {
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

  valCb.addEventListener("change", updateValidationOverlay);

  // --- Item color legend (bottom-left) ---
  const legendEl = document.createElement("div");
  legendEl.style.cssText = "position:absolute;bottom:8px;left:8px;background:rgba(0,0,0,0.6);color:#ccc;font:11px monospace;padding:4px 8px;border-radius:3px;pointer-events:none;z-index:10;display:none;max-height:300px;overflow-y:auto";
  container.appendChild(legendEl);

  let pinnedRow: HTMLDivElement | null = null;

  function unpinRow(): void {
    if (pinnedRow) {
      pinnedRow.style.background = "";
      pinnedRow = null;
    }
    clearPulse();
  }

  // --- Validation issues panel (right side, below toggles) ---
  // top:112px = stacked toggle buttons (debug ~42px, trace ~44px, validation ~26px) + 8px gap.
  // Update if the toggle stack height changes.
  const issuesPanel = document.createElement("div");
  issuesPanel.style.cssText = "position:absolute;top:112px;right:8px;background:rgba(0,0,0,0.85);color:#e0e0e0;font:11px monospace;padding:6px 8px;border-radius:4px;border:1px solid #555;z-index:10;display:none;max-width:360px;max-height:calc(100% - 130px);overflow-y:auto;line-height:1.4";
  container.appendChild(issuesPanel);

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
    issuesPanel.innerHTML = "";
    pinnedRow = null;
    clearPulse();
    if (!valCb.checked || issues.length === 0) {
      issuesPanel.style.display = "none";
      return;
    }
    issuesPanel.style.display = "block";
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
      issuesPanel.appendChild(row);
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
    if (sidebarEl) sidebarEl.style.opacity = "1";
    sidebarEl?.querySelectorAll("input,select,button").forEach((el) => {
      (el as HTMLInputElement).disabled = false;
    });
  }

  function loadSnapshot(snapshot: LayoutSnapshot): void {
    cachedValidationIssues = snapshot.validation.issues.length > 0
      ? snapshot.validation.issues as unknown as ValidationIssue[]
      : null;

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

    // Show banner
    clearSnapshotBanner();
    const bannerCallbacks: BannerCallbacks = {
      onClear: () => {
        clearSnapshotBanner();
        entityLayer.removeChildren();
        lastLayout = null;
        drawGraph(viewport, null);
        viewport.moveCenter(WORLD_SIZE / 2, WORLD_SIZE / 2);
        legendEl.style.display = "none";
        infoPanel.style.display = "none";
        issuesPanel.style.display = "none";
        populateIssuesPanel([]);
      },
    };
    const sidebarEl = document.getElementById("sidebar");
    if (sidebarEl) {
      activeBanner = showSnapshotBanner(sidebarEl, snapshot, bannerCallbacks);
      // Dim sidebar to indicate snapshot mode
      sidebarEl.style.opacity = "0.5";
      sidebarEl.querySelectorAll("input,select,button").forEach((el) => {
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

  colorCb.addEventListener("change", () => {
    setItemColoring(colorCb.checked);
    if (lastLayout) renderLayoutOnCanvas(lastLayout);
  });

  rateCb.addEventListener("change", () => {
    setRateOverlay(rateCb.checked);
    if (lastLayout) renderLayoutOnCanvas(lastLayout);
  });

  app.canvas.addEventListener("pointermove", (e) => {
    const rect = app.canvas.getBoundingClientRect();
    const sx = e.clientX - rect.left;
    const sy = e.clientY - rect.top;
    const world = viewport.toWorld(sx, sy);
    const tx = Math.floor(world.x / TILE_PX);
    const ty = Math.floor(world.y / TILE_PX);
    coordsEl.textContent = `x:${tx} y:${ty}`;
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
    if (items.size === 0) {
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
    const w = layout.width ?? 0;
    const h = layout.height ?? 0;
    if (w > 0 && h > 0) {
      const pxW = w * 32;
      const pxH = h * 32;
      viewport.fit(true, pxW * 1.1, pxH * 1.2);
      viewport.moveCenter(pxW / 2, pxH / 2);
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
    tabBar.style.cssText = "display:flex;border-bottom:1px solid #333;background:#252526;flex-shrink:0";

    function makeTab(label: string): HTMLButtonElement {
      const btn = document.createElement("button");
      btn.textContent = label;
      btn.style.cssText = "flex:1;padding:8px 4px;background:none;border:none;border-bottom:2px solid transparent;color:#aaa;font:13px sans-serif;cursor:pointer;";
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
      tabGenerate.style.color = isGenerate ? "#e0e0e0" : "#aaa";
      tabCorpus.style.borderBottomColor = isGenerate ? "transparent" : "#569cd6";
      tabCorpus.style.color = isGenerate ? "#aaa" : "#e0e0e0";
    }

    tabGenerate.onclick = () => switchTab("generate");
    tabCorpus.onclick = () => switchTab("corpus");
    switchTab("generate");

    sidebarCtrl = renderSidebar(generatePanel, engine, {
      renderGraph,
      renderLayout: renderLayoutOnCanvas,
    }, {
      getDebugMode: () => debugCb.checked,
    });

    initCorpusPanel(corpusPanel, renderLayoutOnCanvas);
  }
}

main().catch((err) => {
  console.error("Failed to initialize app:", err);
});
