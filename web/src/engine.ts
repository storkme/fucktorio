import wasmInit, {
  init,
  solve as wasmSolve,
  all_producible_items,
  all_producer_machines,
  default_machine_for_item as wasmDefaultMachineForItem,
  export_blueprint as wasmExportBlueprint,
} from "./wasm-pkg/fucktorio_wasm.js";

export type { SolverResult, MachineSpec, ItemFlow, LayoutResult } from "./wasm-pkg/fucktorio_wasm.js";
import type { LayoutResult } from "./wasm-pkg/fucktorio_wasm.js";

let itemsCache: string[] | null = null;
let machinesCache: string[] | null = null;

export async function initEngine(): Promise<void> {
  await wasmInit();
  init();
}

function solve(
  targetItem: string,
  targetRate: number,
  availableInputs: string[],
  machineEntity: string
) {
  return wasmSolve(targetItem, targetRate, availableInputs, machineEntity);
}

function allProducibleItems(): string[] {
  if (itemsCache === null) {
    itemsCache = all_producible_items();
  }
  return itemsCache;
}

function allProducerMachines(): string[] {
  if (machinesCache === null) {
    machinesCache = all_producer_machines();
  }
  return machinesCache;
}

function exportBlueprint(layout: LayoutResult, label: string): string {
  return wasmExportBlueprint(layout, label);
}

function defaultMachineForItem(item: string, fallback: string): string {
  return wasmDefaultMachineForItem(item, fallback);
}

export type Engine = {
  solve: typeof solve;
  allProducibleItems: typeof allProducibleItems;
  allProducerMachines: typeof allProducerMachines;
  exportBlueprint: typeof exportBlueprint;
  defaultMachineForItem: typeof defaultMachineForItem;
};

export function getEngine(): Engine {
  return { solve, allProducibleItems, allProducerMachines, exportBlueprint, defaultMachineForItem };
}
