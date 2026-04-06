import { defineConfig, type Plugin } from "vite";
import wasm from "vite-plugin-wasm";
import topLevelAwait from "vite-plugin-top-level-await";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { resolve } from "node:path";

const __dirname = fileURLToPath(new URL(".", import.meta.url));
const cratesDir = resolve(__dirname, "../crates");

function wasmPackPlugin(): Plugin {
  function buildWasm() {
    console.log("\n[wasm-pack] Building wasm-bindings...");
    const result = spawnSync(
      "wasm-pack",
      ["build", "../crates/wasm-bindings", "--target", "web", "--out-dir", resolve(__dirname, "src/wasm-pkg")],
      { stdio: "inherit", cwd: __dirname },
    );
    if (result.status !== 0) {
      console.error("[wasm-pack] Build failed (exit code " + result.status + ")");
    } else {
      console.log("[wasm-pack] Done.\n");
    }
  }

  return {
    name: "wasm-pack",
    buildStart() {
      buildWasm();
    },
    configureServer(server) {
      server.watcher.add(resolve(cratesDir, "**/*.rs"));
      server.watcher.add(resolve(cratesDir, "**/Cargo.toml"));
      server.watcher.on("change", (file) => {
        if (file.endsWith(".rs") || file.endsWith("Cargo.toml")) {
          buildWasm();
          server.ws.send({ type: "full-reload" });
        }
      });
    },
  };
}

export default defineConfig({
  root: ".",
  plugins: [wasmPackPlugin(), wasm(), topLevelAwait()],
  server: {
    host: "0.0.0.0",
  },
});
