// Junction-zone overlay — clickable rectangles derived directly from
// the layout.trace's junction-cluster events. Replaces the region-based
// SAT-zone rendering.

import { Container, Graphics, Text, TextStyle } from "pixi.js";
import { TILE_PX } from "./entities";
import { terminalIteration, type JunctionCluster } from "../ui/junctionTrace";

const OUTCOME_FILL: Record<string, number> = {
  Solved: 0x3aa04a, // green
  Capped: 0xd4a03a, // amber
  Open: 0xc04040, // red
};

const LABEL_STYLE = new TextStyle({
  fontFamily: "monospace",
  fontSize: 10,
  fill: 0xffffff,
  dropShadow: { color: 0x000000, distance: 1, blur: 2, alpha: 0.8 },
});

const PILL_STYLE = new TextStyle({
  fontFamily: "monospace",
  fontSize: 9,
  fill: 0xffffff,
  dropShadow: { color: 0x000000, distance: 1, blur: 2, alpha: 0.9 },
});

export interface JunctionZoneOverlayResult {
  layer: Container;
  hitTest: (wx: number, wy: number) => JunctionCluster | null;
}

interface Hit {
  cluster: JunctionCluster;
  pxX: number;
  pxY: number;
  pxW: number;
  pxH: number;
}

export function renderJunctionZoneOverlay(
  clusters: readonly JunctionCluster[],
): JunctionZoneOverlayResult {
  const layer = new Container();
  const hits: Hit[] = [];

  for (const cluster of clusters) {
    const term = terminalIteration(cluster);
    if (!term) continue;
    const b = term.bbox;
    if (b.w <= 0 || b.h <= 0) continue;

    const pxX = b.x * TILE_PX;
    const pxY = b.y * TILE_PX;
    const pxW = b.w * TILE_PX;
    const pxH = b.h * TILE_PX;

    const fill = OUTCOME_FILL[cluster.outcome.kind] ?? OUTCOME_FILL.Open;

    const rect = new Graphics();
    rect.rect(pxX, pxY, pxW, pxH).fill({ color: fill, alpha: 0.14 });
    rect.setStrokeStyle({ width: 1, color: 0x000000, alpha: 0.55 });
    rect.rect(pxX - 1, pxY - 1, pxW + 2, pxH + 2).stroke();
    rect.setStrokeStyle({ width: 2, color: fill, alpha: 0.85 });
    rect.rect(pxX, pxY, pxW, pxH).stroke();
    layer.addChild(rect);

    // Seed-coord label top-left
    const label = new Text({
      text: `Junction (${cluster.seed.x},${cluster.seed.y})`,
      style: LABEL_STYLE,
    });
    label.x = pxX + 3;
    label.y = pxY + 2;
    layer.addChild(label);

    // Outcome status pill top-right
    const pill = new Text({
      text: pillText(cluster),
      style: PILL_STYLE,
    });
    pill.x = pxX + pxW - pill.width - 3;
    pill.y = pxY + 2;
    layer.addChild(pill);

    hits.push({ cluster, pxX, pxY, pxW, pxH });
  }

  const hitTest = (wx: number, wy: number): JunctionCluster | null => {
    let best: JunctionCluster | null = null;
    let bestArea = Number.POSITIVE_INFINITY;
    for (const h of hits) {
      if (wx < h.pxX || wy < h.pxY || wx >= h.pxX + h.pxW || wy >= h.pxY + h.pxH) continue;
      const area = h.pxW * h.pxH;
      if (area < bestArea) {
        best = h.cluster;
        bestArea = area;
      }
    }
    return best;
  };

  return { layer, hitTest };
}

function pillText(cluster: JunctionCluster): string {
  const n = cluster.iterations.length;
  switch (cluster.outcome.kind) {
    case "Solved":
      return `Solved @ iter ${cluster.outcome.growthIter} · ${n} iter${n === 1 ? "" : "s"}`;
    case "Capped":
      return `Capped · ${cluster.outcome.iters} iter${cluster.outcome.iters === 1 ? "" : "s"}`;
    case "Open":
      return `Open · ${n} iter${n === 1 ? "" : "s"}`;
  }
}
