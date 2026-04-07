export interface FormState {
  item: string;
  rate: number;
  /** null means "no machine in URL — caller should derive from item". */
  machine: string | null;
  inputs: string[];
  /** Max belt tier override, e.g. "transport-belt". null = auto. */
  belt: string | null;
}

export const DEFAULT_INPUTS: string[] = [
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

export const DEFAULT_ITEM = "iron-gear-wheel";
export const DEFAULT_RATE = 10;
export const DEFAULT_MACHINE = "assembling-machine-3";

export function readUrlState(): FormState {
  const params = new URLSearchParams(window.location.search);

  const item = params.get("item") ?? DEFAULT_ITEM;
  const rawRate = parseFloat(params.get("rate") ?? "");
  const rate = isNaN(rawRate) || rawRate <= 0 ? DEFAULT_RATE : rawRate;
  const machine = params.get("machine");
  const inParam = params.get("in");
  const inputs = inParam ? inParam.split(",").filter((s) => s.length > 0) : DEFAULT_INPUTS;
  const belt = params.get("belt");

  return { item, rate, machine, inputs, belt };
}

export function writeUrlState(state: Omit<FormState, "machine"> & { machine: string }): void {
  const isDefault =
    state.item === DEFAULT_ITEM &&
    state.rate === DEFAULT_RATE &&
    state.machine === DEFAULT_MACHINE &&
    state.inputs.length === DEFAULT_INPUTS.length &&
    state.inputs.every((v, i) => v === DEFAULT_INPUTS[i]) &&
    !state.belt;

  if (isDefault) {
    history.replaceState(null, "", window.location.pathname);
    return;
  }

  const params = new URLSearchParams();
  params.set("item", state.item);
  params.set("rate", String(state.rate));
  params.set("machine", state.machine);
  params.set("in", state.inputs.join(","));
  if (state.belt) params.set("belt", state.belt);
  history.replaceState(null, "", "?" + params.toString());
}
