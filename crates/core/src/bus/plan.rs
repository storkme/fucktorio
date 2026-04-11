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

use crate::bus::bus_router::{
    BusLane, CrossingTileSet, DroppedBridge, SolvedCrossing,
};
use crate::bus::placer::RowSpan;
use crate::common::belt_entity_for_rate;
use crate::models::EntityDirection;
use crate::sat::{self, CrossingZone, ZoneBoundary};

// ---------------------------------------------------------------------------
// Lane ordering
// ---------------------------------------------------------------------------

/// Count total underground crossings for a given lane ordering.
pub(crate) fn score_lane_ordering(ordered: &[BusLane], row_spans: &[RowSpan]) -> usize {
    let mut score = 0;

    fn active_range(lane: &BusLane, row_spans: &[RowSpan]) -> (i32, i32) {
        let all_p = lane.all_producers();

        if !all_p.is_empty() && !lane.consumer_rows.is_empty() {
            let start = all_p.iter()
                .map(|&p| row_spans[p].output_belt_y)
                .min()
                .unwrap();
            let end = if !lane.tap_off_ys.is_empty() {
                lane.tap_off_ys.iter().copied().max().unwrap()
            } else {
                start
            };
            (start, end)
        } else if !lane.tap_off_ys.is_empty() {
            let end = lane.tap_off_ys.iter().copied().max().unwrap();
            (lane.source_y, end)
        } else {
            let end = all_p.iter()
                .map(|&p| row_spans[p].output_belt_y)
                .max()
                .unwrap_or(lane.source_y);
            (lane.source_y, end)
        }
    }

    let ranges: Vec<(i32, i32)> = ordered.iter().map(|ln| active_range(ln, row_spans)).collect();

    for (pos, lane) in ordered.iter().enumerate() {
        for &tap_y in &lane.tap_off_ys {
            for &(rs, re) in &ranges[(pos + 1)..] {
                if rs <= tap_y && tap_y <= re {
                    score += 1;
                }
            }
        }

        let all_producers = lane.all_producers();
        for &pri in &all_producers {
            let ret_y = row_spans[pri].output_belt_y;
            for &(rs, re) in &ranges[(pos + 1)..] {
                if rs <= ret_y && ret_y <= re {
                    score += 1;
                }
            }
        }
    }

    // Family feeder landing conflict: penalise when a family lane's template
    // input landing column overlaps with a lane to the RIGHT of the family
    // block, so the optimizer places family blocks rightmost.
    let templates = crate::bus::balancer_library::balancer_templates();
    let n = ordered.len();
    for (pos, lane) in ordered.iter().enumerate() {
        if let Some(fid) = lane.family_id {
            if pos > 0 && ordered[pos - 1].family_id == Some(fid) {
                continue;
            }
            let fam_count = ordered[pos..].iter()
                .take_while(|l| l.family_id == Some(fid))
                .count();
            let ox = pos + 1;
            let (fn_, fm) = {
                let all_p = lane.all_producers();
                (all_p.len().max(1), fam_count)
            };
            if let Some(tpl) = templates.get(&(fn_ as u32, fm as u32)) {
                for &(dx, _) in tpl.input_tiles {
                    let landing_x = (ox as i32) + dx + 1;
                    for rpos in (pos + fam_count)..n {
                        let rx = (rpos + 1) as i32;
                        if rx == landing_x {
                            score += 100;
                        }
                    }
                }
            }
        }
    }

    score
}

fn family_contiguous(ordered: &[BusLane]) -> bool {
    let mut seen_ranges: FxHashMap<usize, (usize, usize)> = FxHashMap::default();
    for (i, ln) in ordered.iter().enumerate() {
        if let Some(fid) = ln.family_id {
            let (lo, hi) = seen_ranges.get(&fid).copied().unwrap_or((i, i));
            seen_ranges.insert(fid, (lo.min(i), hi.max(i)));
        }
    }
    let mut counts: FxHashMap<usize, usize> = FxHashMap::default();
    for ln in ordered {
        if let Some(fid) = ln.family_id {
            *counts.entry(fid).or_insert(0) += 1;
        }
    }
    seen_ranges.iter().all(|(fid, (lo, hi))| hi - lo + 1 == counts[fid])
}

/// Find best permutation of solid lanes that respects family contiguity.
fn find_best_permutation(solid: &[BusLane], row_spans: &[RowSpan]) -> Vec<BusLane> {
    if solid.is_empty() {
        return Vec::new();
    }

    let n = solid.len();
    let mut indices: Vec<usize> = (0..n).collect();
    let mut best_order: Vec<usize> = indices.clone();
    let mut best_score = score_lane_ordering(
        &indices.iter().map(|&i| solid[i].clone()).collect::<Vec<_>>(),
        row_spans,
    );

    // Heap's algorithm
    let mut c = vec![0; n];
    let mut i = 0;
    while i < n {
        if c[i] < i {
            if i % 2 == 0 {
                indices.swap(0, i);
            } else {
                indices.swap(c[i], i);
            }
            let ordered: Vec<BusLane> = indices.iter().map(|&idx| solid[idx].clone()).collect();
            if family_contiguous(&ordered) {
                let score = score_lane_ordering(&ordered, row_spans);
                if score < best_score {
                    best_score = score;
                    best_order = indices.clone();
                }
            }
            c[i] += 1;
            i = 0;
        } else {
            c[i] = 0;
            i += 1;
        }
    }

    best_order.iter().map(|&i| solid[i].clone()).collect()
}

