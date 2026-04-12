//! Ghost A* bus router — Phases 2+3 of the ghost-cluster routing rewrite.
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
//! 5. SAT-resolve each cluster: extract boundary ports, build CrossingZones,
//!    solve, replace ghost surface belts with proper UG pairs.
//! 6. Merge output rows via the existing `merge_output_rows` helper.
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
use crate::common::{belt_entity_for_rate, machine_size, machine_tiles, ug_max_reach};
use crate::models::{EntityDirection, LayoutRegion, PlacedEntity, PortEdge, PortIo, PortSpec, SolverResult};
use crate::sat::{self, CrossingZone, ZoneBoundary};
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
                pre_ghost_belts.insert((x, y));
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
    // Tracks which item each ghost-routed tile carries, so we can distinguish
    // same-item overlaps (not conflicts) from different-item overlaps (real).
    let mut ghost_item_at: FxHashMap<(i32, i32), String> = FxHashMap::default();

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
                // Skip ghost belt entities on tiles already occupied by
                // pre-existing belts or same-item ghost belts to avoid
                // entity overlaps — those connections are intentional.
                entities.extend(path_ents.into_iter().filter(|e| {
                    !pre_ghost_belts.contains(&(e.x, e.y))
                        && !ghost_item_at.contains_key(&(e.x, e.y))
                }));

                // Add new tiles to existing_belts for subsequent specs
                for &tile in &path {
                    existing_belts.insert(tile);
                }

                // Filter crossings: only keep tiles that represent real
                // conflicts (different-item ghost overlaps).
                // - Pre-existing belt overlaps → connections, not conflicts
                // - Same-item ghost overlaps → redundant belts, not conflicts
                // - Different-item ghost overlaps → real SAT crossings
                all_ghost_crossings.extend(crossings.into_iter().filter(|t| {
                    if pre_ghost_belts.contains(t) {
                        return false;
                    }
                    match ghost_item_at.get(t) {
                        Some(existing_item) => *existing_item != spec.item,
                        None => false, // first ghost at this tile, no conflict
                    }
                }));

                // Record this spec's item at each tile
                for &tile in &path {
                    ghost_item_at.entry(tile).or_insert_with(|| spec.item.clone());
                }
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
    // Step 6: Resolve ghost crossings — templates first, SAT fallback
    // -------------------------------------------------------------------------
    let crossing_set: FxHashSet<(i32, i32)> = all_ghost_crossings.iter().copied().collect();
    let effective_belt = max_belt_tier.unwrap_or("transport-belt");

    // Pre-existing entity positions (for template/SAT overlap avoidance)
    let pre_existing_set: FxHashSet<(i32, i32)> = entities
        .iter()
        .filter(|e| !e.segment_id.as_ref().is_some_and(|s| s.starts_with("ghost:")))
        .map(|e| (e.x, e.y))
        .chain(row_entities.iter().map(|e| (e.x, e.y)))
        .collect();

    // Step 6a: Try perpendicular crossing templates on individual tiles.
    // Solved tiles are removed from the crossing set before clustering.
    let mut template_zones: Vec<(Vec<PlacedEntity>, ClusterZone)> = Vec::new();
    let mut template_regions: Vec<LayoutRegion> = Vec::new();
    let mut remaining_crossings: FxHashSet<(i32, i32)> = FxHashSet::default();

    for &tile in &crossing_set {
        let single_set: FxHashSet<(i32, i32)> = [tile].into_iter().collect();
        if let Some(info) = classify_perpendicular(&single_set, &routed_paths, &specs) {
            if let Some((ents, zone)) =
                solve_perpendicular_template(&info, &hard, &pre_existing_set)
            {
                trace::emit(trace::TraceEvent::GhostClusterSolved {
                    cluster_id: template_zones.len(),
                    zone_x: zone.x,
                    zone_y: zone.y,
                    zone_w: zone.w,
                    zone_h: zone.h,
                    boundary_count: 4,
                    variables: 0,
                    clauses: 0,
                    solve_time_us: 0,
                });
                template_regions.push(LayoutRegion {
                    kind: "junction_template".to_string(),
                    x: zone.x,
                    y: zone.y,
                    width: zone.w as i32,
                    height: zone.h as i32,
                    inputs: vec![info.spec_a.0.clone(), info.spec_b.0.clone()],
                    outputs: vec![info.spec_a.0, info.spec_b.0],
                    ports: Vec::new(),
                    variables: 0,
                    clauses: 0,
                    solve_time_us: 0,
                });
                template_zones.push((ents, zone));
                continue; // tile solved — don't add to remaining
            }
        }
        remaining_crossings.insert(tile);
    }

    // Step 6b: Cluster remaining unsolved crossings and SAT-solve them.
    let merge_dist = ug_max_reach(effective_belt) as i32 + 1;
    let crossing_list: Vec<(i32, i32)> = remaining_crossings.iter().copied().collect();
    let n_tiles = crossing_list.len();
    let mut uf: Vec<usize> = (0..n_tiles).collect();

    for (i, &(ax, ay)) in crossing_list.iter().enumerate() {
        for (j, &(bx, by)) in crossing_list.iter().enumerate().skip(i + 1) {
            if (ax - bx).abs() + (ay - by).abs() <= merge_dist {
                let ra = uf_find(&mut uf, i);
                let rb = uf_find(&mut uf, j);
                if ra != rb {
                    uf[ra] = rb;
                }
            }
        }
    }

    let mut cluster_tile_counts: FxHashMap<usize, usize> = FxHashMap::default();
    for i in 0..n_tiles {
        let root = uf_find(&mut uf, i);
        *cluster_tile_counts.entry(root).or_insert(0) += 1;
    }
    let cluster_count = cluster_tile_counts.len() + template_zones.len();
    let max_cluster_tiles = cluster_tile_counts.values().copied().max().unwrap_or(0);

    let (mut sat_zones, mut sat_regions, failed_count) = resolve_clusters(
        &crossing_list,
        &mut uf,
        &routed_paths,
        &specs,
        max_belt_tier,
        &entities,
        &hard,
    );

    // Merge template and SAT results
    let mut solved_zones = template_zones;
    solved_zones.append(&mut sat_zones);
    let mut regions = template_regions;
    regions.append(&mut sat_regions);

    if failed_count > 0 {
        return Err(format!(
            "ghost router: {} of {} clusters failed SAT resolution",
            failed_count, cluster_count
        ));
    }

    // Remove ghost entities inside solved cluster zones, replace with SAT output.
    // Only ghost-routed entities (segment_id starts with "ghost:") are removed;
    // trunks, row template belts, splitters, and balancer entities stay in place.
    if !solved_zones.is_empty() {
        let zone_bboxes: Vec<&ClusterZone> = solved_zones.iter().map(|(_, z)| z).collect();

        entities.retain(|e| {
            let in_zone = zone_bboxes.iter().any(|z| z.contains(e.x, e.y));
            if !in_zone {
                return true;
            }
            // Keep non-ghost entities inside zones
            let is_ghost = e
                .segment_id
                .as_ref()
                .is_some_and(|s| s.starts_with("ghost:"));
            !is_ghost
        });

        // Build set of occupied positions: row_entities + non-ghost entities
        // in our entity list. SAT output must not overlap these.
        let mut occupied: FxHashSet<(i32, i32)> =
            row_entities.iter().map(|e| (e.x, e.y)).collect();
        for e in entities.iter() {
            occupied.insert((e.x, e.y));
        }

        for (sat_entities, _zone) in &solved_zones {
            // Skip SAT entities that would overlap pre-existing entities
            entities.extend(
                sat_entities
                    .iter()
                    .filter(|e| !occupied.contains(&(e.x, e.y)))
                    .cloned(),
            );
        }
    }

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
    })
}

