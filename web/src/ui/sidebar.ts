import type { Engine, SolverResult, LayoutResult, ItemFlow } from "../engine.js";
import { readUrlState, writeUrlState, DEFAULT_INPUTS } from "../state.js";
import { beltTierForRate, hexToCss } from "../renderer/colors.js";

const STYLE = `
  .sidebar-inner {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px;
    height: 100%;
    box-sizing: border-box;
    overflow-y: auto;
    background: #1e1e1e;
    color: #e0e0e0;
    font-family: sans-serif;
    font-size: 13px;
  }
  .sidebar-inner h1 {
    margin: 0 0 4px;
    font-size: 16px;
    font-weight: 700;
    letter-spacing: 0.05em;
    color: #c8c8c8;
  }
  .sidebar-inner label {
    display: block;
    margin-bottom: 3px;
    color: #aaa;
    font-size: 12px;
  }
  .sidebar-inner select,
  .sidebar-inner input[type="number"],
  .sidebar-inner input[type="text"] {
    width: 100%;
    box-sizing: border-box;
    background: #252526;
    color: #e0e0e0;
    border: 1px solid #444;
    border-radius: 3px;
    padding: 4px 6px;
    font-size: 13px;
  }
  .sidebar-inner input[type="text"].item-invalid {
    border-color: #c44;
    color: #f88;
  }
  .sidebar-inner .inputs-section {
    background: #252526;
    border-radius: 4px;
    padding: 8px;
  }
  .sidebar-inner .inputs-section label {
    display: flex;
    align-items: center;
    gap: 6px;
    color: #ccc;
    margin-bottom: 4px;
    font-size: 12px;
  }
  .sidebar-inner .inputs-section label input[type="checkbox"] {
    accent-color: #569cd6;
  }
  .sidebar-inner .result-error {
    color: #f44;
    font-family: monospace;
    font-size: 12px;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .sidebar-inner .result-section h3 {
    margin: 0 0 6px;
    font-size: 13px;
    font-weight: 600;
    color: #9cdcfe;
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }
  .sidebar-inner .machine-entry {
    background: #252526;
    border-radius: 4px;
    padding: 7px 9px;
    margin-bottom: 6px;
    font-family: monospace;
    font-size: 12px;
  }
  .sidebar-inner .machine-title {
    font-weight: 700;
    color: #dcdcaa;
  }
  .sidebar-inner .machine-recipe {
    color: #888;
    margin-bottom: 4px;
  }
  .sidebar-inner .machine-flows {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .sidebar-inner .flow-in {
    color: #9cdcfe;
  }
  .sidebar-inner .flow-out {
    color: #b5cea8;
  }
  .sidebar-inner .ext-flow {
    font-family: monospace;
    font-size: 12px;
    color: #c8c8c8;
    padding: 2px 0;
  }
  .sidebar-inner .totals-section {
    background: #252526;
    border-radius: 4px;
    padding: 8px 10px;
    font-family: monospace;
    font-size: 12px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .sidebar-inner .totals-section .totals-row {
    color: #c8c8c8;
  }
  .sidebar-inner .belt-chip {
    display: inline-block;
    padding: 1px 6px;
    border-radius: 3px;
    font-size: 11px;
    font-weight: 600;
    border-left: 3px solid;
    margin-left: 6px;
  }
  .sidebar-inner .belt-overflow {
    color: #f88;
  }
  .sidebar-inner .tier-rate {
    font-weight: 700;
  }
  .sidebar-inner button.layout-btn {
    width: 100%;
    padding: 8px;
    background: #0e639c;
    border: none;
    color: #fff;
    border-radius: 3px;
    cursor: pointer;
    font-size: 13px;
    font-weight: 500;
  }
  .sidebar-inner button.layout-btn:disabled {
    background: #333;
    color: #777;
    cursor: default;
  }
  .sidebar-inner button.copy-btn {
    width: 100%;
    padding: 8px;
    background: #2d7a2d;
    border: none;
    color: #fff;
    border-radius: 3px;
    cursor: pointer;
    font-size: 13px;
    font-weight: 500;
  }
  .sidebar-inner .copy-status {
    margin-top: 4px;
    font-size: 11px;
    color: #7ec87e;
    text-align: center;
  }
`;

function makeOption(value: string, defaultValue: string): HTMLOptionElement {
  const opt = document.createElement("option");
  opt.value = value;
  opt.textContent = value;
  if (value === defaultValue) opt.selected = true;
  return opt;
}