/// Hill-climbing lane order optimizer for larger sets (>7 lanes).
fn hill_climb_lane_order(solid: &[BusLane], row_spans: &[RowSpan]) -> Vec<BusLane> {
    let mut order = solid.to_vec();
    order.sort_by_key(|ln| {
        let fid = ln.family_id.unwrap_or(usize::MAX) as i32;
        let y = ln.tap_off_ys.iter().min().copied().map(|y| -y).unwrap_or(9999);
        (fid, y)
    });

    let n = order.len();
    let mut best_score = score_lane_ordering(&order, row_spans);

    loop {
        let mut improved = false;
        'outer: for i in 0..n {
            for j in (i + 1)..n {
                order.swap(i, j);
                if family_contiguous(&order) {
                    let score = score_lane_ordering(&order, row_spans);
                    if score < best_score {
                        best_score = score;
                        improved = true;
                        continue 'outer;
                    }
                }
                order.swap(i, j);
            }
        }
        if !improved { break; }
    }

    order
}

/// Optimize lane left-to-right ordering to minimize underground crossings.
pub fn optimize_lane_order(lanes: &[BusLane], row_spans: &[RowSpan]) -> Vec<BusLane> {
    if lanes.len() <= 1 {
        return lanes.to_vec();
    }

    let solid: Vec<BusLane> = lanes.iter().filter(|ln| !ln.is_fluid).cloned().collect();
    let fluid: Vec<BusLane> = lanes.iter().filter(|ln| ln.is_fluid).cloned().collect();

    // Exact search for tiny sets (≤7 lanes: 7! = 5040 permutations).
    // Hill-climbing for larger sets.
    let best_solid = if solid.len() <= 7 {
        find_best_permutation(&solid, row_spans)
    } else {
        hill_climb_lane_order(&solid, row_spans)
    };

    let mut result = best_solid;
    result.extend(fluid);

    let crossing_score = score_lane_ordering(&result, row_spans);
    crate::trace::emit(crate::trace::TraceEvent::LaneOrderOptimized {
        ordering: result.iter().map(|ln| ln.item.clone()).collect(),
        crossing_score,
    });

    result
}

// ---------------------------------------------------------------------------
// SAT crossing-zone extraction
// ---------------------------------------------------------------------------