// ---------------------------------------------------------------------------
// Union-find helper (used by both route_bus_ghost and resolve_clusters)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Phase 3: Ghost cluster SAT resolution
// ---------------------------------------------------------------------------

/// Bounding box for a ghost cluster zone (padded by 1 tile on each side).
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

impl ClusterZone {
    fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x
            && px < self.x + self.w as i32
            && py >= self.y
            && py < self.y + self.h as i32
    }

    fn on_edge(&self, px: i32, py: i32) -> bool {
        self.contains(px, py)
            && (px == self.x
                || px == self.x + self.w as i32 - 1
                || py == self.y
                || py == self.y + self.h as i32 - 1)
    }
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

/// Which edge of the zone a tile is on (for PortSpec construction).
fn tile_edge(zone: &ClusterZone, px: i32, py: i32) -> PortEdge {
    if py == zone.y {
        PortEdge::N
    } else if py == zone.y + zone.h as i32 - 1 {
        PortEdge::S
    } else if px == zone.x {
        PortEdge::W
    } else {
        PortEdge::E
    }
}

/// Offset of a tile along its edge (from top-left corner of the zone).
fn edge_offset(zone: &ClusterZone, edge: &PortEdge, px: i32, py: i32) -> u32 {
    match edge {
        PortEdge::N | PortEdge::S => (px - zone.x) as u32,
        PortEdge::W | PortEdge::E => (py - zone.y) as u32,
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

/// Try to classify a single-tile cluster as a perpendicular crossing.
fn classify_perpendicular(
    cluster_tiles: &FxHashSet<(i32, i32)>,
    routed_paths: &FxHashMap<String, Vec<(i32, i32)>>,
    specs: &[BeltSpec],
) -> Option<CrossingInfo> {
    // Must be exactly 1 crossing tile
    if cluster_tiles.len() != 1 {
        return None;
    }
    let &tile = cluster_tiles.iter().next().unwrap();
    let (cx, cy) = tile;

    // Find specs whose paths pass through this tile
    let spec_map: FxHashMap<&str, &BeltSpec> = specs.iter().map(|s| (s.key.as_str(), s)).collect();
    let mut crossing_specs: Vec<(&BeltSpec, EntityDirection)> = Vec::new();

    for (key, path) in routed_paths {
        let spec = match spec_map.get(key.as_str()) {
            Some(s) => s,
            None => continue,
        };
        // Find this tile in the path and compute direction
        for (i, &(px, py)) in path.iter().enumerate() {
            if px == cx && py == cy {
                // Compute direction at this point
                let dir = if i + 1 < path.len() {
                    let (nx, ny) = path[i + 1];
                    step_direction(nx - px, ny - py)
                } else if i > 0 {
                    let (px2, py2) = path[i - 1];
                    step_direction(px - px2, py - py2)
                } else {
                    continue;
                };
                crossing_specs.push((spec, dir));
                break;
            }
        }
    }

    // Must be exactly 2 specs crossing perpendicularly
    if crossing_specs.len() != 2 {
        return None;
    }
    let (sa, da) = crossing_specs[0];
    let (sb, db) = crossing_specs[1];

    if !is_perpendicular(da, db) {
        return None;
    }

    Some(CrossingInfo {
        tile,
        spec_a: (sa.item.clone(), da),
        spec_b: (sb.item.clone(), db),
        belt_a: sa.belt_name,
        belt_b: sb.belt_name,
    })
}

fn ug_for_belt(belt: &str) -> &'static str {
    match belt {
        "fast-transport-belt" => "fast-underground-belt",
        "express-transport-belt" => "express-underground-belt",
        _ => "underground-belt",
    }
}

/// Solve a perpendicular crossing with a deterministic template.
///
/// One path stays on the surface, the other goes underground via a UG pair.
/// Prefers bridging the vertical path so horizontal connections to row inputs
/// stay on the surface.
fn solve_perpendicular_template(
    info: &CrossingInfo,
    hard_obstacles: &FxHashSet<(i32, i32)>,
    pre_existing: &FxHashSet<(i32, i32)>,
) -> Option<(Vec<PlacedEntity>, ClusterZone)> {
    let (cx, cy) = info.tile;

    // Decide which path to bridge (put underground).
    // Prefer bridging vertical so horizontal row connections stay on surface.
    // If the horizontal path is blocked, bridge it instead.
    let (surface_item, surface_dir, surface_belt, bridge_item, bridge_dir, bridge_belt) =
        if is_horizontal(info.spec_a.1) {
            // A is horizontal (surface), B is vertical (bridge)
            (&info.spec_a.0, info.spec_a.1, info.belt_a, &info.spec_b.0, info.spec_b.1, info.belt_b)
        } else {
            // B is horizontal (surface), A is vertical (bridge)
            (&info.spec_b.0, info.spec_b.1, info.belt_b, &info.spec_a.0, info.spec_a.1, info.belt_a)
        };

    // Compute UG entry/exit positions for the bridged path
    let (dx, dy) = match bridge_dir {
        EntityDirection::North => (0, -1),
        EntityDirection::South => (0, 1),
        EntityDirection::East => (1, 0),
        EntityDirection::West => (-1, 0),
    };
    // UG-in is one tile BEFORE the crossing (opposite of travel direction)
    let ug_in = (cx - dx, cy - dy);
    // UG-out is one tile AFTER the crossing (in travel direction)
    let ug_out = (cx + dx, cy + dy);

    // Check that UG positions are not blocked
    if hard_obstacles.contains(&ug_in) || hard_obstacles.contains(&ug_out) {
        return None; // can't place template here
    }

    let ug_name = ug_for_belt(bridge_belt);
    let seg = Some(format!("junction:{}:{},{}", bridge_item, cx, cy));

    let mut entities = Vec::new();

    // Surface belt at the crossing tile (the non-bridged path)
    if !pre_existing.contains(&(cx, cy)) {
        entities.push(PlacedEntity {
            name: surface_belt.to_string(),
            x: cx,
            y: cy,
            direction: surface_dir,
            carries: Some(surface_item.clone()),
            segment_id: seg.clone(),
            ..Default::default()
        });
    }

    // UG-input (before crossing)
    if !pre_existing.contains(&ug_in) {
        entities.push(PlacedEntity {
            name: ug_name.to_string(),
            x: ug_in.0,
            y: ug_in.1,
            direction: bridge_dir,
            io_type: Some("input".to_string()),
            carries: Some(bridge_item.clone()),
            segment_id: seg.clone(),
            ..Default::default()
        });
    }

    // UG-output (after crossing)
    if !pre_existing.contains(&ug_out) {
        entities.push(PlacedEntity {
            name: ug_name.to_string(),
            x: ug_out.0,
            y: ug_out.1,
            direction: bridge_dir,
            io_type: Some("output".to_string()),
            carries: Some(bridge_item.clone()),
            segment_id: seg.clone(),
            ..Default::default()
        });
    }

    // Zone bbox: 3x3 centered on the crossing (or 3x1/1x3 oriented)
    let zone = ClusterZone {
        x: cx.min(ug_in.0).min(ug_out.0),
        y: cy.min(ug_in.1).min(ug_out.1),
        w: ((cx - ug_in.0).abs().max((cx - ug_out.0).abs()) * 2 + 1) as u32,
        h: ((cy - ug_in.1).abs().max((cy - ug_out.1).abs()) * 2 + 1) as u32,
    };

    Some((entities, zone))
}

/// Resolve ghost clusters into SAT crossing zones.
///
/// For each cluster: compute padded bbox, extract boundary ports from paths
/// that pass through the zone, build a CrossingZone, and SAT-solve it.
///
/// Returns solved entity lists per zone (with their bboxes), LayoutRegions
/// for telemetry, and the count of failed clusters.
fn resolve_clusters(
    crossing_list: &[(i32, i32)],
    uf: &mut [usize],
    routed_paths: &FxHashMap<String, Vec<(i32, i32)>>,
    specs: &[BeltSpec],
    max_belt_tier: Option<&str>,
    all_entities: &[PlacedEntity],
    hard_obstacles: &FxHashSet<(i32, i32)>,
) -> (Vec<(Vec<PlacedEntity>, ClusterZone)>, Vec<LayoutRegion>, usize) {
    let n_tiles = crossing_list.len();
    if n_tiles == 0 {
        return (Vec::new(), Vec::new(), 0);
    }

    // Build spec lookup: key → &BeltSpec
    let spec_map: FxHashMap<&str, &BeltSpec> = specs.iter().map(|s| (s.key.as_str(), s)).collect();

    // Group crossing tiles by UF root
    let mut cluster_tiles: FxHashMap<usize, Vec<(i32, i32)>> = FxHashMap::default();
    for (i, &tile) in crossing_list.iter().enumerate() {
        let root = uf_find(uf, i);
        cluster_tiles
            .entry(root)
            .or_default()
            .push(tile);
    }

    // Build zones from clusters
    let mut zones: Vec<(usize, ClusterZone, FxHashSet<(i32, i32)>)> = Vec::new();
    for (&root, tiles) in &cluster_tiles {
        let min_x = tiles.iter().map(|t| t.0).min().unwrap();
        let max_x = tiles.iter().map(|t| t.0).max().unwrap();
        let min_y = tiles.iter().map(|t| t.1).min().unwrap();
        let max_y = tiles.iter().map(|t| t.1).max().unwrap();

        // +2 padding on each side: gives the SAT solver room for UG
        // entry/exit pairs around each crossing point.
        let zone = ClusterZone {
            x: min_x - 2,
            y: min_y - 2,
            w: (max_x - min_x + 5) as u32,
            h: (max_y - min_y + 5) as u32,
        };
        let tile_set: FxHashSet<(i32, i32)> = tiles.iter().copied().collect();
        zones.push((root, zone, tile_set));
    }

    // Check for overlapping padded bboxes and merge if needed
    // (iterate until no merges happen)
    loop {
        let mut merged = false;
        let mut i = 0;
        while i < zones.len() {
            let mut j = i + 1;
            while j < zones.len() {
                // Check if zones[i] and zones[j] padded bboxes overlap
                let (_, zi, _) = &zones[i];
                let (_, zj, _) = &zones[j];
                let i_x2 = zi.x + zi.w as i32;
                let i_y2 = zi.y + zi.h as i32;
                let j_x2 = zj.x + zj.w as i32;
                let j_y2 = zj.y + zj.h as i32;

                if zi.x < j_x2 && zj.x < i_x2 && zi.y < j_y2 && zj.y < i_y2 {
                    // Merge j into i: compute merged bbox from immutable refs
                    let new_min_x = zi.x.min(zj.x);
                    let new_min_y = zi.y.min(zj.y);
                    let new_max_x = i_x2.max(j_x2);
                    let new_max_y = i_y2.max(j_y2);
                    let tiles_j = zones[j].2.clone();

                    // Now mutate zones[i]
                    zones[i].1 = ClusterZone {
                        x: new_min_x,
                        y: new_min_y,
                        w: (new_max_x - new_min_x) as u32,
                        h: (new_max_y - new_min_y) as u32,
                    };
                    zones[i].2.extend(tiles_j);
                    zones.remove(j);
                    merged = true;
                } else {
                    j += 1;
                }
            }
            i += 1;
        }
        if !merged {
            break;
        }
    }

    let mut solved_zones: Vec<(Vec<PlacedEntity>, ClusterZone)> = Vec::new();
    let mut regions: Vec<LayoutRegion> = Vec::new();
    let mut failed_count = 0;

    // Determine belt tier for SAT solver
    let effective_belt = max_belt_tier.unwrap_or("transport-belt");
    let max_reach = ug_max_reach(effective_belt);

    // Pre-existing entity positions — boundary ports must not land here
    let pre_existing_positions: FxHashSet<(i32, i32)> = all_entities
        .iter()
        .filter(|e| !e.segment_id.as_ref().is_some_and(|s| s.starts_with("ghost:")))
        .map(|e| (e.x, e.y))
        .collect();

    for (cluster_idx, (_root_id, zone, _cluster_tile_set)) in zones.into_iter().enumerate() {
        // Collect all paths that have any tile inside this zone's padded bbox
        let mut boundaries: Vec<ZoneBoundary> = Vec::new();
        let mut port_specs: Vec<PortSpec> = Vec::new();

        for (key, path) in routed_paths {
            let spec = match spec_map.get(key.as_str()) {
                Some(s) => s,
                None => continue,
            };

            // Check if this path intersects the zone at all
            let touches_zone = path.iter().any(|&(px, py)| zone.contains(px, py));
            if !touches_zone {
                continue;
            }

            // Walk the path and find entry/exit boundary ports
            for i in 0..path.len() {
                let (px, py) = path[i];
                if !zone.contains(px, py) {
                    continue;
                }

                // Entry: previous tile is outside the zone (or this is the first tile)
                let prev_outside = if i == 0 {
                    true
                } else {
                    let (ppx, ppy) = path[i - 1];
                    !zone.contains(ppx, ppy)
                };

                // Exit: next tile is outside the zone (or this is the last tile)
                let next_outside = if i == path.len() - 1 {
                    true
                } else {
                    let (npx, npy) = path[i + 1];
                    !zone.contains(npx, npy)
                };

                // Skip boundary ports at positions occupied by hard obstacles
                // or pre-existing entities — those would conflict with the
                // SAT solution or get filtered out, breaking connectivity.
                let occupied_by_existing =
                    hard_obstacles.contains(&(px, py)) || pre_existing_positions.contains(&(px, py));

                if prev_outside && zone.on_edge(px, py) && !occupied_by_existing {
                    // Entry port: direction is the direction of travel INTO the zone
                    let dir = if i == 0 {
                        if path.len() > 1 {
                            let (npx, npy) = path[i + 1];
                            step_direction(npx - px, npy - py)
                        } else {
                            if spec.start.0 <= spec.goal.0 {
                                EntityDirection::East
                            } else {
                                EntityDirection::West
                            }
                        }
                    } else {
                        let (ppx, ppy) = path[i - 1];
                        step_direction(px - ppx, py - ppy)
                    };

                    let edge = tile_edge(&zone, px, py);
                    let offset = edge_offset(&zone, &edge, px, py);
                    boundaries.push(ZoneBoundary {
                        x: px,
                        y: py,
                        direction: dir,
                        item: spec.item.clone(),
                        is_input: true,
                    });
                    port_specs.push(PortSpec {
                        edge,
                        offset,
                        io: PortIo::Input,
                        item: Some(spec.item.clone()),
                        direction: Some(dir),
                    });
                }

                if next_outside && zone.on_edge(px, py) && !occupied_by_existing {
                    // Exit port: direction of travel OUT of the zone
                    let dir = if i == path.len() - 1 {
                        if path.len() > 1 {
                            let (ppx, ppy) = path[i - 1];
                            step_direction(px - ppx, py - ppy)
                        } else {
                            if spec.start.0 <= spec.goal.0 {
                                EntityDirection::East
                            } else {
                                EntityDirection::West
                            }
                        }
                    } else {
                        let (npx, npy) = path[i + 1];
                        step_direction(npx - px, npy - py)
                    };

                    let edge = tile_edge(&zone, px, py);
                    let offset = edge_offset(&zone, &edge, px, py);
                    boundaries.push(ZoneBoundary {
                        x: px,
                        y: py,
                        direction: dir,
                        item: spec.item.clone(),
                        is_input: false,
                    });
                    port_specs.push(PortSpec {
                        edge,
                        offset,
                        io: PortIo::Output,
                        item: Some(spec.item.clone()),
                        direction: Some(dir),
                    });
                }
            }
        }

        // ── Flow-balance filter ──────────────────────────────────────
        // 1. Deduplicate ports at the same (x, y, direction, is_input).
        // 2. For each item, check that it has at least one input AND one
        //    output.  Drop all ports for items that are unbalanced — the
        //    SAT solver cannot route items that have no exit (it creates
        //    loops) or no entrance (dead ends).
        {
            // Dedup
            let mut seen: FxHashSet<(i32, i32, u8, bool)> = FxHashSet::default();
            let mut deduped_b: Vec<ZoneBoundary> = Vec::new();
            let mut deduped_p: Vec<PortSpec> = Vec::new();
            for (b, p) in boundaries.into_iter().zip(port_specs.into_iter()) {
                let dir_byte = match b.direction {
                    EntityDirection::North => 0,
                    EntityDirection::East => 1,
                    EntityDirection::South => 2,
                    EntityDirection::West => 3,
                };
                if seen.insert((b.x, b.y, dir_byte, b.is_input)) {
                    deduped_b.push(b);
                    deduped_p.push(p);
                }
            }

            boundaries = deduped_b;
            port_specs = deduped_p;
        }

        if boundaries.is_empty() {
            continue;
        }

        // Mark occupied tiles inside the zone as forced-empty so the SAT
        // solver doesn't place entities on top of hard obstacles (machines,
        // poles, pipes) or pre-existing belts (trunks, row templates).
        let boundary_set: FxHashSet<(i32, i32)> =
            boundaries.iter().map(|b| (b.x, b.y)).collect();
        let mut forced_empty_set: FxHashSet<(i32, i32)> = FxHashSet::default();

        // Hard obstacles (machines, poles, pipes, fluid lanes)
        for &(hx, hy) in hard_obstacles {
            if zone.contains(hx, hy) && !boundary_set.contains(&(hx, hy)) {
                forced_empty_set.insert((hx, hy));
            }
        }
        // Pre-existing non-ghost entities (trunks, row template belts, splitters)
        for e in all_entities {
            if zone.contains(e.x, e.y)
                && !e
                    .segment_id
                    .as_ref()
                    .is_some_and(|s| s.starts_with("ghost:"))
                && !boundary_set.contains(&(e.x, e.y))
            {
                forced_empty_set.insert((e.x, e.y));
            }
        }
        let forced_empty: Vec<(i32, i32)> = forced_empty_set.into_iter().collect();

        let crossing_zone = CrossingZone {
            x: zone.x,
            y: zone.y,
            width: zone.w,
            height: zone.h,
            boundaries: boundaries.clone(),
            forced_empty,
        };

        match sat::solve_crossing_zone(&crossing_zone, max_reach, effective_belt) {
            Some(solution) => {
                trace::emit(trace::TraceEvent::GhostClusterSolved {
                    cluster_id: cluster_idx,
                    zone_x: zone.x,
                    zone_y: zone.y,
                    zone_w: zone.w,
                    zone_h: zone.h,
                    boundary_count: boundaries.len(),
                    variables: solution.stats.variables,
                    clauses: solution.stats.clauses,
                    solve_time_us: solution.stats.solve_time_us,
                });

                let input_items: Vec<String> = boundaries
                    .iter()
                    .filter(|b| b.is_input)
                    .map(|b| b.item.clone())
                    .collect::<FxHashSet<_>>()
                    .into_iter()
                    .collect();
                let output_items: Vec<String> = boundaries
                    .iter()
                    .filter(|b| !b.is_input)
                    .map(|b| b.item.clone())
                    .collect::<FxHashSet<_>>()
                    .into_iter()
                    .collect();

                regions.push(LayoutRegion {
                    kind: "ghost_cluster".to_string(),
                    x: zone.x,
                    y: zone.y,
                    width: zone.w as i32,
                    height: zone.h as i32,
                    inputs: input_items,
                    outputs: output_items,
                    ports: port_specs,
                    variables: solution.stats.variables,
                    clauses: solution.stats.clauses,
                    solve_time_us: solution.stats.solve_time_us,
                });

                solved_zones.push((solution.entities, zone));
            }
            None => {
                trace::emit(trace::TraceEvent::GhostClusterFailed {
                    cluster_id: cluster_idx,
                    zone_x: zone.x,
                    zone_y: zone.y,
                    zone_w: zone.w,
                    zone_h: zone.h,
                    boundary_count: boundaries.len(),
                });
                failed_count += 1;
            }
        }
    }

    (solved_zones, regions, failed_count)
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
