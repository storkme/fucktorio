import type { LayoutResult } from "../engine";
import { getTracePhases, type TraceEvent } from "../renderer/traceOverlay";
import "./stepThrough.css";

export interface StepThroughDeps {
  getLayout(): LayoutResult | null;
  /** True when the master Debug toggle AND the Step-through sub-toggle are on. */
  isEnabled(): boolean;
  /** Called when the user advances/rewinds the phase; main.ts re-runs the trace overlay. */
  onPhaseChange(): void;
  /** Called when the user clicks the failure badge; main.ts pans + pulses the marker. */
  onJumpToFailure(fromX: number, fromY: number): void;
}

export interface StepThroughControls {
  update(): void;
  getPhaseIndex(): number;
  reset(): void;
}

export function createStepThrough(
  container: HTMLElement,
  deps: StepThroughDeps,
): StepThroughControls {
  let phaseIndex = -1;

  const bar = document.createElement("div");
  bar.className = "step-through-bar";
  const prevBtn = document.createElement("button");
  prevBtn.className = "step-through-btn";
  prevBtn.textContent = "\u25C0";
  const phaseLabel = document.createElement("span");
  phaseLabel.className = "step-through-label";
  phaseLabel.textContent = "all";
  const nextBtn = document.createElement("button");
  nextBtn.className = "step-through-btn";
  nextBtn.textContent = "\u25B6";
  const failBtn = document.createElement("button");
  failBtn.className = "step-through-fail";
  bar.appendChild(prevBtn);
  bar.appendChild(phaseLabel);
  bar.appendChild(nextBtn);
  bar.appendChild(failBtn);
  container.appendChild(bar);

  function update(): void {
    const layout = deps.getLayout();
    if (!deps.isEnabled() || !layout?.trace?.length) {
      bar.style.display = "none";
      failBtn.style.display = "none";
      return;
    }
    const trace = layout.trace as TraceEvent[];
    const phases = getTracePhases(trace);
    if (phases.length === 0) {
      bar.style.display = "none";
      failBtn.style.display = "none";
      return;
    }
    bar.style.display = "flex";

    const timeEvents = trace.filter(e => e.phase === "PhaseTime") as Extract<TraceEvent, { phase: "PhaseTime" }>[];
    const completeEvents = trace.filter(e => e.phase === "PhaseComplete") as Extract<TraceEvent, { phase: "PhaseComplete" }>[];

    if (phaseIndex < 0) {
      const totalMs = timeEvents.reduce((s, t) => s + t.data.duration_ms, 0);
      const totalEntities = completeEvents.length > 0 ? completeEvents[completeEvents.length - 1].data.entity_count : 0;
      phaseLabel.textContent = `all (${phases.length}) — ${totalEntities} entities, ${totalMs}ms`;
    } else {
      const phaseEndIdx = phases[phaseIndex].eventIndex;
      let elapsedMs = 0;
      for (const t of timeEvents) {
        const tIdx = trace.indexOf(t as TraceEvent);
        if (tIdx <= phaseEndIdx) elapsedMs += t.data.duration_ms;
      }
      const entityCount = completeEvents.find(c => c.data.phase === phases[phaseIndex].name)?.data.entity_count ?? 0;
      phaseLabel.textContent = `${phases[phaseIndex].name} — ${entityCount} entities, ${elapsedMs}ms`;
    }
    prevBtn.disabled = phaseIndex <= 0 && phaseIndex !== -1;
    nextBtn.disabled = phaseIndex >= phases.length - 1;

    const failCount = trace.filter(e => e.phase === "RouteFailure").length;
    if (failCount > 0) {
      failBtn.textContent = `\u26A0 ${failCount}`;
      failBtn.style.display = "inline-block";
    } else {
      failBtn.style.display = "none";
    }
  }

  function stepPrev(): void {
    const layout = deps.getLayout();
    const phases = getTracePhases((layout?.trace ?? []) as TraceEvent[]);
    if (phaseIndex === -1) phaseIndex = phases.length - 1;
    else if (phaseIndex > 0) phaseIndex--;
    deps.onPhaseChange();
  }

  function stepNext(): void {
    const layout = deps.getLayout();
    const phases = getTracePhases((layout?.trace ?? []) as TraceEvent[]);
    if (phaseIndex < phases.length - 1) phaseIndex++;
    deps.onPhaseChange();
  }

  function jumpToFailure(): void {
    const layout = deps.getLayout();
    if (!layout?.trace) return;
    const failures = (layout.trace as TraceEvent[]).filter(e => e.phase === "RouteFailure") as Extract<TraceEvent, { phase: "RouteFailure" }>[];
    if (failures.length === 0) return;
    const first = failures[0].data;
    deps.onJumpToFailure(first.from_x, first.from_y);
  }

  prevBtn.addEventListener("click", stepPrev);
  nextBtn.addEventListener("click", stepNext);
  failBtn.addEventListener("click", jumpToFailure);

  document.addEventListener("keydown", (e) => {
    const tag = (e.target as HTMLElement)?.tagName;
    if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;
    if (bar.style.display === "none") return;

    if (e.key === "ArrowLeft") {
      e.preventDefault();
      stepPrev();
    } else if (e.key === "ArrowRight") {
      e.preventDefault();
      stepNext();
    } else if (e.key === "f") {
      e.preventDefault();
      jumpToFailure();
    }
  });

  return {
    update,
    getPhaseIndex: () => phaseIndex,
    reset: () => { phaseIndex = -1; },
  };
}
