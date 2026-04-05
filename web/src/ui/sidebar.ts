import type { Engine, SolverResult, ItemFlow } from "../engine.js";

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
  .sidebar-inner input[type="number"] {
    width: 100%;
    box-sizing: border-box;
    background: #252526;
    color: #e0e0e0;
    border: 1px solid #444;
    border-radius: 3px;
    padding: 4px 6px;
    font-size: 13px;
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
  .sidebar-inner button {
    background: #0e639c;
    color: #fff;
    border: none;
    border-radius: 3px;
    padding: 7px 12px;
    cursor: pointer;
    font-size: 13px;
    font-weight: 600;
  }
  .sidebar-inner button:hover {
    background: #1177bb;
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
`;

const DEFAULT_INPUTS = [
  "iron-plate",
  "copper-plate",
  "steel-plate",
  "stone",
  "coal",
  "water",
  "crude-oil",
  "iron-ore",
  "copper-ore",
];

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
    el.textContent = `${prefix}${flow.rate.toFixed(2)}/s ${flow.item}`;
    container.appendChild(el);
  });
}

export function renderSidebar(el: HTMLElement, engine: Engine): void {
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

  const itemSelect = document.createElement("select");
  engine.allProducibleItems().forEach((item) => itemSelect.appendChild(makeOption(item, "iron-gear-wheel")));
  inner.appendChild(itemSelect);

  const rateLabel = document.createElement("label");
  rateLabel.textContent = "Rate (items/sec)";
  inner.appendChild(rateLabel);

  const rateInput = document.createElement("input");
  rateInput.type = "number";
  rateInput.value = "10";
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
    cb.checked = true;
    cb.value = inp;
    checkboxes.set(inp, cb);
    lbl.appendChild(cb);
    lbl.appendChild(document.createTextNode(inp));
    inputsSection.appendChild(lbl);
  });
  inner.appendChild(inputsSection);

  const solveBtn = document.createElement("button");
  solveBtn.textContent = "Solve";
  inner.appendChild(solveBtn);

  const resultContainer = document.createElement("div");
  inner.appendChild(resultContainer);

  el.appendChild(inner);

  solveBtn.addEventListener("click", () => {
    resultContainer.innerHTML = "";
    const targetItem = itemSelect.value;
    const targetRate = parseFloat(rateInput.value);
    const machineEntity = machineSelect.value;
    const availableInputs = DEFAULT_INPUTS.filter((inp) => checkboxes.get(inp)?.checked);

    try {
      renderResult(resultContainer, engine.solve(targetItem, targetRate, availableInputs, machineEntity));
    } catch (err) {
      const errDiv = document.createElement("div");
      errDiv.className = "result-error";
      errDiv.textContent = String(err);
      resultContainer.appendChild(errDiv);
    }
  });
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
}
