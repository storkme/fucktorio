# Trace Events Completion Plan

## Overview
Four related tasks from the trace events track, implemented as a single PR.

## Task 1: Emit remaining trace events (Rust only)

Add `trace::emit()` calls for the 4 defined-but-unemitted variants:

### LaneSplit — `bus_router.rs:split_overflowing_lanes()`
- After the `n_splits` calculation (line ~638), emit when `n_splits > 1`:
  ```rust
  trace::emit(TraceEvent::LaneSplit {
      item: lane.item.clone(),
      rate: lane.rate,
      max_lane_cap,
      n_splits,
  });
  ```

### LaneOrderOptimized — `bus_router.rs:optimize_lane_order()`
- At the end of the function (before `result` is returned, line ~533):
  ```rust
  let ordering: Vec<String> = result.iter().map(|ln| ln.item.clone()).collect();
  trace::emit(TraceEvent::LaneOrderOptimized {
      crossing_score: /* computed */,
      ordering,
  });
  ```
- Need to compute the crossing score. The `find_best_permutation` already computes it. We need to capture it. Simplest: add a return value or compute inline.

### CrossingZoneSkipped — `bus_router.rs:extract_and_solve_crossings()`
- In the loop over `zone_specs` (line ~2360), when SAT returns `None`:
  ```rust
  } else {
      trace::emit(TraceEvent::CrossingZoneSkipped {
          tap_item: tap_item.clone(),
          tap_x: *tap_x,
          tap_y: *tap_y,
          reason: "sat-unsolved".into(),
      });
  }
  ```

### TapoffRouted — `bus_router.rs:route_belt_lane()`
- After each tap-off path is rendered (around lines 1793, 1810, 1827), emit:
  ```rust
  trace::emit(TraceEvent::TapoffRouted {
      item: lane.item.clone(),
      from_x: x,
      from_y: tap_y,
      to_x: /* end of path or consumer */,
      to_y: /* end of path */,
      path_len: /* path entity count */,
  });
  ```
- The challenge is getting (to_x, to_y, path_len) from `render_path`. Simplest: have `render_path` return the count and end position, or compute from the path itself.

## Task 2: Web app debug overlay

### New file: `web/src/renderer/traceOverlay.ts`
- Renders trace events as visual overlays on the layout canvas
- Each event type gets a visual representation:
  - **RowsPlaced/RowSplit**: Row boundary lines (horizontal dashed)
  - **LanesPlanned**: Lane column highlights (vertical strips)
  - **LaneSplit/LaneOrderOptimized**: Log-only (no spatial rendering)
  - **CrossingZoneSolved**: Blue-tinted rectangles (already rendered via regions, but trace gives per-zone timing)
  - **CrossingZoneSkipped**: Orange-tinted rectangles for skipped zones
  - **BalancerStamped**: Magenta rectangles
  - **LaneRouted/TapoffRouted**: Path lines (green for routed, dim for tap-offs)
  - **OutputMerged/MergerBlockPlaced**: Yellow rectangles
  - **PolesPlaced**: Log-only

### Integration: `entities.ts`
- Add a `renderTraceOverlay(trace, container)` call after entity rendering
- The existing crossing_zone region rendering in entities.ts stays (it uses LayoutResult.regions)

### Integration: `main.ts`
- Add a "Debug" toggle checkbox (like the existing "Item colours" and "Rates" toggles)
- When enabled, re-renders with the trace overlay layer visible
- The sidebar's "Generate Layout" button switches to `buildLayoutTraced()` when debug mode is on

## Task 3: Validation issue overlay

### WASM exposure
- Add a `validate` function to `crates/wasm-bindings/src/lib.rs` that calls `validate_layout` and returns `Vec<ValidationIssue>`
- ValidationIssue already has `x: Option<i32>`, `y: Option<i32>`, `severity`, `category`, `message`

### New file: `web/src/renderer/validationOverlay.ts`
- For each ValidationIssue with position data:
  - Error: red pulsing circle
  - Warning: yellow circle
  - Info: blue circle
- Hover shows the issue message in a tooltip

### Integration
- Run validation after layout generation in the sidebar
- Add a "Validation" toggle checkbox
- Show issue count badge on the toggle

## Task 4: Intermediate state snapshots

### Rust changes: `crates/core/src/bus/layout.rs`
- In `build_bus_layout_traced()`, instead of just draining events at the end, capture snapshots at key pipeline boundaries:
  - After row placement
  - After lane planning
  - After bus routing
  - After output merging
- Add a `LayoutSnapshot` struct to `trace.rs`:
  ```rust
  pub struct LayoutSnapshot {
      pub phase: String,       // "rows_placed", "lanes_planned", etc.
      pub entities: Vec<PlacedEntity>,
      pub width: i32,
      pub height: i32,
  }
  ```
- Add `TraceEvent::PhaseComplete { phase, entity_count }` variant (lightweight — just phase name and count)

### Web app: step-through UI
- Add prev/next buttons in the debug overlay
- Each step renders the entity snapshot from that phase
- Shows the trace events accumulated up to that point

## Implementation order

1. **Task 1** — Emit remaining events (Rust only, no web changes)
2. **Task 3** — Validation overlay (independent, touches WASM + web only)
3. **Task 2** — Trace debug overlay (web rendering of trace events)
4. **Task 4** — Intermediate snapshots (Rust + web step-through)

Tasks 1 and 3 are independent and could be done in parallel. Task 2 builds on Task 1. Task 4 extends both.

## Files to modify

### Rust
- `crates/core/src/bus/bus_router.rs` — emit LaneSplit, LaneOrderOptimized, CrossingZoneSkipped, TapoffRouted
- `crates/core/src/trace.rs` — add PhaseComplete variant + LayoutSnapshot
- `crates/core/src/bus/layout.rs` — emit PhaseComplete snapshots
- `crates/core/src/models.rs` — (no changes needed, LayoutResult.trace already exists)
- `crates/wasm-bindings/src/lib.rs` — add validate function

### Web
- `web/src/engine.ts` — add validate function, expose TraceEvent types
- `web/src/renderer/traceOverlay.ts` (new) — trace event rendering
- `web/src/renderer/validationOverlay.ts` (new) — validation issue markers
- `web/src/main.ts` — add Debug + Validation toggles
- `web/src/ui/sidebar.ts` — use buildLayoutTraced when debug enabled
