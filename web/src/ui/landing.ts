/**
 * Landing/showcase page.
 *
 * Displays the recipe complexity ladder as interactive cards.
 * Clicking a card solves + builds the layout via WASM, then
 * renders it in a floating PixiJS modal with staggered entity animation.
 */

import { Application, Container } from "pixi.js";
import { Viewport } from "pixi-viewport";
import type { SolverResult, LayoutResult } from "../engine";
import type { Engine } from "../engine";
import { TILE_PX, setRecipeFlows } from "../renderer/entities";

// ---- Showcase definitions ----

interface ShowcaseEntry {
  label: string;
  item: string;
  rate: number;
  inputs: string[];
  machine: string;
  tier: number;
  status: "solved" | "partial" | "wip";
  desc: string;
}

const SHOWCASE: ShowcaseEntry[] = [
  {
    label: "Iron Gear Wheel",
    item: "iron-gear-wheel",
    rate: 10,
    inputs: ["iron-plate"],
    machine: "assembling-machine-2",
    tier: 1,
    status: "solved",
    desc: "1 recipe, 1 solid input",
  },
  {
    label: "Electronic Circuit",
    item: "electronic-circuit",
    rate: 10,
    inputs: ["iron-plate", "copper-plate"],
    machine: "assembling-machine-2",
    tier: 2,
    status: "solved",
    desc: "2 recipes, 2 solid inputs",
  },
  {
    label: "Electronic Circuit (ores)",
    item: "electronic-circuit",
    rate: 10,
    inputs: ["iron-ore", "copper-ore"],
    machine: "assembling-machine-2",
    tier: 2,
    status: "solved",
    desc: "From ores — smelting included",
  },
  {
    label: "Plastic Bar",
    item: "plastic-bar",
    rate: 10,
    inputs: ["coal", "petroleum-gas"],
    machine: "chemical-plant",
    tier: 3,
    status: "solved",
    desc: "1 recipe, fluid + solid input",
  },
  {
    label: "Advanced Circuit",
    item: "advanced-circuit",
    rate: 10,
    inputs: ["iron-plate", "copper-plate", "plastic-bar"],
    machine: "assembling-machine-2",
    tier: 4,
    status: "partial",
    desc: "5+ recipes, mixed solid/fluid",
  },
];

// ---- Style ----

