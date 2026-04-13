import { Container, Graphics, Text, TextStyle } from "pixi.js";
import { TILE_PX, itemColor } from "./entities";
import type { LayoutResult, LayoutRegion, EntityDirection, RegionKind } from "../engine";
import { classifyRegion, kindColor, classColor, classLabel, type RegionClassification } from "./regionClassify";

// ---------------------------------------------------------------------------
// Types mirroring the Rust PortSpec / PortEdge / PortIo that come through
// serde but are not part of the generated tsify .d.ts.
// ---------------------------------------------------------------------------

type PortEdge = "N" | "S" | "E" | "W";
type PortIo = "Input" | "Output";

interface PortSpec {
  edge: PortEdge;
  offset: number;
  io: PortIo;
  item?: string;
  direction?: EntityDirection;
}

interface LayoutRegionWithPorts {
  kind: RegionKind;
  x: number;
  y: number;
  width: number;
  height: number;
  ports?: PortSpec[];
}

// ---------------------------------------------------------------------------
// Zone colours — sourced from regionClassify (kind / class-based palettes).
// Each region is drawn with its kind as fill and class as outline, so you
// can see both channels at once.
// ---------------------------------------------------------------------------

const INPUT_COLOR = 0x50c050;  // green
const OUTPUT_COLOR = 0xd04040; // red

const LABEL_STYLE = new TextStyle({
  fontFamily: "monospace",
  fontSize: 10,
  fill: 0xffffff,
  dropShadow: { color: 0x000000, distance: 1, blur: 2, alpha: 0.8 },
});

const PORT_LABEL_STYLE = new TextStyle({
  fontFamily: "monospace",
  fontSize: 9,
  fill: 0xffffff,
  dropShadow: { color: 0x000000, distance: 1, blur: 2, alpha: 0.9 },
});

// ---------------------------------------------------------------------------
// Arrow drawing helper
// ---------------------------------------------------------------------------

/** Draw a directional arrow at (cx, cy) in pixel coords. */
function drawArrow(
  g: Graphics,
  cx: number,
  cy: number,
  direction: EntityDirection | undefined,
  color: number,
): void {
  const size = TILE_PX * 0.45;
  g.setStrokeStyle({ width: 3, color, alpha: 0.95 });

  // Direction vectors
  let dx = 0, dy = -1; // default North
  switch (direction) {
    case "East":  dx = 1;  dy = 0; break;
    case "South": dx = 0;  dy = 1; break;
    case "West":  dx = -1; dy = 0; break;
  }

  const tipX = cx + dx * size;
  const tipY = cy + dy * size;
  const tailX = cx - dx * size;
  const tailY = cy - dy * size;

  // Shaft
  g.moveTo(tailX, tailY).lineTo(tipX, tipY).stroke();

  // Arrowhead wings (perpendicular)
  const wingLen = size * 0.55;
  const wx = -dy * wingLen;
  const wy = dx * wingLen;
  g.moveTo(tipX - dx * wingLen + wx, tipY - dy * wingLen + wy)
    .lineTo(tipX, tipY)
    .lineTo(tipX - dx * wingLen - wx, tipY - dy * wingLen - wy)
    .stroke();
}

// ---------------------------------------------------------------------------
// Port world position
// ---------------------------------------------------------------------------

