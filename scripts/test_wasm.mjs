// Run wasm solver + layout directly from Node, timing each call.
// Usage: node scripts/test_wasm.mjs [recipe] [rate]
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const pkgDir = resolve(__dirname, "../web/src/wasm-pkg");

// Dynamic import of wasm pkg
const wasmMod = await import(`${pkgDir}/fucktorio_wasm.js`);
const wasmBytes = readFileSync(`${pkgDir}/fucktorio_wasm_bg.wasm`);
await wasmMod.default(new WebAssembly.Module(wasmBytes));
wasmMod.init();

const recipe = process.argv[2] ?? "advanced-circuit";
const rate = parseFloat(process.argv[3] ?? "0.5");

console.log(`Testing ${recipe} @ ${rate}/s\n`);

const t1 = performance.now();
const result = wasmMod.solve(recipe, rate, [], "assembling-machine-3");
const t2 = performance.now();
console.log(`solve():  ${(t2 - t1).toFixed(1)}ms  → ${result.machines.length} machines`);

const t3 = performance.now();
let layout;
try {
  layout = wasmMod.layout(result);
} catch (e) {
  console.log(`layout() threw after ${(performance.now() - t3).toFixed(1)}ms:`);
  console.log(e);
  process.exit(1);
}
const t4 = performance.now();
console.log(`layout(): ${(t4 - t3).toFixed(1)}ms  → ${layout.entities.length} entities, ${layout.width}×${layout.height}`);

console.log(`\nTotal: ${(t4 - t1).toFixed(1)}ms`);
