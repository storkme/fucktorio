// Junction solver trace grouping.
//
// Walks a flat `layout.trace: TraceEvent[]` stream once and assembles
// per-cluster step-through records keyed by seed tile. The junction
// solver emits a cluster of events for each growing region:
//
//   JunctionGrowthStarted      — one per cluster (seed + participating specs)
//   JunctionGrowthIteration    — one per growth iteration
//   JunctionStrategyAttempt    — one per (iter, strategy) pair
//   SatInvocation              — at most one per iter (SAT strategy)
//   RegionWalkerVeto           — optional, per iter, emitted when the
//                                 walker rejects an otherwise-valid SAT solve
//   JunctionSolved / JunctionGrowthCapped — terminal event for the cluster
//
// Note the field-naming split: the growth events carry `seed_x/seed_y`,
// while the terminal and veto events carry `tile_x/tile_y`. Both refer
// to the same seed tile; this helper reconciles them.

import type {
  BoundarySnapshot,
  ExternalFeederSnapshot,
  ParticipatingSpec,
  StampedNeighbor,
  TraceEvent,
} from "../wasm-pkg/fucktorio_wasm.js";

type GrowthStarted = Extract<TraceEvent, { phase: "JunctionGrowthStarted" }>;
type GrowthIteration = Extract<TraceEvent, { phase: "JunctionGrowthIteration" }>;
type StrategyAttempt = Extract<TraceEvent, { phase: "JunctionStrategyAttempt" }>;
type SatInvocationEvent = Extract<TraceEvent, { phase: "SatInvocation" }>;
type SolvedEvent = Extract<TraceEvent, { phase: "JunctionSolved" }>;
type CappedEvent = Extract<TraceEvent, { phase: "JunctionGrowthCapped" }>;
type VetoEvent = Extract<TraceEvent, { phase: "RegionWalkerVeto" }>;

