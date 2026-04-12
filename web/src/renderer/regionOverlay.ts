import { Container, Graphics, Text, TextStyle } from "pixi.js";
import { TILE_PX, itemColor } from "./entities";
import type { LayoutResult, EntityDirection } from "../engine";

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
  kind: string;
  x: number;
  y: number;
  width: number;
  height: number;
  inputs: string[];
  outputs: string[];
  ports?: PortSpec[];
  variables: number;
  clauses: number;
  solve_time_us: number;
}

// ---------------------------------------------------------------------------
// Zone colours — each region gets its own tint from this palette, cycled.
// ---------------------------------------------------------------------------

const ZONE_PALETTE: number[] = [
  0x569cd6, // steel blue
  0xd0a040, // amber
  0x6ac080, // mint
  0xc06090, // rose
  0x70b0e0, // sky
  0xb080d0, // lavender
  0xe07050, // coral
  0x60c0c0, // teal
];

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

export function renderRegionOverlay(layout: LayoutResult): Container {
  const layer = new Container();
  const regions = (layout.regions ?? []) as LayoutRegionWithPorts[];
  if (regions.length === 0) return layer;

  for (let i = 0; i < regions.length; i++) {
    const region = regions[i];
    const zoneColor = ZONE_PALETTE[i % ZONE_PALETTE.length];

    const rx = region.x * TILE_PX;
    const ry = region.y * TILE_PX;
    const rw = region.width * TILE_PX;
    const rh = region.height * TILE_PX;

    // Semi-transparent zone rectangle with visible outline
    const rect = new Graphics();
    rect.rect(rx, ry, rw, rh).fill({ color: zoneColor, alpha: 0.12 });
    // Outer border — darker outline for visibility on light backgrounds
    rect.setStrokeStyle({ width: 1, color: 0x000000, alpha: 0.5 });
    rect.rect(rx - 1, ry - 1, rw + 2, rh + 2).stroke();
    // Inner coloured border
    rect.setStrokeStyle({ width: 2, color: zoneColor, alpha: 0.8 });
    rect.rect(rx, ry, rw, rh).stroke();
    layer.addChild(rect);

    // Dimension label at top-left corner of zone
    const label = new Text({
      text: `${region.width}x${region.height}`,
      style: LABEL_STYLE,
    });
    label.x = rx + 3;
    label.y = ry + 2;
    layer.addChild(label);

    // Boundary ports
    const ports = region.ports ?? [];
    for (const port of ports) {
      const [wx, wy] = portWorldPos(region, port);
      const px = wx * TILE_PX + TILE_PX / 2;
      const py = wy * TILE_PX + TILE_PX / 2;

      // Filled circle at the port position (larger for visibility)
      const portColor = port.io === "Input" ? INPUT_COLOR : OUTPUT_COLOR;
      const pg = new Graphics();
      pg.circle(px, py, TILE_PX * 0.3).fill({ color: portColor, alpha: 0.8 });
      layer.addChild(pg);

      // Directional arrow if direction is specified
      if (port.direction) {
        const ag = new Graphics();
        // Use item color if available, otherwise use io color
        const arrowColor = port.item ? itemColor(port.item) : portColor;
        drawArrow(ag, px, py, port.direction, arrowColor);
        layer.addChild(ag);
      }

      // Text label showing abbreviated item name + IN/OUT
      const ioTag = port.io === "Input" ? "IN" : "OUT";
      const itemAbbr = port.item ? port.item.slice(0, 3) : "?";
      const portLabel = new Text({
        text: `${itemAbbr} ${ioTag}`,
        style: PORT_LABEL_STYLE,
      });
      // Position label offset from the port based on edge
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

  return layer;
}
