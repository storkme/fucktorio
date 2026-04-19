import type { Engine, SolverResult, LayoutResult, ItemFlow, ValidationIssue, TraceEvent } from "../engine.js";
import { readUrlState, writeUrlState, DEFAULT_INPUTS } from "../state.js";
import { beltTierForRate, hexToCss } from "../renderer/colors.js";
import { niceName, setRecipeFlows } from "../renderer/entities.js";
import { renderDebugPanel } from "./debugPanel.js";
import "./sidebar.css";

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
  /** Begin a new streaming layout render. Returns the per-event callback that
   *  sidebar passes into `engine.buildLayoutStreaming`. Cancels any prior
   *  streaming render first. */
  startStreaming: () => (evt: TraceEvent) => void;
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
}

export function renderSidebar(
  el: HTMLElement,
  engine: Engine,
  callbacks: SidebarCallbacks,
  options?: SidebarOptions,
): { getParams(): SidebarParams | null; setParams(params: SidebarParams, opts?: { skipAutoSolve?: boolean }): void; updateValidation(issues: ValidationIssue[], onPanToTile: (x: number, y: number) => void): void } {
  el.innerHTML = "";

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

  function makeField(label: string, control: HTMLElement): HTMLDivElement {
    const row = document.createElement("div");
    row.className = "sb-field";
    const lbl = document.createElement("span");
    lbl.className = "sb-field-label";
    lbl.textContent = label;
    row.appendChild(lbl);
    control.style.flex = "1";
    control.style.minWidth = "0";
    row.appendChild(control);
    return row;
  }

  // Item (full-width, no label prefix — the section header is "Target")
  const itemInput = document.createElement("input");
  itemInput.type = "text";
  itemInput.className = "sb-input";
  itemInput.setAttribute("list", "fucktorio-items-datalist");
  itemInput.autocomplete = "off";
  itemInput.placeholder = "Search item…";
  targetBody.appendChild(itemInput);

  // Machine
  const machineSelect = document.createElement("select");
  machineSelect.className = "sb-select";
  engine.allProducerMachines().forEach((m) => machineSelect.appendChild(makeOption(m, "assembling-machine-3")));
  targetBody.appendChild(makeField("Machine", machineSelect));

  // Belt tier (Auto / Yellow / Red / Blue) — moved up from the former Layout section
  const beltSelect = document.createElement("select");
  beltSelect.className = "sb-select";
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
  targetBody.appendChild(makeField("Belt", beltSelect));

  // Rate (numeric with /s suffix)
  const rateRow = document.createElement("div");
  rateRow.className = "sb-field";
  const rateLabel = document.createElement("span");
  rateLabel.className = "sb-field-label";
  rateLabel.textContent = "Rate";
  rateRow.appendChild(rateLabel);
  const rateInput = document.createElement("input");
  rateInput.type = "number";
  rateInput.className = "sb-input";
  rateInput.step = "0.5";
  rateInput.min = "0.1";
  rateInput.style.cssText = "flex:1;min-width:0";
  rateInput.placeholder = "10";
  rateRow.appendChild(rateInput);
  const rateSuffix = document.createElement("span");
  rateSuffix.className = "sb-rate-suffix";
  rateSuffix.textContent = "/s";
  rateRow.appendChild(rateSuffix);
  targetBody.appendChild(rateRow);

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

  // Copy-blueprint action — appended to solver body when a layout is ready.
  const blueprintSection = document.createElement("div");
  blueprintSection.className = "sb-actions";
  blueprintSection.style.display = "none";

  const copyBtn = document.createElement("button");
  copyBtn.className = "sb-btn sb-btn-secondary";
  copyBtn.textContent = "Copy Blueprint";
  copyBtn.style.flex = "1";
  blueprintSection.appendChild(copyBtn);

  const copyStatus = document.createElement("div");
  copyStatus.className = "sb-copy-status";
  blueprintSection.appendChild(copyStatus);
  solverBody.appendChild(blueprintSection);

  // ==================== VALIDATION ====================
  const { section: valSection, body: valBody, countEl: valCountEl } = makeSection(
    `<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><circle cx="8" cy="8" r="6"/><line x1="8" y1="5" x2="8" y2="8.5"/><circle cx="8" cy="11" r="0.8" fill="currentColor" stroke="none"/></svg>`,
    "Validation",
    "",
  );
  valSection.style.display = "none";
  inner.appendChild(valSection);

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
  let currentLayout: LayoutResult | null = null;
  let solveGeneration = 0;

  function scheduleAutoSolve(): void {
    if (debounceTimer !== null) clearTimeout(debounceTimer);
    debounceTimer = setTimeout(() => {
      runSolve().catch((err) => console.error("runSolve failed:", err));
    }, 150);
  }

  async function runSolve(): Promise<void> {
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

    const gen = ++solveGeneration;
    resultContainer.innerHTML = "";
    currentLayout = null;
    blueprintSection.style.display = "none";

    let result: SolverResult;
    try {
      result = await engine.solve(targetItem, targetRate, availableInputs, machineSelect.value);
    } catch (err) {
      if (gen !== solveGeneration) return;
      callbacks.renderGraph(null);
      if (solverCount) solverCount.textContent = "error";
      const errDiv = document.createElement("div");
      errDiv.className = "sb-result-error";
      errDiv.textContent = String(err);
      resultContainer.appendChild(errDiv);
      return;
    }
    if (gen !== solveGeneration) return;

    renderResult(resultContainer, result);
    callbacks.renderGraph(result);
    const totalMachines = result.machines.reduce((sum, m) => sum + Math.ceil(m.count), 0);
    if (solverCount) solverCount.textContent = `${totalMachines} machines`;

    let layout: LayoutResult;
    try {
      const maxTier = beltSelect.value || undefined;
      const onEvent = callbacks.startStreaming();
      layout = await engine.buildLayoutStreaming(result, maxTier, onEvent);
    } catch (err) {
      if (gen !== solveGeneration) return;
      const errDiv = document.createElement("div");
      errDiv.className = "sb-result-error";
      errDiv.textContent = `Layout error: ${err}`;
      resultContainer.appendChild(errDiv);
      return;
    }
    if (gen !== solveGeneration) return;

    currentLayout = layout;
    setRecipeFlows(result.machines);
    callbacks.renderLayout(layout);
    if (layout.warnings?.length) {
      for (const w of layout.warnings) {
        const wDiv = document.createElement("div");
        wDiv.className = "sb-warning";
        wDiv.textContent = `\u26A0 ${w}`;
        resultContainer.appendChild(wDiv);
      }
      blueprintSection.style.display = "none";
    } else {
      blueprintSection.style.display = "flex";
    }
    if (layout.trace?.length && options?.getDebugMode?.()) {
      resultContainer.appendChild(renderDebugPanel(layout.trace));
    }
  }

  copyBtn.addEventListener("click", async () => {
    if (!currentLayout) return;
    const bp = await engine.exportBlueprint(currentLayout, itemInput.value.trim());
    await navigator.clipboard.writeText(bp);
    copyStatus.textContent = "Copied!";
    setTimeout(() => { copyStatus.textContent = ""; }, 2000);
  });

  itemInput.addEventListener("input", scheduleAutoSolve);
  rateInput.addEventListener("input", scheduleAutoSolve);
  machineSelect.addEventListener("change", scheduleAutoSolve);
  beltSelect.addEventListener("change", scheduleAutoSolve);
  checkboxes.forEach((cb) => cb.addEventListener("change", scheduleAutoSolve));

  runSolve().catch((err) => console.error("runSolve failed:", err));

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