const STYLE = `
@import url('https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@300;400;500;600;700&family=Space+Grotesk:wght@400;500;600;700&display=swap');

.fucktorio-landing {
  position: fixed;
  inset: 0;
  background: #0c0c0c;
  color: #d4d4d4;
  font-family: 'JetBrains Mono', monospace;
  display: flex;
  flex-direction: column;
  align-items: center;
  overflow-y: auto;
  z-index: 2000;
}

.fucktorio-landing::before {
  content: '';
  position: fixed;
  inset: 0;
  background-image:
    linear-gradient(rgba(56,189,248,0.03) 1px, transparent 1px),
    linear-gradient(90deg, rgba(56,189,248,0.03) 1px, transparent 1px);
  background-size: 24px 24px;
  pointer-events: none;
  z-index: 0;
}

.fucktorio-landing-inner {
  position: relative;
  z-index: 1;
  width: 100%;
  max-width: 900px;
  padding: 60px 32px 80px;
  display: flex;
  flex-direction: column;
  align-items: center;
}

/* Header */

.fucktorio-landing-header {
  text-align: center;
  margin-bottom: 56px;
}

.fucktorio-landing-title {
  font-family: 'Space Grotesk', sans-serif;
  font-size: 52px;
  font-weight: 700;
  color: #f0f0f0;
  letter-spacing: -2px;
  margin: 0 0 8px;
  line-height: 1;
}

.fucktorio-landing-title span {
  background: linear-gradient(135deg, #38bdf8 0%, #818cf8 50%, #c084fc 100%);
  -webkit-background-clip: text;
  -webkit-text-fill-color: transparent;
  background-clip: text;
}

.fucktorio-landing-subtitle {
  font-size: 13px;
  font-weight: 300;
  color: #6b7280;
  letter-spacing: 2.5px;
  text-transform: uppercase;
  margin: 0;
}

/* Ladder */

.fucktorio-landing-ladder {
  width: 100%;
  display: flex;
  flex-direction: column;
  gap: 2px;
  margin-bottom: 48px;
}

.fucktorio-landing-ladder-header {
  display: grid;
  grid-template-columns: 64px 1fr 100px 80px;
  padding: 0 16px 10px;
  font-size: 10px;
  text-transform: uppercase;
  letter-spacing: 1.5px;
  color: #4b5563;
  border-bottom: 1px solid #1f2937;
  margin-bottom: 2px;
}

/* Card */

.fucktorio-landing-card {
  display: grid;
  grid-template-columns: 64px 1fr 100px 80px;
  align-items: center;
  padding: 14px 16px;
  background: rgba(255,255,255,0.02);
  border: 1px solid rgba(255,255,255,0.04);
  border-radius: 4px;
  cursor: pointer;
  transition: all 0.2s ease;
  position: relative;
  overflow: hidden;
}

.fucktorio-landing-card::before {
  content: '';
  position: absolute;
  left: 0;
  top: 0;
  bottom: 0;
  width: 3px;
  background: transparent;
  transition: background 0.2s ease;
}

.fucktorio-landing-card:hover {
  background: rgba(255,255,255,0.05);
  border-color: rgba(255,255,255,0.08);
}

.fucktorio-landing-card.solved:hover::before { background: #34d399; }
.fucktorio-landing-card.partial:hover::before { background: #fbbf24; }
.fucktorio-landing-card.wip { opacity: 0.4; cursor: default; }
.fucktorio-landing-card.wip:hover { background: rgba(255,255,255,0.02); border-color: rgba(255,255,255,0.04); }
.fucktorio-landing-card.loading { pointer-events: none; }

.fucktorio-landing-tier {
  font-size: 11px;
  font-weight: 600;
  color: #4b5563;
}
.fucktorio-landing-tier span {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  border-radius: 50%;
  border: 1.5px solid #374151;
  color: #6b7280;
  font-size: 12px;
}

.fucktorio-landing-card-body {
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.fucktorio-landing-card-title {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 14px;
  font-weight: 500;
  color: #e5e7eb;
}

.fucktorio-landing-card-icon {
  width: 22px;
  height: 22px;
  image-rendering: pixelated;
  flex-shrink: 0;
}

.fucktorio-landing-card-rate {
  font-size: 11px;
  color: #6b7280;
  font-weight: 300;
}

.fucktorio-landing-card-desc {
  font-size: 11px;
  color: #4b5563;
  font-weight: 300;
}

.fucktorio-landing-status {
  font-size: 10px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.8px;
  padding: 3px 8px;
  border-radius: 3px;
  text-align: center;
  justify-self: center;
}
.fucktorio-landing-status.solved { background: rgba(52,211,153,0.12); color: #34d399; }
.fucktorio-landing-status.partial { background: rgba(251,191,36,0.12); color: #fbbf24; }
.fucktorio-landing-status.wip { background: rgba(107,114,128,0.12); color: #6b7280; }

.fucktorio-landing-entities {
  font-size: 11px;
  color: #4b5563;
  text-align: right;
  font-weight: 300;
}

/* Footer */

.fucktorio-landing-footer {
  margin-top: 16px;
  text-align: center;
}

.fucktorio-landing-launch {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  background: rgba(255,255,255,0.04);
  color: #9ca3af;
  border: 1px solid rgba(255,255,255,0.08);
  padding: 12px 28px;
  border-radius: 6px;
  font-family: 'JetBrains Mono', monospace;
  font-size: 13px;
  cursor: pointer;
  transition: all 0.2s ease;
  letter-spacing: 0.3px;
}
.fucktorio-landing-launch:hover {
  background: rgba(255,255,255,0.08);
  color: #e5e7eb;
  border-color: rgba(255,255,255,0.15);
}
.fucktorio-landing-launch svg {
  width: 16px;
  height: 16px;
}

/* Modal */

.fucktorio-preview-backdrop {
  position: fixed;
  inset: 0;
  background: rgba(0,0,0,0.75);
  backdrop-filter: blur(8px);
  z-index: 3000;
  display: flex;
  align-items: center;
  justify-content: center;
  animation: fucktorio-fadeIn 0.2s ease forwards;
}

@keyframes fucktorio-fadeIn { to { opacity: 1; } }

.fucktorio-preview-modal {
  background: #141414;
  border: 1px solid #2a2a2a;
  border-radius: 8px;
  overflow: hidden;
  display: flex;
  flex-direction: column;
  width: 75vw;
  max-width: 1000px;
  height: 70vh;
  max-height: 700px;
  box-shadow: 0 25px 60px rgba(0,0,0,0.5);
  animation: fucktorio-modalIn 0.3s cubic-bezier(0.16, 1, 0.3, 1) forwards;
  transform: scale(0.95) translateY(10px);
  opacity: 0;
}

@keyframes fucktorio-modalIn { to { transform: scale(1) translateY(0); opacity: 1; } }

.fucktorio-preview-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 16px;
  border-bottom: 1px solid #1f2937;
  background: rgba(255,255,255,0.02);
  flex-shrink: 0;
}

.fucktorio-preview-title {
  font-size: 13px;
  font-weight: 500;
  color: #9ca3af;
  display: flex;
  align-items: center;
  gap: 8px;
  font-family: 'JetBrains Mono', monospace;
}

.fucktorio-preview-title img {
  width: 18px;
  height: 18px;
  image-rendering: pixelated;
}

.fucktorio-preview-stats {
  display: flex;
  gap: 16px;
  font-size: 11px;
  color: #4b5563;
  font-family: 'JetBrains Mono', monospace;
}

.fucktorio-preview-stats span { color: #6b7280; }

.fucktorio-preview-close {
  background: none;
  border: 1px solid #333;
  color: #6b7280;
  width: 28px;
  height: 28px;
  border-radius: 4px;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 16px;
  transition: all 0.15s ease;
  font-family: 'JetBrains Mono', monospace;
}
.fucktorio-preview-close:hover {
  background: rgba(255,255,255,0.06);
  color: #e5e7eb;
  border-color: #555;
}

.fucktorio-preview-canvas {
  flex: 1;
  position: relative;
  overflow: hidden;
  background: #111;
}

.fucktorio-preview-canvas canvas {
  display: block;
}

.fucktorio-preview-badge {
  position: absolute;
  bottom: 12px;
  right: 12px;
  background: rgba(0,0,0,0.75);
  color: #6b7280;
  font-size: 11px;
  padding: 4px 10px;
  border-radius: 4px;
  font-family: 'JetBrains Mono', monospace;
  pointer-events: none;
  z-index: 1;
}

.fucktorio-spinner {
  width: 16px;
  height: 16px;
  border: 2px solid #1f2937;
  border-top-color: #38bdf8;
  border-radius: 50%;
  animation: fucktorio-spin 0.6s linear infinite;
  display: inline-block;
}

@keyframes fucktorio-spin { to { transform: rotate(360deg); } }
`;

