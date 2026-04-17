/**
 * Busy overlay — a small spinner that appears over the canvas while the
 * WASM worker is churning on a solve/layout/validate. Avoids the old UX of
 * the main thread freezing during a long layout.
 *
 * Shows after a short grace period so trivially fast calls don't flicker.
 * Driven by `onEngineActivity` from engine.ts.
 */

import { onEngineActivity } from "../engine";

const STYLE = `
.fucktorio-busy {
  position: absolute;
  top: 12px;
  right: 12px;
  display: flex;
  align-items: center;
  gap: 8px;
  background: rgba(20, 20, 20, 0.82);
  color: #d4d4d4;
  padding: 6px 12px;
  border: 1px solid #2a2a2a;
  border-radius: 4px;
  font: 11px 'JetBrains Mono', 'Consolas', monospace;
  letter-spacing: 0.5px;
  pointer-events: none;
  z-index: 20;
  opacity: 0;
  transition: opacity 0.15s ease;
}
.fucktorio-busy.visible { opacity: 1; }
.fucktorio-busy-spin {
  width: 12px;
  height: 12px;
  border: 2px solid #2a2a2a;
  border-top-color: #569cd6;
  border-radius: 50%;
  animation: fucktorio-busy-spin 0.7s linear infinite;
  display: inline-block;
}
@keyframes fucktorio-busy-spin { to { transform: rotate(360deg); } }
`;

function injectStyle(): void {
  if (document.getElementById("fucktorio-busy-style")) return;
  const el = document.createElement("style");
  el.id = "fucktorio-busy-style";
  el.textContent = STYLE;
  document.head.appendChild(el);
}

const SHOW_GRACE_MS = 120;

export function attachBusyOverlay(container: HTMLElement): void {
  injectStyle();

  const overlay = document.createElement("div");
  overlay.className = "fucktorio-busy";

  const spinner = document.createElement("span");
  spinner.className = "fucktorio-busy-spin";
  overlay.appendChild(spinner);

  const label = document.createElement("span");
  label.textContent = "computing…";
  overlay.appendChild(label);

  container.appendChild(overlay);

  let showTimer: ReturnType<typeof setTimeout> | null = null;

  onEngineActivity((active) => {
    if (active > 0) {
      if (showTimer === null && !overlay.classList.contains("visible")) {
        showTimer = setTimeout(() => {
          overlay.classList.add("visible");
          showTimer = null;
        }, SHOW_GRACE_MS);
      }
    } else {
      if (showTimer !== null) {
        clearTimeout(showTimer);
        showTimer = null;
      }
      overlay.classList.remove("visible");
    }
  });
}
