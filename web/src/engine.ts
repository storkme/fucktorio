import wasmInit, {
  init,
  solve as wasmSolve,
  all_producible_items,
  all_producer_machines,
  default_machine_for_item as wasmDefaultMachineForItem,
  export_blueprint as wasmExportBlueprint,
  layout as wasmLayout,
  layout_ghost as wasmLayoutGhost,
  layout_direct as wasmLayoutDirect,
  layout_bare as wasmLayoutBare,
  layout_traced as wasmLayoutTraced,
  parse_blueprint as wasmParseBlueprint,
  validate_layout as wasmValidateLayout,
} from "./wasm-pkg/fucktorio_wasm.js";

export type {
  SolverResult,
  MachineSpec,
  ItemFlow,
  LayoutResult,
  LayoutRegion,
  PortSpec,
  PortEdge,
  PortIo,
  PlacedEntity,
  EntityDirection,
  ValidationIssue,
  TraceEvent,
} from "./wasm-pkg/fucktorio_wasm.js";
import type { SolverResult, LayoutResult, ValidationIssue } from "./wasm-pkg/fucktorio_wasm.js";

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
  machineEntity: string,
): SolverResult {
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

function buildLayout(result: SolverResult, maxBeltTier?: string): LayoutResult {
  return wasmLayout(result, maxBeltTier ?? null);
}

function buildLayoutGhost(result: SolverResult, maxBeltTier?: string): LayoutResult {
  return wasmLayoutGhost(result, maxBeltTier ?? null);
}

function buildLayoutDirect(result: SolverResult, maxBeltTier?: string): LayoutResult {
  return wasmLayoutDirect(result, maxBeltTier ?? null);
}

function buildLayoutBare(result: SolverResult, maxBeltTier?: string): LayoutResult {
  return wasmLayoutBare(result, maxBeltTier ?? null);
}

function buildLayoutTraced(result: SolverResult, maxBeltTier?: string): LayoutResult {
  return wasmLayoutTraced(result, maxBeltTier ?? null);
}

function exportBlueprint(layout: LayoutResult, label: string): string {
  return wasmExportBlueprint(layout, label);
}

function defaultMachineForItem(item: string, fallback: string): string {
  return wasmDefaultMachineForItem(item, fallback);
}

function validateLayout(layout: LayoutResult, solverResult: SolverResult | null): ValidationIssue[] {
  return wasmValidateLayout(layout, solverResult, "Bus");
}

export function parseBlueprint(bpString: string): LayoutResult {
  return wasmParseBlueprint(bpString);
}

export type Engine = {
  solve: typeof solve;
  allProducibleItems: typeof allProducibleItems;
  allProducerMachines: typeof allProducerMachines;
  buildLayout: typeof buildLayout;
  buildLayoutGhost: typeof buildLayoutGhost;
  buildLayoutDirect: typeof buildLayoutDirect;
  buildLayoutBare: typeof buildLayoutBare;
  buildLayoutTraced: typeof buildLayoutTraced;
  exportBlueprint: typeof exportBlueprint;
  defaultMachineForItem: typeof defaultMachineForItem;
  validateLayout: typeof validateLayout;
};

export function getEngine(): Engine {
  return {
    solve,
    allProducibleItems,
    allProducerMachines,
    buildLayout,
    buildLayoutGhost,
    buildLayoutDirect,
    buildLayoutBare,
    buildLayoutTraced,
    exportBlueprint,
    defaultMachineForItem,
    validateLayout,
  };
}