function injectStyle(): void {
  if (document.getElementById("fucktorio-landing-style")) return;
  const el = document.createElement("style");
  el.id = "fucktorio-landing-style";
  el.textContent = STYLE;
  document.head.appendChild(el);
}

// ---- Exported API ----

export interface LandingCallbacks {
  onOpenGenerator: () => void;
}

export function renderLanding(
  parent: HTMLElement,
  engine: Engine,
  callbacks: LandingCallbacks,
): void {
  injectStyle();
  parent.innerHTML = "";

  const root = document.createElement("div");
  root.className = "fucktorio-landing";
  parent.appendChild(root);

  const inner = document.createElement("div");
  inner.className = "fucktorio-landing-inner";
  root.appendChild(inner);

  // Header
  const header = document.createElement("div");
  header.className = "fucktorio-landing-header";
  inner.appendChild(header);

  const title = document.createElement("h1");
  title.className = "fucktorio-landing-title";
  title.innerHTML = "Fuck<span>torio</span>";
  header.appendChild(title);

  const subtitle = document.createElement("p");
  subtitle.className = "fucktorio-landing-subtitle";
  subtitle.textContent = "Automated Factory Blueprint Generator";
  header.appendChild(subtitle);

  // Ladder
  const ladder = document.createElement("div");
  ladder.className = "fucktorio-landing-ladder";
  inner.appendChild(ladder);

  const ladderHeader = document.createElement("div");
  ladderHeader.className = "fucktorio-landing-ladder-header";
  ladderHeader.innerHTML = "<span>Tier</span><span>Recipe</span><span>Status</span><span>Entities</span>";
  ladder.appendChild(ladderHeader);

  for (const entry of SHOWCASE) {
    const card = document.createElement("div");
    card.className = `fucktorio-landing-card ${entry.status}`;

    const tierEl = document.createElement("div");
    tierEl.className = "fucktorio-landing-tier";
    tierEl.innerHTML = `<span>${entry.tier}</span>`;
    card.appendChild(tierEl);

    const body = document.createElement("div");
    body.className = "fucktorio-landing-card-body";

    const titleRow = document.createElement("div");
    titleRow.className = "fucktorio-landing-card-title";
    const icon = document.createElement("img");
    icon.src = `${import.meta.env.BASE_URL}icons/${entry.item}.png`;
    icon.className = "fucktorio-landing-card-icon";
    icon.onerror = () => { icon.style.display = "none"; };
    titleRow.appendChild(icon);
    titleRow.appendChild(document.createTextNode(entry.label));
    const rateTag = document.createElement("span");
    rateTag.className = "fucktorio-landing-card-rate";
    rateTag.textContent = `${entry.rate}/s`;
    titleRow.appendChild(rateTag);
    body.appendChild(titleRow);

    const desc = document.createElement("div");
    desc.className = "fucktorio-landing-card-desc";
    desc.textContent = entry.desc;
    body.appendChild(desc);
    card.appendChild(body);

    const statusEl = document.createElement("div");
    statusEl.className = `fucktorio-landing-status ${entry.status}`;
    statusEl.textContent = entry.status === "solved" ? "Solved" : entry.status === "partial" ? "Partial" : "WIP";
    card.appendChild(statusEl);

    const entityCountEl = document.createElement("div");
    entityCountEl.className = "fucktorio-landing-entities";
    entityCountEl.textContent = "\u2014";
    card.appendChild(entityCountEl);

    if (entry.status !== "wip") {
      card.addEventListener("click", () => {
        openPreview(engine, entry, card, entityCountEl);
      });
    }

    ladder.appendChild(card);
  }

  // Footer
  const footer = document.createElement("div");
  footer.className = "fucktorio-landing-footer";
  inner.appendChild(footer);

  const launchBtn = document.createElement("button");
  launchBtn.className = "fucktorio-landing-launch";
  launchBtn.innerHTML = `<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M3 8h10M9 4l4 4-4 4"/></svg>Open Generator`;
  launchBtn.addEventListener("click", () => {
    root.style.transition = "opacity 0.3s ease";
    root.style.opacity = "0";
    setTimeout(() => {
      root.remove();
      callbacks.onOpenGenerator();
    }, 300);
  });
  footer.appendChild(launchBtn);
}

