import type { Engine, SolverResult, LayoutResult, ItemFlow, ValidationIssue } from "../engine.js";
import { readUrlState, writeUrlState, DEFAULT_INPUTS } from "../state.js";
import { beltTierForRate, hexToCss } from "../renderer/colors.js";
import { niceName, setRecipeFlows } from "../renderer/entities.js";
import { renderDebugPanel } from "./debugPanel.js";

// ---------------------------------------------------------------------------
// Style
// ---------------------------------------------------------------------------

const STYLE = `
.sidebar-inner {
  display: flex;
  flex-direction: column;
  height: 100%;
  box-sizing: border-box;
  overflow-y: auto;
  background: #1a1a1a;
  color: #d4d4d4;
  font-family: 'JetBrains Mono', 'Consolas', monospace;
  font-size: 12px;
  scrollbar-width: thin;
  scrollbar-color: #333 #1a1a1a;
}

/* ---- Section ---- */
.sb-section {
  padding: 10px 12px;
  border-bottom: 1px solid #252525;
}
.sb-section:last-child { border-bottom: none; }

.sb-section-header {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-bottom: 8px;
  font-size: 10px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 1.2px;
  color: #6b7280;
}
.sb-section-header .sb-section-icon {
  width: 14px;
  height: 14px;
  opacity: 0.5;
}
.sb-section-header .sb-section-count {
  margin-left: auto;
  color: #4b5563;
  font-weight: 400;
  font-size: 10px;
  letter-spacing: 0;
}

/* ---- Inputs ---- */
.sb-input {
  width: 100%;
  box-sizing: border-box;
  background: #222;
  color: #d4d4d4;
  border: 1px solid #333;
  border-radius: 3px;
  padding: 5px 7px;
  font-size: 12px;
  font-family: inherit;
  outline: none;
  transition: border-color 0.15s;
}
.sb-input:focus { border-color: #555; }
.sb-input.item-invalid {
  border-color: #c44;
  color: #f88;
}

.sb-row {
  display: flex;
  gap: 6px;
  align-items: center;
}

.sb-select {
  background: #222;
  color: #d4d4d4;
  border: 1px solid #333;
  border-radius: 3px;
  padding: 4px 6px;
  font-size: 12px;
  font-family: inherit;
  outline: none;
  cursor: pointer;
}
.sb-select:focus { border-color: #555; }

/* ---- Tag pills ---- */
.sb-tags {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
}

.sb-tag {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  padding: 3px 7px;
  background: #222;
  border: 1px solid #333;
  border-radius: 3px;
  cursor: pointer;
  user-select: none;
  font-size: 11px;
  color: #999;
  transition: all 0.12s;
}
.sb-tag:hover { border-color: #444; background: #282828; }
.sb-tag.active {
  background: #1a2a1a;
  border-color: #3a5a3a;
  color: #b5cea8;
}
.sb-tag img {
  width: 14px;
  height: 14px;
  image-rendering: pixelated;
}
.sb-tag .sb-tag-check {
  font-size: 10px;
  opacity: 0.4;
}
.sb-tag.active .sb-tag-check { opacity: 1; color: #b5cea8; }

/* Fluid inputs get a blue tint */
.sb-tag.active.fluid {
  background: #1a1a2a;
  border-color: #3a3a5a;
  color: #9cdcfe;
}
.sb-tag.active.fluid .sb-tag-check { color: #9cdcfe; }

/* ---- Solver results ---- */
.sb-solver-empty {
  color: #4b5563;
  font-style: italic;
  padding: 4px 0;
  font-size: 11px;
}

.sb-ext-flow {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 2px 0;
  font-size: 11px;
  color: #9cdcfe;
}
.sb-ext-flow img {
  width: 14px;
  height: 14px;
  image-rendering: pixelated;
}
.sb-ext-flow .sb-ext-rate {
  color: #6b7280;
  margin-left: auto;
  font-variant-numeric: tabular-nums;
}

.sb-machine-group {
  background: #1e1e1e;
  border: 1px solid #262626;
  border-radius: 4px;
  margin-bottom: 5px;
  overflow: hidden;
}
.sb-machine-group-header {
  display: flex;
  align-items: center;
  gap: 5px;
  padding: 5px 8px;
  background: #1e1e1e;
  border-bottom: 1px solid #262626;
}
.sb-machine-group-header img {
  width: 16px;
  height: 16px;
  image-rendering: pixelated;
}
.sb-machine-group-name {
  font-weight: 600;
  color: #dcdcaa;
  font-size: 11px;
}
.sb-machine-group-count {
  margin-left: auto;
  color: #6b7280;
  font-size: 10px;
  font-variant-numeric: tabular-nums;
}
.sb-machine-group-body {
  padding: 4px 8px 6px;
}
.sb-machine-flow {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 1px 0;
  font-size: 11px;
  line-height: 1.4;
}
.sb-machine-flow img {
  width: 13px;
  height: 13px;
  image-rendering: pixelated;
}
.sb-machine-flow.flow-in { color: #9cdcfe; }
.sb-machine-flow.flow-out { color: #b5cea8; }
.sb-machine-flow .flow-rate {
  font-variant-numeric: tabular-nums;
}

.sb-divider {
  height: 1px;
  background: #262626;
  margin: 5px 0;
}

.sb-ext-section-title {
  font-size: 10px;
  text-transform: uppercase;
  letter-spacing: 1px;
  color: #4b5563;
  margin-bottom: 4px;
  margin-top: 2px;
}

/* ---- Status bar ---- */
.sb-status {
  display: flex;
  gap: 12px;
  font-size: 10px;
  color: #4b5563;
  padding: 4px 0 0;
}
.sb-status span { color: #6b7280; }

/* ---- Belt tier chips ---- */
.sb-belt-chip {
  display: inline-block;
  padding: 1px 5px;
  border-radius: 2px;
  font-size: 10px;
  font-weight: 600;
  border-left: 3px solid;
  margin-left: 4px;
}
.sb-belt-overflow {
  color: #f88;
}

/* ---- Buttons ---- */
.sb-btn {
  width: 100%;
  padding: 7px;
  border: 1px solid #333;
  border-radius: 3px;
  cursor: pointer;
  font-size: 12px;
  font-weight: 500;
  font-family: inherit;
  transition: all 0.15s;
  outline: none;
}
.sb-btn:disabled {
  opacity: 0.35;
  cursor: default;
}
.sb-btn-primary {
  background: #1a3a1a;
  color: #6a6;
  border-color: #3a5a3a;
}
.sb-btn-primary:hover:not(:disabled) {
  background: #1e4a1e;
  border-color: #4a6a4a;
}
.sb-btn-secondary {
  background: #1a1a2a;
  color: #69c;
  border-color: #3a3a5a;
}
.sb-btn-secondary:hover:not(:disabled) {
  background: #1e1e3a;
  border-color: #4a4a6a;
}

.sb-copy-status {
  margin-top: 3px;
  font-size: 10px;
  color: #6a6;
  text-align: center;
}

/* ---- Display toggles ---- */
.sb-toggles {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 4px 8px;
}
.sb-toggle {
  display: flex;
  align-items: center;
  gap: 5px;
  cursor: pointer;
  user-select: none;
  font-size: 11px;
  color: #888;
}
.sb-toggle input {
  accent-color: #569cd6;
  width: 13px;
  height: 13px;
  margin: 0;
}

/* ---- Errors (inline) ---- */
.sb-result-error {
  color: #f44;
  font-family: monospace;
  font-size: 11px;
  white-space: pre-wrap;
  word-break: break-word;
  padding: 4px 0;
}

/* ---- Warnings ---- */
.sb-warning {
  color: #fa0;
  font-size: 11px;
  padding: 2px 0;
}

/* ---- Rate suffix ---- */
.sb-rate-suffix {
  color: #6b7280;
  font-size: 10px;
  margin-left: 2px;
}

/* ---- Validation issues list ---- */
.sb-val-ok {
  color: #6a6;
  font-size: 11px;
  padding: 4px 0;
}
.sb-val-group {
  margin-bottom: 4px;
  border-radius: 3px;
  overflow: hidden;
  border: 1px solid #2a2a2a;
}
.sb-val-group-header {
  display: flex;
  align-items: center;
  gap: 5px;
  padding: 4px 7px;
  background: #1e1e1e;
  cursor: pointer;
  user-select: none;
  font-size: 11px;
}
.sb-val-group-header:hover { background: #242424; }
.sb-val-group-dot {
  display: inline-block;
  width: 7px;
  height: 7px;
  border-radius: 50%;
  flex-shrink: 0;
}
.sb-val-group-name { flex: 1; color: #ccc; }
.sb-val-group-count {
  font-size: 10px;
  color: #666;
  font-variant-numeric: tabular-nums;
}
.sb-val-group-chevron { color: #555; font-size: 10px; }
.sb-val-group-body { background: #191919; }
.sb-val-issue {
  padding: 3px 7px 3px 19px;
  font-size: 11px;
  color: #bbb;
  border-top: 1px solid #222;
  line-height: 1.4;
  word-break: break-word;
}
.sb-val-issue.clickable {
  cursor: pointer;
}
.sb-val-issue.clickable:hover { background: rgba(255,255,255,0.05); }
.sb-val-issue.pinned { background: rgba(255,255,255,0.08); }
`;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function itemIcon(slug: string, size = 14): HTMLImageElement {
  const img = document.createElement("img");
  img.src = `${import.meta.env.BASE_URL}icons/${slug}.png`;
  img.width = size;
  img.height = size;
  img.style.cssText = "image-rendering:pixelated";
  img.onerror = () => { img.style.display = "none"; };
  return img;
}

