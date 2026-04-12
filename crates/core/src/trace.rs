//! Structured trace event collection for the bus layout pipeline.
//!
//! Thread-local collector — zero overhead when no trace is active.
//! Use `start_trace()` to begin collection, `emit()` to record events,
//! and `drain_events()` to retrieve them.

use std::cell::RefCell;

use serde::{Deserialize, Serialize};
use crate::models::PlacedEntity;

// ---------------------------------------------------------------------------
// Collector
// ---------------------------------------------------------------------------

thread_local! {
    static COLLECTOR: RefCell<Option<Vec<TraceEvent>>> = const { RefCell::new(None) };
}

/// Start trace collection for the current thread. Returns a guard that
/// cleans up on drop.
pub fn start_trace() -> TraceGuard {
    COLLECTOR.with(|c| *c.borrow_mut() = Some(Vec::new()));
    TraceGuard
}

/// RAII guard — clears the collector on drop.
pub struct TraceGuard;

impl Drop for TraceGuard {
    fn drop(&mut self) {
        COLLECTOR.with(|c| *c.borrow_mut() = None);
    }
}

/// Emit a trace event. No-op if no trace is active.
pub fn emit(event: TraceEvent) {
    COLLECTOR.with(|c| {
        if let Some(ref mut events) = *c.borrow_mut() {
            events.push(event);
        }
    });
}

/// Drain collected events from the current thread.
pub fn drain_events() -> Vec<TraceEvent> {
    COLLECTOR.with(|c| c.borrow_mut().take().unwrap_or_default())
}

/// Check if a trace is currently active.
#[allow(dead_code)]
pub fn is_active() -> bool {
    COLLECTOR.with(|c| c.borrow().is_some())
}

// ---------------------------------------------------------------------------
// Trace event types
// ---------------------------------------------------------------------------

#[cfg_attr(feature = "wasm", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "phase", content = "data")]
pub enum TraceEvent {
    // Phase 1: Row Placement
    RowsPlaced { rows: Vec<RowInfo> },
    RowSplit {
        recipe: String,
        original_count: usize,
        split_into: usize,
        reason: String,
    },

    // Phase 2: Lane Planning
    LanesPlanned {
        lanes: Vec<LaneInfo>,
        families: Vec<FamilyInfo>,
        bus_width: i32,
    },
    LaneSplit {
        item: String,
        rate: f64,
        max_lane_cap: f64,
        n_splits: usize,
    },
    LaneOrderOptimized {
        ordering: Vec<String>,
        crossing_score: usize,
    },

    // Phase 3: Bus Routing
    CrossingZoneSolved {
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        solve_time_us: u64,
    },
    CrossingZoneSkipped {
        tap_item: String,
        tap_x: i32,
        tap_y: i32,
        reason: String,
    },
    BalancerStamped {
        item: String,
        shape: (usize, usize),
        y_start: i32,
        y_end: i32,
        template_found: bool,
    },
    LaneRouted {
        item: String,
        x: i32,
        is_fluid: bool,
        trunk_segments: usize,
        tapoffs: usize,
    },
    TapoffRouted {
        item: String,
        from_x: i32,
        from_y: i32,
        to_x: i32,
        to_y: i32,
        path_len: usize,
    },

    // Phase 4: Output Merging
    OutputMerged {
        item: String,
        rows: Vec<usize>,
        merge_y: i32,
    },
    MergerBlockPlaced {
        item: String,
        lanes: usize,
        block_y: i32,
        block_height: i32,
    },

    // Phase 5: Power Poles
    PolesPlaced {
        count: usize,
        strategy: String,
    },

    // Phase boundary markers
    PhaseComplete {
        phase: String,
        entity_count: usize,
    },
    /// Full entity snapshot at a phase boundary (only emitted when tracing is active).
    PhaseSnapshot {
        phase: String,
        entities: Vec<PlacedEntity>,
        width: i32,
        height: i32,
    },

    // Phase timing (wall-clock milliseconds per major phase)
    PhaseTime {
        phase: String,
        duration_ms: u64,
    },

    // Negotiate (A*) summary
    NegotiateComplete {
        specs: usize,
        iterations: u32,
        duration_ms: u64,
    },

    // Solver output — emitted at the start of build_bus_layout
    SolverCompleted {
        recipe_count: usize,
        machine_count: usize,
        external_input_count: usize,
        external_output_count: usize,
        machines: Vec<MachineTrace>,
    },

    // A* route failure — a spec had no valid path after all iterations
    RouteFailure {
        /// The lane key (e.g. "tap:iron-plate:3:45" or "trunk:copper-wire:2")
        spec_key: String,
        item: String,
        from_x: i32,
        from_y: i32,
        to_x: i32,
        to_y: i32,
    },

    // Validation results — emitted by validate() after all checks run
    ValidationCompleted {
        error_count: usize,
        warning_count: usize,
        issues: Vec<ValidationIssueTrace>,
    },

    // External input lane consolidation — N consumer rows served by M trunk lanes
    LaneConsolidated {
        item: String,
        /// Total rate this item is consumed at
        rate: f64,
        /// Number of recipe rows that consume this item
        consumer_count: usize,
        /// Number of trunk lanes used (< consumer_count means sharing)
        n_trunk_lanes: usize,
        rate_per_lane: f64,
    },

    // SAT crossing zone removed because it conflicted with a splitter stamp tile
    CrossingZoneConflict {
        /// The crossing segment ID that was removed
        segment_id: String,
        /// Tile position of the conflict
        conflict_x: i32,
        conflict_y: i32,
    },

