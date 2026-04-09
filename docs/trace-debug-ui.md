# Trace Debug UI — Design Document

The Rust pipeline emits 22 structured trace events during bus layout generation. This document describes a phased plan to surface those events in the web app as interactive visuals, performance data, and algorithm diagnostics — giving the developer insight into what's tunable and why layouts succeed or fail.

## Current State

**Already visualized** in `web/src/renderer/traceOverlay.ts` (8 events):

| Event | Visual |
|---|---|
| LanesPlanned | Vertical lane column strips (green=solid, blue=fluid) |
| RowsPlaced | Horizontal row boundary lines |
| CrossingZoneSolved | Blue SAT zone rectangles |
| CrossingZoneSkipped | Orange skipped zone rectangles |
| BalancerStamped | Purple horizontal band per balancer |
| TapoffRouted | Green diagonal lines from trunk to row |
| MergerBlockPlaced | Gold horizontal merger bands |
| Phase summary | Text overlay listing phases |

**Not visualized** (14 events): `RouteFailure`, `CrossingZoneConflict`, `LaneConsolidated`, `RowSplit`, `LaneOrderOptimized`, `LaneSplit`, `LaneRouted`, `OutputMerged`, `PolesPlaced`, `PhaseTime`, `NegotiateComplete`, `SolverCompleted`, `LaneConsolidated`, `ValidationCompleted`

**Existing UI entry points:**
- Debug checkbox (`main.ts`) — enables `buildLayoutTraced()` + shows overlay
- Step-through bar (`main.ts`) — ◀/▶ through PhaseComplete/PhaseSnapshot boundaries
- `options?.getDebugMode?.()` already passed from `main.ts` → `sidebar.ts`
- `validationOverlay.ts` — renders validation issue circles (separate toggle)

---

## Phase 1 — New Grid Overlays for Spatial Events

**What to build:** Add 5 new overlay types to `renderTraceOverlay()` in `web/src/renderer/traceOverlay.ts`.

### 1.1 RouteFailure — red ✕ cross

When A* can't find a path, draw a red cross marker at the source tile and a dashed line to the target.

```
Visual: Two diagonal lines forming ✕, 6px span, at (from_x * TILE_PX, from_y * TILE_PX + TILE_PX/2)
        Dashed red line from (from_x, from_y) to (to_x, to_y)
Color:  0xff3333
Hover:  "Route failed: {item} ({from_x},{from_y})→({to_x},{to_y}) [{spec_key}]"
```

Route failures are the most actionable signal — they mean the layout has routing conflicts that need space or strategy changes.

### 1.2 CrossingZoneConflict — magenta exclamation tile

A SAT crossing segment was removed because it conflicted with a splitter stamp tile.

```
Visual: 1-tile square outline (border only, no fill) at (conflict_x, conflict_y)
        Small "!" text centered inside
Color:  0xff44ff
Hover:  "Crossing conflict: segment {segment_id} at ({conflict_x},{conflict_y})"
```

### 1.3 LaneConsolidated — sharing badge on lane column

When multiple recipe rows share fewer trunk lanes than they have consumers, show a badge at the top of that lane column.

```
Visual: Small text badge "÷{n_trunk_lanes}" drawn at y=0 of the lane column (x from LanesPlanned by item match)
Color:  0xffaa44
Hover:  "{item}: {consumer_count} consumers share {n_trunk_lanes} lane(s) @ {rate_per_lane.toFixed(1)}/s each"
```

### 1.4 RowSplit — split indicator on row boundary

When a recipe row is split for throughput, annotate the row boundary line.

```
Visual: Small text "⊕{split_into}" drawn at the y_end of the last sub-row for that recipe (from RowsPlaced)
Color:  0xffcc44
Hover:  "{recipe}: split {original_count}→{split_into} rows — {reason}"
```

### 1.5 LaneOrderOptimized — crossing score in summary

Append the crossing score to the existing phase summary text label at the top-left of the overlay.

```
Append: " | lane order: {crossing_score} crossings"
```

### Phase 1 Validation

- Generate `electronic-circuit` at 10/s with Debug enabled
- ✕ markers visible where routes fail (if any); absent when routing succeeds cleanly  
- Toggle Debug off → all new markers disappear
- Hover over ✕ shows correct item name + coordinate pair
- Crossing score appears in the phase summary text

