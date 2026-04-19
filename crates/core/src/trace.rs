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
    static SINK: RefCell<Option<Box<dyn FnMut(&TraceEvent)>>> = const { RefCell::new(None) };
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

/// Install a sink that sees every emitted event as it happens.
/// Coexists with the collector — both fire on each emit. Returns a guard
/// that removes the sink on drop.
pub fn set_sink(sink: Box<dyn FnMut(&TraceEvent)>) -> SinkGuard {
    SINK.with(|s| *s.borrow_mut() = Some(sink));
    SinkGuard
}

/// RAII guard — clears the sink on drop.
pub struct SinkGuard;

impl Drop for SinkGuard {
    fn drop(&mut self) {
        SINK.with(|s| *s.borrow_mut() = None);
    }
}

/// Emit a trace event. No-op if neither a collector nor a sink is active.
pub fn emit(event: TraceEvent) {
    SINK.with(|s| {
        if let Some(ref mut sink) = *s.borrow_mut() {
            sink(&event);
        }
    });
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

    // Emitted after a junction solution is stamped and ghost-surface
    // tiles belonging to participating specs are evicted from inside
    // the footprint. `participating_count` is the number of specs the
    // strategy claimed authority over; `released_count` is how many
    // ghost tiles the release call actually evicted (may include
    // ghosts from earlier phases that still held a GhostSurface claim).
    GhostResidueCleared {
        zone_x: i32,
        zone_y: i32,
        zone_w: u32,
        zone_h: u32,
        participating_count: usize,
        released_count: usize,
    },

    // Sync-gap assertion. After a junction solution is stamped, any
    // `ghost:*` entity still in the local entity list whose (x,y)
    // sits inside the footprint is a leaked ghost — the release set
    // missed it, or a new code path pushes it after the sync. Never
    // fires on a healthy pipeline; its presence in a snapshot is the
    // signal to investigate.
    GhostResidueLeaked {
        zone_x: i32,
        zone_y: i32,
        leaked_tiles: Vec<(i32, i32)>,
    },

    // Emitted by `try_bridge` in ghost_router.rs whenever a per-tile
    // perpendicular template rejection happens. One event per failed
    // bridge attempt, so a fully-rejected perpendicular crossing emits
    // two (vertical-first, horizontal-fallback). Drives the
    // diagnose_junctions step for correlating "unresolved" regions with
    // their rejection cause.
    JunctionTemplateRejected {
        tile_x: i32,
        tile_y: i32,
        bridge_dir: String,
        reason: String,
    },

    // DIAGNOSTIC: fires once per `build_bus_layout` run, after row
    // placement + lane planning. Captures a compact fingerprint of the
    // layout's geometric decisions so we can compare native-vs-WASM
    // output and pin down where target-dependent iteration order
    // leaks into the pipeline. Not used by any renderer; purely for
    // root-causing reproducibility bugs.
    PipelineDiagnostics {
        /// Solver's dependency_order in iteration order.
        dep_order: Vec<String>,
        /// Row layout fingerprint, row-index order. Each entry packs
        /// `recipe,y_start,y_end` into a single string so the trace
        /// serialises cleanly through `tsify_next` (which chokes on
        /// heterogeneous tuples in `Vec`).
        rows: Vec<String>,
        /// Bus lane layout fingerprint, lane-order. Each entry packs
        /// `item,x,rate,is_fluid`.
        lanes: Vec<String>,
    },

    // Emitted by `junction_solver::solve_crossing` when a strategy
    // accepts the junction and its solution is chosen as the winner for
    // this growth iteration. Terminal event — at most one per cluster.
    JunctionSolved {
        tile_x: i32,
        tile_y: i32,
        strategy: String,
        growth_iter: usize,
        region_tiles: usize,
    },

    // Emitted once per variant whose strategy produces a walker-valid
    // solution. Multiple candidates per iter are expected — the cost
    // score decides which one `JunctionSolved` will ship. The loser
    // candidates exist only in the trace, not in the final layout.
    JunctionCandidateSolved {
        tile_x: i32,
        tile_y: i32,
        strategy: String,
        growth_iter: usize,
        /// `""` for the primary attempt on the current region,
        /// `"variant-west"` / `-north` / `-east` / `-south` for the
        /// speculative single-side expansions.
        variant: String,
        region_tiles: usize,
        cost: u32,
    },

    // Emitted at the point `solve_crossing` picks the cheapest candidate
    // across all variants of a single growth iter. `considered` is the
    // full `(variant_label, cost)` list the selector chose from, in the
    // order candidates were produced — the debugger uses it to show why
    // a particular variant won and what the alternatives cost.
    JunctionVariantChosen {
        tile_x: i32,
        tile_y: i32,
        iter: usize,
        variant: String,
        cost: u32,
        considered: Vec<(String, u32)>,
    },
    // Emitted when the growth loop gives up: either frontier exhausted
    // (all participating belts fully consumed) or tile cap hit.
    JunctionGrowthCapped {
        tile_x: i32,
        tile_y: i32,
        iters: usize,
        region_tiles: usize,
        reason: String,
    },
    // Emitted when the region walker rejects a strategy's proposed
    // solution because it would break a routed path that touches the
    // region's footprint. Caller treats this the same as the strategy
    // returning `None`: fall through to the next strategy, and if all
    // strategies fail (or are vetoed), grow and retry.
    RegionWalkerVeto {
        tile_x: i32,
        tile_y: i32,
        strategy: String,
        growth_iter: usize,
        /// Variant label (see `JunctionGrowthIteration::variant`). Empty
        /// for the primary attempt at this iter.
        variant: String,
        /// Segment id of the first broken path (there may be more).
        broken_segment: String,
        /// Tile where the walker's check fired for that path.
        break_tile_x: i32,
        break_tile_y: i32,
        /// Total number of breaks (one per affected path that failed).
        break_count: usize,
    },

    // Junction solver step-through instrumentation.
    // These fire alongside the coarser `JunctionSolved` /
    // `JunctionGrowthCapped` / `JunctionTemplateRejected` /
    // `RegionWalkerVeto` events to give a full per-iteration view of
    // the growth loop and each strategy attempt. Designed for CLI
    // replay + UI step-through.

    /// Emitted once per `solve_crossing` call, at entry (iteration 0
    /// not yet attempted). Reports the seed and the specs that will
    /// participate.
    JunctionGrowthStarted {
        seed_x: i32,
        seed_y: i32,
        participating: Vec<ParticipatingSpec>,
        /// Stamped entities within `seed_bbox + 1` perimeter that could
        /// physically affect the zone (splitters, belts, UG belts).
        /// Useful for understanding external feeds before growth starts.
        nearby_stamped: Vec<StampedNeighbor>,
    },

    /// Emitted at the start of each growth iteration, *before*
    /// strategies are tried. Reports the full zone state at that
    /// moment.
    JunctionGrowthIteration {
        seed_x: i32,
        seed_y: i32,
        iter: usize,
        /// Sub-iteration label. Empty string for the primary attempt on
        /// the current region; otherwise names a speculative single-side
        /// expansion variant ("variant-west", "variant-north",
        /// "variant-east", "variant-south"). The debugger groups the
        /// per-iter state keyed by `(iter, variant)` so variants don't
        /// overwrite each other.
        variant: String,
        bbox_x: i32,
        bbox_y: i32,
        bbox_w: u32,
        bbox_h: u32,
        tiles: Vec<(i32, i32)>,
        forbidden_tiles: Vec<(i32, i32)>,
        boundaries: Vec<BoundarySnapshot>,
        participating: Vec<String>,
        encountered: Vec<String>,
    },

    /// Emitted after each strategy.try_solve call within an iteration.
    /// One per (iter, strategy) pair. Carries the outcome verdict —
    /// includes walker-veto as Vetoed, template-rejection as Rejected,
    /// SAT UNSAT as Unsatisfiable, success as Solved.
    JunctionStrategyAttempt {
        seed_x: i32,
        seed_y: i32,
        iter: usize,
        /// Variant label (see `JunctionGrowthIteration::variant`). Empty
        /// for the primary attempt at this iter.
        variant: String,
        strategy: String,
        outcome: String,
        detail: String,
        elapsed_us: u64,
    },

    /// Emitted by the SAT strategy every time `solve_crossing_zone` is
    /// called, with the full invocation signature. This is enough to
    /// replay a single SAT solve in isolation (outside the larger
    /// junction solver). Complements JunctionStrategyAttempt with
    /// SAT-specific numbers.
    // Emitted once per cost-descent iteration in the SAT strategy.
    // `descent_iter` is 0-indexed; `cap` is the hard cost cap used
    // on that attempt. `satisfied=true` means SAT found a layout
    // within the cap (descent continues with a tighter cap);
    // `satisfied=false` means UNSAT (descent halts, prior best is
    // optimal at this cap). Terminal: at most `cost_descent_max_iters`
    // per winning SAT invocation.
    SatCostDescent {
        seed_x: i32,
        seed_y: i32,
        iter: usize,
        variant: String,
        descent_iter: u8,
        cap: u32,
        satisfied: bool,
        solve_time_us: u64,
    },

    SatInvocation {
        seed_x: i32,
        seed_y: i32,
        iter: usize,
        /// Variant label (see `JunctionGrowthIteration::variant`). Empty
        /// for the primary attempt at this iter.
        variant: String,
        zone_x: i32,
        zone_y: i32,
        zone_w: u32,
        zone_h: u32,
        boundaries: Vec<BoundarySnapshot>,
        forced_empty: Vec<(i32, i32)>,
        belt_tier: String,
        max_reach: u32,
        satisfied: bool,
        variables: u32,
        clauses: u32,
        solve_time_us: u64,
        entities_raw: usize,
        /// Entities SAT produced, captured before `prune_dangling_sat_entities`.
        /// Empty when `satisfied=false`. Lets the junction debugger render
        /// the candidate layout — especially useful on walker veto, where
        /// the solution is otherwise discarded.
        proposed_entities: Vec<SatProposedEntity>,
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

    // SAT solution pruned of dangling (unreachable / dead-end) belt entities.
    SatPruned {
        zone_x: i32,
        zone_y: i32,
        total: usize,
        kept: usize,
    },
}

// ---------------------------------------------------------------------------
// Summary structs (lightweight, serializable versions of internal types)
// ---------------------------------------------------------------------------

#[cfg_attr(feature = "wasm", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParticipatingSpec {
    pub key: String,
    pub item: String,
    pub initial_tile_x: i32,
    pub initial_tile_y: i32,
    /// Full path tile count (for context on how much can be grown into
    /// the region from each end of this spec).
    pub path_len: usize,
    /// Initial frontier (start, end) index into the path.
    pub initial_start: usize,
    pub initial_end: usize,
}