    // A foreign-trunk UG bridge was dropped because its output collided with
    // the trunk's own tap-off. Surfaced by `route_belt_lane`/`route_intermediate_lane`
    // to `build_bus_layout` so it can push rows apart and retry.
    BridgeDropped {
        trunk_item: String,
        trunk_x: i32,
        range_start: i32,
        range_end: i32,
        colliding_tap_y: i32,
    },

    // `build_bus_layout` is retrying place_rows → plan_bus_lanes → route_bus
    // after seeing dropped bridges from the previous attempt. `attempt` is
    // the retry number (1 = first retry, so second overall attempt).
    BridgeRetry {
        attempt: u32,
        dropped_count: usize,
        extra_gap_updates: usize,
    },

    // All retries exhausted (hit MAX_BRIDGE_RETRIES) but bridges are still
    // being dropped. Layout will render with the current (possibly broken)
    // state and the validator will flag remaining issues.
    BridgeRetryExhausted {
        final_dropped_count: usize,
        max_retries: u32,
    },

    // Per-band measurement emitted after a successful route_bus. One event
    // per adjacent row pair. Used by the compaction baseline/scoreboard to
    // measure total inter-row gap tiles before any shrinking is applied.
    InterRowBand {
        upper_row_idx: usize,
        lower_row_idx: usize,
        band_y_start: i32,
        band_y_end: i32,
        gap_height: i32,
        trunk_count: usize,
        distinct_items: usize,
    },

    // Ghost routing (Phase 2) — emitted by route_bus_ghost in ghost_router.rs
    GhostRoutingComplete {
        entity_count: usize,
        cluster_count: usize,
        max_cluster_tiles: usize,
        unroutable_count: usize,
    },
    GhostSpecRouted {
        spec_key: String,
        path_len: usize,
        crossings: usize,
        turns: usize,
        tiles: Vec<(i32, i32)>,
        crossing_tiles: Vec<(i32, i32)>,
    },
    GhostSpecFailed {
        spec_key: String,
        from_x: i32,
        from_y: i32,
        to_x: i32,
        to_y: i32,
    },

    // Ghost routing (Phase 3) — emitted by resolve_clusters in ghost_router.rs
    GhostClusterSolved {
        cluster_id: usize,
        zone_x: i32,
        zone_y: i32,
        zone_w: u32,
        zone_h: u32,
        boundary_count: usize,
        variables: u32,
        clauses: u32,
        solve_time_us: u64,
    },
    GhostClusterFailed {
        cluster_id: usize,
        zone_x: i32,
        zone_y: i32,
        zone_w: u32,
        zone_h: u32,
        boundary_count: usize,
    },

    // Phase-1 instrumentation: emitted after all ghost specs are routed but
    // before crossing resolution. Reports per-tile axis occupancy so we can
    // see same-axis conflicts (Phase 2 negotiation target).
    GhostAxisOccupancy {
        tiles: Vec<GhostAxisOccupancyTile>,
        same_axis_conflict_count: u32,
        perpendicular_crossing_count: u32,
    },

    // Phase-2 negotiation: emitted once per iteration of the negotiation
    // loop in `route_bus_ghost`. The loop bumps a per-tile per-axis cost
    // grid each time it sees same-axis pile-ups, and re-routes until the
    // conflict count stops improving.
    GhostNegotiationIteration {
        iter: u32,
        same_axis_conflict_count: u32,
        perpendicular_crossing_count: u32,
        unroutable_count: u32,
        cost_grid_size: u32,
    },
}

// ---------------------------------------------------------------------------
// Summary structs (lightweight, serializable versions of internal types)
// ---------------------------------------------------------------------------

#[cfg_attr(feature = "wasm", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GhostAxisOccupancyTile {
    pub x: i32,
    pub y: i32,
    /// Number of routed specs whose axis at this tile is Vertical (N/S).
    pub vert_count: u32,
    /// Number of routed specs whose axis at this tile is Horizontal (E/W).
    pub horiz_count: u32,
}

#[cfg_attr(feature = "wasm", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RowInfo {
    pub index: usize,
    pub recipe: String,
    pub machine: String,
    pub machine_count: usize,
    pub y_start: i32,
    pub y_end: i32,
    pub row_kind: String,
}

#[cfg_attr(feature = "wasm", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaneInfo {
    pub item: String,
    pub x: i32,
    pub rate: f64,
    pub is_fluid: bool,
    pub source_y: i32,
    pub tap_off_ys: Vec<i32>,
    pub consumer_rows: Vec<usize>,
    pub producer_row: Option<usize>,
    pub family_id: Option<usize>,
}

#[cfg_attr(feature = "wasm", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FamilyInfo {
    pub item: String,
    pub shape: (usize, usize),
    pub lane_xs: Vec<i32>,
    pub balancer_y_start: i32,
    pub balancer_y_end: i32,
    pub total_rate: f64,
    pub producer_rows: Vec<usize>,
}

#[cfg_attr(feature = "wasm", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineTrace {
    pub recipe: String,
    pub machine: String,
    /// Fractional machine count (e.g. 2.4 → ceil to 3 in practice)
    pub count: f64,
    /// Total output rate of this machine group (items/s)
    pub rate: f64,
}

#[cfg_attr(feature = "wasm", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationIssueTrace {
    pub severity: String,
    pub category: String,
    pub message: String,
    pub x: Option<i32>,
    pub y: Option<i32>,
}
