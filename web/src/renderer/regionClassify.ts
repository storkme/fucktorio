// Region classifier: takes a LayoutRegion and tries to assign it one of the
// template classes from the junction solver RFP. Pure function, no state.
//
// The goal is educational: when the user hovers a region, show both what
// the engine labelled it as (kind: junction_template / corridor_template /
// ghost_cluster) AND what a template-based classifier would do with it.
// Gaps between the two tell us what the taxonomy is missing.

import type { LayoutRegion, PortSpec, PortEdge, EntityDirection } from "../engine";

export type RegionClass =
  | "perpendicular"        // T1: one horizontal item × one vertical item, single-tile crossing
  | "corridor"             // T2: one horizontal item crossing N vertical trunks, or vice versa
  | "same-direction"       // T3: multiple items all on the same axis
  | "complex"              // T4: 3+ items, no simple template match
  | "single-item"          // 1 item with matching in/out — trivial routing
  | "unbalanced"           // some item has no matching in/out pair
  | "no-ports"             // degenerate: region has no boundary ports
  | "unknown";             // fallback

export interface RegionClassification {
  cls: RegionClass;
  summary: string;
  /** Items keyed by name with their port breakdown. */
  items: Map<string, ItemPorts>;
}

export interface ItemPorts {
  name: string;
  axis: "horizontal" | "vertical" | "mixed";
  inputs: PortSpec[];
  outputs: PortSpec[];
}

function portAxis(port: PortSpec): "horizontal" | "vertical" {
  // E/W edges → horizontal flow (the spec enters or exits along a horizontal
  // direction). N/S edges → vertical flow.
  return port.edge === "E" || port.edge === "W" ? "horizontal" : "vertical";
}

function itemAxis(ports: PortSpec[]): "horizontal" | "vertical" | "mixed" {
  const axes = new Set(ports.map(portAxis));
  if (axes.size === 1) return [...axes][0];
  return "mixed";
}

