import { Container, Graphics, Text, TextStyle, CanvasTextMetrics } from "pixi.js";
import type { Viewport } from "pixi-viewport";
import type { SolverResult, MachineSpec } from "../engine.js";
import { ENTITY_COLORS, DEFAULT_COLOR } from "./colors.js";

const COL_WIDTH = 240;
const ROW_HEIGHT = 80;
const NODE_W = 180;
const NODE_H = 60;
const GRAPH_ORIGIN_X = 100;
const GRAPH_ORIGIN_Y = 100;
const GRAPH_NAME = "production-graph";

function edgeColorForRate(rate: number): number {
  if (rate <= 15) return 0xc8b560;
  if (rate <= 30) return 0xe05050;
  if (rate <= 45) return 0x50a0e0;
  return 0xff0088;
}

const titleStyle = new TextStyle({
  fontSize: 13,
  fontWeight: "bold",
  fill: 0xe0e0e0,
  fontFamily: "sans-serif",
  wordWrap: true,
  wordWrapWidth: NODE_W - 12,
});

const subtitleStyle = new TextStyle({
  fontSize: 11,
  fill: 0x9cdcfe,
  fontFamily: "sans-serif",
  wordWrap: true,
  wordWrapWidth: NODE_W - 12,
});

const edgeLabelStyle = new TextStyle({
  fontSize: 10,
  fill: 0xffffff,
  fontFamily: "sans-serif",
});

interface NodeInfo {
  x: number;
  y: number;
  w: number;
  h: number;
  machine?: MachineSpec;
}

/**
 * Draw the production DAG as a layered node-and-arrow diagram.
 * Destroys any previous "production-graph" container before drawing.
 * Returns the new container (already added to viewport).
 */