---

## Phase 2 — Debug Stats Panel in Sidebar

**What to build:**
1. New file: `web/src/ui/debugPanel.ts` — exports `renderDebugPanel(events: TraceEvent[]): HTMLElement`
2. Wire into `web/src/ui/sidebar.ts` — appended to `resultContainer` after layout when Debug mode is on

The panel matches the existing sidebar dark theme (`#1e1e1e` / `#252526` cards, monospace 12px). Sections are `<details>/<summary>` — Timeline and A* Routing default-open, others collapsed.

### Section: Performance Timeline

Source events: `PhaseTime`

```
Total: 312ms
[████ rows 12ms][████ lanes 8ms][██████████████████████████ route 286ms][█ poles 3ms]
```

- Horizontal stacked bar, segment width = `duration_ms / total_ms * 100%`
- Color map: `rows`=`#4a9`, `lanes`=`#49c`, `route`=`#c84`, `validate`=`#888`, `poles`=`#cc4`, default=`#69c`
- Each segment has `title` tooltip: `"{phase}: {ms}ms ({pct}%)"` 
- Total ms label below bar
- **What to tune:** if `route` dominates → A* bottleneck (try fewer specs or wider layout); if `lanes` is large → lane ordering issue (too many permutations)

### Section: Solver Summary

Source event: `SolverCompleted`

```
5 recipes · 12 machines
2 external inputs · 1 external output

assembling-machine-3:
  3× → electronic-circuit @ 10.0/s
  6× → copper-cable @ 30.0/s
  3× → iron-gear-wheel @ 5.0/s
```

### Section: Layout Stats

Source events: `LanesPlanned`, `RowsPlaced`, `LaneSplit`

```
Bus width: 9 tiles
6 solid lanes · 2 fluid lanes
5 rows · 2 balancer families
3 lane splits (iron-plate 30/s→2 lanes, copper-cable 30/s→2 lanes, ...)
```

### Section: Lane Ordering

Source event: `LaneOrderOptimized`

```
Crossing score: 14  (lower = fewer underground hops)
Order: [copper-plate] [iron-plate] [copper-cable] [plastic-bar] [circuit]
```

- Items displayed as colored monospace chips
- **What to tune:** high score means items with shared consumers are far apart in the lane order. Rearranging recipe dependencies or rates can reduce it.

### Section: A\* Routing

Source events: `NegotiateComplete`, `RouteFailure`

```
47 specs · 3 iterations · 29ms
✓ all routes resolved
```

If failures:
```
⚠ 2 route failures:
  copper-wire: (0,5)→(12,8) [tap:copper-wire:3:45]
  iron-plate:  (2,3)→(14,6) [trunk:iron-plate:1]
```

- Failure items styled in `#ff6666`
- **What to tune:** failures mean layout is too constrained — more space between rows, wider bus, or fewer lanes on a congested column

### Section: SAT Zones

Source events: `CrossingZoneSolved`, `CrossingZoneSkipped`, `CrossingZoneConflict`

```
5 zones solved · 1 skipped · 2 conflicts
Total solve time: 4,230µs

Skipped:
  iron-plate @ (8,23): splitter_stamp_conflict
```

### Section: Lane Consolidation

Source event: `LaneConsolidated` (only shown if any events exist)

```
Item          | Consumers | Lanes | Rate/lane
iron-plate    |     4     |   2   | 15.0/s
copper-plate  |     3     |   1   | 22.5/s ⚠ near capacity
```

- Rate/lane near belt tier cap shown in amber
- **What to tune:** consolidated lanes at high utilization may cause downstream throughput issues

### Section: Power

Source event: `PolesPlaced`

```
18 poles (strategy: grid)
```

### Section: Validation Summary

Source event: `ValidationCompleted`

```
0 errors · 2 warnings
  [warning] lane-throughput: lane at x=5 y=8 exceeds single-lane capacity
  [warning] belt-dead-end: orphaned belt at x=12 y=3
```

### Phase 2 Validation