export function classifyRegion(region: LayoutRegion): RegionClassification {
  const ports = region.ports ?? [];

  if (ports.length === 0) {
    return {
      cls: "no-ports",
      summary: "Region has no boundary ports — degenerate region with no flow.",
      items: new Map(),
    };
  }

  // Group ports by item.
  const items = new Map<string, ItemPorts>();
  for (const p of ports) {
    const name = p.item ?? "?";
    if (!items.has(name)) {
      items.set(name, { name, axis: "horizontal", inputs: [], outputs: [] });
    }
    const ip = items.get(name)!;
    if (p.io === "Input") ip.inputs.push(p);
    else ip.outputs.push(p);
  }

  // Compute axis per item (using all of its ports).
  for (const ip of items.values()) {
    ip.axis = itemAxis([...ip.inputs, ...ip.outputs]);
  }

  // Check balance: every item should have ≥1 input and ≥1 output.
  const unbalanced: string[] = [];
  for (const ip of items.values()) {
    if (ip.inputs.length === 0 || ip.outputs.length === 0) {
      unbalanced.push(ip.name);
    }
  }
  if (unbalanced.length > 0) {
    return {
      cls: "unbalanced",
      summary: `${unbalanced.length} item(s) have unbalanced ports (missing input or output): ${unbalanced.slice(0, 3).join(", ")}${unbalanced.length > 3 ? "…" : ""}. The SAT solver would normally filter these out before solving.`,
      items,
    };
  }

  const itemList = [...items.values()];

  // Single-item regions: 1 input + 1 output on the same item.
  if (itemList.length === 1) {
    const item = itemList[0];
    if (item.inputs.length === 1 && item.outputs.length === 1) {
      return {
        cls: "single-item",
        summary: `Single-item passthrough: ${item.name} (${item.axis}). 1 input → 1 output. Trivial routing, no crossing needed.`,
        items,
      };
    }
    return {
      cls: "same-direction",
      summary: `Same-direction, single item: ${item.name} with ${item.inputs.length} inputs / ${item.outputs.length} outputs on the ${item.axis} axis. Could be a merge point.`,
      items,
    };
  }

  // Two-item regions: check for perpendicular crossing (T1).
  if (itemList.length === 2) {
    const [a, b] = itemList;
    const axes = [a.axis, b.axis];

    if (
      a.inputs.length === 1 && a.outputs.length === 1 &&
      b.inputs.length === 1 && b.outputs.length === 1 &&
      axes.includes("horizontal") && axes.includes("vertical")
    ) {
      const horiz = a.axis === "horizontal" ? a : b;
      const vert = a.axis === "vertical" ? a : b;
      return {
        cls: "perpendicular",
        summary: `Perpendicular crossing (T1): ${horiz.name} (horizontal) crosses ${vert.name} (vertical). A UG bridge in 3 tiles would route this deterministically — no SAT needed.`,
        items,
      };
    }

    if (a.axis === b.axis) {
      return {
        cls: "same-direction",
        summary: `Same-direction overlap (T3): ${a.name} and ${b.name} both on ${a.axis} axis. One needs to go underground past the other.`,
        items,
      };
    }

    // 1 horizontal + multiple vertical (or vice versa) with balanced ports:
    // corridor run pattern (T2).
    const horizItems = itemList.filter(i => i.axis === "horizontal");
    const vertItems = itemList.filter(i => i.axis === "vertical");
    if (horizItems.length === 1 && vertItems.length === 1) {
      // Already handled above, but if it didn't match perpendicular it has
      // multiple in/out per item — it's a more complex crossing.
      return {
        cls: "complex",
        summary: `2-item crossing with multiple ports per item — the horizontal spec has ${horizItems[0].inputs.length}/${horizItems[0].outputs.length} in/out, the vertical has ${vertItems[0].inputs.length}/${vertItems[0].outputs.length}. Not a simple T1 crossing.`,
        items,
      };
    }

    return {
      cls: "complex",
      summary: `2-item mixed-axis region that doesn't match T1 or T3.`,
      items,
    };
  }

  // 3+ items: check for corridor pattern (T2) — one horizontal crossing many verticals.
  const horizItems = itemList.filter(i => i.axis === "horizontal");
  const vertItems = itemList.filter(i => i.axis === "vertical");

  if (horizItems.length === 1 && vertItems.length === itemList.length - 1) {
    const horiz = horizItems[0];
    const allVertSingle = vertItems.every(v => v.inputs.length === 1 && v.outputs.length === 1);
    if (allVertSingle && horiz.inputs.length === 1 && horiz.outputs.length === 1) {
      return {
        cls: "corridor",
        summary: `Corridor run (T2): horizontal ${horiz.name} crosses ${vertItems.length} vertical trunks. A single long UG bridge would route this in ~${vertItems.length + 1} tiles.`,
        items,
      };
    }
  }

  if (vertItems.length === 1 && horizItems.length === itemList.length - 1) {
    return {
      cls: "corridor",
      summary: `Corridor run (T2, rotated): vertical ${vertItems[0].name} crosses ${horizItems.length} horizontal specs.`,
      items,
    };
  }

  return {
    cls: "complex",
    summary: `Multi-path cluster (T4): ${itemList.length} items (${horizItems.length} horizontal, ${vertItems.length} vertical). No simple template matches — this is SAT territory.`,
    items,
  };
}

// ---------------------------------------------------------------------------
// Per-kind and per-class colour palettes
// ---------------------------------------------------------------------------

/** Colour for a region based on engine-assigned kind. */
export function kindColor(kind: string): number {
  switch (kind) {
    case "corridor_template":  return 0x3d7bb5; // blue
    case "junction_template":  return 0x4aa66f; // green
    case "ghost_cluster":      return 0xd08040; // orange
    default:                   return 0x808080; // gray
  }
}

/** Colour for a region based on classifier output. */
export function classColor(cls: RegionClass): number {
  switch (cls) {
    case "perpendicular":   return 0x4aa66f; // green — simple
    case "corridor":        return 0x3d7bb5; // blue — simple
    case "same-direction":  return 0xd0a040; // amber — known template
    case "single-item":     return 0x70b0e0; // sky — trivial
    case "complex":         return 0xd04040; // red — no template, SAT territory
    case "unbalanced":      return 0xb0b0b0; // gray — filtered out
    case "no-ports":        return 0x505050; // dark gray — degenerate
    case "unknown":         return 0xff00ff; // magenta — bug bait
  }
}

export function classLabel(cls: RegionClass): string {
  switch (cls) {
    case "perpendicular":   return "T1 perpendicular";
    case "corridor":        return "T2 corridor run";
    case "same-direction":  return "T3 same-direction";
    case "complex":         return "T4 multi-path";
    case "single-item":     return "single-item";
    case "unbalanced":      return "unbalanced";
    case "no-ports":        return "no-ports";
    case "unknown":         return "unknown";
  }
}

/** Ignored for direction, used only in JSDoc hover context. */
export type _ForDoc = PortEdge | EntityDirection;
