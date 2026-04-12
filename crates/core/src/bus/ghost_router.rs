//! Ghost A* bus router — Phase 2 of the ghost-cluster routing rewrite.
//!
//! Gated behind `FUCKTORIO_GHOST_ROUTING=1`.
//!
//! Algorithm overview:
//! 1. Build a hard-obstacle set from row_entities (machine footprints, poles, etc.)
//!    and fluid lane tile reservations.
//! 2. Place trunks as hard obstacles (South-facing belts at each lane's column).
//!    Splitter stamps and balancer blocks are also added to hard obstacles.
//! 3. Route each connecting-belt spec (tap-offs, returns, feeders) with
//!    `ghost_astar`. Belts are transparent — A* ghosts through them and records
//!    each crossing tile for Phase 3's SAT resolver.
//! 4. Union-find ghost crossings into clusters.
//! 5. Merge output rows via the existing `merge_output_rows` helper.
//!
//! Returns a `GhostRouteResult` containing all placed entities, ghost crossing
//! tiles, cluster info, and layout dimensions.
//!
//! See `docs/rfp-ghost-cluster-routing.md` for the full design.

use rustc_hash::{FxHashMap, FxHashSet};

use crate::astar::ghost_astar;
use crate::bus::bus_router::{
    BusLane, LaneFamily, is_intermediate, merge_output_rows, render_path,
    splitter_for_belt, stamp_family_balancer, trunk_segments, MACHINE_ENTITIES,
};
use crate::bus::placer::RowSpan;
use crate::common::{belt_entity_for_rate, machine_size, machine_tiles};
use crate::models::{EntityDirection, LayoutRegion, PlacedEntity, SolverResult};
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
}

/// A spec for one connecting belt run.
struct BeltSpec {
    key: String,
    start: (i32, i32),
    goal: (i32, i32),
    item: String,
    belt_name: &'static str,
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
    let mut max_y = total_height;
    let mut merge_max_x = 0i32;

    let width = (bw + 200).max(200);
    let height = (total_height + 50).max(200);

    // -------------------------------------------------------------------------
    // Step 1: Build hard obstacle set from row_entities
    // -------------------------------------------------------------------------
    let mut hard: FxHashSet<(i32, i32)> = FxHashSet::default();
    let mut existing_belts: FxHashSet<(i32, i32)> = FxHashSet::default();