function makeOption(value: string, defaultValue: string): HTMLOptionElement {
  const opt = document.createElement("option");
  opt.value = value;
  opt.textContent = niceName(value);
  if (value === defaultValue) opt.selected = true;
  return opt;
}

/** Create a section block with icon + title. */
function makeSection(
  iconSvg: string,
  title: string,
  extra?: string,
): { section: HTMLDivElement; body: HTMLDivElement; countEl: HTMLSpanElement | null } {
  const section = document.createElement("div");
  section.className = "sb-section";

  const header = document.createElement("div");
  header.className = "sb-section-header";

  const iconEl = document.createElement("span");
  iconEl.className = "sb-section-icon";
  iconEl.innerHTML = iconSvg;
  header.appendChild(iconEl);

  const titleEl = document.createElement("span");
  titleEl.textContent = title;
  header.appendChild(titleEl);

  let countEl: HTMLSpanElement | null = null;
  if (extra !== undefined) {
    countEl = document.createElement("span");
    countEl.className = "sb-section-count";
    countEl.textContent = extra;
    header.appendChild(countEl);
  }

  section.appendChild(header);

  const body = document.createElement("div");
  section.appendChild(body);

  return { section, body, countEl };
}

function appendFlows(
  container: HTMLElement,
  flows: ItemFlow[],
  className: string,
  prefix: string,
): void {
  for (const flow of flows) {
    const el = document.createElement("div");
    el.className = `sb-machine-flow ${className}`;
    if (prefix) el.appendChild(document.createTextNode(prefix));
    el.appendChild(itemIcon(flow.item, 13));
    el.appendChild(document.createTextNode(niceName(flow.item)));
    const rateSpan = document.createElement("span");
    rateSpan.className = "flow-rate";
    const tier = beltTierForRate(flow.rate);
    const rateColor = tier ? hexToCss(tier.color) : "#f88";
    rateSpan.style.color = rateColor;
    rateSpan.textContent = `${flow.rate.toFixed(1)}/s`;
    el.appendChild(rateSpan);
    container.appendChild(el);
  }
}