// ---- Preview ----

function openPreview(
  engine: Engine,
  entry: ShowcaseEntry,
  card: HTMLDivElement,
  entityCountEl: HTMLElement,
): void {
  if (card.classList.contains("loading")) return;
  card.classList.add("loading");
  entityCountEl.innerHTML = '<span class="fucktorio-spinner"></span>';

  // Yield to let the spinner paint, then do synchronous WASM work
  requestAnimationFrame(() => {
    requestAnimationFrame(() => {
      let solverResult: SolverResult;
      let layout: LayoutResult;
      try {
        const machine = engine.defaultMachineForItem(entry.item, entry.machine);
        solverResult = engine.solve(entry.item, entry.rate, entry.inputs, machine);
        layout = engine.buildLayout(solverResult);
      } catch (err) {
        card.classList.remove("loading");
        entityCountEl.textContent = "error";
        console.error("Landing solve/layout failed:", err);
        return;
      }

      entityCountEl.textContent = String(layout.entities.length);
      card.classList.remove("loading");

      setRecipeFlows(
        solverResult.machines.map((m) => ({
          recipe: m.recipe,
          count: m.count,
          inputs: m.inputs.map((f) => ({ item: f.item, rate: f.rate })),
          outputs: m.outputs.map((f) => ({ item: f.item, rate: f.rate })),
        })),
      );

      showModal(entry, layout, solverResult);
    });
  });
}