function portWorldPos(
  region: LayoutRegionWithPorts,
  port: PortSpec,
): [number, number] {
  switch (port.edge) {
    case "N": return [region.x + port.offset, region.y];
    case "S": return [region.x + port.offset, region.y + region.height - 1];
    case "W": return [region.x, region.y + port.offset];
    case "E": return [region.x + region.width - 1, region.y + port.offset];
  }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

export interface RegionOverlayItem {
  region: LayoutRegion;
  classification: RegionClassification;
  bboxPixels: { x: number; y: number; w: number; h: number };
}

export interface RegionOverlayResult {
  layer: Container;
  items: RegionOverlayItem[];
  /** Returns the region whose bbox contains the given world-pixel point, or null. */
  hitTest: (wx: number, wy: number) => RegionOverlayItem | null;
}

export function renderRegionOverlayDetailed(layout: LayoutResult): RegionOverlayResult {
  const layer = new Container();
  const regions = (layout.regions ?? []) as LayoutRegionWithPorts[];
  const items: RegionOverlayItem[] = [];

  if (regions.length === 0) {
    return { layer, items, hitTest: () => null };
  }

  for (const region of regions) {
    const classification = classifyRegion(region as LayoutRegion);
    const fillColor = kindColor(region.kind);
    const strokeColor = classColor(classification.cls);

    const rx = region.x * TILE_PX;
    const ry = region.y * TILE_PX;
    const rw = region.width * TILE_PX;
    const rh = region.height * TILE_PX;

    items.push({
      region: region as LayoutRegion,
      classification,
      bboxPixels: { x: rx, y: ry, w: rw, h: rh },
    });

    const rect = new Graphics();
    rect.rect(rx, ry, rw, rh).fill({ color: fillColor, alpha: 0.14 });
    // Thin dark outer edge for contrast against light belts
    rect.setStrokeStyle({ width: 1, color: 0x000000, alpha: 0.55 });
    rect.rect(rx - 1, ry - 1, rw + 2, rh + 2).stroke();
    // Class-colored inner border (this is the "classification visible" channel)
    rect.setStrokeStyle({ width: 2, color: strokeColor, alpha: 0.85 });
    rect.rect(rx, ry, rw, rh).stroke();
    layer.addChild(rect);

    // Dimension + class label at top-left corner
    const labelText = `${region.width}×${region.height}  ${classLabel(classification.cls)}`;
    const label = new Text({ text: labelText, style: LABEL_STYLE });
    label.x = rx + 3;
    label.y = ry + 2;
    layer.addChild(label);

    // Boundary ports — unchanged visual; existing port labels still useful
    const ports = region.ports ?? [];
    for (const port of ports) {
      const [wx, wy] = portWorldPos(region, port);
      const px = wx * TILE_PX + TILE_PX / 2;
      const py = wy * TILE_PX + TILE_PX / 2;

      const portColor = port.io === "Input" ? INPUT_COLOR : OUTPUT_COLOR;
      const pg = new Graphics();
      pg.circle(px, py, TILE_PX * 0.3).fill({ color: portColor, alpha: 0.8 });
      layer.addChild(pg);

      if (port.direction) {
        const ag = new Graphics();
        const arrowColor = port.item ? itemColor(port.item) : portColor;
        drawArrow(ag, px, py, port.direction, arrowColor);
        layer.addChild(ag);
      }

      const ioTag = port.io === "Input" ? "IN" : "OUT";
      const itemAbbr = port.item ? port.item.slice(0, 3) : "?";
      const portLabel = new Text({
        text: `${itemAbbr} ${ioTag}`,
        style: PORT_LABEL_STYLE,
      });
      switch (port.edge) {
        case "N":
          portLabel.x = px - portLabel.width / 2;
          portLabel.y = py - TILE_PX * 0.9;
          break;
        case "S":
          portLabel.x = px - portLabel.width / 2;
          portLabel.y = py + TILE_PX * 0.5;
          break;
        case "W":
          portLabel.x = px - TILE_PX * 1.2;
          portLabel.y = py - 5;
          break;
        case "E":
          portLabel.x = px + TILE_PX * 0.5;
          portLabel.y = py - 5;
          break;
      }
      layer.addChild(portLabel);
    }
  }

  // Hit test: smallest-area containing region wins, so nested rectangles
  // prefer the inner one.
  const hitTest = (wx: number, wy: number): RegionOverlayItem | null => {
    let best: RegionOverlayItem | null = null;
    let bestArea = Infinity;
    for (const it of items) {
      const b = it.bboxPixels;
      if (wx >= b.x && wx < b.x + b.w && wy >= b.y && wy < b.y + b.h) {
        const area = b.w * b.h;
        if (area < bestArea) {
          bestArea = area;
          best = it;
        }
      }
    }
    return best;
  };

  return { layer, items, hitTest };
}