    for e in row_entities {
        if is_belt_like(&e.name) {
            existing_belts.insert((e.x, e.y));
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
    // Step 2: Place trunk belts and splitter stamps as hard obstacles
    // -------------------------------------------------------------------------
    for lane in lanes {
        if lane.is_fluid {
            continue;
        }

        let x = lane.x;
        let belt_name = belt_entity_for_rate(lane.rate * 2.0, max_belt_tier);
        let trunk_seg_id = Some(format!("trunk:{}", lane.item));
        let last_tap_y = lane.tap_off_ys.iter().copied().max();

        // Compute skip_ys for trunk: tap_off_ys + splitter rows + balancer_y + family_balancer_range
        let mut skip_ys: FxHashSet<i32> = lane.tap_off_ys.iter().copied().collect();
        for &ty in &lane.tap_off_ys {
            if lane.tap_off_ys.len() > 1 && Some(ty) != last_tap_y {
                skip_ys.insert(ty - 1); // splitter row above tap
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
        for &pri in &lane.extra_producer_rows {
            if pri < row_spans.len() {
                all_ys.push(row_spans[pri].output_belt_y);
            }
        }
        if let Some(pr) = lane.producer_row {
            if pr < row_spans.len() {
                all_ys.push(row_spans[pr].output_belt_y);
            }
        }
        let start_y = lane.source_y;
        let end_y = all_ys.iter().copied().max().unwrap_or(start_y);
        let end_y = if let Some(by) = lane.balancer_y {
            end_y.max(by + 1)
        } else {
            end_y
        };

        // Place trunk surface belts for each contiguous segment
        for (seg_start, seg_end) in trunk_segments(start_y, end_y, &skip_ys) {
            for y in seg_start..=seg_end {
                let ent = PlacedEntity {
                    name: belt_name.to_string(),
                    x,
                    y,
                    direction: EntityDirection::South,
                    carries: Some(lane.item.clone()),
                    segment_id: trunk_seg_id.clone(),
                    rate: Some(lane.rate),
                    ..Default::default()
                };
                hard.insert((x, y));
                existing_belts.insert((x, y));
                entities.push(ent);
            }
        }

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
            } else {
                hard.insert((ent.x, ent.y));
            }
        }
        entities.extend(balancer_ents);
    }

    // -------------------------------------------------------------------------
    // Step 4: Build connecting-belt spec list
    // -------------------------------------------------------------------------
    let mut specs: Vec<BeltSpec> = Vec::new();

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
                });
            }
        }

        // Return specs for intermediate lanes (no family balancer)
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
                let out_y = row_spans[pri].output_belt_y;
                let ret_key = format!("ret:{}:{}:{}", lane.item, x, out_y);
                specs.push(BeltSpec {
                    key: ret_key,
                    start: (bw - 1, out_y),
                    goal: (x + 1, out_y),
                    item: lane.item.clone(),
                    belt_name: horiz_belt,
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
                let out_y = row_spans[pri].output_belt_y;
                let ret_key = format!("ret:{}:{}:{}", lane.item, x, out_y);
                specs.push(BeltSpec {
                    key: ret_key,
                    start: (bw - 1, out_y),
                    goal: (x + 1, out_y),
                    item: lane.item.clone(),
                    belt_name: horiz_belt,
                });
            }
        }

        // Feeder specs for family balancer lanes
        if let Some(family_id) = lane.family_id {
            if let Some(fam) = families.get(family_id) {
                let mut all_producers = Vec::new();
                if let Some(pr) = lane.producer_row {
                    all_producers.push(pr);
                }
                all_producers.extend(&lane.extra_producer_rows);

                // Get the balancer's input tiles
                let templates = crate::bus::balancer_library::balancer_templates();
                let (n, m) = (fam.shape.0 as u32, fam.shape.1 as u32);
                if let Some(template) = templates.get(&(n, m)) {
                    let origin_x = fam.lane_xs.iter().copied().min().unwrap_or(x);
                    let origin_y = fam.balancer_y_start;

                    let mut inputs: Vec<(i32, i32)> = template.input_tiles.to_vec();
                    inputs.sort_by_key(|t| t.0);

                    let feeder_belt = belt_entity_for_rate(fam.total_rate, max_belt_tier);

                    for (i, &pri) in all_producers.iter().enumerate() {
                        if pri >= row_spans.len() {
                            continue;
                        }
                        let out_y = row_spans[pri].output_belt_y;
                        if let Some(&(input_x_rel, _input_y_rel)) = inputs.get(i) {
                            let input_x = origin_x + input_x_rel;
                            let input_y = origin_y;
                            let feeder_key = format!(
                                "feeder:{}:{}:{}",
                                lane.item, input_x, out_y
                            );
                            specs.push(BeltSpec {
                                key: feeder_key,
                                start: (bw - 1, out_y),
                                goal: (input_x, input_y),
                                item: lane.item.clone(),
                                belt_name: feeder_belt,
                            });
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

    let mut routed_paths: FxHashMap<String, Vec<(i32, i32)>> = FxHashMap::default();
    let mut all_ghost_crossings: Vec<(i32, i32)> = Vec::new();
    let mut unroutable_specs: Vec<String> = Vec::new();

    for spec in &specs {
        match ghost_astar(
            spec.start,
            spec.goal,
            &hard,
            &existing_belts,
            width,
            height,
            TURN_PENALTY,
        ) {
            Some((path, crossings)) => {
                let turns = count_turns(&path);
                trace::emit(trace::TraceEvent::GhostSpecRouted {
                    spec_key: spec.key.clone(),
                    path_len: path.len(),
                    crossings: crossings.len(),
                    turns,
                });

                // Emit entities via render_path
                let direction_hint = if spec.start.0 <= spec.goal.0 {
                    EntityDirection::East
                } else {
                    EntityDirection::West
                };
                let spec_seg_id = Some(format!("ghost:{}", spec.key));
                let path_ents = render_path(
                    &path,
                    &spec.item,
                    spec.belt_name,
                    direction_hint,
                    spec_seg_id,
                    None,
                );
                entities.extend(path_ents);

                // Add new tiles to existing_belts for subsequent specs
                for &tile in &path {
                    existing_belts.insert(tile);
                }
                all_ghost_crossings.extend(crossings);
                routed_paths.insert(spec.key.clone(), path);
            }
            None => {
                trace::emit(trace::TraceEvent::GhostSpecFailed {
                    spec_key: spec.key.clone(),
                    from_x: spec.start.0,
                    from_y: spec.start.1,
                    to_x: spec.goal.0,
                    to_y: spec.goal.1,
                });
                unroutable_specs.push(spec.key.clone());
            }
        }
    }

    // -------------------------------------------------------------------------
    // Step 6: Union-find ghost crossings into clusters
    // -------------------------------------------------------------------------
    let crossing_set: FxHashSet<(i32, i32)> = all_ghost_crossings.iter().copied().collect();

    // Two crossings in the same path are in the same cluster.
    // We union-find by path: collect crossing indices per path, union them all.
    let crossing_list: Vec<(i32, i32)> = crossing_set.iter().copied().collect();
    let n_tiles = crossing_list.len();
    let mut uf: Vec<usize> = (0..n_tiles).collect();
    let tile_index: FxHashMap<(i32, i32), usize> = crossing_list
        .iter()
        .enumerate()
        .map(|(i, &t)| (t, i))
        .collect();

    fn uf_find(p: &mut [usize], i: usize) -> usize {
        let mut r = i;
        while p[r] != r {
            r = p[r];
        }
        let mut cur = i;
        while p[cur] != r {
            let next = p[cur];
            p[cur] = r;
            cur = next;
        }
        r
    }

    for path in routed_paths.values() {
        let path_crossings: Vec<usize> = path
            .iter()
            .filter_map(|t| tile_index.get(t).copied())
            .collect();
        if path_crossings.len() >= 2 {
            for &idx in &path_crossings[1..] {
                let ra = uf_find(&mut uf, path_crossings[0]);
                let rb = uf_find(&mut uf, idx);
                if ra != rb {
                    uf[ra] = rb;
                }
            }
        }
    }

    // Count clusters and their sizes
    let mut cluster_tile_counts: FxHashMap<usize, usize> = FxHashMap::default();
    for i in 0..n_tiles {
        let root = uf_find(&mut uf, i);
        *cluster_tile_counts.entry(root).or_insert(0) += 1;
    }
    let cluster_count = cluster_tile_counts.len();
    let max_cluster_tiles = cluster_tile_counts.values().copied().max().unwrap_or(0);

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
        regions: Vec::new(),
    })
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
