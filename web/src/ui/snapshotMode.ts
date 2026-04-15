import type { LayoutResult, ValidationIssue } from "../engine";
import { showSnapshotBanner, type LayoutSnapshot, type BannerCallbacks } from "./snapshotLoader";

export interface SnapshotModeDeps {
  sidebarEl: HTMLElement | null;
  getSidebarCtrl(): { setParams(p: unknown, opts: { skipAutoSolve: boolean }): void; updateValidation(issues: ValidationIssue[], panToTile: (x: number, y: number) => void): void } | null;
  renderLayoutOnCanvas(layout: LayoutResult): void;
  setCachedValidationIssues(issues: ValidationIssue[] | null): void;
  updateValidationOverlay(): void;
  panToTile(x: number, y: number): void;
  onDebugEnable(): void;
  onValEnable(): void;
  onClear(): void;
}

export interface SnapshotModeControls {
  load(snapshot: LayoutSnapshot): void;
  clear(): void;
}

export function createSnapshotMode(deps: SnapshotModeDeps): SnapshotModeControls {
  let activeBanner: HTMLDivElement | null = null;

  function clear(): void {
    if (activeBanner) {
      activeBanner.remove();
      activeBanner = null;
    }
    const { sidebarEl } = deps;
    sidebarEl?.querySelectorAll("input,select,button").forEach((el) => {
      (el as HTMLInputElement).disabled = false;
    });
  }

  function load(snapshot: LayoutSnapshot): void {
    const layout: LayoutResult = {
      ...snapshot.layout,
      trace: snapshot.trace.events as LayoutResult["trace"],
    } as LayoutResult;

    if (snapshot.trace.events.length > 0 || snapshot.validation.issues.length > 0) {
      deps.onDebugEnable();
    }
    if (snapshot.validation.issues.length > 0) {
      deps.onValEnable();
    }

    deps.renderLayoutOnCanvas(layout);

    if (snapshot.validation.issues.length > 0) {
      deps.setCachedValidationIssues(snapshot.validation.issues as unknown as ValidationIssue[]);
      deps.updateValidationOverlay();
    }

    deps.getSidebarCtrl()?.setParams(snapshot.params, { skipAutoSolve: true });

    clear();
    const bannerCallbacks: BannerCallbacks = {
      onClear: () => deps.onClear(),
    };
    const { sidebarEl } = deps;
    if (sidebarEl) {
      activeBanner = showSnapshotBanner(sidebarEl, snapshot, bannerCallbacks);
      sidebarEl.querySelectorAll("input,select,button").forEach((el) => {
        if (el.closest("[data-snapshot-keep]")) return;
        (el as HTMLInputElement).disabled = true;
      });
    }
  }

  return { load, clear };
}