// Fluid items that get blue-tinted tag pills
const FLUID_ITEMS = new Set(["water", "crude-oil", "petroleum-gas", "light-oil", "heavy-oil", "sulfuric-acid", "lubricant", "steam"]);

// ---------------------------------------------------------------------------
// Exports
// ---------------------------------------------------------------------------

export interface SidebarCallbacks {
  renderGraph: (result: SolverResult | null) => void;
  renderLayout: (layout: LayoutResult) => void;
}

export interface SidebarParams {
  item: string;
  rate: number;
  machine?: string;
  inputs?: string[];
  belt?: string | null;
}

/** Callbacks the sidebar can use to read canvas overlay state. */
export interface SidebarOptions {
  getDebugMode?: () => boolean;
  /** Called after the sidebar creates its display toggles. */
  onDisplayToggles?: (toggles: DisplayToggles) => void;
}

/** Controls exposed for the display toggle checkboxes. */
export interface DisplayToggles {
  colorCb: HTMLInputElement;
  rateCb: HTMLInputElement;
  debugCb: HTMLInputElement;
  valCb: HTMLInputElement;
  regionsCb: HTMLInputElement;
  soloRegionsCb: HTMLInputElement;
}

export function renderSidebar(
  el: HTMLElement,
  engine: Engine,
  callbacks: SidebarCallbacks,
  options?: SidebarOptions,
): { getParams(): SidebarParams | null; setParams(params: SidebarParams, opts?: { skipAutoSolve?: boolean }): void; updateValidation(issues: ValidationIssue[], onPanToTile: (x: number, y: number) => void): void } {
  el.innerHTML = "";

  if (!document.getElementById("fucktorio-sidebar-style")) {
    const styleEl = document.createElement("style");
    styleEl.id = "fucktorio-sidebar-style";
    styleEl.textContent = STYLE;
    document.head.appendChild(styleEl);
  }

  const inner = document.createElement("div");
  inner.className = "sidebar-inner";

  // ==================== TARGET ====================
  const { section: targetSection, body: targetBody } = makeSection(
    `<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><circle cx="8" cy="8" r="6"/><circle cx="8" cy="8" r="2"/></svg>`,
    "Target",
  );

  const datalist = document.createElement("datalist");
  datalist.id = "fucktorio-items-datalist";
  const allItems = engine.allProducibleItems();
  const itemSet = new Set(allItems);
  allItems.forEach((item) => {
    const opt = document.createElement("option");
    opt.value = item;
    datalist.appendChild(opt);
  });
  targetBody.appendChild(datalist);

  const itemInput = document.createElement("input");
  itemInput.type = "text";
  itemInput.className = "sb-input";
  itemInput.setAttribute("list", "fucktorio-items-datalist");
  itemInput.autocomplete = "off";
  itemInput.placeholder = "Search item…";
  targetBody.appendChild(itemInput);

  // Rate + Machine row
  const rateMachineRow = document.createElement("div");
  rateMachineRow.className = "sb-row";
  rateMachineRow.style.cssText = "margin-top:6px";

  const rateInput = document.createElement("input");
  rateInput.type = "number";
  rateInput.className = "sb-input";
  rateInput.step = "0.5";
  rateInput.min = "0.1";
  rateInput.style.cssText = "width:72px;flex-shrink:0";
  rateInput.placeholder = "10";
  rateMachineRow.appendChild(rateInput);

  const rateSuffix = document.createElement("span");
  rateSuffix.className = "sb-rate-suffix";
  rateSuffix.textContent = "/s";
  rateMachineRow.appendChild(rateSuffix);

  const machineSelect = document.createElement("select");
  machineSelect.className = "sb-select";
  machineSelect.style.cssText = "flex:1;min-width:0";
  engine.allProducerMachines().forEach((m) => machineSelect.appendChild(makeOption(m, "assembling-machine-3")));
  rateMachineRow.appendChild(machineSelect);

  targetBody.appendChild(rateMachineRow);
  inner.appendChild(targetSection);

  // ==================== INPUTS ====================
  const { section: inputsSection, body: inputsBody } = makeSection(
    `<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><rect x="2" y="5" width="12" height="6" rx="1"/><line x1="5" y1="8" x2="11" y2="8"/></svg>`,
    "Inputs",
  );

  const tagsWrap = document.createElement("div");
  tagsWrap.className = "sb-tags";

  const checkboxes = new Map<string, HTMLInputElement>();
  DEFAULT_INPUTS.forEach((inp) => {
    const tag = document.createElement("label");
    tag.className = `sb-tag${FLUID_ITEMS.has(inp) ? " fluid" : ""}`;

    const checkSpan = document.createElement("span");
    checkSpan.className = "sb-tag-check";
    checkSpan.textContent = "\u2713";

    const cb = document.createElement("input");
    cb.type = "checkbox";
    cb.value = inp;
    cb.style.display = "none";
    checkboxes.set(inp, cb);

    tag.appendChild(checkSpan);
    tag.appendChild(itemIcon(inp, 14));
    tag.appendChild(document.createTextNode(niceName(inp)));
    tag.appendChild(cb);

    // Toggle active class on check/uncheck
    cb.addEventListener("change", () => {
      tag.classList.toggle("active", cb.checked);
    });

    tagsWrap.appendChild(tag);
  });
  inputsBody.appendChild(tagsWrap);
  inner.appendChild(inputsSection);

  // ==================== SOLVER ====================
  const { section: solverSection, body: solverBody, countEl: solverCount } = makeSection(
    `<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M3 8h10M9 4l4 4-4 4"/></svg>`,
    "Solver",
    "",
  );

  const resultContainer = document.createElement("div");
  solverBody.appendChild(resultContainer);
  inner.appendChild(solverSection);

  // ==================== LAYOUT ====================
  const { section: layoutSection, body: layoutBody } = makeSection(
    `<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><rect x="2" y="2" width="12" height="12" rx="1"/><line x1="8" y1="2" x2="8" y2="14"/><line x1="2" y1="8" x2="14" y2="8"/></svg>`,
    "Layout",
  );

  const beltRow = document.createElement("div");
  beltRow.className = "sb-row";
  beltRow.style.cssText = "margin-bottom:6px;align-items:center";
  const beltLabel = document.createElement("span");
  beltLabel.style.cssText = "color:#6b7280;font-size:10px;text-transform:uppercase;letter-spacing:0.8px;flex-shrink:0";
  beltLabel.textContent = "Belt";
  beltRow.appendChild(beltLabel);
  const beltSelect = document.createElement("select");
  beltSelect.className = "sb-select";
  beltSelect.style.cssText = "flex:1;min-width:0";
  [
    ["Auto", ""],
    ["Yellow 15/s", "transport-belt"],
    ["Red 30/s", "fast-transport-belt"],
    ["Blue 45/s", "express-transport-belt"],
  ].forEach(([label, value]) => {
    const opt = document.createElement("option");
    opt.value = value;
    opt.textContent = label;
    beltSelect.appendChild(opt);
  });
  beltRow.appendChild(beltSelect);
  layoutBody.appendChild(beltRow);

  const layoutBtn = document.createElement("button");
  layoutBtn.className = "sb-btn sb-btn-primary";
  layoutBtn.textContent = "Generate Layout";
  layoutBtn.disabled = true;
  layoutBtn.style.cssText = "margin-bottom:5px";
  layoutBody.appendChild(layoutBtn);

  const blueprintSection = document.createElement("div");
  blueprintSection.style.display = "none";

  const copyBtn = document.createElement("button");
  copyBtn.className = "sb-btn sb-btn-secondary";
  copyBtn.textContent = "Copy Blueprint";
  blueprintSection.appendChild(copyBtn);

  const copyStatus = document.createElement("div");
  copyStatus.className = "sb-copy-status";
  blueprintSection.appendChild(copyStatus);
  layoutBody.appendChild(blueprintSection);

  inner.appendChild(layoutSection);

  // ==================== VALIDATION ====================
  const { section: valSection, body: valBody, countEl: valCountEl } = makeSection(
    `<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><circle cx="8" cy="8" r="6"/><line x1="8" y1="5" x2="8" y2="8.5"/><circle cx="8" cy="11" r="0.8" fill="currentColor" stroke="none"/></svg>`,
    "Validation",
    "",
  );
  valSection.style.display = "none";
  inner.appendChild(valSection);

  // ==================== DISPLAY ====================
  const { section: displaySection, body: displayBody } = makeSection(
    `<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><circle cx="8" cy="8" r="3"/><path d="M8 2v2M8 12v2M2 8h2M12 8h2M4.05 4.05l1.41 1.41M10.54 10.54l1.41 1.41M4.05 11.95l1.41-1.41M10.54 5.46l1.41-1.41"/></svg>`,
    "Display",
  );

  const togglesWrap = document.createElement("div");
  togglesWrap.className = "sb-toggles";

  const colorCb = document.createElement("input");
  colorCb.type = "checkbox";
  colorCb.checked = true;
  const colorToggle = document.createElement("label");
  colorToggle.className = "sb-toggle";
  colorToggle.appendChild(colorCb);
  colorToggle.appendChild(document.createTextNode("Item colours"));
  togglesWrap.appendChild(colorToggle);

  const rateCb = document.createElement("input");
  rateCb.type = "checkbox";
  rateCb.checked = false;
  const rateToggle = document.createElement("label");
  rateToggle.className = "sb-toggle";
  rateToggle.appendChild(rateCb);
  rateToggle.appendChild(document.createTextNode("Rates"));
  togglesWrap.appendChild(rateToggle);

  const debugCb = document.createElement("input");
  debugCb.type = "checkbox";
  debugCb.checked = false;
  const debugToggle = document.createElement("label");
  debugToggle.className = "sb-toggle";
  debugToggle.appendChild(debugCb);
  debugToggle.appendChild(document.createTextNode("Debug"));
  togglesWrap.appendChild(debugToggle);

  const valCb = document.createElement("input");
  valCb.type = "checkbox";
  valCb.checked = false;
  const valToggle = document.createElement("label");
  valToggle.className = "sb-toggle";
  valToggle.appendChild(valCb);
  valToggle.appendChild(document.createTextNode("Validation"));
  togglesWrap.appendChild(valToggle);

  const regionsCb = document.createElement("input");
  regionsCb.type = "checkbox";
  regionsCb.checked = false;
  const regionsToggle = document.createElement("label");
  regionsToggle.className = "sb-toggle";
  regionsToggle.appendChild(regionsCb);
  regionsToggle.appendChild(document.createTextNode("SAT Zones"));
  togglesWrap.appendChild(regionsToggle);

  const soloRegionsCb = document.createElement("input");
  soloRegionsCb.type = "checkbox";
  soloRegionsCb.checked = false;
  const soloRegionsToggle = document.createElement("label");
  soloRegionsToggle.className = "sb-toggle";
  soloRegionsToggle.appendChild(soloRegionsCb);
  soloRegionsToggle.appendChild(document.createTextNode("Solo regions"));
  togglesWrap.appendChild(soloRegionsToggle);

  displayBody.appendChild(togglesWrap);
  // Mark display section so snapshot mode doesn't disable its controls
  displaySection.setAttribute("data-snapshot-keep", "");
  inner.appendChild(displaySection);

  // Expose toggles to main.ts
  options?.onDisplayToggles?.({ colorCb, rateCb, debugCb, valCb, regionsCb, soloRegionsCb });

  el.appendChild(inner);

  // ==================== State init ====================
  const urlState = readUrlState();
  itemInput.value = urlState.item;
  rateInput.value = String(urlState.rate);
  machineSelect.value =
    urlState.machine ?? engine.defaultMachineForItem(urlState.item, "assembling-machine-3");
  checkboxes.forEach((cb, name) => {
    cb.checked = urlState.inputs.includes(name);
    // Sync tag pill active state
    const tag = cb.closest(".sb-tag") as HTMLLabelElement;
    if (tag) tag.classList.toggle("active", cb.checked);
  });
  if (urlState.belt) beltSelect.value = urlState.belt;

  let debounceTimer: ReturnType<typeof setTimeout> | null = null;
  let previousItem = urlState.item;
  let currentResult: SolverResult | null = null;
  let currentLayout: LayoutResult | null = null;

  function scheduleAutoSolve(): void {
    if (debounceTimer !== null) clearTimeout(debounceTimer);
    debounceTimer = setTimeout(runSolve, 150);
  }

  function runSolve(): void {
    const targetItem = itemInput.value.trim();
    const targetRate = parseFloat(rateInput.value);
    const machineEntity = machineSelect.value;
    const availableInputs = DEFAULT_INPUTS.filter((inp) => checkboxes.get(inp)?.checked);

    if (!itemSet.has(targetItem)) {
      itemInput.classList.add("item-invalid");
      return;
    }
    itemInput.classList.remove("item-invalid");

    if (isNaN(targetRate) || targetRate <= 0) return;

    if (targetItem !== previousItem) {
      const suggestedMachine = engine.defaultMachineForItem(targetItem, machineEntity);
      const machineOpts = machineSelect.options;
      for (let i = 0; i < machineOpts.length; i++) {
        if (machineOpts[i].value === suggestedMachine) {
          machineSelect.selectedIndex = i;
          break;
        }
      }
      previousItem = targetItem;
    }

    writeUrlState({
      item: targetItem,
      rate: targetRate,
      machine: machineSelect.value,
      inputs: availableInputs,
      belt: beltSelect.value || null,
    });

    resultContainer.innerHTML = "";
    currentLayout = null;
    blueprintSection.style.display = "none";
    try {
      const result = engine.solve(targetItem, targetRate, availableInputs, machineSelect.value);
      currentResult = result;
      renderResult(resultContainer, result);
      callbacks.renderGraph(result);
      layoutBtn.disabled = false;
      const totalMachines = result.machines.reduce((sum, m) => sum + Math.ceil(m.count), 0);
      if (solverCount) solverCount.textContent = `${totalMachines} machines`;
    } catch (err) {
      currentResult = null;
      callbacks.renderGraph(null);
      layoutBtn.disabled = true;
      if (solverCount) solverCount.textContent = "error";
      const errDiv = document.createElement("div");
      errDiv.className = "sb-result-error";
      errDiv.textContent = String(err);
      resultContainer.appendChild(errDiv);
    }
  }

  layoutBtn.addEventListener("click", () => {
    if (!currentResult) return;
    try {
      const maxTier = beltSelect.value || undefined;
      const useTraced = options?.getDebugMode?.() ?? false;
      currentLayout = useTraced
        ? engine.buildLayoutTraced(currentResult, maxTier)
        : engine.buildLayout(currentResult, maxTier);
      setRecipeFlows(currentResult.machines);
      callbacks.renderLayout(currentLayout);
      if (currentLayout.warnings?.length) {
        for (const w of currentLayout.warnings) {
          const wDiv = document.createElement("div");
          wDiv.className = "sb-warning";
          wDiv.textContent = `\u26A0 ${w}`;
          resultContainer.appendChild(wDiv);
        }
        blueprintSection.style.display = "none";
      } else {
        blueprintSection.style.display = "block";
      }
      if (currentLayout.trace?.length && options?.getDebugMode?.()) {
        resultContainer.appendChild(renderDebugPanel(currentLayout.trace));
      }
    } catch (err) {
      const errDiv = document.createElement("div");
      errDiv.className = "sb-result-error";
      errDiv.textContent = `Layout error: ${err}`;
      resultContainer.appendChild(errDiv);
    }
  });

  copyBtn.addEventListener("click", async () => {
    if (!currentLayout) return;
    const bp = engine.exportBlueprint(currentLayout, itemInput.value.trim());
    await navigator.clipboard.writeText(bp);
    copyStatus.textContent = "Copied!";
    setTimeout(() => { copyStatus.textContent = ""; }, 2000);
  });

  itemInput.addEventListener("input", scheduleAutoSolve);
  rateInput.addEventListener("input", scheduleAutoSolve);
  machineSelect.addEventListener("change", scheduleAutoSolve);
  checkboxes.forEach((cb) => cb.addEventListener("change", scheduleAutoSolve));

  runSolve();

  return {
    getParams() {
      const item = itemInput.value.trim();
      const rate = parseFloat(rateInput.value);
      if (!item || isNaN(rate) || rate <= 0) return null;
      return { item, rate };
    },
    setParams(params, opts) {
      itemInput.value = params.item;
      rateInput.value = String(params.rate);
      if (params.machine) {
        machineSelect.value = params.machine;
      } else {
        machineSelect.value = engine.defaultMachineForItem(params.item, "assembling-machine-3");
      }
      if (params.inputs) {
        checkboxes.forEach((cb, name) => {
          cb.checked = params.inputs!.includes(name);
          const tag = cb.closest(".sb-tag") as HTMLLabelElement;
          if (tag) tag.classList.toggle("active", cb.checked);
        });
      }
      if (params.belt) {
        beltSelect.value = params.belt;
      } else {
        beltSelect.value = "";
      }
      previousItem = params.item;
      if (!opts?.skipAutoSolve) {
        scheduleAutoSolve();
      }
    },
    updateValidation(issues: ValidationIssue[], onPanToTile: (x: number, y: number) => void) {
      valBody.innerHTML = "";
      if (issues.length === 0) {
        valSection.style.display = "none";
        if (valCountEl) valCountEl.textContent = "";
        return;
      }
      valSection.style.display = "";

      const errors = issues.filter(i => i.severity === "Error").length;
      const warns = issues.length - errors;
      if (valCountEl) {
        if (errors > 0) {
          valCountEl.textContent = `${errors} error${errors !== 1 ? "s" : ""}`;
          valCountEl.style.color = "#f66";
        } else {
          valCountEl.textContent = `${warns} warning${warns !== 1 ? "s" : ""}`;
          valCountEl.style.color = "#fa0";
        }
      }

      // Group by category
      const groups = new Map<string, ValidationIssue[]>();
      for (const issue of issues) {
        let g = groups.get(issue.category);
        if (!g) { g = []; groups.set(issue.category, g); }
        g.push(issue);
      }

      for (const [category, groupIssues] of groups) {
        const hasErrors = groupIssues.some(i => i.severity === "Error");
        const dotColor = hasErrors ? "#f44" : "#fa0";

        const groupEl = document.createElement("div");
        groupEl.className = "sb-val-group";

        const header = document.createElement("div");
        header.className = "sb-val-group-header";

        const dot = document.createElement("span");
        dot.className = "sb-val-group-dot";
        dot.style.background = dotColor;
        header.appendChild(dot);

        const name = document.createElement("span");
        name.className = "sb-val-group-name";
        name.textContent = category;
        header.appendChild(name);

        const count = document.createElement("span");
        count.className = "sb-val-group-count";
        count.textContent = String(groupIssues.length);
        header.appendChild(count);

        const chevron = document.createElement("span");
        chevron.className = "sb-val-group-chevron";
        chevron.textContent = "\u25be"; // down triangle (open)
        header.appendChild(chevron);

        const body = document.createElement("div");
        body.className = "sb-val-group-body";

        // Toggle collapse on header click
        header.addEventListener("click", () => {
          const collapsed = body.style.display === "none";
          body.style.display = collapsed ? "" : "none";
          chevron.textContent = collapsed ? "\u25be" : "\u25b8";
        });

        for (const issue of groupIssues) {
          const row = document.createElement("div");
          const hasPos = issue.x != null && issue.y != null;
          row.className = "sb-val-issue" + (hasPos ? " clickable" : "");
          row.textContent = issue.message;
          if (!hasPos) row.style.opacity = "0.6";
          if (hasPos) {
            row.addEventListener("click", (e) => {
              e.stopPropagation();
              // Toggle pin style
              const wasPinned = row.classList.contains("pinned");
              // Unpin all rows in this panel
              valBody.querySelectorAll(".sb-val-issue.pinned").forEach(el => el.classList.remove("pinned"));
              if (!wasPinned) {
                row.classList.add("pinned");
              }
              onPanToTile(issue.x!, issue.y!);
            });
          }
          body.appendChild(row);
        }

        groupEl.appendChild(header);
        groupEl.appendChild(body);
        valBody.appendChild(groupEl);
      }
    },
  };
}