function makeSection(heading: string): { section: HTMLDivElement; body: HTMLDivElement } {
  const section = document.createElement("div");
  section.className = "result-section";
  const h3 = document.createElement("h3");
  h3.textContent = heading;
  section.appendChild(h3);
  const body = document.createElement("div");
  section.appendChild(body);
  return { section, body };
}

function appendFlows(container: HTMLElement, flows: ItemFlow[], className: string, prefix: string): void {
  flows.forEach((flow) => {
    const el = document.createElement("div");
    el.className = className;
    const tier = beltTierForRate(flow.rate);
    const rateColor = tier ? hexToCss(tier.color) : "#f88";
    el.innerHTML =
      `${prefix}<span class="tier-rate" style="color:${rateColor}">${flow.rate.toFixed(2)}/s</span> ${flow.item}`;
    container.appendChild(el);
  });
}

export interface SidebarCallbacks {
  renderGraph: (result: SolverResult | null) => void;
  renderLayout: (layout: LayoutResult) => void;
}

export function renderSidebar(
  el: HTMLElement,
  engine: Engine,
  callbacks: SidebarCallbacks,
): void {
  el.innerHTML = "";

  if (!document.getElementById("fucktorio-sidebar-style")) {
    const styleEl = document.createElement("style");
    styleEl.id = "fucktorio-sidebar-style";
    styleEl.textContent = STYLE;
    document.head.appendChild(styleEl);
  }

  const inner = document.createElement("div");
  inner.className = "sidebar-inner";

  const heading = document.createElement("h1");
  heading.textContent = "Fucktorio";
  inner.appendChild(heading);

  const itemLabel = document.createElement("label");
  itemLabel.textContent = "Target item";
  inner.appendChild(itemLabel);

  const datalist = document.createElement("datalist");
  datalist.id = "fucktorio-items-datalist";
  const allItems = engine.allProducibleItems();
  const itemSet = new Set(allItems);
  allItems.forEach((item) => {
    const opt = document.createElement("option");
    opt.value = item;
    datalist.appendChild(opt);
  });
  inner.appendChild(datalist);

  const itemInput = document.createElement("input");
  itemInput.type = "text";
  itemInput.setAttribute("list", "fucktorio-items-datalist");
  itemInput.autocomplete = "off";
  inner.appendChild(itemInput);

  const rateLabel = document.createElement("label");
  rateLabel.textContent = "Rate (items/sec)";
  inner.appendChild(rateLabel);

  const rateInput = document.createElement("input");
  rateInput.type = "number";
  rateInput.step = "0.5";
  rateInput.min = "0.1";
  inner.appendChild(rateInput);

  const machineLabel = document.createElement("label");
  machineLabel.textContent = "Machine";
  inner.appendChild(machineLabel);

  const machineSelect = document.createElement("select");
  engine.allProducerMachines().forEach((m) => machineSelect.appendChild(makeOption(m, "assembling-machine-3")));
  inner.appendChild(machineSelect);

  const inputsLabel = document.createElement("label");
  inputsLabel.textContent = "Available inputs";
  inner.appendChild(inputsLabel);

  const inputsSection = document.createElement("div");
  inputsSection.className = "inputs-section";

  const checkboxes = new Map<string, HTMLInputElement>();
  DEFAULT_INPUTS.forEach((inp) => {
    const lbl = document.createElement("label");
    const cb = document.createElement("input");
    cb.type = "checkbox";
    cb.value = inp;
    checkboxes.set(inp, cb);
    lbl.appendChild(cb);
    lbl.appendChild(document.createTextNode(inp));
    inputsSection.appendChild(lbl);
  });
  inner.appendChild(inputsSection);

  const resultContainer = document.createElement("div");
  inner.appendChild(resultContainer);

  const beltLabel = document.createElement("label");
  beltLabel.textContent = "Max belt tier";
  inner.appendChild(beltLabel);

  const beltSelect = document.createElement("select");
  [
    ["Auto", ""],
    ["Yellow (15/s)", "transport-belt"],
    ["Red (30/s)", "fast-transport-belt"],
    ["Blue (45/s)", "express-transport-belt"],
  ].forEach(([label, value]) => {
    const opt = document.createElement("option");
    opt.value = value;
    opt.textContent = label;
    beltSelect.appendChild(opt);
  });
  if (urlState.belt) beltSelect.value = urlState.belt;
  inner.appendChild(beltSelect);

  const layoutBtn = document.createElement("button");
  layoutBtn.className = "layout-btn";
  layoutBtn.textContent = "Generate Layout";
  layoutBtn.disabled = true;
  inner.appendChild(layoutBtn);

  const blueprintSection = document.createElement("div");
  blueprintSection.style.display = "none";
  const copyBtn = document.createElement("button");
  copyBtn.className = "copy-btn";
  copyBtn.textContent = "Copy Blueprint";
  blueprintSection.appendChild(copyBtn);
  const copyStatus = document.createElement("div");
  copyStatus.className = "copy-status";
  blueprintSection.appendChild(copyStatus);
  inner.appendChild(blueprintSection);

  el.appendChild(inner);

  const urlState = readUrlState();
  itemInput.value = urlState.item;
  rateInput.value = String(urlState.rate);
  machineSelect.value =
    urlState.machine ?? engine.defaultMachineForItem(urlState.item, "assembling-machine-3");
  const machineOpts = machineSelect.options;
  checkboxes.forEach((cb, name) => {
    cb.checked = urlState.inputs.includes(name);
  });

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
    // Invalidate any previous layout — inputs have changed.
    currentLayout = null;
    blueprintSection.style.display = "none";
    try {
      const result = engine.solve(targetItem, targetRate, availableInputs, machineSelect.value);
      currentResult = result;
      renderResult(resultContainer, result);
      callbacks.renderGraph(result);
      layoutBtn.disabled = false;
    } catch (err) {
      currentResult = null;
      callbacks.renderGraph(null);
      layoutBtn.disabled = true;
      const errDiv = document.createElement("div");
      errDiv.className = "result-error";
      errDiv.textContent = String(err);
      resultContainer.appendChild(errDiv);
    }
  }

  layoutBtn.addEventListener("click", () => {
    if (!currentResult) return;
    try {
      const maxTier = beltSelect.value || undefined;
      currentLayout = engine.buildLayout(currentResult, maxTier);
      callbacks.renderLayout(currentLayout);
      blueprintSection.style.display = "block";
      if (currentLayout.warnings?.length) {
        for (const w of currentLayout.warnings) {
          const wDiv = document.createElement("div");
          wDiv.className = "result-error";
          wDiv.textContent = `\u26A0 ${w}`;
          resultContainer.appendChild(wDiv);
        }
      }
    } catch (err) {
      const errDiv = document.createElement("div");
      errDiv.className = "result-error";
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
}

function renderResult(container: HTMLElement, result: SolverResult): void {
  const { section: machinesSection, body: machinesBody } = makeSection("Machines");

  result.machines.forEach((machine) => {
    const entry = document.createElement("div");
    entry.className = "machine-entry";

    const title = document.createElement("div");
    title.className = "machine-title";
    title.textContent = `${machine.count.toFixed(2)} × ${machine.entity}`;
    entry.appendChild(title);

    const recipe = document.createElement("div");
    recipe.className = "machine-recipe";
    recipe.textContent = `  → ${machine.recipe}`;
    entry.appendChild(recipe);

    const flows = document.createElement("div");
    flows.className = "machine-flows";
    appendFlows(flows, machine.inputs, "flow-in", "  ▶ ");
    appendFlows(flows, machine.outputs, "flow-out", "  ◀ ");
    entry.appendChild(flows);

    machinesBody.appendChild(entry);
  });

  container.appendChild(machinesSection);

  if (result.external_inputs.length > 0) {
    const { section: extSection, body: extBody } = makeSection("External inputs");
    appendFlows(extBody, result.external_inputs, "ext-flow", "");
    container.appendChild(extSection);
  }

  const totalsDiv = document.createElement("div");
  totalsDiv.className = "totals-section";

  const totalMachines = result.machines.reduce((sum, m) => sum + Math.ceil(m.count), 0);
  const chainDepth = result.dependency_order.length;

  const machinesRow = document.createElement("div");
  machinesRow.className = "totals-row";
  machinesRow.textContent = `Total machines: ${totalMachines}`;
  totalsDiv.appendChild(machinesRow);

  const depthRow = document.createElement("div");
  depthRow.className = "totals-row";
  depthRow.textContent = `Chain depth: ${chainDepth}`;
  totalsDiv.appendChild(depthRow);

  result.external_outputs.forEach((flow) => {
    const row = document.createElement("div");
    row.className = "totals-row";
    const tier = beltTierForRate(flow.rate);
    if (tier) {
      const tierColor = hexToCss(tier.color);
      row.innerHTML =
        `${flow.item} ${flow.rate.toFixed(1)}/s` +
        `<span class="belt-chip" style="border-color:${tierColor};color:${tierColor}">${tier.name}</span>`;
    } else {
      row.innerHTML =
        `${flow.item} ${flow.rate.toFixed(1)}/s` +
        `<span class="belt-overflow">⚠ overflow (needs multiple belts)</span>`;
    }
    totalsDiv.appendChild(row);
  });

  container.appendChild(totalsDiv);
}
