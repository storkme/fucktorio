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

export interface ValidationOverlayResult {
  layer: Container;
  /** Map from "x,y" to the Graphics circle for that issue position. */
  circleMap: Map<string, Graphics>;
}

export function renderValidationOverlay(
  issues: ValidationIssue[],
  container: Container,
  onHover: (text: string | null) => void,
): ValidationOverlayResult {
  const layer = new Container();
  const circleMap = new Map<string, Graphics>();
  for (const issue of issues) {
    if (issue.x == null || issue.y == null) continue;
    const color = COLORS[issue.severity] ?? 0x44aaff;
    const g = new Graphics();
    g.circle(issue.x * TILE_PX + TILE_PX / 2, issue.y * TILE_PX + TILE_PX / 2, TILE_PX * 0.4)
      .fill({ color, alpha: 0.35 })
      .stroke({ width: 1.5, color, alpha: 0.7 });
    g.eventMode = "static";
    g.on("pointerenter", () => onHover(`[${issue.severity}] ${issue.category}: ${issue.message}`));
    g.on("pointerleave", () => onHover(null));
    layer.addChild(g);
    circleMap.set(`${issue.x},${issue.y}`, g);
  }
  container.addChild(layer);
  return { layer, circleMap };
}