// ---------------------------------------------------------------------------
// Result renderer — external inputs at top, then grouped machines
// ---------------------------------------------------------------------------

function renderResult(container: HTMLElement, result: SolverResult): void {
  // External inputs at top
  if (result.external_inputs.length > 0) {
    const extTitle = document.createElement("div");
    extTitle.className = "sb-ext-section-title";
    extTitle.textContent = "External inputs";
    container.appendChild(extTitle);

    for (const flow of result.external_inputs) {
      const row = document.createElement("div");
      row.className = "sb-ext-flow";
      row.appendChild(itemIcon(flow.item, 14));
      row.appendChild(document.createTextNode(niceName(flow.item)));
      const rateSpan = document.createElement("span");
      rateSpan.className = "sb-ext-rate";
      rateSpan.textContent = `${flow.rate.toFixed(1)}/s`;
      row.appendChild(rateSpan);
      container.appendChild(row);
    }

    const divider = document.createElement("div");
    divider.className = "sb-divider";
    container.appendChild(divider);
  }

  // Group machines by entity type (e.g. all assembling-machine-2 together)
  const groups = new Map<string, typeof result.machines>();
  for (const m of result.machines) {
    let group = groups.get(m.entity);
    if (!group) { group = []; groups.set(m.entity, group); }
    group.push(m);
  }

  for (const [entity, machines] of groups) {
    const totalCount = machines.reduce((s, m) => s + Math.ceil(m.count), 0);

    const groupEl = document.createElement("div");
    groupEl.className = "sb-machine-group";

    const header = document.createElement("div");
    header.className = "sb-machine-group-header";
    header.appendChild(itemIcon(entity, 16));
    const nameSpan = document.createElement("span");
    nameSpan.className = "sb-machine-group-name";
    nameSpan.textContent = niceName(entity);
    header.appendChild(nameSpan);
    const countSpan = document.createElement("span");
    countSpan.className = "sb-machine-group-count";
    countSpan.textContent = `\u00d7${totalCount}`;
    header.appendChild(countSpan);
    groupEl.appendChild(header);

    const body = document.createElement("div");
    body.className = "sb-machine-group-body";

    for (const machine of machines) {
      const recipeRow = document.createElement("div");
      recipeRow.className = "sb-machine-flow";
      recipeRow.style.cssText = "color:#6b7280;margin-bottom:2px";
      recipeRow.appendChild(document.createTextNode("\u2192 "));
      recipeRow.appendChild(itemIcon(machine.recipe, 13));
      recipeRow.appendChild(document.createTextNode(niceName(machine.recipe)));
      body.appendChild(recipeRow);

      appendFlows(body, machine.inputs, "flow-in", "\u25b6 ");
      appendFlows(body, machine.outputs, "flow-out", "\u25c0 ");
    }

    groupEl.appendChild(body);
    container.appendChild(groupEl);
  }

  // Status bar: totals + external outputs
  const statusDiv = document.createElement("div");
  statusDiv.className = "sb-status";
  statusDiv.style.cssText = "margin-top:6px";

  const totalMachines = result.machines.reduce((sum, m) => sum + Math.ceil(m.count), 0);
  const chainDepth = result.dependency_order.length;

  const machinesSpan = document.createElement("span");
  machinesSpan.textContent = `${totalMachines} machines`;
  statusDiv.appendChild(machinesSpan);

  const depthSpan = document.createElement("span");
  depthSpan.textContent = `depth ${chainDepth}`;
  statusDiv.appendChild(depthSpan);

  container.appendChild(statusDiv);

  // External outputs
  if (result.external_outputs.length > 0) {
    for (const flow of result.external_outputs) {
      const row = document.createElement("div");
      row.style.cssText = "display:flex;align-items:center;gap:4px;padding:2px 0;font-size:11px;color:#b5cea8";
      row.appendChild(itemIcon(flow.item, 13));
      row.appendChild(document.createTextNode(niceName(flow.item)));
      const tier = beltTierForRate(flow.rate);
      if (tier) {
        const tierColor = hexToCss(tier.color);
        row.appendChild(document.createTextNode(`${flow.rate.toFixed(1)}/s`));
        const chip = document.createElement("span");
        chip.className = "sb-belt-chip";
        chip.style.borderColor = tierColor;
        chip.style.color = tierColor;
        chip.textContent = tier.name;
        row.appendChild(chip);
      } else {
        row.appendChild(document.createTextNode(`${flow.rate.toFixed(1)}/s`));
        const warn = document.createElement("span");
        warn.className = "sb-belt-overflow";
        warn.textContent = "\u26a0 overflow";
        row.appendChild(warn);
      }
      container.appendChild(row);
    }
  }
}