export function drawGraph(
  viewport: Viewport,
  result: SolverResult | null,
): Container {
  const existing = viewport.getChildByName(GRAPH_NAME);
  if (existing) {
    existing.destroy({ children: true });
    viewport.removeChild(existing);
  }

  const root = new Container();
  root.label = GRAPH_NAME;
  viewport.addChild(root);

  if (!result || result.machines.length === 0) {
    return root;
  }

  // dependency_order[0] = target recipe (rightmost in display), last = leaves (leftmost)
  const { dependency_order: depOrder } = result;
  const numCols = depOrder.length;

  // recipe → display column (0 = leftmost leaf, numCols-1 = target)
  const recipeCol = new Map<string, number>();
  depOrder.forEach((recipe, idx) => {
    recipeCol.set(recipe, numCols - 1 - idx);
  });

  // External inputs sit in visual column 0; machines are offset by 1
  const MACHINE_COL_OFFSET = 1;

  // Group and sort machines by column
  const machinesByCol = new Map<number, MachineSpec[]>();
  for (const m of result.machines) {
    const col = recipeCol.get(m.recipe) ?? 0;
    if (!machinesByCol.has(col)) machinesByCol.set(col, []);
    machinesByCol.get(col)!.push(m);
  }
  for (const arr of machinesByCol.values()) {
    arr.sort((a, b) => a.recipe.localeCompare(b.recipe));
  }

  // One external-input node per unique item
  const extInputItems = [...new Set(result.external_inputs.map((f) => f.item))].sort();

  // Build lookup: item → machine that produces it (for fast edge routing)
  const itemProducer = new Map<string, MachineSpec>();
  for (const m of result.machines) {
    for (const o of m.outputs) {
      itemProducer.set(o.item, m);
    }
  }

  // Build lookup: item → external flow rate
  const extFlowRate = new Map<string, number>();
  for (const f of result.external_inputs) {
    extFlowRate.set(f.item, f.rate);
  }

  // Compute node positions — store MachineSpec alongside for O(1) lookup during draw
  const machineNodeMap = new Map<string, NodeInfo>();
  for (const [col, machines] of machinesByCol) {
    machines.forEach((m, rowIdx) => {
      machineNodeMap.set(m.recipe, {
        x: GRAPH_ORIGIN_X + (col + MACHINE_COL_OFFSET) * COL_WIDTH,
        y: GRAPH_ORIGIN_Y + rowIdx * ROW_HEIGHT,
        w: NODE_W,
        h: NODE_H,
        machine: m,
      });
    });
  }

  const extNodeMap = new Map<string, NodeInfo>();
  extInputItems.forEach((item, rowIdx) => {
    extNodeMap.set(item, {
      x: GRAPH_ORIGIN_X,
      y: GRAPH_ORIGIN_Y + rowIdx * ROW_HEIGHT,
      w: 140,
      h: 40,
    });
  });

  // Draw edges (under nodes)
  const edgeGfx = new Graphics();
  root.addChild(edgeGfx);

  for (const machine of result.machines) {
    const consumerNode = machineNodeMap.get(machine.recipe);
    if (!consumerNode) continue;

    for (const inputFlow of machine.inputs) {
      const producer = itemProducer.get(inputFlow.item);
      const producerNode = producer
        ? machineNodeMap.get(producer.recipe)
        : extNodeMap.get(inputFlow.item);

      if (!producerNode) continue;

      const color = edgeColorForRate(inputFlow.rate);

      const px = producerNode.x + producerNode.w;
      const py = producerNode.y + (producerNode.h * 2) / 3;
      const cx = consumerNode.x;
      const cy = consumerNode.y + consumerNode.h / 3;
      const midX = (px + cx) / 2;

      edgeGfx
        .moveTo(px, py)
        .lineTo(midX, py)
        .lineTo(midX, cy)
        .lineTo(cx, cy)
        .stroke({ color, width: 2, alpha: 0.85 });

      const labelText = `${inputFlow.rate.toFixed(1)}/s ${inputFlow.item}`;
      const labelX = (px + midX) / 2;
      const labelY = py - 14;
      const metrics = CanvasTextMetrics.measureText(labelText, edgeLabelStyle);

      const labelBg = new Graphics();
      labelBg
        .rect(labelX - 2, labelY - 1, metrics.width + 4, metrics.height + 2)
        .fill({ color: 0x1e1e1e, alpha: 0.7 });
      root.addChild(labelBg);

      const label = new Text({ text: labelText, style: edgeLabelStyle });
      label.position.set(labelX, labelY);
      root.addChild(label);
    }
  }

  // Draw machine nodes
  for (const node of machineNodeMap.values()) {
    const machine = node.machine!;
    const entityColor = ENTITY_COLORS[machine.entity] ?? DEFAULT_COLOR;

    const nodeGfx = new Graphics();
    nodeGfx
      .rect(node.x, node.y, node.w, node.h)
      .fill({ color: entityColor, alpha: 0.6 })
      .stroke({ color: entityColor, width: 2 });
    root.addChild(nodeGfx);

    const title = new Text({
      text: `${machine.count.toFixed(1)} × ${machine.entity}`,
      style: titleStyle,
    });
    title.position.set(node.x + 6, node.y + 6);
    root.addChild(title);

    const subtitle = new Text({ text: machine.recipe, style: subtitleStyle });
    subtitle.position.set(node.x + 6, node.y + 24);
    root.addChild(subtitle);
  }

  // Draw external input nodes
  for (const [item, node] of extNodeMap) {
    const rate = extFlowRate.get(item);
    const rateLabel = rate !== undefined ? `${rate.toFixed(1)}/s` : "";

    const nodeGfx = new Graphics();
    nodeGfx
      .rect(node.x, node.y, node.w, node.h)
      .fill({ color: 0x2a2a2a, alpha: 0.8 })
      .stroke({ color: 0x888888, width: 1.5 });
    root.addChild(nodeGfx);

    const title = new Text({ text: rateLabel, style: titleStyle });
    title.position.set(node.x + 6, node.y + 4);
    root.addChild(title);

    const subtitle = new Text({ text: item, style: subtitleStyle });
    subtitle.position.set(node.x + 6, node.y + 20);
    root.addChild(subtitle);
  }

  // Center viewport on the graph bounding box
  let minX = Infinity, maxX = -Infinity, minY = Infinity, maxY = -Infinity;
  for (const n of [...machineNodeMap.values(), ...extNodeMap.values()]) {
    if (n.x < minX) minX = n.x;
    if (n.x + n.w > maxX) maxX = n.x + n.w;
    if (n.y < minY) minY = n.y;
    if (n.y + n.h > maxY) maxY = n.y + n.h;
  }
  viewport.moveCenter((minX + maxX) / 2, (minY + maxY) / 2);

  return root;
}
