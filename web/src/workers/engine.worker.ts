import wasmInit, {
  init,
  solve,
  all_producible_items,
  all_producer_machines,
  default_machine_for_item,
  export_blueprint,
  layout,
  layout_traced,
  parse_blueprint,
  validate_layout,
} from "../wasm-pkg/fucktorio_wasm.js";
import type {
  SolverResult,
  LayoutResult,
} from "../wasm-pkg/fucktorio_wasm.js";

type Request =
  | { id: number; method: "init" }
  | { id: number; method: "allProducibleItems" }
  | { id: number; method: "allProducerMachines" }
  | { id: number; method: "defaultMachinesForItems"; items: string[]; fallback: string }
  | {
      id: number;
      method: "solve";
      targetItem: string;
      targetRate: number;
      availableInputs: string[];
      machineEntity: string;
    }
  | { id: number; method: "layout"; result: SolverResult; maxBeltTier: string | null }
  | { id: number; method: "layoutTraced"; result: SolverResult; maxBeltTier: string | null }
  | { id: number; method: "exportBlueprint"; layout: LayoutResult; label: string }
  | {
      id: number;
      method: "validateLayout";
      layout: LayoutResult;
      solverResult: SolverResult | null;
    }
  | { id: number; method: "parseBlueprint"; bp: string };

let ready: Promise<void> | null = null;

function ensureReady(): Promise<void> {
  if (!ready) {
    ready = (async () => {
      await wasmInit();
      init();
    })();
  }
  return ready;
}

function post(id: number, ok: boolean, value: unknown): void {
  if (ok) {
    (self as unknown as Worker).postMessage({ id, ok: true, result: value });
  } else {
    (self as unknown as Worker).postMessage({ id, ok: false, error: value });
  }
}

self.onmessage = async (e: MessageEvent<Request>) => {
  const req = e.data;
  try {
    await ensureReady();
    let result: unknown;
    switch (req.method) {
      case "init":
        result = null;
        break;
      case "allProducibleItems":
        result = all_producible_items();
        break;
      case "allProducerMachines":
        result = all_producer_machines();
        break;
      case "defaultMachinesForItems": {
        const out: [string, string][] = [];
        for (const item of req.items) {
          out.push([item, default_machine_for_item(item, req.fallback)]);
        }
        result = out;
        break;
      }
      case "solve":
        result = solve(req.targetItem, req.targetRate, req.availableInputs, req.machineEntity);
        break;
      case "layout":
        result = layout(req.result, req.maxBeltTier ?? undefined);
        break;
      case "layoutTraced":
        result = layout_traced(req.result, req.maxBeltTier ?? undefined);
        break;
      case "exportBlueprint":
        result = export_blueprint(req.layout, req.label);
        break;
      case "validateLayout":
        result = validate_layout(req.layout, req.solverResult ?? undefined, "Bus");
        break;
      case "parseBlueprint":
        result = parse_blueprint(req.bp);
        break;
    }
    post(req.id, true, result);
  } catch (err) {
    post(req.id, false, err instanceof Error ? err.message : String(err));
  }
};
