//! Ghost A* bus router — Phases 2+3 of the ghost-cluster routing rewrite.
//!
//! Algorithm overview:
//! 1. Build a hard-obstacle set from row_entities (machine footprints, poles, etc.)
//!    and fluid lane tile reservations.
//! 2. Stamp splitters and balancer entities.
//! 3. Pre-stamp trunk belts (South-facing) directly as Permanent entities.
//!    Synthetic column paths are recorded in `trunk_synth_paths` and injected
//!    into `routed_paths` after A* so the junction solver can classify
//!    trunk/tap crossings correctly.
//! 4. Route each connecting-belt spec (tap-offs, returns, feeders) via
//!    `ghost_astar`. Trunk tiles are passable so A* ghosts through them and
//!    records crossing tiles for the junction resolver.
//! 5. Negotiate lane conflicts iteratively; adopt best routing.
//! 6. Resolve crossings: perpendicular template first, SAT fallback.
//! 7. Merge output rows via the existing `merge_output_rows` helper.
//!
//! Returns a `GhostRouteResult` containing all placed entities, ghost crossing
//! tiles, cluster info, and layout dimensions.
//!
//! See `docs/archive/rfp-ghost-cluster-routing.md` for the full design.

use rustc_hash::{FxHashMap, FxHashSet};

use crate::astar::ghost_astar;
use crate::bus::balancer::{splitter_for_belt, stamp_family_balancer};
use crate::bus::lane_planner::{BusLane, LaneFamily, MACHINE_ENTITIES};
use crate::bus::output_merger::merge_output_rows;
use crate::bus::trunk_renderer::{is_intermediate, render_path, trunk_segments};
use crate::bus::junction::{BeltTier, Rect};
use crate::bus::junction_sat_strategy::SatStrategy;
use crate::bus::junction_solver::{
    self, JunctionSolution, JunctionStrategy, JunctionStrategyContext,
};
use crate::bus::placer::RowSpan;
use crate::common::{belt_entity_for_rate, machine_size, machine_tiles, ug_max_reach};
use crate::models::{EntityDirection, LayoutRegion, PlacedEntity, SolverResult};
// sat.rs is retained in the tree as a standalone library; route_bus_ghost
// no longer uses it after the per-tile "unresolved" rewrite. The junction
// solver (docs/archive/rfp-junction-solver.md) may reintroduce it as a T4
// fallback strategy, in which case reimport at that point.
use crate::trace;

const TURN_PENALTY: u32 = 8;

/// Output of the ghost router.
pub struct GhostRouteResult {
    pub entities: Vec<PlacedEntity>,
    /// All tiles where two or more routed paths overlap.
    pub ghost_crossing_tiles: FxHashSet<(i32, i32)>,
    /// Number of union-find clusters among the ghost crossings.
    pub cluster_count: usize,
    /// Tile count of the largest cluster.
    pub max_cluster_tiles: usize,
    /// Specs that could not be routed (no path found).
    pub unroutable_specs: Vec<String>,
    /// Total layout height (y extent).
    pub max_y: i32,
    /// Maximum x used by output mergers.
    pub merge_max_x: i32,
    /// Layout regions (empty for Phase 2; SAT fills these in Phase 3).
    pub regions: Vec<LayoutRegion>,
    /// Non-fatal warnings (direct/bare modes produce these for cases
    /// that would hard-error in the default pipeline).
    pub warnings: Vec<String>,
}

/// A spec for one connecting belt run.
struct BeltSpec {
    key: String,
    start: (i32, i32),
    goal: (i32, i32),
    item: String,
    belt_name: &'static str,
    /// Explicit exit direction for the final belt on this path. Set when the
    /// planner knows the spec's topology — producer-row orientation, trunk
    /// axis — at emission time. When `Some`, render_path uses this for
    /// single-tile paths and the last tile of multi-tile paths instead of
    /// inferring direction from start/goal coordinate comparisons (which is
    /// ambiguous for degenerate start == goal specs and length-1 blocked A*
    /// fallbacks). See plan file abundant-gliding-turing.md for context.
    exit_dir: Option<EntityDirection>,
}

