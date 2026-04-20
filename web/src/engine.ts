import type {
  SolverResult,
  LayoutResult,
  PlacedEntity,
  ValidationIssue,
  TraceEvent,
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

/**
 * Result shape for `engine.solveFixture` — mirrors the
 * `SolveFixtureResponse` in `crates/wasm-bindings/src/lib.rs`.
 */
export interface SolveFixtureResult {
  entities: PlacedEntity[];
  cost: number;
  stats: SatStats;
}

export interface SatStats {
  variables: number;
  clauses: number;
  solve_time_us: number;
  zone_width: number;
  zone_height: number;
}

type WorkerResponse =
  | { id: number; ok: true; result: unknown }
  | { id: number; ok: false; error: string }
  | { id: number; streamEvents: unknown[] };

interface PendingEntry {
  resolve: (v: unknown) => void;
  reject: (e: Error) => void;
  onEvent?: (evt: TraceEvent) => void;
}

let worker: Worker | null = null;
let nextId = 0;
const pending = new Map<number, PendingEntry>();
let activeStreamingId: number | null = null;

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

function call<T>(payload: Record<string, unknown>, onEvent?: (evt: TraceEvent) => void): Promise<T> {
  if (!worker) throw new Error("Engine not initialized — call initEngine() first");
  const id = ++nextId;
  onActive(+1);
  return new Promise<T>((resolve, reject) => {
    pending.set(id, {
      resolve: (v) => {
        onActive(-1);
        if (activeStreamingId === id) activeStreamingId = null;
        resolve(v as T);
      },
      reject: (e) => {
        onActive(-1);
        if (activeStreamingId === id) activeStreamingId = null;
        reject(e);
      },
      onEvent,
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
    if ("streamEvents" in e.data) {
      // Partial batch of events during a streaming call — forward to listener,
      // keep pending open until the final response arrives.
      if (p.onEvent) {
        for (const evt of e.data.streamEvents) p.onEvent(evt as TraceEvent);
      }
      return;
    }
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

async function solve(
  targetItem: string,
  targetRate: number,
  availableInputs: string[],
  machineEntity: string,
): Promise<SolverResult> {
  // If a streaming layout is in flight, the user has just typed a new target
  // and is waiting for feedback — kill the old WASM work so solve isn't
  // queued behind a slow layout that's about to be thrown away anyway.
  if (activeStreamingId !== null) await supersedeWorker();
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

/**
 * Kill the current worker and respawn a fresh one. Rejects all pending
 * promises so stale callers see the supersession. Used to cancel an
 * in-flight streaming layout when the user triggers a new solve.
 */
async function supersedeWorker(): Promise<void> {
  if (!worker) return;
  worker.terminate();
  worker = null;
  const superseded = new Error("Engine superseded by a newer request");
  for (const [, p] of pending) p.reject(superseded);
  pending.clear();
  activeStreamingId = null;
  await initEngine();
}

async function buildLayoutStreaming(
  result: SolverResult,
  maxBeltTier: string | undefined,
  onEvent: (evt: TraceEvent) => void,
): Promise<LayoutResult> {
  if (activeStreamingId !== null) {
    await supersedeWorker();
  }
  const id = ++nextId;
  activeStreamingId = id;
  onActive(+1);
  return new Promise<LayoutResult>((resolve, reject) => {
    pending.set(id, {
      resolve: (v) => {
        onActive(-1);
        if (activeStreamingId === id) activeStreamingId = null;
        resolve(v as LayoutResult);
      },
      reject: (e) => {
        onActive(-1);
        if (activeStreamingId === id) activeStreamingId = null;
        reject(e);
      },
      onEvent,
    });
    worker!.postMessage({
      id,
      method: "layoutStreaming",
      result,
      maxBeltTier: maxBeltTier ?? null,
    });
  });
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

/**
 * Solve a SAT-zone fixture, optionally pinning a set of painted
 * entities as solver assumptions. Resolves to `null` when the solver
 * returns UNSAT or when any pin was rejected (out of bounds, on a
 * forbidden tile, unsupported entity, item not in the fixture).
 *
 * Used by the F2 SAT-zone editor to drive the live validity indicator
 * and the ghost-completion overlay (`entities \ pins`).
 */
function solveFixture(
  fixtureJson: string,
  pins: PlacedEntity[],
): Promise<SolveFixtureResult | null> {
  return call<SolveFixtureResult | null>({
    method: "solveFixture",
    fixtureJson,
    pinsJson: JSON.stringify(pins),
  });
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
  buildLayoutStreaming: typeof buildLayoutStreaming;
  exportBlueprint: typeof exportBlueprint;
  defaultMachineForItem: typeof defaultMachineForItem;
  validateLayout: typeof validateLayout;
  solveFixture: typeof solveFixture;
};

export function getEngine(): Engine {
  return {
    solve,
    allProducibleItems,
    allProducerMachines,
    buildLayout,
    buildLayoutTraced,
    buildLayoutStreaming,
    exportBlueprint,
    defaultMachineForItem,
    validateLayout,
    solveFixture,
  };
}