export interface Bbox {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface AttemptRecord {
  strategy: string;
  outcome: string;
  detail: string;
  elapsedUs: number;
}

export type SatInvocationData = Omit<SatInvocationEvent["data"], "seed_x" | "seed_y" | "iter">;
export type WalkerVetoData = Omit<VetoEvent["data"], "tile_x" | "tile_y" | "growth_iter">;

export interface JunctionIteration {
  iter: number;
  bbox: Bbox;
  tiles: [number, number][];
  forbidden: [number, number][];
  boundaries: BoundarySnapshot[];
  participating: string[];
  encountered: string[];
  attempts: AttemptRecord[];
  sat: SatInvocationData | null;
  veto: WalkerVetoData | null;
}

export type ClusterOutcome =
  | { kind: "Solved"; strategy: string; growthIter: number; regionTiles: number }
  | { kind: "Capped"; iters: number; regionTiles: number; reason: string }
  | { kind: "Open" };

export interface JunctionCluster {
  seed: { x: number; y: number };
  participating: ParticipatingSpec[];
  nearbyStamped: StampedNeighbor[];
  iterations: JunctionIteration[];
  outcome: ClusterOutcome;
  /** Iteration index (0-based) to show by default when the modal opens. */
  defaultIterIndex: number;
}

type JunctionPhase =
  | "JunctionGrowthStarted"
  | "JunctionGrowthIteration"
  | "JunctionStrategyAttempt"
  | "SatInvocation"
  | "JunctionSolved"
  | "JunctionGrowthCapped"
  | "RegionWalkerVeto";

const JUNCTION_PHASES: ReadonlySet<JunctionPhase> = new Set<JunctionPhase>([
  "JunctionGrowthStarted",
  "JunctionGrowthIteration",
  "JunctionStrategyAttempt",
  "SatInvocation",
  "JunctionSolved",
  "JunctionGrowthCapped",
  "RegionWalkerVeto",
]);

function isJunctionEvent(
  e: TraceEvent,
): e is
  | GrowthStarted
  | GrowthIteration
  | StrategyAttempt
  | SatInvocationEvent
  | SolvedEvent
  | CappedEvent
  | VetoEvent {
  return JUNCTION_PHASES.has(e.phase as JunctionPhase);
}

function eventSeed(
  e:
    | GrowthStarted
    | GrowthIteration
    | StrategyAttempt
    | SatInvocationEvent
    | SolvedEvent
    | CappedEvent
    | VetoEvent,
): [number, number] {
  const d = e.data as { seed_x?: number; seed_y?: number; tile_x?: number; tile_y?: number };
  if (typeof d.seed_x === "number" && typeof d.seed_y === "number") {
    return [d.seed_x, d.seed_y];
  }
  return [d.tile_x as number, d.tile_y as number];
}

function seedKey(x: number, y: number): string {
  return `${x},${y}`;
}

/**
 * Group all junction-related trace events into per-cluster records.
 * Clusters appear in the order their first event was emitted.
 */
export function groupJunctionClusters(trace: readonly TraceEvent[]): JunctionCluster[] {
  interface Builder {
    seed: { x: number; y: number };
    participating: ParticipatingSpec[];
    nearbyStamped: StampedNeighbor[];
    // Map from iter number → record under construction
    iters: Map<number, JunctionIteration>;
    outcome: ClusterOutcome;
    order: number;
  }

  const builders = new Map<string, Builder>();

  const getOrInit = (x: number, y: number): Builder => {
    const k = seedKey(x, y);
    let b = builders.get(k);
    if (!b) {
      b = {
        seed: { x, y },
        participating: [],
        nearbyStamped: [],
        iters: new Map(),
        outcome: { kind: "Open" },
        order: builders.size,
      };
      builders.set(k, b);
    }
    return b;
  };

  const getIter = (b: Builder, iter: number): JunctionIteration => {
    let it = b.iters.get(iter);
    if (!it) {
      it = {
        iter,
        bbox: { x: 0, y: 0, w: 0, h: 0 },
        tiles: [],
        forbidden: [],
        boundaries: [],
        participating: [],
        encountered: [],
        attempts: [],
        sat: null,
        veto: null,
      };
      b.iters.set(iter, it);
    }
    return it;
  };

  for (const ev of trace) {
    if (!isJunctionEvent(ev)) continue;
    const [sx, sy] = eventSeed(ev);
    const b = getOrInit(sx, sy);

    switch (ev.phase) {
      case "JunctionGrowthStarted": {
        b.participating = ev.data.participating;
        b.nearbyStamped = ev.data.nearby_stamped;
        break;
      }
      case "JunctionGrowthIteration": {
        const it = getIter(b, ev.data.iter);
        it.bbox = {
          x: ev.data.bbox_x,
          y: ev.data.bbox_y,
          w: ev.data.bbox_w,
          h: ev.data.bbox_h,
        };
        it.tiles = ev.data.tiles;
        it.forbidden = ev.data.forbidden_tiles;
        it.boundaries = ev.data.boundaries;
        it.participating = ev.data.participating;
        it.encountered = ev.data.encountered;
        break;
      }
      case "JunctionStrategyAttempt": {
        const it = getIter(b, ev.data.iter);
        it.attempts.push({
          strategy: ev.data.strategy,
          outcome: ev.data.outcome,
          detail: ev.data.detail,
          elapsedUs: ev.data.elapsed_us,
        });
        break;
      }
      case "SatInvocation": {
        const it = getIter(b, ev.data.iter);
        // Store the SAT-specific data (excluding redundant keys).
        const {
          seed_x: _sx,
          seed_y: _sy,
          iter: _iter,
          ...rest
        } = ev.data;
        it.sat = rest;
        break;
      }
      case "RegionWalkerVeto": {
        const it = getIter(b, ev.data.growth_iter);
        const {
          tile_x: _tx,
          tile_y: _ty,
          growth_iter: _gi,
          ...rest
        } = ev.data;
        it.veto = rest;
        break;
      }
      case "JunctionSolved": {
        b.outcome = {
          kind: "Solved",
          strategy: ev.data.strategy,
          growthIter: ev.data.growth_iter,
          regionTiles: ev.data.region_tiles,
        };
        break;
      }
      case "JunctionGrowthCapped": {
        b.outcome = {
          kind: "Capped",
          iters: ev.data.iters,
          regionTiles: ev.data.region_tiles,
          reason: ev.data.reason,
        };
        break;
      }
    }
  }

  const clusters: JunctionCluster[] = [];
  const sorted = Array.from(builders.values()).sort((a, b) => a.order - b.order);
  for (const b of sorted) {
    const iterations = Array.from(b.iters.values()).sort((a, b) => a.iter - b.iter);
    // Default iter: Solved → growthIter's index; otherwise last iter.
    let defaultIterIndex = Math.max(0, iterations.length - 1);
    if (b.outcome.kind === "Solved") {
      const idx = iterations.findIndex((it) => it.iter === (b.outcome as { growthIter: number }).growthIter);
      if (idx >= 0) defaultIterIndex = idx;
    }
    clusters.push({
      seed: b.seed,
      participating: b.participating,
      nearbyStamped: b.nearbyStamped,
      iterations,
      outcome: b.outcome,
      defaultIterIndex,
    });
  }
  return clusters;
}

/**
 * Hit-test: find the cluster whose terminal iteration's bbox contains
 * the given world tile. Smallest-area wins when multiple clusters
 * overlap. Returns null if none match or if the cluster has no iterations.
 */
export function clusterAtTile(
  clusters: readonly JunctionCluster[],
  tx: number,
  ty: number,
): JunctionCluster | null {
  let best: JunctionCluster | null = null;
  let bestArea = Number.POSITIVE_INFINITY;
  for (const c of clusters) {
    const it = terminalIteration(c);
    if (!it) continue;
    const b = it.bbox;
    if (tx < b.x || ty < b.y || tx >= b.x + b.w || ty >= b.y + b.h) continue;
    const area = b.w * b.h;
    if (area < bestArea) {
      best = c;
      bestArea = area;
    }
  }
  return best;
}

/**
 * The terminal iteration of a cluster: the iteration SAT committed at
 * (for Solved), the last iteration tried (for Capped), or the last
 * seen iteration (for Open).
 */
export function terminalIteration(cluster: JunctionCluster): JunctionIteration | null {
  if (cluster.iterations.length === 0) return null;
  return cluster.iterations[cluster.defaultIterIndex] ?? cluster.iterations[cluster.iterations.length - 1];
}

/**
 * Look up a boundary's external-feeder label, if any. Useful for the
 * detail panel.
 */
export function formatFeeder(f: ExternalFeederSnapshot | undefined): string {
  if (!f) return "";
  return `${f.entity_name}@(${f.entity_x},${f.entity_y}) ${f.direction}`;
}
