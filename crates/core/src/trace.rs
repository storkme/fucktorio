//! Structured trace event collection for the bus layout pipeline.
//!
//! Thread-local collector — zero overhead when no trace is active.
//! Use `start_trace()` to begin collection, `emit()` to record events,
//! and `drain_events()` to retrieve them.

use std::cell::RefCell;

use serde::{Deserialize, Serialize};

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
}

// ---------------------------------------------------------------------------
// Summary structs (lightweight, serializable versions of internal types)
// ---------------------------------------------------------------------------

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
