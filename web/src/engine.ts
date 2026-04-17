import type {
  SolverResult,
  LayoutResult,
  ValidationIssue,
} from "./wasm-pkg/fucktorio_wasm.js";

export type {
  SolverResult,
  MachineSpec,
  ItemFlow,
  LayoutResult,
  LayoutRegion,
  RegionKind,
  RegionPort,
  PortPoint,
  PortIo,
  PlacedEntity,
  EntityDirection,
  ValidationIssue,
  TraceEvent,
} from "./wasm-pkg/fucktorio_wasm.js";

type WorkerResponse = { id: number; ok: true; result: unknown } | { id: number; ok: false; error: string };

let worker: Worker | null = null;
let nextId = 0;
const pending = new Map<number, { resolve: (v: unknown) => void; reject: (e: Error) => void }>();

let itemsCache: string[] = [];
let machinesCache: string[] = [];
let defaultMachineCache = new Map<string, string>();

let activeCountListeners = new Set<(active: number) => void>();
let activeCount = 0;

function onActive(delta: number): void {
  activeCount += delta;
  for (const cb of activeCountListeners) cb(activeCount);
}

/** Subscribe to engine activity (>0 while any RPC is in flight). Returns an unsubscribe fn. */
export function onEngineActivity(cb: (active: number) => void): () => void {
  activeCountListeners.add(cb);
  cb(activeCount);
  return () => activeCountListeners.delete(cb);
}

function call<T>(payload: Record<string, unknown>): Promise<T> {
  if (!worker) throw new Error("Engine not initialized — call initEngine() first");
  const id = ++nextId;
  onActive(+1);
  return new Promise<T>((resolve, reject) => {
    pending.set(id, {
      resolve: (v) => {
        onActive(-1);
        resolve(v as T);
      },
      reject: (e) => {
        onActive(-1);
        reject(e);
      },
    });
    worker!.postMessage({ id, ...payload });
  });
}

export async function initEngine(): Promise<void> {
  if (worker) return;
  worker = new Worker(new URL("./workers/engine.worker.ts", import.meta.url), {
    type: "module",
    name: "fucktorio-engine",
  });
  worker.onmessage = (e: MessageEvent<WorkerResponse>) => {
    const { id } = e.data;
    const p = pending.get(id);
    if (!p) return;
    pending.delete(id);
    if (e.data.ok) p.resolve(e.data.result);
    else p.reject(new Error(e.data.error));
  };
  worker.onerror = (e) => {
    console.error("[engine.worker] error", e);
  };

  await call<null>({ method: "init" });
  itemsCache = await call<string[]>({ method: "allProducibleItems" });
  machinesCache = await call<string[]>({ method: "allProducerMachines" });
  const defaults = await call<[string, string][]>({
    method: "defaultMachinesForItems",
    items: itemsCache,
    fallback: "assembling-machine-3",
  });
  defaultMachineCache = new Map(defaults);
}

function solve(
  targetItem: string,
  targetRate: number,
  availableInputs: string[],
  machineEntity: string,
): Promise<SolverResult> {
  return call<SolverResult>({
    method: "solve",
    targetItem,
    targetRate,
    availableInputs,
    machineEntity,
  });
}

function allProducibleItems(): string[] {
  return itemsCache;
}

function allProducerMachines(): string[] {
  return machinesCache;
}

function buildLayout(result: SolverResult, maxBeltTier?: string): Promise<LayoutResult> {
  return call<LayoutResult>({ method: "layout", result, maxBeltTier: maxBeltTier ?? null });
}

function buildLayoutTraced(result: SolverResult, maxBeltTier?: string): Promise<LayoutResult> {
  return call<LayoutResult>({ method: "layoutTraced", result, maxBeltTier: maxBeltTier ?? null });
}

function exportBlueprint(layout: LayoutResult, label: string): Promise<string> {
  return call<string>({ method: "exportBlueprint", layout, label });
}

function defaultMachineForItem(item: string, fallback: string): string {
  return defaultMachineCache.get(item) ?? fallback;
}

function validateLayout(
  layout: LayoutResult,
  solverResult: SolverResult | null,
): Promise<ValidationIssue[]> {
  return call<ValidationIssue[]>({ method: "validateLayout", layout, solverResult });
}

export function parseBlueprint(bpString: string): Promise<LayoutResult> {
  return call<LayoutResult>({ method: "parseBlueprint", bp: bpString });
}

export type Engine = {
  solve: typeof solve;
  allProducibleItems: typeof allProducibleItems;
  allProducerMachines: typeof allProducerMachines;
  buildLayout: typeof buildLayout;
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
    buildLayoutTraced,
    exportBlueprint,
    defaultMachineForItem,
    validateLayout,
  };
}
