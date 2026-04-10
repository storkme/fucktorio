import { Graphics, Container } from "pixi.js";
import { TILE_PX } from "./entities";

export interface ValidationIssue {
  severity: "Error" | "Warning";
  category: string;
  message: string;
  x?: number;
  y?: number;
}

const COLORS: Record<string, number> = {
  Error: 0xff4444,
  Warning: 0xffaa00,
};

/** Alpha for the resting (non-pulsed) fill of validation circles. */
export const VALIDATION_CIRCLE_ALPHA = 0.35;

export interface ValidationOverlayResult {
  layer: Container;
  /** Map from "x,y" to all Graphics circles at that tile position. */
  circleMap: Map<string, Graphics[]>;
}

export function renderValidationOverlay(
  issues: ValidationIssue[],
  container: Container,
  onHover: (text: string | null) => void,
): ValidationOverlayResult {
  const layer = new Container();
  const circleMap = new Map<string, Graphics[]>();
  for (const issue of issues) {
    if (issue.x == null || issue.y == null) continue;
    const color = COLORS[issue.severity] ?? 0x44aaff;
    const g = new Graphics();
    g.circle(issue.x * TILE_PX + TILE_PX / 2, issue.y * TILE_PX + TILE_PX / 2, TILE_PX * 0.4)
      .fill({ color, alpha: VALIDATION_CIRCLE_ALPHA })
      .stroke({ width: 1.5, color, alpha: 0.7 });
    g.eventMode = "static";
    g.on("pointerenter", () => onHover(`[${issue.severity}] ${issue.category}: ${issue.message}`));
    g.on("pointerleave", () => onHover(null));
    layer.addChild(g);
    const key = `${issue.x},${issue.y}`;
    const existing = circleMap.get(key);
    if (existing) {
      existing.push(g);
    } else {
      circleMap.set(key, [g]);
    }
  }
  container.addChild(layer);
  return { layer, circleMap };
}