async function showModal(
  entry: ShowcaseEntry,
  layout: LayoutResult,
  solverResult: SolverResult,
): Promise<void> {
  // Backdrop
  const backdrop = document.createElement("div");
  backdrop.className = "fucktorio-preview-backdrop";
  document.body.appendChild(backdrop);

  const handleKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") closeModal();
  };

  let closed = false;
  let pixiApp: Application | null = null;
  function closeModal(): void {
    if (closed) return;
    closed = true;
    document.removeEventListener("keydown", handleKey);
    if (pixiApp) pixiApp.destroy(true);
    backdrop.remove();
  }

  document.addEventListener("keydown", handleKey);
  backdrop.addEventListener("click", (e) => {
    if (e.target === backdrop) closeModal();
  });

  // Modal structure
  const modal = document.createElement("div");
  modal.className = "fucktorio-preview-modal";
  backdrop.appendChild(modal);

  // Header
  const header = document.createElement("div");
  header.className = "fucktorio-preview-header";

  const titleEl = document.createElement("div");
  titleEl.className = "fucktorio-preview-title";
  const iconImg = document.createElement("img");
  iconImg.src = `${import.meta.env.BASE_URL}icons/${entry.item}.png`;
  iconImg.onerror = () => { iconImg.style.display = "none"; };
  titleEl.appendChild(iconImg);
  titleEl.appendChild(document.createTextNode(` ${entry.label} \u2014 ${entry.rate}/s`));
  header.appendChild(titleEl);

  const statsEl = document.createElement("div");
  statsEl.className = "fucktorio-preview-stats";
  const dims = `${layout.width ?? 0}\u00d7${layout.height ?? 0}`;
  const machineCount = solverResult.machines.reduce((s, m) => s + Math.ceil(m.count), 0);
  statsEl.innerHTML = `<span>${machineCount} machines</span><span>${dims} tiles</span>`;
  header.appendChild(statsEl);

  const closeBtn = document.createElement("button");
  closeBtn.className = "fucktorio-preview-close";
  closeBtn.textContent = "\u00d7";
  closeBtn.addEventListener("click", closeModal);
  header.appendChild(closeBtn);

  modal.appendChild(header);

  // Canvas container
  const canvasWrap = document.createElement("div");
  canvasWrap.className = "fucktorio-preview-canvas";
  modal.appendChild(canvasWrap);

  const badge = document.createElement("div");
  badge.className = "fucktorio-preview-badge";
  badge.textContent = `0 / ${layout.entities.length}`;
  canvasWrap.appendChild(badge);

  // Init PixiJS
  pixiApp = new Application();
  await pixiApp.init({
    resizeTo: canvasWrap,
    background: 0x111111,
    antialias: true,
  });
  canvasWrap.insertBefore(pixiApp.canvas, badge);
  pixiApp.canvas.addEventListener("contextmenu", (e: Event) => e.preventDefault());

  const layoutW = (layout.width ?? 20) * TILE_PX;
  const layoutH = (layout.height ?? 20) * TILE_PX;
  const worldSize = Math.max(layoutW, layoutH, 600) + 200;

  const viewport = new Viewport({
    screenWidth: canvasWrap.clientWidth,
    screenHeight: canvasWrap.clientHeight,
    worldWidth: worldSize,
    worldHeight: worldSize,
    events: pixiApp.renderer.events,
  });
  viewport.drag({ mouseButtons: "left" }).pinch().wheel().decelerate();
  pixiApp.stage.addChild(viewport);

  const entityLayer = new Container();
  viewport.addChild(entityLayer);

  // Fit layout into view
  viewport.fit(true, layoutW * 1.15, layoutH * 1.2);
  viewport.moveCenter(layoutW / 2, layoutH / 2);

  // Animated render: import the heavy render function and do it incrementally
  const { renderLayoutAnimated } = await import("../renderer/animated");
  renderLayoutAnimated(layout, entityLayer, badge, () => {
    // After animation completes — nothing extra needed
  });
}
