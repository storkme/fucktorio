//! Global routing plan — decides cross-lane conflicts before A* runs.
//!
//! Today's bus router scatters conflict detection across `foreign_trunk_skip_ys`,
//! `bridgeable_ranges`, splitter stamp placement, and the A* spec generator.
//! Each has its own escape hatch and they don't compose, which is how the
//! tier-2 electronic-circuit sideload-into-UG-input bug happened (three
//! escape hatches stacked and A* fell back to an invalid pattern).
//!
//! This module is the single place that owns those decisions. `plan_layout`
//! takes the planned lanes + row spans, derives which trunks must yield
//! (UG-bridge over foreign tap-offs), which tap-off tiles are pinned to a
//! specific direction + entry set, and returns a `Plan` that `route_bus`
//! consumes instead of rederiving its own skip sets.
//!
//! Phase 3a (this commit): skeleton only. `plan_layout` returns an empty
//! plan; `route_bus` ignores it. Subsequent phases migrate foreign_yields
//! (3b) and bridge/retry logic (3c) into this module.

use rustc_hash::{FxHashMap, FxHashSet};

use crate::bus::bus_router::{BusLane, DroppedBridge};
use crate::bus::placer::RowSpan;
use crate::models::EntityDirection;

// ---------------------------------------------------------------------------
// Foreign yield derivation
// ---------------------------------------------------------------------------

/// Compute the set of foreign-tap-off yields this lane would need if its
/// trunk ran from `trunk_start_y` to `trunk_end_y` (both exclusive of the
/// bounds — mirrors the original `foreign_trunk_skip_ys` filter).
///
/// Two conflict classes produce yields:
/// 1. A west-neighbor lane's output-return row sits inside this trunk's
///    range — the neighbor needs a free landing tile, so this trunk yields.
/// 2. A left-lane's tap-off would cross this trunk's column. The tap-off
///    travels East on the surface and would sideload into this trunk unless
///    this trunk goes underground past it.
///
/// Yields whose bridge-output (`y + 1`) would collide with this lane's own
/// tap-off are still emitted here — the filter that drops such yields lives
/// with the bridge logic (Phase 3c will fold it in alongside the retry loop).
pub fn compute_foreign_yields_for_lane(
    lane: &BusLane,
    all_lanes: &[BusLane],
    row_spans: &[RowSpan],
    trunk_start_y: i32,
    trunk_end_y: i32,
) -> Vec<Yield> {
    let mut yields: Vec<Yield> = Vec::new();

    // Case 1: west neighbor's output-return rows need a free landing tile here.
    let west_col = lane.x - 1;
    if let Some(neighbor) = all_lanes.iter().find(|other| {
        !other.is_fluid && !std::ptr::eq(*other, lane) && other.x == west_col
    }) {
        let mut producer_rows: Vec<usize> = Vec::new();
        if let Some(pr) = neighbor.producer_row {
            producer_rows.push(pr);
        }
        producer_rows.extend(&neighbor.extra_producer_rows);
        for p in producer_rows {
            if p >= row_spans.len() {
                continue;
            }
            let y = row_spans[p].output_belt_y;
            if trunk_start_y < y && y < trunk_end_y {
                yields.push(Yield {
                    trunk_x: lane.x,
                    y_range: (y, y),
                    reason: YieldReason::ForeignTapoff {
                        foreign_item: neighbor.item.clone(),
                        tap_y: y,
                    },
                });
            }
        }
    }

    // Case 2: any left-lane tap-off that would cross this trunk column.
    // Note: we DON'T apply the own-tap-off collision filter here — that's
    // the bridgeable_ranges logic that Phase 3c will fold into plan_layout.
    // This function matches the original foreign_trunk_skip_ys semantics,
    // which also skip the collision check and rely on bridgeable_ranges
    // downstream to drop unbuildable bridges.
    let own_tap_set: std::collections::HashSet<i32> = lane.tap_off_ys.iter().copied().collect();
    for other in all_lanes {
        if other.is_fluid || std::ptr::eq(other, lane) || other.x >= lane.x {
            continue;
        }
        let other_last_tap = other.tap_off_ys.iter().copied().max();
        for &tap_y in &other.tap_off_ys {
            if !(trunk_start_y < tap_y && tap_y < trunk_end_y) {
                continue;
            }
            if own_tap_set.contains(&(tap_y + 1)) {
                continue;
            }
            // Only bridge if the tap-off travels surface all the way to
            // this trunk. If any intermediate lane has a surface belt at
            // tap_y, the tap-off already went underground before reaching
            // lane.x — no bridge needed here.
            let all_intermediate_clear = all_lanes.iter()
                .filter(|mid| !mid.is_fluid && mid.x > other.x && mid.x < lane.x)
                .all(|mid| {
                    mid.tap_off_ys.contains(&tap_y)
                        || mid.tap_off_ys.iter().all(|&y| y < tap_y)
                });
            if all_intermediate_clear {
                yields.push(Yield {
                    trunk_x: lane.x,
                    y_range: (tap_y, tap_y),
                    reason: YieldReason::ForeignTapoff {
                        foreign_item: other.item.clone(),
                        tap_y,
                    },
                });
                // Non-last splitter tap-offs also occupy (other.x+1, tap_y-1)
                // (splitter right half) and (other.x+1, tap_y) (belt East).
                // If this trunk IS that adjacent column, skip tap_y-1 too.
                let is_non_last = other.tap_off_ys.len() > 1
                    && Some(tap_y) != other_last_tap;
                if is_non_last && lane.x == other.x + 1
                    && trunk_start_y < tap_y - 1 && tap_y - 1 < trunk_end_y
                    && !own_tap_set.contains(&tap_y)
                {
                    yields.push(Yield {
                        trunk_x: lane.x,
                        y_range: (tap_y - 1, tap_y - 1),
                        reason: YieldReason::ForeignTapoff {
                            foreign_item: other.item.clone(),
                            tap_y,
                        },
                    });
                }
            }
        }
    }

    yields
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Why a trunk is being asked to yield (go underground) across a y-range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum YieldReason {
    /// The yield crosses a foreign lane's tap-off (perpendicular East belt).
    ForeignTapoff { foreign_item: String, tap_y: i32 },
}

/// A trunk yield: the trunk at `trunk_x` goes underground across
/// `y_range` (inclusive), re-emerging at `y_range.1 + 1`.
#[derive(Debug, Clone)]
pub struct Yield {
    pub trunk_x: i32,
    pub y_range: (i32, i32),
    pub reason: YieldReason,
}

/// Allowed entry directions for a pinned tile. Restricts what A* may do
/// when generating the tile's neighbours — used to forbid sideload-into-UG.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntryDirection {
    /// Straight entry (same direction as the pinned tile faces).
    Straight,
    SideloadFromNorth,
    SideloadFromSouth,
    SideloadFromEast,
    SideloadFromWest,
}

