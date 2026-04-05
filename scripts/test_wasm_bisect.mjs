// Bisect where the layout hangs by calling negotiate_and_route via Rust CLI
// vs via WASM and timing each.
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const pkgDir = resolve(__dirname, "../web/src/wasm-pkg");

const wasmMod = await import(`${pkgDir}/fucktorio_wasm.js`);
const wasmBytes = readFileSync(`${pkgDir}/fucktorio_wasm_bg.wasm`);
await wasmMod.default(new WebAssembly.Module(wasmBytes));
wasmMod.init();

const recipe = process.argv[2] ?? "advanced-circuit";
const rate = parseFloat(process.argv[3] ?? "0.5");

console.log(`Testing ${recipe} @ ${rate}/s\n`);

const t1 = performance.now();
const solverResult = wasmMod.solve(recipe, rate, [], "assembling-machine-3");
const t2 = performance.now();
console.log(`solve():  ${(t2 - t1).toFixed(1)}ms`);
console.log(`  machines: ${solverResult.machines.length}`);
console.log(`  external_inputs: ${solverResult.external_inputs.length}`);
console.log(`  external_outputs: ${solverResult.external_outputs.length}`);
console.log(`  dependency_order: ${solverResult.dependency_order.join(", ")}`);

// Print machine details
for (const m of solverResult.machines) {
  console.log(`    ${m.recipe}: ${Math.ceil(m.count)}x ${m.entity}`);
}

console.log("\nAttempting layout (will hang if buggy):");
const t3 = performance.now();
let done = false;
const heartbeat = setInterval(() => {
  if (!done) console.log(`  ... ${((performance.now() - t3) / 1000).toFixed(1)}s elapsed`);
}, 1000);

try {
  const layout = wasmMod.layout(solverResult);
  done = true;
  clearInterval(heartbeat);
  console.log(`layout() completed in ${(performance.now() - t3).toFixed(1)}ms`);
  console.log(`  ${layout.entities.length} entities, ${layout.width}×${layout.height}`);
} catch (e) {
  done = true;
  clearInterval(heartbeat);
  console.log(`layout() threw:`, e);
}
