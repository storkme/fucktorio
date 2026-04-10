/**
 * Snapshot loader — decode .fls layout snapshots and hydrate the web app.
 *
 * Wire format: "fls1" + base64(gzip(JSON))
 */

import { gunzipSync } from "fflate";

// ---------------------------------------------------------------------------
// Types (mirror crates/core/src/snapshot.rs)
// ---------------------------------------------------------------------------

export interface SnapshotParams {
  item: string;
  rate: number;
  machine: string;
  belt_tier: string | null;
  inputs: string[];
}

export interface SnapshotContext {
  test_name?: string;
  label?: string;
  git_sha?: string;
}

export interface SnapshotValidation {
  issues: SnapshotValidationIssue[];
  truncated: boolean;
}

export interface SnapshotValidationIssue {
  severity: "Error" | "Warning";
  category: string;
  message: string;
  x?: number;
  y?: number;
}

export interface SnapshotTrace {
  events: unknown[];
  complete: boolean;
}

export interface LayoutSnapshot {
  version: number;
  created_at: string;
  source: "test" | "manual" | "ci";
  params: SnapshotParams;
  context: SnapshotContext;
  layout: {
    entities: unknown[];
    width?: number;
    height?: number;
    warnings?: string[];
    regions?: unknown[];
    trace?: unknown[];
  };
  validation: SnapshotValidation;
  trace: SnapshotTrace;
  solver?: unknown;
}

// ---------------------------------------------------------------------------
// Decoder
// ---------------------------------------------------------------------------

const MAGIC = "fls1";

export async function decodeSnapshot(input: string | ArrayBuffer): Promise<LayoutSnapshot> {
  const text = typeof input === "string" ? input : new TextDecoder().decode(input);
  if (!text.startsWith(MAGIC)) {
    throw new Error(`Not a layout snapshot: expected "${MAGIC}" prefix, got "${text.slice(0, 4)}"`);
  }
  const b64 = text.slice(MAGIC.length);
  const gz = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
  const jsonBytes = gunzipSync(gz);
  const json = new TextDecoder().decode(jsonBytes);
  return JSON.parse(json) as LayoutSnapshot;
}

// ---------------------------------------------------------------------------
// File loader helpers
// ---------------------------------------------------------------------------

export function readSnapshotFile(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result as string);
    reader.onerror = () => reject(new Error("Failed to read file"));
    reader.readAsText(file);
  });
}

// ---------------------------------------------------------------------------
// Drag-drop setup
// ---------------------------------------------------------------------------

export function setupSnapshotDropZone(
  element: HTMLElement,
  onLoad: (snapshot: LayoutSnapshot) => void,
): void {
  element.addEventListener("dragover", (e) => {
    e.preventDefault();
    e.stopPropagation();
    element.style.outline = "2px dashed #569cd6";
  });

  element.addEventListener("dragleave", () => {
    element.style.outline = "none";
  });

  element.addEventListener("drop", async (e) => {
    e.preventDefault();
    e.stopPropagation();
    element.style.outline = "none";

    const file = e.dataTransfer?.files[0];
    if (!file) return;
    if (!file.name.endsWith(".fls")) {
      alert("Expected a .fls snapshot file");
      return;
    }
    try {
      const text = await readSnapshotFile(file);
      const snapshot = await decodeSnapshot(text);
      onLoad(snapshot);
    } catch (err) {
      alert(`Failed to load snapshot: ${err}`);
    }
  });
}

// ---------------------------------------------------------------------------
// Banner UI
// ---------------------------------------------------------------------------

export interface BannerCallbacks {
  onClear: () => void;
}

export function showSnapshotBanner(
  parent: HTMLElement,
  snapshot: LayoutSnapshot,
  callbacks: BannerCallbacks,
): HTMLDivElement {
  const banner = document.createElement("div");
  banner.style.cssText =
    "background:rgba(0,40,80,0.85);color:#e0e0e0;font:11px monospace;padding:6px 10px;border-bottom:1px solid #569cd6;display:flex;align-items:center;gap:8px;flex-wrap:wrap;z-index:20";

  const { params, context, trace, validation } = snapshot;
  const label = context.test_name ?? context.label ?? "snapshot";

  let info = `<span style="color:#569cd6;font-weight:bold">${label}</span>`;
  if (context.git_sha) info += ` <span style="color:#888">(git: ${context.git_sha})</span>`;
  info += ` <span style="color:#aaa">${snapshot.created_at}</span>`;

  let detail = `${params.item} @ ${params.rate}/s`;
  detail += ` · ${params.machine}`;
  if (params.belt_tier) detail += ` · ${params.belt_tier}`;
  if (params.inputs.length) detail += ` · from ${params.inputs.join(", ")}`;

  const infoSpan = document.createElement("span");
  infoSpan.innerHTML = info;
  banner.appendChild(infoSpan);

  const detailSpan = document.createElement("span");
  detailSpan.style.cssText = "color:#888;margin-left:8px";
  detailSpan.textContent = detail;
  banner.appendChild(detailSpan);

  // Incomplete trace warning
  if (!trace.complete) {
    const warn = document.createElement("span");
    warn.style.cssText = "color:#ff6b6b;margin-left:8px";
    warn.textContent = "⚠ Incomplete trace";
    banner.appendChild(warn);
  }
  if (validation.truncated) {
    const warn = document.createElement("span");
    warn.style.cssText = "color:#ff6b6b;margin-left:4px";
    warn.textContent = "⚠ Validation truncated";
    banner.appendChild(warn);
  }

  // Validation summary
  const errors = validation.issues.filter((i) => i.severity === "Error").length;
  const warnings = validation.issues.length - errors;
  if (validation.issues.length > 0) {
    const badge = document.createElement("span");
    badge.style.cssText = "margin-left:8px";
    badge.innerHTML = `<span style="color:#f66">${errors} errors</span> <span style="color:#fa0">${warnings} warnings</span>`;
    banner.appendChild(badge);
  }

  // Buttons
  const spacer = document.createElement("span");
  spacer.style.cssText = "flex:1";
  banner.appendChild(spacer);

  const reSolveBtn = document.createElement("button");
  reSolveBtn.textContent = "Re-solve";
  reSolveBtn.title = "Not yet implemented";
  reSolveBtn.disabled = true;
  reSolveBtn.style.cssText =
    "background:#222;border:1px solid #444;color:#666;padding:2px 8px;border-radius:3px;font:11px monospace;cursor:not-allowed";
  banner.appendChild(reSolveBtn);

  const clearBtn = document.createElement("button");
  clearBtn.textContent = "Clear";
  clearBtn.style.cssText =
    "background:#333;border:1px solid #666;color:#ccc;padding:2px 8px;border-radius:3px;cursor:pointer;font:11px monospace";
  clearBtn.addEventListener("click", () => callbacks.onClear());
  banner.appendChild(clearBtn);

  parent.insertBefore(banner, parent.firstChild);
  return banner;
}