- Generate `electronic-circuit` at 10/s with Debug on → all sections appear, populated with real data
- Generate `advanced-circuit` at 5/s → route failures section shows failures in red
- Toggle Debug off → debug panel absent
- New layout solve (different item) → panel re-renders with fresh data

---

## Phase 3 — Enhanced Step-Through Debugger

**What to build:** Extend the existing ◀/▶ step-through controls in `web/src/main.ts` and `web/src/renderer/traceOverlay.ts`.

### 3.1 Phase stats in step label

Show entity count + cumulative elapsed time next to the phase name:

```
Before: [◀] route [▶]
After:  [◀] route — 847 entities, 286ms elapsed [▶]
```

Source: `PhaseComplete.entity_count` + cumulative sum of `PhaseTime.duration_ms` up to that phase.

### 3.2 Entity delta highlight

When stepping from phase N to N+1, newly added entities (present in snapshot N+1 but not N) flash a highlight color for 1 second. Implemented as a temporary tint via PixiJS `tint` property on entity sprites.

### 3.3 Jump to failure shortcut

If `RouteFailure` events exist, add a `⚠ N` badge to the step bar. Clicking it:
1. Jumps the pixi-viewport to center on the first failure tile
2. Highlights the ✕ marker with a pulse animation

### 3.4 Keyboard shortcuts

- `ArrowLeft`/`ArrowRight` → prev/next phase (when step bar is visible)
- `f` → jump to first route failure (if any)

### Phase 3 Validation

- Step through a multi-phase layout; entity count increases correctly per phase
- New entities flash on step-forward
- `f` key jumps viewport to failure marker
- Arrow keys navigate phases without clicking

---

## Phase 4 — Performance History & Comparison

**What to build:** Run history stored in `localStorage`, comparison view in the debug panel.

### 4.1 Run history storage

After each traced layout, store in `localStorage["fucktorio-trace-history"]`:

```json
[
  { "item": "electronic-circuit", "rate": 10, "machine": "assembling-machine-3",
    "timestamp": 1712345678000, "total_ms": 312,
    "phases": { "rows": 12, "lanes": 8, "route": 286, "validate": 4, "poles": 2 } }
]
```

Keep last 20 entries. Show a "History" button in the debug panel header.

### 4.2 Comparison timeline

When history is open, show stacked timeline bars for:
- Current run (highlighted)
- Best run (shortest total_ms)
- Worst run

Bars aligned to same scale so relative performance is visually obvious.

### 4.3 Hotspot annotation

If any phase takes >50% of total time, annotate its segment with an amber `⚡` hotspot icon and a tooltip: `"Hotspot: {phase} is {pct}% of total time"`.

### 4.4 Export trace JSON

"Copy trace JSON" button in debug panel header → copies the raw `TraceEvent[]` array as JSON. Useful for pasting into external profiling tools.

### Phase 4 Validation

- Run same recipe twice → history stores both entries
- Comparison view shows both timelines on same scale
- Hotspot annotation appears when route phase >> 50%
- Exported JSON is valid and parseable

---

## Implementation Notes

### TypeScript narrowing pattern

All existing overlay code uses `Extract<TraceEvent, { phase: "..." }>` narrowing. Follow the same pattern:

```typescript
type RouteFailureEvent = Extract<TraceEvent, { phase: "RouteFailure" }>;
for (const evt of events) {
  if (evt.phase !== "RouteFailure") continue;
  const d = (evt as RouteFailureEvent).data;
  // ...
}
```

### Key files

| File | Role |
|---|---|
| `web/src/renderer/traceOverlay.ts` | All grid overlays — add Phase 1 events here |
| `web/src/ui/debugPanel.ts` | New — Phase 2 stats panel renderer |
| `web/src/ui/sidebar.ts` | Wire debugPanel; add CSS for new components |
| `web/src/main.ts` | Phase 3 step controls + keyboard shortcuts |
| `web/src/state.ts` | Phase 4 history persistence |
| `crates/core/src/trace.rs` | Reference — all TraceEvent variants (no changes needed) |

### Build verification

```bash
# After any change:
cd web && npm run build      # tsc --noEmit + vite build

# Full WASM rebuild (only if Rust changed):
wasm-pack build crates/wasm-bindings --target web --out-dir ../../web/src/wasm-pkg
```