/// Extract crossing zones from the lane plan and solve them via SAT.
///
/// Returns (solved_crossings, tile_set) where tile_set contains all (x,y)
/// positions owned by crossing zone entities. SAT-rendered trunk bridges are
/// applied via `crossing_tiles` (trunks skip those rows); the tap-off A* runs
/// the full row width and naturally fills the `forced_empty` middle tiles.
pub(crate) fn extract_and_solve_crossings(
    lanes: &[BusLane],
    max_belt_tier: Option<&str>,
) -> (Vec<SolvedCrossing>, CrossingTileSet) {
    let effective_belt = belt_entity_for_rate(f64::MAX, max_belt_tier);
    let max_reach = crate::common::ug_max_reach(effective_belt);

    let mut zone_specs: Vec<(String, i32, i32, Vec<(i32, String)>)> = Vec::new();

    for tapping_lane in lanes {
        if tapping_lane.is_fluid {
            continue;
        }
        for &tap_y in &tapping_lane.tap_off_ys {
            let mut crossed_trunks: Vec<(i32, String)> = Vec::new();

            for trunk_lane in lanes {
                if trunk_lane.is_fluid
                    || std::ptr::eq(trunk_lane, tapping_lane)
                    || trunk_lane.x <= tapping_lane.x
                {
                    continue;
                }
                let trunk_extends = trunk_lane.tap_off_ys.iter().any(|&y| y >= tap_y);
                let trunk_skips_tap_y = trunk_lane.tap_off_ys.contains(&tap_y);
                let trunk_above_source = tap_y < trunk_lane.source_y;
                let trunk_in_balancer = trunk_lane.family_balancer_range
                    .is_some_and(|(bs, be)| tap_y >= bs && tap_y <= be);
                let ug_output_on_tapoff = trunk_lane.tap_off_ys.contains(&(tap_y + 1));
                if !trunk_extends || trunk_skips_tap_y || trunk_above_source
                    || trunk_in_balancer || ug_output_on_tapoff
                {
                    continue;
                }
                let all_clear = lanes.iter()
                    .filter(|mid| {
                        !mid.is_fluid
                            && mid.x > tapping_lane.x
                            && mid.x < trunk_lane.x
                    })
                    .all(|mid| {
                        mid.tap_off_ys.contains(&tap_y)
                            || mid.tap_off_ys.iter().all(|&y| y < tap_y)
                    });
                if all_clear {
                    crossed_trunks.push((trunk_lane.x, trunk_lane.item.clone()));
                }
            }

            if !crossed_trunks.is_empty() {
                crossed_trunks.sort_by_key(|(x, _)| *x);
                zone_specs.push((
                    tapping_lane.item.clone(),
                    tapping_lane.x,
                    tap_y,
                    crossed_trunks,
                ));
            }
        }
    }

    let mut solved = Vec::new();
    let mut entity_tiles = FxHashSet::default();
    let mut all_tiles = FxHashSet::default();

    for (tap_item, tap_x, tap_y, crossed) in &zone_specs {
        let x_min = crossed.first().unwrap().0;
        let x_max = crossed.last().unwrap().0;
        let zone_width = (x_max - x_min + 1) as u32;
        let zone_height: u32 = 3;
        let zone_x = x_min;
        let zone_y = tap_y - 1;

        let mut boundaries = Vec::new();
        let mut forced_empty = Vec::new();
        for (trunk_x, trunk_item) in crossed {
            boundaries.push(ZoneBoundary {
                x: *trunk_x,
                y: zone_y,
                direction: EntityDirection::South,
                item: trunk_item.clone(),
                is_input: true,
            });
            boundaries.push(ZoneBoundary {
                x: *trunk_x,
                y: zone_y + zone_height as i32 - 1,
                direction: EntityDirection::South,
                item: trunk_item.clone(),
                is_input: false,
            });
            forced_empty.push((*trunk_x, *tap_y));
        }

        let zone = CrossingZone {
            x: zone_x,
            y: zone_y,
            width: zone_width,
            height: zone_height,
            boundaries,
            forced_empty,
        };

        if let Some(solution) = sat::solve_crossing_zone(&zone, max_reach, effective_belt) {
            for e in &solution.entities {
                entity_tiles.insert((e.x, e.y));
                all_tiles.insert((e.x, e.y));
            }
            for &(fx, fy) in &zone.forced_empty {
                all_tiles.insert((fx, fy));
            }
            solved.push(SolvedCrossing { zone, solution });
        } else {
            crate::trace::emit(crate::trace::TraceEvent::CrossingZoneSkipped {
                tap_item: tap_item.clone(),
                tap_x: *tap_x,
                tap_y: *tap_y,
                reason: "sat-unsolved".into(),
            });
        }
    }

    let tile_sets = CrossingTileSet::from_parts(all_tiles, entity_tiles);
    (solved, tile_sets)
}

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
                yields.push(Yield { y });
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
                yields.push(Yield { y: tap_y });
                // Non-last splitter tap-offs also occupy (other.x+1, tap_y-1)
                // (splitter right half) and (other.x+1, tap_y) (belt East).
                // If this trunk IS that adjacent column, skip tap_y-1 too.
                let is_non_last = other.tap_off_ys.len() > 1
                    && Some(tap_y) != other_last_tap;
                if is_non_last && lane.x == other.x + 1
                    && trunk_start_y < tap_y - 1 && tap_y - 1 < trunk_end_y
                    && !own_tap_set.contains(&tap_y)
                {
                    yields.push(Yield { y: tap_y - 1 });
                }
            }
        }
    }

    yields
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A trunk yield: the trunk goes underground at y-position `y`,
/// re-emerging at `y + 1`.
#[derive(Debug, Clone)]
pub struct Yield {
    pub y: i32,
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
fn resolve_bridge_conflicts_for_lane(
    lane: &BusLane,
    foreign_yields: &[Yield],
) -> Vec<DroppedBridge> {
    let ys: FxHashSet<i32> = foreign_yields.iter().map(|y| y.y).collect();
    let merged = merge_consecutive(&ys);
    let own_tap_set: FxHashSet<i32> = lane.tap_off_ys.iter().copied().collect();

    let mut dropped: Vec<DroppedBridge> = Vec::new();

    for (range_start, range_end) in merged {
        if own_tap_set.contains(&(range_end + 1)) {
            dropped.push(DroppedBridge {
                trunk_item: lane.item.clone(),
                trunk_x: lane.x,
                range: (range_start, range_end),
            });
        }
    }

    dropped
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
) -> Result<(), PlanError> {
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
        let dropped = resolve_bridge_conflicts_for_lane(lane, &foreign_yields);

        all_dropped.extend(dropped);
    }

    if !all_dropped.is_empty() {
        return Err(PlanError::DroppedBridges { dropped: all_dropped });
    }

    Ok(())
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