#[cfg_attr(feature = "wasm", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StampedNeighbor {
    pub x: i32,
    pub y: i32,
    pub name: String,
    /// Direction the entity faces (belts / splitters / UG).
    pub direction: String,
    pub carries: Option<String>,
    pub segment_id: Option<String>,
    /// True if this entity's output would land on a tile within the
    /// initial seed's 1-tile perimeter (hint for "this might sideload").
    pub feeds_seed_area: bool,
}

#[cfg_attr(feature = "wasm", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoundarySnapshot {
    pub x: i32,
    pub y: i32,
    pub direction: String,
    pub item: String,
    pub is_input: bool,
    /// True iff the strategy moved this boundary onto a Permanent
    /// entity's tile inside the bbox (in `forced_empty`). The encoder
    /// then propagates flow constraints to the in-zone neighbour rather
    /// than placing an entity at this tile.
    pub interior: bool,
    /// Spec key that produced this boundary. Useful for correlating a
    /// growth iteration with the specs' movement frontiers.
    pub spec_key: String,
    /// Whether this boundary comes from a spec that seeded the cluster
    /// (`"participating"`) or a spec that merely passes through the
    /// cluster's bbox (`"encountered"`). Encountered specs contribute
    /// boundary pairs so SAT can route them instead of treating their
    /// belts as forbidden obstacles.
    pub origin: String,
    /// If a physical external feeder landed items on this tile, the
    /// feeder's entity name + output direction. `None` means no
    /// external feeder — SAT will assume native (opposite(direction))
    /// arrival.
    pub external_feeder: Option<ExternalFeederSnapshot>,
}

#[cfg_attr(feature = "wasm", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalFeederSnapshot {
    pub entity_name: String,
    pub entity_x: i32,
    pub entity_y: i32,
    pub direction: String,
}

/// Minimal view of an entity SAT proposed for a crossing zone. Captured
/// pre-prune so the junction debugger can show exactly what SAT produced
/// — including entities that the dangling-prune step later drops.
#[cfg_attr(feature = "wasm", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SatProposedEntity {
    pub x: i32,
    pub y: i32,
    pub name: String,
    pub direction: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub carries: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub io_type: Option<String>,
}

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