/// Route all bus belts using the ghost A* approach.
#[allow(clippy::too_many_arguments)]
pub fn route_bus_ghost(
    lanes: &[BusLane],
    row_spans: &[RowSpan],
    total_height: i32,
    bw: i32,
    max_belt_tier: Option<&str>,
    solver_result: &SolverResult,
    families: &[LaneFamily],
    row_entities: &[PlacedEntity],
) -> Result<GhostRouteResult, String> {
    let mut entities: Vec<PlacedEntity> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut max_y = total_height;
    let mut merge_max_x = 0i32;

    let width = (bw + 200).max(200);
    let height = (total_height + 50).max(200);

    // -------------------------------------------------------------------------
    // Step 1: Build hard obstacle set from row_entities
    // -------------------------------------------------------------------------
    let mut hard: FxHashSet<(i32, i32)> = FxHashSet::default();
    let mut existing_belts: FxHashSet<(i32, i32)> = FxHashSet::default();
    // Tracks belt positions that existed before ghost routing (row templates,
    // trunks, splitters, balancers).  Crossings against these are not
    // ghost-vs-ghost conflicts and are filtered out of the crossing set.
    let mut pre_ghost_belts: FxHashSet<(i32, i32)> = FxHashSet::default();

    for e in row_entities {
        if is_belt_like(&e.name) {
            existing_belts.insert((e.x, e.y));
            pre_ghost_belts.insert((e.x, e.y));
        } else if MACHINE_ENTITIES.contains(&e.name.as_str()) {
            let sz = machine_size(&e.name);
            for t in machine_tiles(e.x, e.y, sz) {
                hard.insert(t);
            }
        } else {
            hard.insert((e.x, e.y));
        }
    }

    // Reserve fluid lane tiles as hard obstacles (same logic as pole placer
    // in layout.rs: fluid lanes reserve the column from source_y to last tap_y).
    for lane in lanes {
        if lane.is_fluid {
            let end_y = lane.tap_off_ys.iter().copied().max().unwrap_or(lane.source_y);
            for y in lane.source_y..=end_y {
                hard.insert((lane.x, y));
            }
        }
    }

    // -------------------------------------------------------------------------
    // Step 2: Place splitter stamps as hard obstacles.
    // -------------------------------------------------------------------------
    for lane in lanes {
        if lane.is_fluid {
            continue;
        }

        let x = lane.x;
        let belt_name = belt_entity_for_rate(lane.rate * 2.0, max_belt_tier);
        let trunk_seg_id = Some(format!("trunk:{}", lane.item));
        let last_tap_y = lane.tap_off_ys.iter().copied().max();

        // Place splitter stamps for non-last tap-offs
        if lane.tap_off_ys.len() > 1 {
            let splitter_name = splitter_for_belt(belt_name);
            let tapoff_seg_id = Some(format!("tapoff:{}", lane.item));
            for &tap_y in &lane.tap_off_ys {
                if Some(tap_y) == last_tap_y {
                    continue;
                }
                // Splitter at (x, tap_y-1), East belt at (x+1, tap_y-1)
                // Trunk-continue belt at (x, tap_y)
                entities.push(PlacedEntity {
                    name: splitter_name.to_string(),
                    x,
                    y: tap_y - 1,
                    direction: EntityDirection::South,
                    carries: Some(lane.item.clone()),
                    segment_id: tapoff_seg_id.clone(),
                    rate: Some(lane.rate),
                    ..Default::default()
                });
                entities.push(PlacedEntity {
                    name: belt_name.to_string(),
                    x,
                    y: tap_y,
                    direction: EntityDirection::South,
                    carries: Some(lane.item.clone()),
                    segment_id: trunk_seg_id.clone(),
                    rate: Some(lane.rate),
                    ..Default::default()
                });
                // The splitter occupies 2 tiles (x, tap_y-1) and (x+1, tap_y-1)
                hard.insert((x, tap_y - 1));
                hard.insert((x + 1, tap_y - 1));
                hard.insert((x, tap_y));
                existing_belts.insert((x, tap_y - 1));
                existing_belts.insert((x, tap_y));
                pre_ghost_belts.insert((x, tap_y - 1));
                pre_ghost_belts.insert((x, tap_y));
            }
        }
    }

    // -------------------------------------------------------------------------
    // Step 3: Stamp balancer blocks as hard obstacles
    // -------------------------------------------------------------------------
    for fam in families {
        let balancer_ents = stamp_family_balancer(fam, max_belt_tier)
            .map_err(|e| format!("ghost router: balancer stamp failed for {:?}: {}", fam.shape, e))?;
        crate::trace::emit(crate::trace::TraceEvent::BalancerStamped {
            item: fam.item.clone(),
            shape: fam.shape,
            y_start: fam.balancer_y_start,
            y_end: fam.balancer_y_end,
            template_found: !balancer_ents.is_empty(),
        });
        for ent in &balancer_ents {
            if is_belt_like(&ent.name) {
                hard.insert((ent.x, ent.y));
                existing_belts.insert((ent.x, ent.y));
                pre_ghost_belts.insert((ent.x, ent.y));
            } else {
                hard.insert((ent.x, ent.y));
            }
        }
        entities.extend(balancer_ents);
    }

    // -------------------------------------------------------------------------
    // Step 3.5: Stamp trunk belts directly (no A*).
    //
    // Each trunk segment is stamped as a South-facing Permanent entity before
    // Occupancy construction and before A* runs. This replaces the old approach
    // of routing trunks through ghost_astar. Benefits:
    //   - 1-tile trunk stubs (start == goal) are unconstructable as degenerate
    //     A* paths; a single South-facing entity is stamped directly instead.
    //   - Trunk direction is always exactly South — no A* bending.
    //   - Trunk entities land in `permanent_inits` before Occupancy::new, so
    //     the Permanent claim is present during all downstream steps.
    //   - Trunk keys are absent from `routed_paths`, so the junction solver
    //     only sees crossing specs that are genuinely ghost-routed.
    //
    // Trunks stay in `existing_belts` (transparent to A*) so tap-offs and
    // returns can still route in a straight line through trunk columns. They are
    // NOT added to `pre_ghost_belts` — instead their tile→item pairs are
    // collected into `trunk_tile_items` and injected into `ghost_item_at` after
    // the materialisation reset. This preserves the OLD crossing-detection
    // mechanism: the `ghost_item_at` filter drops tap entities that land on a
    // trunk tile, and the `all_ghost_crossings` check still fires for different-
    // item overlaps so the junction solver can bridge them.
    // -------------------------------------------------------------------------
    let mut trunk_tile_items: FxHashMap<(i32, i32), String> = FxHashMap::default();
    // Synthetic column paths for each trunk lane, keyed by "trunk:{item}:{x}".
    // Keyed per-column (not just per-item) because multi-lane items like a
    // split copper-cable trunk have multiple vertical columns — merging them
    // into one path produces bogus horizontal dx between (x,y) and (x+1,y)
    // tiles at the same y, which mis-classifies the trunk axis.
    // Injected into `routed_paths` after routing so classify_crossing and
    // the junction solver can see trunk specs at crossing tiles — the same
    // way they saw them in the old code when trunks were BeltSpecs routed
    // through A*.
    let mut trunk_synth_paths: FxHashMap<String, Vec<(i32, i32)>> = FxHashMap::default();
    for lane in lanes {
        if lane.is_fluid {
            continue;
        }
        let x = lane.x;
        let belt_name = belt_entity_for_rate(lane.rate * 2.0, max_belt_tier);
        let trunk_seg_id = Some(format!("trunk:{}", lane.item));
        let last_tap_y = lane.tap_off_ys.iter().copied().max();

        let mut producer_ys: FxHashSet<i32> = FxHashSet::default();
        if let Some(pr) = lane.producer_row {
            if pr < row_spans.len() {
                producer_ys.insert(row_spans[pr].output_belt_y);
            }
        }
        for &pri in &lane.extra_producer_rows {
            if pri < row_spans.len() {
                producer_ys.insert(row_spans[pri].output_belt_y);
            }
        }

        // skip_ys mirrors the old trunk-BeltSpec logic exactly: skip all
        // tap_off_ys (non-last handled by step 2 splitter/continue-belt
        // stamps; last_tap_y left for the tap spec to stamp an East-facing
        // belt) and all balancer rows (step 3 already stamped those).
        let mut skip_ys: FxHashSet<i32> = lane.tap_off_ys.iter().copied().collect();
        for &ty in &lane.tap_off_ys {
            if lane.tap_off_ys.len() > 1 && Some(ty) != last_tap_y {
                skip_ys.insert(ty - 1);
            }
        }
        if let Some(by) = lane.balancer_y {
            skip_ys.insert(by);
        }
        if let Some((by_start, by_end)) = lane.family_balancer_range {
            for y in by_start..=by_end {
                skip_ys.insert(y);
            }
        }

        let mut all_ys: Vec<i32> = lane.tap_off_ys.clone();
        all_ys.extend(producer_ys.iter().copied());
        let start_y = lane.source_y;
        let end_y = all_ys.iter().copied().max().unwrap_or(start_y);
        let end_y = if let Some(by) = lane.balancer_y {
            end_y.max(by + 1)
        } else {
            end_y
        };

        for (seg_start, seg_end) in trunk_segments(start_y, end_y, &skip_ys) {
            for y in seg_start..=seg_end {
                let tile = (x, y);
                if hard.contains(&tile) || existing_belts.contains(&tile) {
                    continue;
                }
                entities.push(PlacedEntity {
                    name: belt_name.to_string(),
                    x,
                    y,
                    direction: EntityDirection::South,
                    carries: Some(lane.item.clone()),
                    segment_id: trunk_seg_id.clone(),
                    ..Default::default()
                });
                // Passable to A* (existing_belts). Not in pre_ghost_belts —
                // the ghost_item_at mechanism handles dropping conflicting
                // tap/ret entities at trunk tiles, and preserves crossing
                // detection so the junction solver can bridge them.
                existing_belts.insert(tile);
                trunk_tile_items.insert(tile, lane.item.clone());
                trunk_synth_paths
                    .entry(format!("trunk:{}:{}", lane.item, x))
                    .or_default()
                    .push(tile);
            }
        }
    }
    // Sort each synth path so tiles are ordered top-to-bottom (ascending y).
    for path in trunk_synth_paths.values_mut() {
        path.sort_by_key(|&(_, y)| y);
    }

    // -------------------------------------------------------------------------
    // Occupancy refactor (Steps 2-3.5): construct the parallel `Occupancy` from
    // the inputs to steps 1-3 of this function. Step 3 of the rollout uses it
    // to mirror materialisation writes; Step 4+ will switch the template and
    // SAT phases over to it as the source of obstacle truth. See
    // `docs/archive/rfp-ghost-occupancy-refactor.md`.
    // -------------------------------------------------------------------------
    // Row template entities split by permeability: belts are
    // `RowEntity` (boundary ports may land on them); machines,
    // inserters, poles, and pipes are `Permanent` (real obstacles).
    let (row_belts, row_non_belts): (Vec<PlacedEntity>, Vec<PlacedEntity>) = row_entities
        .iter()
        .cloned()
        .partition(|e| is_belt_like(&e.name));
    let mut permanent_inits = row_non_belts;
    permanent_inits.extend(entities.iter().cloned());
    let mut occupancy = crate::bus::ghost_occupancy::Occupancy::new(
        hard.clone(),
        row_belts,
        permanent_inits,
    );

    #[cfg(debug_assertions)]
    {
        for &tile in &hard {
            debug_assert!(
                occupancy.is_claimed(tile),
                "occupancy refactor: hard tile {:?} not claimed in parallel Occupancy",
                tile,
            );
        }
        for &tile in &pre_ghost_belts {
            // Row template belts now sit in `Occupancy` as
            // `RowEntity` claims (not `Permanent`), to mirror
            // today's `pre_existing_positions` semantics. They
            // should still be `is_claimed`.
            debug_assert!(
                occupancy.is_claimed(tile),
                "occupancy refactor: pre_ghost_belts tile {:?} not claimed in parallel Occupancy",
                tile,
            );
        }
    }

    // -------------------------------------------------------------------------
    // Step 4: Build connecting-belt spec list
    // -------------------------------------------------------------------------
    let mut specs: Vec<BeltSpec> = Vec::new();

    // Helper: compute the row-exit origin tile for a ret/feeder spec based on
    // the producer row's orientation. For westward rows (intermediate
    // producers feeding a trunk column to their left) items exit at
    // (output_belt_x_min - 1, out_y) flowing West. For eastward rows (final
    // output rows) items exit at (output_belt_x_max + 1, out_y) flowing East.
    // The ret/feeder spec always walks back horizontally toward the trunk,
    // so the *direction* of the bend at the origin tile is always "toward the
    // trunk column": West for westward rows (trunk is west of the exit), West
    // for eastward rows too (trunk is west of the row start at x = bus_width,
    // and the ret walks back west to it).
    let row_exit_origin = |row: &RowSpan| -> (i32, i32) {
        let out_y = row.output_belt_y;
        if row.output_east {
            (row.output_belt_x_max + 1, out_y)
        } else {
            (row.output_belt_x_min - 1, out_y)
        }
    };

    for lane in lanes {
        if lane.is_fluid {
            continue;
        }
        let x = lane.x;
        let has_consumers = !lane.consumer_rows.is_empty();
        let has_producers = lane.producer_row.is_some() || !lane.extra_producer_rows.is_empty();
        let last_tap_y = lane.tap_off_ys.iter().copied().max();
        let horiz_belt = belt_entity_for_rate(lane.rate * 2.0, max_belt_tier);

        // Tap-off specs
        if has_consumers {
            for &tap_y in &lane.tap_off_ys {
                let is_last = Some(tap_y) == last_tap_y;
                // Non-last: start from (x+1, tap_y) (splitter right output)
                // Last: start from (x, tap_y) (trunk terminates here)
                let start_x = if is_last { x } else { x + 1 };
                // Goal: right edge of the bus
                let goal_x = bw - 1;
                let tap_key = format!("tap:{}:{}:{}", lane.item, x, tap_y);
                specs.push(BeltSpec {
                    key: tap_key,
                    start: (start_x, tap_y),
                    goal: (goal_x, tap_y),
                    item: lane.item.clone(),
                    belt_name: horiz_belt,
                    exit_dir: Some(EntityDirection::East),
                });
            }
        }

        // Return specs for intermediate lanes (no family balancer).
        // Producers feed the trunk from the west side of the bus; start tile
        // is orientation-aware (westward rows exit left of the row, eastward
        // rows exit right of the row). Items always walk West back to the
        // trunk, so exit_dir is West — this makes the bend belt that lands
        // at (x+1, out_y) face West regardless of path length, so it sideloads
        // correctly into the South-facing trunk at (x, out_y).
        if is_intermediate(lane) && lane.family_balancer_range.is_none() {
            let mut all_producers = Vec::new();
            if let Some(pr) = lane.producer_row {
                all_producers.push(pr);
            }
            all_producers.extend(&lane.extra_producer_rows);

            for &pri in &all_producers {
                if pri >= row_spans.len() {
                    continue;
                }
                let row = &row_spans[pri];
                let (start_x, out_y) = row_exit_origin(row);
                let goal_x = x + 1;
                // Skip the spec entirely when the exit lands west of (or at)
                // the goal — the row's own exit belt already covers it and
                // no additional ret belt is needed. This can happen when a
                // westward row's output_belt_x_min is adjacent to the trunk.
                if start_x < goal_x {
                    continue;
                }
                let ret_key = format!("ret:{}:{}:{}", lane.item, x, out_y);
                specs.push(BeltSpec {
                    key: ret_key,
                    start: (start_x, out_y),
                    goal: (goal_x, out_y),
                    item: lane.item.clone(),
                    belt_name: horiz_belt,
                    exit_dir: Some(EntityDirection::West),
                });
            }
        }

        // Collector lanes (producers only, no consumers): ret specs
        if has_producers && !has_consumers && lane.family_balancer_range.is_none() {
            let mut all_producers = Vec::new();
            if let Some(pr) = lane.producer_row {
                all_producers.push(pr);
            }
            all_producers.extend(&lane.extra_producer_rows);

            for &pri in &all_producers {
                if pri >= row_spans.len() {
                    continue;
                }
                let row = &row_spans[pri];
                let (start_x, out_y) = row_exit_origin(row);
                let goal_x = x + 1;
                if start_x < goal_x {
                    continue;
                }
                let ret_key = format!("ret:{}:{}:{}", lane.item, x, out_y);
                specs.push(BeltSpec {
                    key: ret_key,
                    start: (start_x, out_y),
                    goal: (goal_x, out_y),
                    item: lane.item.clone(),
                    belt_name: horiz_belt,
                    exit_dir: Some(EntityDirection::West),
                });
            }
        }

        // Feeder specs for family balancer lanes
        // Feeder specs for family-balanced lanes — generate once per family
        // (when this lane is the leftmost of its family). Producers are stored
        // on `LaneFamily.producer_rows`, not on the lane itself.
        if let Some(family_id) = lane.family_id {
            if let Some(fam) = families.get(family_id) {
                let is_first_lane_in_family = fam
                    .lane_xs
                    .iter()
                    .copied()
                    .min()
                    .map(|min_x| min_x == lane.x)
                    .unwrap_or(false);
                if is_first_lane_in_family {
                    let templates = crate::bus::balancer_library::balancer_templates();
                    let (n, m) = (fam.shape.0 as u32, fam.shape.1 as u32);
                    if let Some(template) = templates.get(&(n, m)) {
                        let origin_x = fam.lane_xs.iter().copied().min().unwrap_or(x);
                        let origin_y = fam.balancer_y_start;
                        let mut inputs: Vec<(i32, i32)> = template.input_tiles.to_vec();
                        inputs.sort_by_key(|t| t.0);
                        let feeder_belt = belt_entity_for_rate(fam.total_rate, max_belt_tier);

                        for (i, &pri) in fam.producer_rows.iter().enumerate() {
                            if pri >= row_spans.len() {
                                continue;
                            }
                            let row = &row_spans[pri];
                            let (start_x, out_y) = row_exit_origin(row);
                            if let Some(&(input_x_rel, _input_y_rel)) = inputs.get(i) {
                                let input_x = origin_x + input_x_rel;
                                let input_y = origin_y;
                                let feeder_key =
                                    format!("feeder:{}:{}:{}", lane.item, input_x, out_y);
                                // Feeders walk from the row exit to a
                                // balancer input tile. exit_dir aims at the
                                // balancer — South when the input is below
                                // the row (typical), West otherwise.
                                let feeder_exit_dir = if input_y > out_y {
                                    EntityDirection::South
                                } else if input_y < out_y {
                                    EntityDirection::North
                                } else if input_x < start_x {
                                    EntityDirection::West
                                } else {
                                    EntityDirection::East
                                };
                                specs.push(BeltSpec {
                                    key: feeder_key,
                                    start: (start_x, out_y),
                                    goal: (input_x, input_y),
                                    item: lane.item.clone(),
                                    belt_name: feeder_belt,
                                    exit_dir: Some(feeder_exit_dir),
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    // -------------------------------------------------------------------------
    // Step 5: Route each spec with ghost_astar
    // -------------------------------------------------------------------------
    let count_turns = |path: &[(i32, i32)]| -> usize {
        let mut t = 0;
        for w in path.windows(3) {
            let d1 = (w[1].0 - w[0].0, w[1].1 - w[0].1);
            let d2 = (w[2].0 - w[1].0, w[2].1 - w[1].1);
            if d1 != d2 {
                t += 1;
            }
        }
        t
    };

    #[allow(clippy::needless_late_init)]
    let mut routed_paths: FxHashMap<String, Vec<(i32, i32)>>;
    let mut all_ghost_crossings: Vec<(i32, i32)> = Vec::new();
    #[allow(clippy::needless_late_init)]
    let unroutable_specs: Vec<String>;
    // Tracks which item each ghost-routed tile carries, so we can distinguish
    // same-item overlaps (not conflicts) from different-item overlaps (real).
    let mut ghost_item_at: FxHashMap<(i32, i32), String> = FxHashMap::default();

    // All remaining specs (taps, returns, feeders) — no ordering constraint
    // since trunks are now stamped as hard obstacles before A* runs.
    let ordered_specs: Vec<&BeltSpec> = specs.iter().collect();

    // -------------------------------------------------------------------------
    // Step 5: Negotiation loop — route all specs, measure same-axis conflicts,
    // bump per-tile per-axis cost, re-route. Converges when no improvement.
    // -------------------------------------------------------------------------
    // Snapshot pre-routing state so each iteration starts from the same place.
    let pre_routing_existing_belts = existing_belts.clone();

    const MAX_NEGOTIATION_ITERATIONS: u32 = 8;
    // History penalty: accumulated across iterations on tiles that had
    // same-axis conflicts in previous iterations.
    const HISTORY_PENALTY_K: u32 = 4;
    // Present penalty: bumped per spec INSIDE an iteration. Each spec's
    // routing pays a per-tile cost based on how many already-routed specs
    // in the current iteration used that tile in the same axis.
    const PRESENT_PENALTY_K: u32 = 6;
    const MAX_NO_IMPROVEMENT: u32 = 2;

    let mut history_cost_grid: FxHashMap<(i32, i32), (u32, u32)> = FxHashMap::default();
    let mut best_paths: FxHashMap<String, Vec<(i32, i32)>> = FxHashMap::default();
    let mut best_unroutable: Vec<String> = Vec::new();
    let mut best_same_axis: u32 = u32::MAX;
    let mut no_improvement_streak: u32 = 0;

    for iter in 0..MAX_NEGOTIATION_ITERATIONS {
        // Reset per-iteration routing state.
        let mut iter_routed: FxHashMap<String, Vec<(i32, i32)>> = FxHashMap::default();
        let mut iter_existing = pre_routing_existing_belts.clone();
        let mut iter_unroutable: Vec<String> = Vec::new();
        // Per-iteration cost grid = history (carried across iters) + present
        // (rebuilt each iter, bumped after each spec routes).
        let mut iter_cost_grid: FxHashMap<(i32, i32), (u32, u32)> =
            history_cost_grid.clone();

        for spec in ordered_specs.iter().copied() {
            match ghost_astar(
                spec.start,
                spec.goal,
                &hard,
                &iter_existing,
                width,
                height,
                TURN_PENALTY,
                &iter_cost_grid,
            ) {
                Some((path, _crossings)) => {
                    // Incrementally bump the present cost for tiles used by
                    // this spec, in the spec's axis at each tile. Subsequent
                    // specs in this iteration will pay the bumped cost.
                    if path.len() >= 2 {
                        let last_idx = path.len() - 1;
                        for (i, &tile) in path.iter().enumerate() {
                            let (dx, dy) = if i < last_idx {
                                (path[i + 1].0 - tile.0, path[i + 1].1 - tile.1)
                            } else {
                                (tile.0 - path[i - 1].0, tile.1 - path[i - 1].1)
                            };
                            let entry = iter_cost_grid.entry(tile).or_insert((0, 0));
                            if dx == 0 && dy != 0 {
                                entry.0 += PRESENT_PENALTY_K;
                            } else if dy == 0 && dx != 0 {
                                entry.1 += PRESENT_PENALTY_K;
                            }
                        }
                    }
                    for &tile in &path {
                        iter_existing.insert(tile);
                    }
                    iter_routed.insert(spec.key.clone(), path);
                }
                None => {
                    iter_unroutable.push(spec.key.clone());
                }
            }
        }

        // Compute axis counts for this iteration.
        let mut axis_counts: FxHashMap<(i32, i32), (u32, u32)> = FxHashMap::default();
        for path in iter_routed.values() {
            if path.len() < 2 {
                continue;
            }
            let last_idx = path.len() - 1;
            for (i, &tile) in path.iter().enumerate() {
                let (dx, dy) = if i < last_idx {
                    (path[i + 1].0 - tile.0, path[i + 1].1 - tile.1)
                } else {
                    (tile.0 - path[i - 1].0, tile.1 - path[i - 1].1)
                };
                let entry = axis_counts.entry(tile).or_insert((0, 0));
                if dx == 0 && dy != 0 {
                    entry.0 += 1;
                } else if dy == 0 && dx != 0 {
                    entry.1 += 1;
                }
            }
        }

        let mut iter_same_axis: u32 = 0;
        let mut iter_perp: u32 = 0;
        for &(v, h) in axis_counts.values() {
            if v >= 2 || h >= 2 {
                iter_same_axis += 1;
            }
            if v >= 1 && h >= 1 {
                iter_perp += 1;
            }
        }

        trace::emit(trace::TraceEvent::GhostNegotiationIteration {
            iter,
            same_axis_conflict_count: iter_same_axis,
            perpendicular_crossing_count: iter_perp,
            unroutable_count: iter_unroutable.len() as u32,
            cost_grid_size: history_cost_grid.len() as u32,
        });

        // Track the best routing across iterations.
        if iter_same_axis < best_same_axis {
            best_same_axis = iter_same_axis;
            best_paths = iter_routed;
            best_unroutable = iter_unroutable;
            no_improvement_streak = 0;
        } else {
            no_improvement_streak += 1;
        }

        // Stop conditions.
        if iter_same_axis == 0 {
            break;
        }
        if no_improvement_streak >= MAX_NO_IMPROVEMENT {
            break;
        }

        // Bump the HISTORY cost grid for tiles with same-axis conflicts.
        // Per-axis: only the over-crowded axis gets a higher penalty, leaving
        // the other axis free to keep using the tile. This carries across
        // iterations to discourage repeat conflicts at the same tiles.
        for (&tile, &(v, h)) in &axis_counts {
            if v >= 2 {
                let entry = history_cost_grid.entry(tile).or_insert((0, 0));
                entry.0 += HISTORY_PENALTY_K * (v - 1);
            }
            if h >= 2 {
                let entry = history_cost_grid.entry(tile).or_insert((0, 0));
                entry.1 += HISTORY_PENALTY_K * (h - 1);
            }
        }
    }

    // Adopt the best routing as the canonical one.
    routed_paths = best_paths;
    unroutable_specs = best_unroutable;
    // Inject synthetic trunk column paths so classify_crossing and the
    // junction solver find trunk specs at crossing tiles (same role they
    // played when trunks were BeltSpecs routed through A*).
    for (key, path) in &trunk_synth_paths {
        routed_paths.insert(key.clone(), path.clone());
    }

    // -------------------------------------------------------------------------
    // Materialize entities from the converged routed_paths.
    // Replays the per-spec materialization logic in spec order so that
    // existing_belts/ghost_item_at/all_ghost_crossings end up in the same
    // shape they had before the negotiation refactor.
    // -------------------------------------------------------------------------
    existing_belts = pre_routing_existing_belts;
    ghost_item_at.clear();
    // Pre-load trunk tile → item mappings. In the old code, trunk specs
    // materialised first and populated ghost_item_at. Now trunks are
    // pre-stamped and have no routed_paths entry, so we inject them here.
    // This ensures: (a) tap entities on trunk tiles are dropped by the
    // ghost_item_at filter, and (b) different-item crossings at trunk tiles
    // are still added to all_ghost_crossings for the junction solver.
    for (&tile, item) in &trunk_tile_items {
        ghost_item_at.insert(tile, item.clone());
    }

    for spec in ordered_specs.iter().copied() {
        if let Some(path) = routed_paths.get(&spec.key).cloned() {
            // Recompute crossings from the final state of existing_belts so
            // they reflect the converged routing order.
            let crossings: Vec<(i32, i32)> = path
                .iter()
                .copied()
                .filter(|t| existing_belts.contains(t))
                .collect();

            let turns = count_turns(&path);
            trace::emit(trace::TraceEvent::GhostSpecRouted {
                spec_key: spec.key.clone(),
                path_len: path.len(),
                crossings: crossings.len(),
                turns,
                tiles: path.clone(),
                crossing_tiles: crossings.clone(),
            });

            // Emit entities via render_path. When the planner set an explicit
            // exit_dir on the spec (orientation-aware ret/feeder, vertical
            // trunks), use it directly — this is the only reliable direction
            // source for length-1 paths (start == goal, or A* blocked fallback
            // to goal tile only). When unset, infer from start/goal coordinate
            // comparison (legacy behaviour for multi-tile horizontal/vertical
            // specs where the path deltas in render_path carry the direction).
            let direction_hint = if let Some(d) = spec.exit_dir {
                d
            } else if spec.start.1 != spec.goal.1 && spec.start.0 == spec.goal.0 {
                if spec.goal.1 > spec.start.1 {
                    EntityDirection::South
                } else {
                    EntityDirection::North
                }
            } else if spec.start.0 <= spec.goal.0 {
                EntityDirection::East
            } else {
                EntityDirection::West
            };
            let spec_seg_id = if spec.key.starts_with("trunk:") {
                Some(format!("trunk:{}", spec.item))
            } else {
                Some(format!("ghost:{}", spec.key))
            };
            let path_ents = render_path(
                &path,
                &spec.item,
                spec.belt_name,
                direction_hint,
                spec_seg_id,
                None,
            );
            // Materialise the path into entities + Occupancy.
            //
            // Claim kind: trunk specs become load-bearing vertical bus
            // belts (`Permanent` claims). All other specs (tap-offs,
            // returns, horizontals) get `GhostSurface` claims which
            // templates and SAT may replace. This mirrors the old
            // `pre_existing_positions` filter semantics — non-ghost
            // segment IDs (trunks) were kept as obstacles; ghost
            // segment IDs were skipped.
            //
            // Filter: drop path tiles that already hold a pre-existing
            // belt (from row templates / step 2-3 setup), already
            // carry another ghost item (first-spec-wins), or overlap a
            // hard obstacle. The hard-obstacle filter protects against
            // `ghost_astar:695` which allows goal tiles on hard
            // obstacles (and silently also start tiles — no check at
            // `astar.rs:658`). Dropping those entities prevents
            // entity-overlap validator errors on fluid-lane
            // reservations and machine anchors.
            let claim_kind = if spec.key.starts_with("trunk:") {
                crate::bus::ghost_occupancy::ClaimKindTag::Permanent
            } else {
                crate::bus::ghost_occupancy::ClaimKindTag::GhostSurface
            };
            let surviving_ents: Vec<PlacedEntity> = path_ents
                .into_iter()
                .filter(|e| {
                    !pre_ghost_belts.contains(&(e.x, e.y))
                        && !ghost_item_at.contains_key(&(e.x, e.y))
                        && !hard.contains(&(e.x, e.y))
                })
                .collect();
            for ent in &surviving_ents {
                occupancy
                    .place(ent.clone(), claim_kind)
                    .unwrap_or_else(|err| {
                        panic!(
                            "occupancy refactor: place failed for spec {} at ({},{}): {:?}",
                            spec.key, ent.x, ent.y, err
                        );
                    });
            }
            entities.extend(surviving_ents);

            for &tile in &path {
                existing_belts.insert(tile);
            }

            all_ghost_crossings.extend(crossings.into_iter().filter(|t| {
                if pre_ghost_belts.contains(t) {
                    return false;
                }
                match ghost_item_at.get(t) {
                    Some(existing_item) => *existing_item != spec.item,
                    None => false,
                }
            }));

            for &tile in &path {
                ghost_item_at.entry(tile).or_insert_with(|| spec.item.clone());
            }
        }
    }

    // Step 3 of the occupancy refactor: verify that every tile in the
    // post-materialisation `existing_belts` set has a corresponding claim
    // in Occupancy. Reverse direction does not hold — Occupancy includes
    // machines, poles, and fluid-lane reservations that `existing_belts`
    // does not.
    #[cfg(debug_assertions)]
    {
        for &tile in &existing_belts {
            debug_assert!(
                occupancy.is_claimed(tile),
                "occupancy refactor: existing_belts tile {:?} not claimed in Occupancy after materialisation",
                tile,
            );
        }
    }

    // Emit GhostSpecFailed events for specs that didn't route.
    for failed_key in &unroutable_specs {
        if let Some(spec) = ordered_specs.iter().find(|s| &s.key == failed_key) {
            trace::emit(trace::TraceEvent::GhostSpecFailed {
                spec_key: failed_key.clone(),
                from_x: spec.start.0,
                from_y: spec.start.1,
                to_x: spec.goal.0,
                to_y: spec.goal.1,
            });
        }
    }

    // -------------------------------------------------------------------------
    // Phase-1 instrumentation: per-tile axis occupancy
    // -------------------------------------------------------------------------
    // For each tile in routed_paths, determine the spec's outgoing axis
    // (vertical N/S or horizontal E/W). Last tile uses incoming direction.
    // Aggregate counts per tile and emit a summary trace event so the web
    // overlay can visualize same-axis conflicts vs perpendicular crossings.
    {
        use crate::trace::GhostAxisOccupancyTile;

        let mut axis_counts: FxHashMap<(i32, i32), (u32, u32)> = FxHashMap::default();
        for path in routed_paths.values() {
            if path.len() < 2 {
                continue;
            }
            let last_idx = path.len() - 1;
            for (i, &tile) in path.iter().enumerate() {
                let (dx, dy) = if i < last_idx {
                    (path[i + 1].0 - tile.0, path[i + 1].1 - tile.1)
                } else {
                    (tile.0 - path[i - 1].0, tile.1 - path[i - 1].1)
                };
                let entry = axis_counts.entry(tile).or_insert((0, 0));
                if dx == 0 && dy != 0 {
                    entry.0 += 1; // vertical
                } else if dy == 0 && dx != 0 {
                    entry.1 += 1; // horizontal
                }
            }
        }

        let mut tiles: Vec<GhostAxisOccupancyTile> = Vec::new();
        let mut same_axis_conflict_count: u32 = 0;
        let mut perpendicular_crossing_count: u32 = 0;
        for (&(x, y), &(v, h)) in &axis_counts {
            let same_axis = v >= 2 || h >= 2;
            let perp = v >= 1 && h >= 1;
            if same_axis {
                same_axis_conflict_count += 1;
            }
            if perp {
                perpendicular_crossing_count += 1;
            }
            if same_axis || perp {
                tiles.push(GhostAxisOccupancyTile {
                    x,
                    y,
                    vert_count: v,
                    horiz_count: h,
                });
            }
        }
        tiles.sort_by_key(|t| (t.y, t.x));

        trace::emit(trace::TraceEvent::GhostAxisOccupancy {
            tiles,
            same_axis_conflict_count,
            perpendicular_crossing_count,
        });
    }

    // -------------------------------------------------------------------------
    // Step 6: Resolve ghost crossings — templates first, SAT fallback
    // -------------------------------------------------------------------------
    let crossing_set: FxHashSet<(i32, i32)> = all_ghost_crossings.iter().copied().collect();

    // Step 6a-pre: Corridor template — detect runs of adjacent horizontal
    // crossings where one horizontal spec crosses N adjacent vertical trunks.
    // Emit a single long UG bridge for the horizontal instead of N separate
    // per-tile templates that would conflict.
    // Running count of templates emitted in step 6a (both corridor
    // runs and per-tile crossings). Added to `cluster_tile_counts.len()`
    // at the bottom to form `cluster_count`.
    let mut template_count: usize = 0;
    let mut template_regions: Vec<LayoutRegion> = Vec::new();
    let mut remaining_crossings: FxHashSet<(i32, i32)> = FxHashSet::default();
    let mut corridor_handled: FxHashSet<(i32, i32)> = FxHashSet::default();

    // Group crossings by (horizontal spec, y) and find runs of adjacent x.
    // For each horizontal spec, iterate its path tiles and collect crossing
    // tiles that form a consecutive-x run with a consistent horizontal direction.
    let max_reach = ug_max_reach(max_belt_tier.unwrap_or("transport-belt")) as i32;
    // Max span of UG pair (distance between UG-in and UG-out, inclusive).
    // For transport-belt max_reach=4, so UG can span 5 tiles covering 4 trunks.
    let max_corridor_span = max_reach + 1;

    for (key, path) in &routed_paths {
        if path.len() < 2 {
            continue;
        }
        // Determine overall direction from start to a consistent segment
        // (we only care about horizontal specs).
        let first_dx = path[1].0 - path[0].0;
        let first_dy = path[1].1 - path[0].1;
        if first_dy != 0 || first_dx == 0 {
            continue; // not a horizontal spec start
        }

        // Walk the path and find consecutive-x runs of crossings at same y.
        let mut i = 0;
        while i < path.len() {
            let (px, py) = path[i];
            if !crossing_set.contains(&(px, py)) {
                i += 1;
                continue;
            }
            // Collect a run
            let run_y = py;
            let run_start_x = px;
            let mut run_len = 1;
            let mut j = i + 1;
            let step_dx: i32 = if path.get(i + 1).map(|t| t.0 - px).unwrap_or(0) > 0 { 1 } else { -1 };
            while j < path.len() {
                let (nx, ny) = path[j];
                if ny != run_y {
                    break;
                }
                if nx != run_start_x + step_dx * run_len {
                    break;
                }
                if !crossing_set.contains(&(nx, ny)) {
                    break;
                }
                run_len += 1;
                j += 1;
            }

            if (2..max_corridor_span).contains(&run_len) {
                // Try corridor template.
                // For direction East (dx=1): UG-in at (run_start_x-1, y), UG-out at (run_start_x+run_len, y)
                // For direction West (dx=-1): UG-in at (run_start_x+1, y), UG-out at (run_start_x-run_len, y)
                let ug_dir = if step_dx > 0 { EntityDirection::East } else { EntityDirection::West };
                let ug_in_x = run_start_x - step_dx;
                let ug_out_x = run_start_x + step_dx * run_len;
                let ug_in = (ug_in_x, run_y);
                let ug_out = (ug_out_x, run_y);

                // Check: endpoints must not be obstacles, must not overlap a
                // pre-existing belt, must not be a turn point for any spec,
                // and must not have a perpendicular spec passing through them
                // (sideloads onto UG-input fail in Factorio).
                //
                // RowEntity tiles are considered "not permanent" by
                // `is_permanent()` because boundary ports may land on them,
                // but template UG endpoints *physically replace* whatever is
                // at the tile, so we must reject them here or `place` will
                // panic with AlreadyClaimed { RowEntity }. Exposed by
                // direct-routing mode where A* paths walk through row belts.
                let is_row_entity = |t| matches!(
                    occupancy.claim_at(t),
                    Some(crate::bus::ghost_occupancy::Claim::RowEntity { .. })
                );
                let endpoints_free = !occupancy.is_permanent(ug_in)
                    && !occupancy.is_permanent(ug_out)
                    && !is_row_entity(ug_in)
                    && !is_row_entity(ug_out)
                    && !any_spec_turns_at(ug_in, &routed_paths)
                    && !any_spec_turns_at(ug_out, &routed_paths)
                    && ug_endpoint_conflicts(ug_in, ug_dir, key, &routed_paths, &entities).is_none()
                    && ug_endpoint_conflicts(ug_out, ug_dir, key, &routed_paths, &entities).is_none();

                if endpoints_free {
                    // Find the horizontal spec that owns this run (for item/belt info).
                    let horiz_spec = specs.iter().find(|s| &s.key == key);
                    if let Some(hspec) = horiz_spec {
                        let ug_name = ug_for_belt(hspec.belt_name);
                        let seg = Some(format!("corridor:{}:{},{}", hspec.item, ug_in.0, run_y));
                        let mut ents = vec![PlacedEntity {
                            name: ug_name.to_string(),
                            x: ug_in.0,
                            y: ug_in.1,
                            direction: ug_dir,
                            io_type: Some("input".to_string()),
                            carries: Some(hspec.item.clone()),
                            segment_id: seg.clone(),
                            ..Default::default()
                        }];
                        ents.push(PlacedEntity {
                            name: ug_name.to_string(),
                            x: ug_out.0,
                            y: ug_out.1,
                            direction: ug_dir,
                            io_type: Some("output".to_string()),
                            carries: Some(hspec.item.clone()),
                            segment_id: seg.clone(),
                            ..Default::default()
                        });
                        let (zx, zy, zw, zh) = {
                            let xs = [ug_in.0, ug_out.0];
                            let min_x = *xs.iter().min().unwrap();
                            let max_x = *xs.iter().max().unwrap();
                            (min_x, run_y, (max_x - min_x + 1) as u32, 1u32)
                        };
                        let zone = ClusterZone { x: zx, y: zy, w: zw, h: zh };
                        trace::emit(trace::TraceEvent::GhostClusterSolved {
                            cluster_id: template_count,
                            zone_x: zone.x,
                            zone_y: zone.y,
                            zone_w: zone.w,
                            zone_h: zone.h,
                            boundary_count: 2,
                            variables: 0,
                            clauses: 0,
                            solve_time_us: 0,
                        });
                        template_regions.push(LayoutRegion {
                            kind: crate::models::RegionKind::CorridorTemplate,
                            x: zone.x,
                            y: zone.y,
                            width: zone.w as i32,
                            height: zone.h as i32,
                            ports: Vec::new(),
                        });
                        // Step 5a of the occupancy refactor: push the corridor
                        // UG bridge entities directly into `entities` and
                        // mirror to Occupancy as Template, instead of
                        // deferring via template_zones. Same pattern as
                        // Step 4 for per-tile templates. This is what makes
                        // the SAT phase's forced_empty (after Step 5b
                        // switches it to read from Occupancy) include
                        // corridor UG bridge tiles, fixing potential
                        // SAT-vs-corridor entity collisions that exist in
                        // main today.
                        for ent in &ents {
                            let tile = (ent.x, ent.y);
                            if occupancy.is_hard_obstacle(tile) {
                                continue;
                            }
                            if matches!(
                                occupancy.claim_at(tile),
                                Some(crate::bus::ghost_occupancy::Claim::Template { .. })
                            ) {
                                continue;
                            }
                            occupancy
                                .place(
                                    ent.clone(),
                                    crate::bus::ghost_occupancy::ClaimKindTag::Template,
                                )
                                .unwrap_or_else(|err| {
                                    panic!(
                                        "step 5a corridor UG place failed at ({},{}): {:?}",
                                        tile.0, tile.1, err
                                    );
                                });
                        }
                        entities.extend(ents);
                        template_count += 1;
                        // Mark all run tiles as corridor-handled and remove
                        // the horizontal spec's surface belts at those tiles
                        // (they're now underground via the UG bridge).
                        let mut run_tiles: FxHashSet<(i32, i32)> = FxHashSet::default();
                        for k in 0..run_len {
                            let x = run_start_x + step_dx * k;
                            run_tiles.insert((x, run_y));
                            corridor_handled.insert((x, run_y));
                        }
                        // Remove ONLY the bridged spec's surface belts at the
                        // run tiles. Other ghost-routed belts (e.g. another
                        // spec's perpendicular crossing) must stay. The
                        // bridged spec's segment_id is "ghost:{key}".
                        let bridged_seg = format!("ghost:{}", key);
                        entities.retain(|e| {
                            if !run_tiles.contains(&(e.x, e.y)) {
                                return true;
                            }
                            e.segment_id.as_deref() != Some(bridged_seg.as_str())
                        });

                        // Step 5a: mirror the bridged-spec ghost belt removal
                        // in Occupancy. Run tiles only have at most one
                        // ghost belt (the bridged spec's), so releasing all
                        // GhostSurface claims in the corridor zone is
                        // equivalent to the targeted entities.retain above.
                        // The UG endpoint tiles already hold Template claims
                        // from edit 1 above; release_ghost_surface_in only
                        // touches GhostSurface, so they're safe.
                        let release_rect = crate::bus::ghost_occupancy::Rect {
                            x: zone.x,
                            y: zone.y,
                            w: zone.w,
                            h: zone.h,
                        };
                        occupancy.release_ghost_surface_in(&release_rect);

                        // Re-add surface belts for perpendicular specs whose
                        // path was filtered out at materialization (because
                        // the bridged spec claimed the tile first). Only
                        // re-add if the tile is now empty — if a trunk or
                        // other entity is still there, leave it alone.
                        let occupied_after_removal: FxHashSet<(i32, i32)> =
                            entities.iter().map(|e| (e.x, e.y)).collect();
                        for &run_tile in &run_tiles {
                            if occupied_after_removal.contains(&run_tile) {
                                continue;
                            }
                            for (other_key, other_path) in &routed_paths {
                                if other_key == key {
                                    continue;
                                }
                                let pos = other_path.iter().position(|&t| t == run_tile);
                                let pi = match pos {
                                    Some(p) => p,
                                    None => continue,
                                };
                                let last_idx = other_path.len() - 1;
                                let (odx, ody) = if pi < last_idx {
                                    (other_path[pi + 1].0 - run_tile.0, other_path[pi + 1].1 - run_tile.1)
                                } else if pi > 0 {
                                    (run_tile.0 - other_path[pi - 1].0, run_tile.1 - other_path[pi - 1].1)
                                } else {
                                    continue;
                                };
                                let dir = step_direction(odx, ody);
                                let other_spec = specs.iter().find(|s| &s.key == other_key);
                                if let Some(os) = other_spec {
                                    // Use a non-ghost segment_id so the
                                    // post-template retain (which strips
                                    // "ghost:*" inside solved zones) keeps it.
                                    let perp_ent = PlacedEntity {
                                        name: os.belt_name.to_string(),
                                        x: run_tile.0,
                                        y: run_tile.1,
                                        direction: dir,
                                        carries: Some(os.item.clone()),
                                        segment_id: Some(format!(
                                            "corridor-perp:{}:{},{}",
                                            os.item, run_tile.0, run_tile.1
                                        )),
                                        ..Default::default()
                                    };
                                    // Step 5a: mirror to Occupancy as
                                    // Permanent so SAT's forced_empty (after
                                    // 5b) sees these surface belts. The
                                    // tile is free in Occupancy at this
                                    // point because edit 2 above released
                                    // any ghost-surface claim, and the
                                    // outer `occupied_after_removal` check
                                    // already filtered tiles still holding
                                    // a permanent entity.
                                    if !occupancy.is_hard_obstacle((perp_ent.x, perp_ent.y)) {
                                        occupancy
                                            .place(
                                                perp_ent.clone(),
                                                crate::bus::ghost_occupancy::ClaimKindTag::Permanent,
                                            )
                                            .unwrap_or_else(|err| {
                                                panic!(
                                                    "step 5a corridor-perp place failed at ({},{}): {:?}",
                                                    perp_ent.x, perp_ent.y, err
                                                );
                                            });
                                    }
                                    entities.push(perp_ent);
                                    break;
                                }
                            }
                        }
                        i = j;
                        continue;
                    }
                }
            }
            i += 1;
        }
    }

    // Step 6a: Per-tile crossing resolution via the junction-solver
    // growth loop. The loop seeds a `GrowingRegion` from each remaining
    // crossing tile, runs the registered strategies, and grows the
    // region's participating-spec frontier when none succeed. Today
    // the only strategy is `PerpendicularTemplateStrategy`, a wrapper
    // around the existing per-tile template — so behaviour matches the
    // old direct-call path for every crossing the old code solved.
    // Growth-aware strategies land on top of this scaffold.

    let mut spec_belt_tiers: FxHashMap<String, BeltTier> = specs
        .iter()
        .map(|s| {
            (
                s.key.clone(),
                BeltTier::from_name(s.belt_name).unwrap_or(BeltTier::Yellow),
            )
        })
        .collect();
    let mut spec_items: FxHashMap<String, String> = specs
        .iter()
        .map(|s| (s.key.clone(), s.item.clone()))
        .collect();
    let spec_exit_dirs: FxHashMap<String, EntityDirection> = specs
        .iter()
        .filter_map(|s| s.exit_dir.map(|d| (s.key.clone(), d)))
        .collect();
    // Extend with synthetic trunk entries so classify_crossing can resolve
    // item name and belt tier for trunk keys found in routed_paths. Keys
    // match the per-column format used when populating `trunk_synth_paths`
    // ("trunk:{item}:{x}") so multi-lane items register distinct keys per
    // column.
    for lane in lanes {
        if lane.is_fluid {
            continue;
        }
        let key = format!("trunk:{}:{}", lane.item, lane.x);
        spec_items.insert(key.clone(), lane.item.clone());
        spec_belt_tiers.insert(
            key,
            BeltTier::from_name(belt_entity_for_rate(lane.rate * 2.0, max_belt_tier))
                .unwrap_or(BeltTier::Yellow),
        );
    }
    // Build the obstacle set seen by junction strategies. The narrow
    // `hard` set only covers row-template machines and fluid lanes;
    // SAT (and any future strategy) needs the full picture so it
    // doesn't stamp belts on trunks, tap-off splitters, prior template
    // output, or row belts. Pulled from Occupancy at this point in
    // the pipeline — covers everything except `GhostSurface`, which
    // strategies are allowed to replace.
    let junction_hard: FxHashSet<(i32, i32)> = occupancy.snapshot_junction_obstacles();
    // Subset of `junction_hard` whose claims would panic if perp-template
    // stamped over them. `release_for_pertile_template` clears trunks and
    // tapoffs inside the footprint, and the post-place loop
    // (ghost_router.rs:1356) benignly skips `Template` and `RowEntity`
    // collisions. What remains, and what panics, is `Permanent` claims
    // whose segment id is NOT trunk/tapoff/row — balancer belts,
    // corridor-perp re-adds, merger chains — plus hard obstacles.
    // `PerpendicularTemplateStrategy` consults this narrower set so it
    // returns `None` instead of producing a panicking solution.
    let unreleasable_obstacles: FxHashSet<(i32, i32)> = entities
        .iter()
        .filter(|e| {
            let seg = e.segment_id.as_deref().unwrap_or("");
            !seg.starts_with("trunk:")
                && !seg.starts_with("tapoff:")
                && !seg.starts_with("ghost:")
                && !seg.starts_with("row:")
                && !seg.starts_with("junction:")
                && !seg.starts_with("corridor:")
                && !seg.starts_with("crossing:")
        })
        .map(|e| (e.x, e.y))
        .chain(hard.iter().copied())
        .collect();
    let perp_strategy = PerpendicularTemplateStrategy;
    let sat_strategy = SatStrategy;
    // Strategy order = priority: cheap templates run first, then
    // escalation to SAT on the grown region when the fixed-footprint
    // template can't make space.
    let strategies: [&dyn JunctionStrategy; 2] = [&perp_strategy, &sat_strategy];

    for &tile in &crossing_set {
        if corridor_handled.contains(&tile) {
            continue;
        }
        // Collect keys of every spec whose path passes through this
        // tile. classify_crossing gates on "exactly two specs with a
        // valid direction at the tile" — if that fails, the crossing
        // is degenerate and we skip to unresolved.
        if classify_crossing(tile, &routed_paths, &specs, &spec_items, &spec_belt_tiers).is_none() {
            remaining_crossings.insert(tile);
            continue;
        }
        // Scan routed_paths directly so synthetic trunk keys (injected
        // above) are included alongside regular BeltSpec keys.
        let keys_at_tile: Vec<&str> = routed_paths
            .iter()
            .filter(|(_, path)| path.contains(&tile))
            .map(|(key, _)| key.as_str())
            .collect();

        let Some(sol) = junction_solver::solve_crossing(
            tile,
            &keys_at_tile,
            &routed_paths,
            &hard,
            &junction_hard,
            &unreleasable_obstacles,
            &spec_belt_tiers,
            &spec_items,
            &spec_exit_dirs,
            &entities,
            &strategies,
            &crossing_set,
        ) else {
            remaining_crossings.insert(tile);
            continue;
        };

        let footprint = sol.footprint;
        // Mark any other crossing tiles inside this zone's footprint as
        // handled. A grown zone (e.g. a 4-tile wide SAT solution) may span
        // several original crossing tiles; if we let the loop visit them
        // independently the solver produces a second, broken solution that
        // only partially overlaps the first (e.g. a UG output with no input).
        for &ct in &crossing_set {
            if ct.0 >= footprint.x
                && ct.0 < footprint.x + footprint.w as i32
                && ct.1 >= footprint.y
                && ct.1 < footprint.y + footprint.h as i32
            {
                corridor_handled.insert(ct);
            }
        }
        trace::emit(trace::TraceEvent::GhostClusterSolved {
            cluster_id: template_count,
            zone_x: footprint.x,
            zone_y: footprint.y,
            zone_w: footprint.w,
            zone_h: footprint.h,
            boundary_count: 4,
            variables: 0,
            clauses: 0,
            solve_time_us: 0,
        });
        template_regions.push(LayoutRegion {
            kind: crate::models::RegionKind::JunctionTemplate,
            x: footprint.x,
            y: footprint.y,
            width: footprint.w as i32,
            height: footprint.h as i32,
            ports: Vec::new(),
        });

        let release_rect = crate::bus::ghost_occupancy::Rect {
            x: footprint.x,
            y: footprint.y,
            w: footprint.w,
            h: footprint.h,
        };
        // Preserve every tile in the footprint that the strategy is
        // NOT explicitly stamping a new entity at. The strategy's
        // solution is authoritative *exactly* over its proposed
        // tiles; every other tile's existing claim (trunk belt,
        // balancer splitter, non-participating tap, whatever) must
        // remain intact so the chain around the crossing stays
        // connected.
        //
        // This is the minimum-authority rule: we only touch what
        // the solver explicitly promises to replace. Without it,
        // uniformly-grown bboxes wipe out unrelated trunk/splitter
        // entities just because they sit inside the rectangle.
        let proposed_tiles: rustc_hash::FxHashSet<(i32, i32)> =
            sol.entities.iter().map(|e| (e.x, e.y)).collect();
        let preserve_trunk_tiles: rustc_hash::FxHashSet<(i32, i32)> =
            (0..release_rect.h as i32)
                .flat_map(|dy| {
                    (0..release_rect.w as i32)
                        .map(move |dx| (release_rect.x + dx, release_rect.y + dy))
                })
                .filter(|t| !proposed_tiles.contains(t))
                .collect();
        // Only release ghost surface entities that lie on a participating
        // spec path. Ghost entities belonging to non-participating specs
        // (e.g. a copper-cable tap whose path runs through the zone bbox
        // but is NOT being rerouted) must stay so the belt chain is intact.
        let releasable_ghost_tiles: rustc_hash::FxHashSet<(i32, i32)> = keys_at_tile
            .iter()
            .filter_map(|k| routed_paths.get(*k))
            .flatten()
            .filter(|&&t| release_rect.contains(t.0, t.1))
            .copied()
            .collect();
        occupancy.release_for_pertile_template(
            &release_rect,
            Some(&releasable_ghost_tiles),
            Some(&preserve_trunk_tiles),
        );
        for ent in sol.entities {
            let tile = (ent.x, ent.y);
            if occupancy.is_hard_obstacle(tile) {
                continue;
            }
            // Two per-tile templates with overlapping footprints —
            // the second one's stamp is skipped to match the
            // legacy post-hoc `occupied` filter behaviour. We also
            // skip the entity from the entity list entirely, otherwise
            // it leaks an orphan belt at a tile that's already claimed
            // by an earlier template, and the validator flags it as
            // an entity-overlap with the earlier entity.
            if matches!(
                occupancy.claim_at(tile),
                Some(crate::bus::ghost_occupancy::Claim::Template { .. })
                    | Some(crate::bus::ghost_occupancy::Claim::RowEntity { .. })
            ) {
                continue;
            }
            occupancy
                .place(
                    ent.clone(),
                    crate::bus::ghost_occupancy::ClaimKindTag::Template,
                )
                .unwrap_or_else(|err| {
                    panic!(
                        "occupancy refactor: template place failed at ({},{}): {:?}",
                        tile.0, tile.1, err
                    );
                });
            entities.push(ent);
        }
        template_count += 1;
    }

    // Step 6b: Emit per-tile "unresolved" regions for crossings that
    // no template could handle. This replaces the old SAT cluster
    // pipeline — the padded-bbox + union-find + varisat approach was
    // producing broken output on every cluster in real layouts (see
    // docs/archive/sat-band-investigation.md).
    //
    // Each unresolved crossing becomes a 1×1 mini-junction whose specs
    // we record as input/output port pairs on a LayoutRegion with
    // `kind = "unresolved"`. Downstream (UI + diagnostic) can render
    // these as "here's where junction-solver work is needed". No
    // entities are emitted — the layout renders with visible gaps
    // where crossings weren't solved.
    let cluster_count = template_count + remaining_crossings.len();
    let max_cluster_tiles = if remaining_crossings.is_empty() { 0 } else { 1 };
    let mut unresolved_regions = emit_unresolved_junctions(
        &remaining_crossings,
        &routed_paths,
        &specs,
        &spec_items,
        &spec_belt_tiers,
        &ghost_item_at,
    );

    let mut regions = template_regions;
    regions.append(&mut unresolved_regions);

    if !remaining_crossings.is_empty() {
        warnings.push(format!(
            "ghost router: {} unresolved crossings (junction solver not yet implemented)",
            remaining_crossings.len()
        ));
    }

    // Step 6: sync `entities` to Occupancy's released state.
    //
    // Templates and SAT write to both `entities` and Occupancy during
    // their phases. When they release/replace a prior claim (a ghost
    // surface belt, a trunk, or a tapoff) via `release_ghost_surface_in`
    // or `release_for_pertile_template`, Occupancy is updated but the
    // old entity stays in the local `entities` Vec. This pass drops
    // any ghost/trunk/tapoff entity whose Occupancy claim no longer
    // matches — i.e., where a later phase stamped over it.
    //
    // Other entity kinds (row templates, step 2/3 entities, templates,
    // SAT solutions, corridor-perp re-adds) are always kept.
    //
    // The post-hoc add loop that previously re-added template/SAT
    // entities via `solved_zones` is gone — Steps 4-5 push those
    // entities directly into the Vec at the moment they're generated.
    entities.retain(|e| {
        let seg = e.segment_id.as_deref().unwrap_or("");
        let occ_claim = occupancy.claim_at((e.x, e.y));
        if seg.starts_with("ghost:") {
            // Keep only if Occupancy still holds a GhostSurface claim
            // at this tile. If the claim was released by a template
            // or SAT solution, drop the entity.
            return matches!(
                occ_claim,
                Some(crate::bus::ghost_occupancy::Claim::GhostSurface { .. })
            );
        }
        if seg.starts_with("trunk:") || seg.starts_with("tapoff:") {
            // Keep only if Occupancy still holds a Permanent claim.
            // A per-tile template that stamped over the trunk will
            // have released the Permanent claim and replaced it with
            // a Template claim.
            return matches!(
                occ_claim,
                Some(crate::bus::ghost_occupancy::Claim::Permanent { .. })
            );
        }
        true
    });

    // -------------------------------------------------------------------------
    // Step 7: Merge output rows for final products
    // -------------------------------------------------------------------------
    let output_items: FxHashSet<String> = solver_result
        .external_outputs
        .iter()
        .filter(|ext| !ext.is_fluid)
        .map(|ext| ext.item.clone())
        .collect();

    for item in &output_items {
        let output_rows: Vec<usize> = row_spans
            .iter()
            .enumerate()
            .filter(|(_, rs)| rs.spec.outputs.iter().any(|o| &o.item == item && !o.is_fluid))
            .map(|(i, _)| i)
            .collect();

        if !output_rows.is_empty() {
            let (merge_ents, merge_end_y, item_merge_x) =
                merge_output_rows(&output_rows, item, row_spans, max_y, max_belt_tier);
            crate::trace::emit(crate::trace::TraceEvent::OutputMerged {
                item: item.clone(),
                rows: output_rows.clone(),
                merge_y: max_y,
            });
            entities.extend(merge_ents);
            max_y = max_y.max(merge_end_y);
            merge_max_x = merge_max_x.max(item_merge_x);
        }
    }

    // -------------------------------------------------------------------------
    // Emit summary trace event
    // -------------------------------------------------------------------------
    trace::emit(trace::TraceEvent::GhostRoutingComplete {
        entity_count: entities.len(),
        cluster_count,
        max_cluster_tiles,
        unroutable_count: unroutable_specs.len(),
    });

    Ok(GhostRouteResult {
        entities,
        ghost_crossing_tiles: crossing_set,
        cluster_count,
        max_cluster_tiles,
        unroutable_specs,
        max_y,
        merge_max_x,
        regions,
        warnings,
    })
}

// ---------------------------------------------------------------------------
// Cluster zone + per-tile template support (used by Step 6a templates)
// ---------------------------------------------------------------------------

/// Bounding box for a ghost cluster zone (padded by 1 tile on each side).
#[derive(Clone, Copy)]
struct ClusterZone {
    /// Padded bbox left
    x: i32,
    /// Padded bbox top
    y: i32,
    /// Padded bbox width
    w: u32,
    /// Padded bbox height
    h: u32,
}

/// Direction from a (dx, dy) step.
fn step_direction(dx: i32, dy: i32) -> EntityDirection {
    if dx > 0 {
        EntityDirection::East
    } else if dx < 0 {
        EntityDirection::West
    } else if dy > 0 {
        EntityDirection::South
    } else {
        EntityDirection::North
    }
}

/// Classify a cluster's crossing pattern by examining which paths pass
/// through its tiles and their directions.
struct CrossingInfo {
    /// The single crossing tile (only set for single-tile clusters).
    tile: (i32, i32),
    /// The two specs that cross, with their direction at the crossing tile.
    spec_a: (String, EntityDirection), // (item, direction)
    spec_b: (String, EntityDirection),
    /// Belt name for each spec (for entity construction).
    belt_a: &'static str,
    belt_b: &'static str,
}

/// Check if two directions are perpendicular.
fn is_perpendicular(a: EntityDirection, b: EntityDirection) -> bool {
    matches!(
        (a, b),
        (EntityDirection::East | EntityDirection::West, EntityDirection::North | EntityDirection::South)
        | (EntityDirection::North | EntityDirection::South, EntityDirection::East | EntityDirection::West)
    )
}

fn is_horizontal(d: EntityDirection) -> bool {
    matches!(d, EntityDirection::East | EntityDirection::West)
}

/// Try to classify a single crossing tile as a 2-path crossing.
/// Returns CrossingInfo if exactly 2 different-item specs cross at this tile.
///
/// `spec_items` and `spec_belt_tiers` are consulted as a fallback for keys
/// (e.g. synthetic trunk paths) that don't have a corresponding `BeltSpec`
/// in `specs`.
fn classify_crossing(
    tile: (i32, i32),
    routed_paths: &FxHashMap<String, Vec<(i32, i32)>>,
    specs: &[BeltSpec],
    spec_items: &FxHashMap<String, String>,
    spec_belt_tiers: &FxHashMap<String, BeltTier>,
) -> Option<CrossingInfo> {
    let (cx, cy) = tile;

    let spec_map: FxHashMap<&str, &BeltSpec> = specs.iter().map(|s| (s.key.as_str(), s)).collect();
    // Each entry: (item, belt_name, direction)
    let mut crossing_specs: Vec<(String, &'static str, EntityDirection)> = Vec::new();

    for (key, path) in routed_paths {
        // Derive item and belt_name from BeltSpec when available; fall back
        // to the supplementary maps for synthetic trunk paths.
        let (item, belt_name, exit_dir) = if let Some(spec) = spec_map.get(key.as_str()) {
            (spec.item.clone(), spec.belt_name, spec.exit_dir)
        } else if let Some(it) = spec_items.get(key.as_str()) {
            let tier = spec_belt_tiers
                .get(key.as_str())
                .copied()
                .unwrap_or(BeltTier::Yellow);
            (it.clone(), belt_name_for_tier(tier), None)
        } else {
            continue;
        };

        for (i, &(px, py)) in path.iter().enumerate() {
            if px == cx && py == cy {
                let dir = if i + 1 < path.len() {
                    let (nx, ny) = path[i + 1];
                    step_direction(nx - px, ny - py)
                } else if i > 0 {
                    let (px2, py2) = path[i - 1];
                    step_direction(px - px2, py - py2)
                } else if let Some(d) = exit_dir {
                    // 1-tile path: no neighbour to derive direction from.
                    // Use the explicit exit_dir set at emission time.
                    d
                } else {
                    continue;
                };
                crossing_specs.push((item, belt_name, dir));
                break;
            }
        }
    }

    if crossing_specs.len() != 2 {
        return None;
    }
    let (ref item_a, belt_a, da) = crossing_specs[0];
    let (ref item_b, belt_b, db) = crossing_specs[1];

    Some(CrossingInfo {
        tile,
        spec_a: (item_a.clone(), da),
        spec_b: (item_b.clone(), db),
        belt_a,
        belt_b,
    })
}

/// Build one `LayoutRegion { kind: Unresolved }` per remaining crossing
/// tile. Each region is lowered from a `Junction` — a 1×1 mini-junction
/// with one `SpecCrossing` per crossing spec — so the long-term junction
/// solver pass can consume the same internal shape.
///
/// See `docs/archive/rfp-junction-solver.md` for the target replacement.
fn emit_unresolved_junctions(
    remaining: &FxHashSet<(i32, i32)>,
    routed_paths: &FxHashMap<String, Vec<(i32, i32)>>,
    specs: &[BeltSpec],
    spec_items: &FxHashMap<String, String>,
    spec_belt_tiers: &FxHashMap<String, BeltTier>,
    ghost_item_at: &FxHashMap<(i32, i32), String>,
) -> Vec<LayoutRegion> {
    use crate::bus::junction::{Junction, Rect, SpecCrossing};
    use crate::models::{PortPoint, RegionKind};

    let _ = ghost_item_at;

    let mut out: Vec<LayoutRegion> = Vec::with_capacity(remaining.len());

    // Sort tiles for deterministic output (the diagnostic expects stable
    // order across runs).
    let mut tiles: Vec<(i32, i32)> = remaining.iter().copied().collect();
    tiles.sort();

    for (tx, ty) in tiles {
        let bbox = Rect { x: tx, y: ty, w: 1, h: 1 };
        let junction_specs: Vec<SpecCrossing> =
            classify_crossing((tx, ty), routed_paths, specs, spec_items, spec_belt_tiers)
            .map(|info| {
                // 1×1 bbox: entry and exit sit on the same tile; direction
                // encodes the flow. The lowering in `Junction::to_layout_region`
                // picks the correct edges from `(io, direction)`.
                let make = |item: String, dir: EntityDirection, belt: &str| SpecCrossing {
                    item,
                    belt_tier: BeltTier::from_name(belt).unwrap_or(BeltTier::Yellow),
                    entry: PortPoint { x: tx, y: ty, direction: dir },
                    exit: PortPoint { x: tx, y: ty, direction: dir },
                };
                vec![
                    make(info.spec_a.0, info.spec_a.1, info.belt_a),
                    make(info.spec_b.0, info.spec_b.1, info.belt_b),
                ]
            })
            .unwrap_or_default();

        let junction = Junction {
            bbox,
            forbidden: FxHashSet::default(),
            specs: junction_specs,
        };
        out.push(junction.to_layout_region(RegionKind::Unresolved));
    }

    out
}

fn ug_for_belt(belt: &str) -> &'static str {
    match belt {
        "fast-transport-belt" => "fast-underground-belt",
        "express-transport-belt" => "express-underground-belt",
        _ => "underground-belt",
    }
}

/// Returns true if any spec in `routed_paths` has a turn (different incoming
/// and outgoing directions) at the given tile. Used to reject per-tile UG
/// templates: a UG-in/out can't sit on a tile where another spec is turning,
/// because the turning spec would sideload onto the UG and items would be
/// dropped (UG belts only accept items entering from behind in their
/// facing direction, not from a sideload).
fn any_spec_turns_at(
    tile: (i32, i32),
    routed_paths: &FxHashMap<String, Vec<(i32, i32)>>,
) -> bool {
    for path in routed_paths.values() {
        for (i, &t) in path.iter().enumerate() {
            if t != tile {
                continue;
            }
            if i == 0 || i + 1 >= path.len() {
                break;
            }
            let dx_in = t.0 - path[i - 1].0;
            let dy_in = t.1 - path[i - 1].1;
            let dx_out = path[i + 1].0 - t.0;
            let dy_out = path[i + 1].1 - t.1;
            if (dx_in, dy_in) != (dx_out, dy_out) {
                return true;
            }
            break;
        }
    }
    false
}

/// Returns true if a UG-in/out at `tile` facing `bridge_dir` would receive
/// a sideload from a perpendicular spec — either because the tile itself has
/// a perpendicular spec passing through, or because an adjacent SIDE tile
/// (perpendicular to the bridge axis) has a belt flowing INTO this tile.
///
/// In Factorio, sideloads onto UG-input belts only fill the far lane and
/// can dump items wrong; we reject any template that would create one.
///
/// The check considers both (a) in-flight routed ghost specs and (b) already-
/// placed physical entities (splitters, row belts, trunks placed in Step 2-3).
/// Without (b), a splitter whose output drops straight into this tile from a
/// perpendicular direction slips through — the splitter is stamped before
/// ghost routing and never enters `routed_paths`.
///
/// Returns `None` if the tile is fine, or `Some(reason)` identifying the
/// first conflict found. The caller tags the reason with the endpoint
/// it was checking (UG-in vs UG-out).
fn ug_endpoint_conflicts(
    tile: (i32, i32),
    bridge_dir: EntityDirection,
    bridge_spec_key: &str,
    routed_paths: &FxHashMap<String, Vec<(i32, i32)>>,
    placed_entities: &[PlacedEntity],
) -> Option<&'static str> {
    let bridge_axis_vert = matches!(bridge_dir, EntityDirection::North | EntityDirection::South);

    // 1. The tile itself: if any other spec has it in its path AND its axis
    //    at the tile differs from the bridge axis, we'd have two
    //    perpendicular belts at the same tile — conflict.
    for (key, path) in routed_paths {
        if key == bridge_spec_key {
            continue;
        }
        for (i, &t) in path.iter().enumerate() {
            if t != tile {
                continue;
            }
            let last_idx = path.len() - 1;
            let (_dx, dy) = if i < last_idx {
                (path[i + 1].0 - t.0, path[i + 1].1 - t.1)
            } else if i > 0 {
                (t.0 - path[i - 1].0, t.1 - path[i - 1].1)
            } else {
                continue;
            };
            let spec_axis_vert = dy != 0;
            if spec_axis_vert != bridge_axis_vert {
                return Some("axis_conflict");
            }
            break;
        }
    }

    // 2. Adjacent SIDE tiles flowing INTO this tile (sideload). For a
    //    horizontal bridge, the sides are above/below. For a vertical
    //    bridge, the sides are left/right.
    let side_offsets: &[(i32, i32, EntityDirection)] = if bridge_axis_vert {
        // Vertical bridge: sides are east/west; a sideload comes from
        // (x-1, y) facing East, or (x+1, y) facing West.
        &[(-1, 0, EntityDirection::East), (1, 0, EntityDirection::West)]
    } else {
        // Horizontal bridge: sides are north/south; a sideload comes from
        // (x, y-1) facing South, or (x, y+1) facing North.
        &[(0, -1, EntityDirection::South), (0, 1, EntityDirection::North)]
    };
    for &(dx, dy, expected_dir) in side_offsets {
        let side = (tile.0 + dx, tile.1 + dy);
        // 2a. Routed-path sideloads (original check).
        for path in routed_paths.values() {
            for (i, &t) in path.iter().enumerate() {
                if t != side {
                    continue;
                }
                // Compute the spec's direction at this side tile.
                let last_idx = path.len() - 1;
                let (sdx, sdy) = if i < last_idx {
                    (path[i + 1].0 - t.0, path[i + 1].1 - t.1)
                } else if i > 0 {
                    (t.0 - path[i - 1].0, t.1 - path[i - 1].1)
                } else {
                    continue;
                };
                let spec_dir = step_direction(sdx, sdy);
                if spec_dir == expected_dir {
                    return Some("sideload");
                }
                break;
            }
        }
        // 2b. Pre-routing splitters (Step 2-3) dropping items into the
        //     UG endpoint tile from a perpendicular direction. Splitters
        //     never enter `routed_paths` so 2a misses them. Splitters are
        //     two tiles wide perpendicular to their facing direction, so
        //     the `side` tile may be the right half of a splitter placed
        //     one column west.
        for ent in placed_entities {
            let is_splitter = matches!(
                ent.name.as_str(),
                "splitter" | "fast-splitter" | "express-splitter"
            );
            if !is_splitter {
                continue;
            }
            if ent.direction != expected_dir {
                continue;
            }
            let second = match ent.direction {
                EntityDirection::North | EntityDirection::South => (ent.x + 1, ent.y),
                EntityDirection::East | EntityDirection::West => (ent.x, ent.y + 1),
            };
            if (ent.x, ent.y) == side || second == side {
                return Some("splitter_sideload");
            }
        }
    }

    None
}

/// Solve a perpendicular crossing with a deterministic template.
///
/// One path stays on the surface, the other goes underground via a UG pair.
/// Prefers bridging the vertical path so horizontal connections to row inputs
/// stay on the surface.
fn solve_perpendicular_template(
    info: &CrossingInfo,
    hard_obstacles: &FxHashSet<(i32, i32)>,
    unreleasable_obstacles: &FxHashSet<(i32, i32)>,
    routed_paths: &FxHashMap<String, Vec<(i32, i32)>>,
    placed_entities: &[PlacedEntity],
) -> Option<(Vec<PlacedEntity>, ClusterZone)> {
    let perpendicular = is_perpendicular(info.spec_a.1, info.spec_b.1);
    if !perpendicular {
        // Same-direction crossings — single attempt, bridge spec_b arbitrarily.
        return try_bridge(
            info.tile,
            (&info.spec_a.0, info.spec_a.1, info.belt_a),
            (&info.spec_b.0, info.spec_b.1, info.belt_b),
            hard_obstacles,
            unreleasable_obstacles,
            routed_paths,
            placed_entities,
        );
    }

    // Perpendicular: try BOTH bridge directions and pick the first that works.
    // Prefer bridging the vertical first (keeps horizontal connections on the
    // surface for row inputs), but fall back to bridging the horizontal if
    // the vertical option is blocked or has UG-position turn conflicts.
    let (h_spec, v_spec) = if is_horizontal(info.spec_a.1) {
        (&info.spec_a, &info.spec_b)
    } else {
        (&info.spec_b, &info.spec_a)
    };

    let bridge_vertical_first = try_bridge(
        info.tile,
        (&h_spec.0, h_spec.1, if std::ptr::eq(h_spec, &info.spec_a) { info.belt_a } else { info.belt_b }),
        (&v_spec.0, v_spec.1, if std::ptr::eq(v_spec, &info.spec_a) { info.belt_a } else { info.belt_b }),
        hard_obstacles,
        unreleasable_obstacles,
        routed_paths,
        placed_entities,
    );
    if bridge_vertical_first.is_some() {
        return bridge_vertical_first;
    }

    // Fall back to bridging the horizontal (vertical stays on surface).
    try_bridge(
        info.tile,
        (&v_spec.0, v_spec.1, if std::ptr::eq(v_spec, &info.spec_a) { info.belt_a } else { info.belt_b }),
        (&h_spec.0, h_spec.1, if std::ptr::eq(h_spec, &info.spec_a) { info.belt_a } else { info.belt_b }),
        hard_obstacles,
        unreleasable_obstacles,
        routed_paths,
        placed_entities,
    )
}

/// Try to place a UG bridge for the second `(item, dir, belt)` triple over
/// the first one staying on the surface at `crossing`. Returns `None` if the
/// UG positions are obstructed or would receive a sideload.
fn try_bridge(
    crossing: (i32, i32),
    surface: (&String, EntityDirection, &'static str),
    bridge: (&String, EntityDirection, &'static str),
    hard_obstacles: &FxHashSet<(i32, i32)>,
    unreleasable_obstacles: &FxHashSet<(i32, i32)>,
    routed_paths: &FxHashMap<String, Vec<(i32, i32)>>,
    placed_entities: &[PlacedEntity],
) -> Option<(Vec<PlacedEntity>, ClusterZone)> {
    let (cx, cy) = crossing;
    let (surface_item, surface_dir, surface_belt) = surface;
    let (bridge_item, bridge_dir, bridge_belt) = bridge;

    let (dx, dy) = match bridge_dir {
        EntityDirection::North => (0, -1),
        EntityDirection::South => (0, 1),
        EntityDirection::East => (1, 0),
        EntityDirection::West => (-1, 0),
    };
    let ug_in = (cx - dx, cy - dy);
    let ug_out = (cx + dx, cy + dy);

    let bridge_axis_label: &'static str = match bridge_dir {
        EntityDirection::North | EntityDirection::South => "vertical",
        EntityDirection::East | EntityDirection::West => "horizontal",
    };
    let reject = |reason: &'static str| -> Option<(Vec<PlacedEntity>, ClusterZone)> {
        trace::emit(trace::TraceEvent::JunctionTemplateRejected {
            tile_x: cx,
            tile_y: cy,
            bridge_dir: bridge_axis_label.to_string(),
            reason: reason.to_string(),
        });
        None
    };

    if hard_obstacles.contains(&ug_in) {
        return reject("hard_obstacle_ug_in");
    }
    // `release_for_pertile_template` clears trunks/tapoffs inside the 3-tile
    // footprint, so those don't block us. But it leaves Permanent claims with
    // any other segment id — balancer belts, corridor-perp re-adds, row
    // templates, prior stamped templates — alone. Stamping a UG endpoint OR
    // the crossing-tile surface belt on top of one of those panics in
    // `place`. Reject here so the growth loop in `solve_crossing` moves on
    // to SatStrategy at the next iteration.
    if unreleasable_obstacles.contains(&ug_in) {
        return reject("unreleasable_obstacle_ug_in");
    }
    if unreleasable_obstacles.contains(&ug_out) {
        return reject("unreleasable_obstacle_ug_out");
    }
    if unreleasable_obstacles.contains(&(cx, cy)) {
        return reject("unreleasable_obstacle_crossing");
    }
    if hard_obstacles.contains(&ug_out) {
        return reject("hard_obstacle_ug_out");
    }

    // Reject if any spec turns at the UG-in/out tile, or if a perpendicular
    // belt would sideload into them. Sideloads onto UG belts only fill the
    // far lane and dump items.
    if any_spec_turns_at(ug_in, routed_paths) {
        return reject("turn_at_ug_in");
    }
    if any_spec_turns_at(ug_out, routed_paths) {
        return reject("turn_at_ug_out");
    }
    // Find a representative key for the bridged spec to exclude from the
    // conflict check. Any path containing the crossing tile in the bridge
    // direction at that tile counts as the bridged spec.
    let bridge_key = routed_paths
        .iter()
        .find(|(_, path)| path.contains(&crossing))
        .map(|(k, _)| k.as_str())
        .unwrap_or("");
    if let Some(sub) = ug_endpoint_conflicts(ug_in, bridge_dir, bridge_key, routed_paths, placed_entities) {
        return reject(match sub {
            "axis_conflict" => "ug_in_axis_conflict",
            "sideload" => "ug_in_sideload",
            _ => "ug_in_conflict",
        });
    }
    if let Some(sub) = ug_endpoint_conflicts(ug_out, bridge_dir, bridge_key, routed_paths, placed_entities) {
        return reject(match sub {
            "axis_conflict" => "ug_out_axis_conflict",
            "sideload" => "ug_out_sideload",
            _ => "ug_out_conflict",
        });
    }

    let ug_name = ug_for_belt(bridge_belt);
    let seg = Some(format!("junction:{}:{},{}", bridge_item, cx, cy));

    let entities = vec![
        PlacedEntity {
            name: surface_belt.to_string(),
            x: cx,
            y: cy,
            direction: surface_dir,
            carries: Some(surface_item.clone()),
            segment_id: seg.clone(),
            ..Default::default()
        },
        PlacedEntity {
            name: ug_name.to_string(),
            x: ug_in.0,
            y: ug_in.1,
            direction: bridge_dir,
            io_type: Some("input".to_string()),
            carries: Some(bridge_item.clone()),
            segment_id: seg.clone(),
            ..Default::default()
        },
        PlacedEntity {
            name: ug_name.to_string(),
            x: ug_out.0,
            y: ug_out.1,
            direction: bridge_dir,
            io_type: Some("output".to_string()),
            carries: Some(bridge_item.clone()),
            segment_id: seg.clone(),
            ..Default::default()
        },
    ];

    let zone = ClusterZone {
        x: cx.min(ug_in.0).min(ug_out.0),
        y: cy.min(ug_in.1).min(ug_out.1),
        w: ((cx - ug_in.0).abs().max((cx - ug_out.0).abs()) * 2 + 1) as u32,
        h: ((cy - ug_in.1).abs().max((cy - ug_out.1).abs()) * 2 + 1) as u32,
    };

    Some((entities, zone))
}

/// Belt entity name for a junction-solver `BeltTier`. Matches the
/// surface belt names that `BeltSpec::belt_name` holds.
fn belt_name_for_tier(tier: BeltTier) -> &'static str {
    match tier {
        BeltTier::Yellow => "transport-belt",
        BeltTier::Red => "fast-transport-belt",
        BeltTier::Blue => "express-transport-belt",
    }
}

/// The first (and currently only) `JunctionStrategy` wired into the
/// growth loop: a thin wrapper around the existing
/// `solve_perpendicular_template`. Only activates when the junction
/// has exactly two specs with perpendicular directions at the initial
/// crossing tile. Ignores region growth entirely — the underlying
/// template operates on a fixed 3-tile footprint. Real growth-aware
/// strategies will land alongside this one.
struct PerpendicularTemplateStrategy;

impl JunctionStrategy for PerpendicularTemplateStrategy {
    fn name(&self) -> &'static str {
        "perpendicular_template"
    }

    fn try_solve(&self, ctx: &JunctionStrategyContext) -> Option<JunctionSolution> {
        // The underlying per-tile template operates on a fixed 3-tile
        // footprint around the original crossing and has no concept of
        // a grown region. If it fails on the initial 1×1 region, it
        // will fail the same way on every grown iteration — skip
        // subsequent attempts to avoid noisy duplicate trace events.
        if ctx.region.tile_count() > 1 {
            return None;
        }
        if ctx.junction.specs.len() != 2 {
            return None;
        }
        let sa = &ctx.junction.specs[0];
        let sb = &ctx.junction.specs[1];
        let da = sa.entry.direction;
        let db = sb.entry.direction;
        if !is_perpendicular(da, db) {
            return None;
        }
        let info = CrossingInfo {
            tile: ctx.region.initial_tile,
            spec_a: (sa.item.clone(), da),
            spec_b: (sb.item.clone(), db),
            belt_a: belt_name_for_tier(sa.belt_tier),
            belt_b: belt_name_for_tier(sb.belt_tier),
        };
        let (entities, zone) =
            solve_perpendicular_template(
                &info,
                ctx.hard_obstacles,
                ctx.unreleasable_obstacles,
                ctx.routed_paths,
                ctx.placed_entities,
            )?;
        Some(JunctionSolution {
            entities,
            footprint: Rect {
                x: zone.x,
                y: zone.y,
                w: zone.w,
                h: zone.h,
            },
            strategy_name: self.name(),
        })
    }
}

fn is_belt_like(name: &str) -> bool {
    matches!(
        name,
        "transport-belt"
            | "fast-transport-belt"
            | "express-transport-belt"
            | "underground-belt"
            | "fast-underground-belt"
            | "express-underground-belt"
            | "splitter"
            | "fast-splitter"
            | "express-splitter"
    )
}