/// A tap-off pin: declares "this tile MUST be a surface belt facing
/// `direction`, and items may only enter via `allowed_entries`".
///
/// Used to forbid the non-last-tap-off splitter stamp → UG-input sideload
/// pattern. If an A* spec would otherwise place a UG input on a pinned
/// tile, the planner returns a conflict and forces a gap.
#[derive(Debug, Clone)]
pub struct TapoffPin {
    pub pos: (i32, i32),
    pub direction: EntityDirection,
    /// Inclusive set of entry directions permitted at this tile.
    /// Empty = pin exists but imposes no entry restriction.
    pub allowed_entries: Vec<EntryDirection>,
}

/// Per-lane planner output. Indexed by the lane's `x` column in `Plan.per_lane`.
#[derive(Debug, Clone, Default)]
pub struct LanePlan {
    /// Y-ranges this lane's trunk goes underground across — already filtered
    /// for own-tap-off collisions, so these are safe to bridge.
    pub yields: Vec<Yield>,
    /// Tap-off tile pins owned by this lane.
    pub tapoff_pins: Vec<TapoffPin>,
}

/// The global routing plan. Built once per layout attempt before `route_bus`.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub per_lane: FxHashMap<i32, LanePlan>,
}

impl Plan {
    /// Look up the plan for a lane by trunk column x. Returns an empty
    /// LanePlan if the lane has no entries (zero-yield/zero-pin case).
    pub fn lane(&self, x: i32) -> LanePlan {
        self.per_lane.get(&x).cloned().unwrap_or_default()
    }
}

/// Reasons `plan_layout` can fail — surfaced to `build_bus_layout` so it
/// can re-run the pipeline with a wider row spacing.
#[derive(Debug, Clone)]
pub enum PlanError {
    /// One or more trunk yields collided with own tap-offs and could not
    /// be bridged without additional row spacing. The orchestrator should
    /// consume `dropped` (one entry per unbridgeable range) and translate
    /// them into `extra_gap_after_row` updates via `apply_dropped_to_gaps`.
    DroppedBridges { dropped: Vec<DroppedBridge> },
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Merge consecutive y-positions into (start, end) inclusive ranges.
fn merge_consecutive(ys: &FxHashSet<i32>) -> Vec<(i32, i32)> {
    if ys.is_empty() {
        return Vec::new();
    }
    let mut sorted: Vec<i32> = ys.iter().copied().collect();
    sorted.sort_unstable();
    let mut ranges: Vec<(i32, i32)> = Vec::new();
    let mut cur_start = sorted[0];
    let mut cur_end = sorted[0];
    for &y in &sorted[1..] {
        if y == cur_end + 1 {
            cur_end = y;
        } else {
            ranges.push((cur_start, cur_end));
            cur_start = y;
            cur_end = y;
        }
    }
    ranges.push((cur_start, cur_end));
    ranges
}

/// Resolve bridge conflicts for a single lane's trunk.
///
/// Takes the foreign yields computed by `compute_foreign_yields_for_lane`,
/// merges consecutive y-positions into ranges, and filters out any range
/// whose bridge output (`range_end + 1`) collides with one of the lane's
/// own tap-off positions. Dropped ranges are surfaced as `DroppedBridge`
/// entries for the orchestrator to resolve via row-gap updates.
///
/// Returns `(bridgeable, dropped)`.
fn resolve_bridge_conflicts_for_lane(
    lane: &BusLane,
    foreign_yields: &[Yield],
) -> (Vec<Yield>, Vec<DroppedBridge>) {
    let ys: FxHashSet<i32> = foreign_yields.iter().map(|y| y.y_range.0).collect();
    let merged = merge_consecutive(&ys);
    let own_tap_set: FxHashSet<i32> = lane.tap_off_ys.iter().copied().collect();

    let mut bridgeable: Vec<Yield> = Vec::new();
    let mut dropped: Vec<DroppedBridge> = Vec::new();

    for (range_start, range_end) in merged {
        if own_tap_set.contains(&(range_end + 1)) {
            dropped.push(DroppedBridge {
                trunk_item: lane.item.clone(),
                trunk_x: lane.x,
                range: (range_start, range_end),
            });
        } else {
            // Rebuild Yield entries spanning range_start..=range_end.
            // We lose the individual reasons from the merge step, but the
            // orchestrator only reads (trunk_x, y_range) so that's fine.
            bridgeable.push(Yield {
                trunk_x: lane.x,
                y_range: (range_start, range_end),
                reason: foreign_yields
                    .iter()
                    .find(|y| y.y_range.0 >= range_start && y.y_range.0 <= range_end)
                    .map(|y| y.reason.clone())
                    .unwrap_or(YieldReason::ForeignTapoff {
                        foreign_item: String::new(),
                        tap_y: range_start,
                    }),
            });
        }
    }

    (bridgeable, dropped)
}

/// Build the global routing plan.
///
/// For each non-fluid lane, compute foreign yields (from
/// `compute_foreign_yields_for_lane`) and resolve bridge conflicts against
/// the lane's own tap-offs. If any conflicts remain unbridgeable (the
/// bridge output collides with a known tap-off), return
/// `PlanError::DroppedBridges` so the orchestrator can widen row gaps and
/// retry before A* runs.
pub fn plan_layout(
    lanes: &[BusLane],
    row_spans: &[RowSpan],
    _bus_width: i32,
) -> Result<Plan, PlanError> {
    let mut plan = Plan::default();
    let mut all_dropped: Vec<DroppedBridge> = Vec::new();

    for lane in lanes {
        if lane.is_fluid {
            continue;
        }
        // Trunk range: from source_y down to the last tap-off (or end of
        // lane). We compute over a wide range so the planner catches all
        // potential conflicts; the route_lane consumers still apply their
        // own (start_y, end_y) filter when rendering the trunk.
        let end_y = lane
            .tap_off_ys
            .iter()
            .copied()
            .max()
            .unwrap_or(lane.source_y);
        if end_y <= lane.source_y {
            continue;
        }

        let foreign_yields = compute_foreign_yields_for_lane(
            lane,
            lanes,
            row_spans,
            lane.source_y,
            end_y + 1,
        );
        let (bridgeable, dropped) = resolve_bridge_conflicts_for_lane(lane, &foreign_yields);

        let entry = plan.per_lane.entry(lane.x).or_default();
        entry.yields.extend(bridgeable);

        all_dropped.extend(dropped);
    }

    if !all_dropped.is_empty() {
        return Err(PlanError::DroppedBridges { dropped: all_dropped });
    }

    Ok(plan)
}

/// Translate a list of dropped bridges into `extra_gap_after_row` updates.
///
/// For each drop, find the row whose belt-y equals the colliding tap y
/// (`range.1 + 1`) and add 1 tile of gap after the PREVIOUS row so the
/// colliding row moves down. Returns the number of updates applied.
pub fn apply_dropped_to_gaps(
    dropped: &[DroppedBridge],
    row_spans: &[RowSpan],
    extra_gaps: &mut FxHashMap<usize, i32>,
) -> usize {
    let mut updates = 0;
    for db in dropped {
        let colliding_y = db.colliding_tap_y();
        let row_idx_opt = row_spans.iter().position(|rs| {
            rs.input_belt_y.contains(&colliding_y) || rs.output_belt_y == colliding_y
        });
        if let Some(row_idx) = row_idx_opt {
            if row_idx > 0 {
                *extra_gaps.entry(row_idx - 1).or_insert(0) += 1;
                updates += 1;
            }
        }
    }
    updates
}
