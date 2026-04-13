//! Bus layout orchestrator: rows + bus lanes + poles -> LayoutResult.
//!
//! Port of `src/bus/layout.py`.

use std::cell::Cell;

use rustc_hash::{FxHashMap, FxHashSet};

thread_local! {
    /// Thread-local override for ghost routing mode. When `true`, the ghost
    /// router is used regardless of the `FUCKTORIO_GHOST_ROUTING` env var.
    /// WASM callers set this via `GhostModeGuard` because env vars don't
    /// work in the browser.
    static FORCE_GHOST_ROUTING: Cell<bool> = const { Cell::new(false) };
}

/// RAII guard that enables ghost routing for the duration of its lifetime.
/// Primarily used by the WASM bindings; native callers can also use it for
/// scoped overrides.
pub struct GhostModeGuard {
    prev: bool,
}

impl GhostModeGuard {
    pub fn new() -> Self {
        let prev = FORCE_GHOST_ROUTING.with(|c| c.replace(true));
        Self { prev }
    }
}

impl Default for GhostModeGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for GhostModeGuard {
    fn drop(&mut self) {
        let prev = self.prev;
        FORCE_GHOST_ROUTING.with(|c| c.set(prev));
    }
}

pub(crate) fn is_ghost_routing_forced() -> bool {
    FORCE_GHOST_ROUTING.with(|c| c.get())
}

use crate::models::{EntityDirection, LayoutResult, PlacedEntity, SolverResult};
use crate::bus::bus_router::{
    plan_bus_lanes, bus_width_for_lanes, stamp_family_balancer, render_family_input_paths,
    merge_output_rows, place_merger_block, route_lane, negotiate_and_route,
    BusLane, DroppedBridge, LaneFamily, MACHINE_ENTITIES,
};
use crate::bus::placer::{place_rows, RowSpan};
use crate::bus::plan::{plan_layout, apply_dropped_to_gaps, extract_and_solve_crossings, PlanError};

/// Convert a SolverResult into a bus-style LayoutResult.
///
/// Returns a LayoutResult with:
/// - entities: all belts, inserters, machines, power poles
/// - width: maximum x dimension used
/// - height: maximum y dimension used
pub fn build_bus_layout(
    solver_result: &SolverResult,
    max_belt_tier: Option<&str>,
) -> Result<LayoutResult, String> {
    // Final product items get EAST-flowing output belts (merge at right side)
    let final_output_items: FxHashSet<String> = solver_result
        .external_outputs
        .iter()
        .filter(|ext| !ext.is_fluid)
        .map(|ext| ext.item.clone())
        .collect();

    let bus_header = 1;

    crate::trace::emit(crate::trace::TraceEvent::SolverCompleted {
        recipe_count: solver_result.machines.len(),
        machine_count: solver_result.machines.iter().map(|m| m.count.ceil() as usize).sum(),
        external_input_count: solver_result.external_inputs.len(),
        external_output_count: solver_result.external_outputs.len(),
        machines: solver_result.machines.iter().map(|m| crate::trace::MachineTrace {
            recipe: m.recipe.clone(),
            machine: m.entity.clone(),
            count: m.count,
            rate: m.outputs.iter().map(|o| o.rate).sum::<f64>() * m.count,
        }).collect(),
    });

    // First pass: place rows with temp bus width
    let temp_bw = estimate_bus_width(solver_result);
    #[cfg(not(target_arch = "wasm32"))]
    let t_place1 = std::time::Instant::now();
    let (row_entities, row_spans, row_width, total_height) = place_rows(
        &solver_result.machines,
        &solver_result.dependency_order,
        temp_bw,
        bus_header,
        max_belt_tier,
        Some(&final_output_items),
        None,
    );

    #[cfg(not(target_arch = "wasm32"))]
    crate::trace::emit(crate::trace::TraceEvent::PhaseTime { phase: "place_rows_1".to_string(), duration_ms: t_place1.elapsed().as_millis() as u64 });
    #[cfg(not(target_arch = "wasm32"))]
    let t_plan1 = std::time::Instant::now();
    let (lanes, families) = plan_bus_lanes(solver_result, &row_spans, max_belt_tier)?;
    #[cfg(not(target_arch = "wasm32"))]
    crate::trace::emit(crate::trace::TraceEvent::PhaseTime { phase: "plan_bus_lanes_1".to_string(), duration_ms: t_plan1.elapsed().as_millis() as u64 });
    let actual_bw = bus_width_for_lanes(&lanes);

    // Compute extra gaps needed for balancer blocks. The retry loop below
    // may add to this map when `route_bus` reports dropped UG bridges.
    let mut extra_gaps = compute_extra_gaps(&families);

    // Retry loop: place_rows (pass 2) → plan_bus_lanes (pass 2) → route_bus.
    //
    // When `route_bus` reports dropped UG bridges (the filter in
    // `route_belt_lane`/`route_intermediate_lane` couldn't bridge because
    // the bridge output would collide with the trunk's own tap-off), we
    // translate those drops into `extra_gap_after_row` entries that push
    // the conflicting row down by 1, freeing the tile for the bridge, and
    // retry the second pass. Capped at MAX_BRIDGE_RETRIES to prevent
    // pathological cases from looping indefinitely.
    const MAX_BRIDGE_RETRIES: u32 = 3;

    let mut cur_row_entities = row_entities;
    let mut cur_row_spans = row_spans;
    let mut cur_row_width = row_width;
    let mut cur_total_height = total_height;
    let mut cur_lanes = lanes;
    let mut cur_families = families;

    // Assigned inside the retry loop (every iteration overwrites).
    let mut bus_entities: Vec<PlacedEntity>;
    let mut max_y: i32;
    let mut merge_max_x: i32;
    let mut regions: Vec<crate::models::LayoutRegion>;
    // Extra warnings surfaced by the ghost router (direct/bare modes
    // downgrade some hard errors to warnings so layouts still render).
    let mut ghost_warnings: Vec<String> = Vec::new();
    // Pole entities computed from row positions before routing so poles are
    // visible to the router as hard obstacles. Updated each loop iteration
    // when rows are re-placed. The initial value is immediately overwritten
    // on the first loop iteration before any read.
    #[allow(unused_assignments)]
    let mut pole_entities: Vec<PlacedEntity> = Vec::new();

    let mut attempt: u32 = 0;
    loop {
        // Re-place rows + re-plan lanes if this is a retry, or on first
        // iteration if the initial bus width / extra_gaps changed.
        let need_replace = attempt > 0 || actual_bw != temp_bw || !extra_gaps.is_empty();
        if need_replace {
            #[cfg(not(target_arch = "wasm32"))]
            let t_place2 = std::time::Instant::now();
            let (re, rs, rw, th) = place_rows(
                &solver_result.machines,
                &solver_result.dependency_order,
                actual_bw,
                bus_header,
                max_belt_tier,
                Some(&final_output_items),
                Some(&extra_gaps),
            );
            cur_row_entities = re;
            cur_row_spans = rs;
            cur_row_width = rw;
            cur_total_height = th;
            #[cfg(not(target_arch = "wasm32"))]
            crate::trace::emit(crate::trace::TraceEvent::PhaseTime {
                phase: format!("place_rows_2_attempt_{}", attempt),
                duration_ms: t_place2.elapsed().as_millis() as u64,
            });

            #[cfg(not(target_arch = "wasm32"))]
            let t_plan2 = std::time::Instant::now();
            let (nl, nf) = plan_bus_lanes(solver_result, &cur_row_spans, max_belt_tier)?;
            cur_lanes = nl;
            cur_families = nf;
            #[cfg(not(target_arch = "wasm32"))]
            crate::trace::emit(crate::trace::TraceEvent::PhaseTime {
                phase: format!("plan_bus_lanes_2_attempt_{}", attempt),
                duration_ms: t_plan2.elapsed().as_millis() as u64,
            });
        }

        if attempt == 0 {
            crate::trace::emit(crate::trace::TraceEvent::PhaseComplete {
                phase: "rows_placed".into(),
                entity_count: cur_row_entities.len(),
            });
            if crate::trace::is_active() {
                crate::trace::emit(crate::trace::TraceEvent::PhaseSnapshot {
                    phase: "rows_placed".into(),
                    entities: cur_row_entities.clone(),
                    width: cur_row_width.max(actual_bw),
                    height: cur_total_height,
                });
            }
            crate::trace::emit(crate::trace::TraceEvent::PhaseComplete {
                phase: "lanes_planned".into(),
                entity_count: cur_row_entities.len(),
            });
            if crate::trace::is_active() {
                crate::trace::emit(crate::trace::TraceEvent::PhaseSnapshot {
                    phase: "lanes_planned".into(),
                    entities: cur_row_entities.clone(),
                    width: cur_row_width.max(actual_bw),
                    height: cur_total_height,
                });
            }
        }

        // Phase 3c: build the global routing plan. `plan_layout` resolves
        // pre-A* bridge conflicts (foreign tap-offs crossing this trunk) and
        // either returns an empty-drops Plan or a `DroppedBridges` error
        // with the unbridgeable ranges. On error we short-circuit the
        // route_bus call — translate the drops into extra_gap updates and
        // retry the pipeline from place_rows. This is a faster retry path
        // than letting route_bus run A* first and then dropping.
        match plan_layout(&cur_lanes, &cur_row_spans) {
            Ok(()) => {}
            Err(PlanError::DroppedBridges { dropped: pre_drops }) => {
                for db in &pre_drops {
                    crate::trace::emit(crate::trace::TraceEvent::BridgeDropped {
                        trunk_item: db.trunk_item.clone(),
                        trunk_x: db.trunk_x,
                        range_start: db.range.0,
                        range_end: db.range.1,
                        colliding_tap_y: db.colliding_tap_y(),
                    });
                }
                if attempt >= MAX_BRIDGE_RETRIES {
                    crate::trace::emit(crate::trace::TraceEvent::BridgeRetryExhausted {
                        final_dropped_count: pre_drops.len(),
                        max_retries: MAX_BRIDGE_RETRIES,
                    });
                    // Fall through to route_bus so the validator can render
                    // the best-effort layout — same failure mode as the
                    // post-route_bus path.
                }
                let gap_updates = apply_dropped_to_gaps(&pre_drops, &cur_row_spans, &mut extra_gaps);
                if gap_updates == 0 || attempt >= MAX_BRIDGE_RETRIES {
                    // No actionable updates — route_bus and let the
                    // post-A* retry handle any remaining drops.
                } else {
                    attempt += 1;
                    crate::trace::emit(crate::trace::TraceEvent::BridgeRetry {
                        attempt,
                        dropped_count: pre_drops.len(),
                        extra_gap_updates: gap_updates,
                    });
                    continue;
                }
            }
        }

        // Place power poles from machine positions before routing so the router
        // sees them as hard obstacles. The occupied set includes row-entity tiles
        // AND planned fluid-lane columns — the router hasn't placed pipe/PTG
        // entities yet, but those columns will be occupied once routing runs, so
        // poles must avoid them now.
        {
            let mut row_occupied: FxHashSet<(i32, i32)> = FxHashSet::default();
            let mut machines_for_poles: Vec<(i32, i32, i32)> = Vec::new();
            for ent in &cur_row_entities {
                if MACHINE_ENTITIES.contains(&ent.name.as_str()) {
                    let sz = crate::common::machine_size(&ent.name) as i32;
                    for dx in 0..sz {
                        for dy in 0..sz {
                            row_occupied.insert((ent.x + dx, ent.y + dy));
                        }
                    }
                    machines_for_poles.push((ent.x + sz / 2, ent.y, sz));
                } else {
                    row_occupied.insert((ent.x, ent.y));
                }
            }
            // Reserve fluid lane tiles so poles don't land on pipe/PTG
            // entities the router will place later. Covers:
            //   1. Vertical trunk: pipe/PTG at lane.x on connection ys
            //   2. Horizontal port connections: the port tile itself and
            //      the trunk-side pipe tile at port_y. The underground
            //      stretch between them is empty on the surface, so we
            //      don't reserve it (doing so over-constrains pole
            //      placement and breaks coverage near oil refineries).
            for lane in &cur_lanes {
                if !lane.is_fluid {
                    continue;
                }
                // Vertical trunk: reserve connection ys + PTG bridge tiles.
                let mut trunk_ys: Vec<i32> = vec![lane.source_y];
                trunk_ys.extend(lane.tap_off_ys.iter().copied());
                for &(_ri, _px, py) in &lane.fluid_output_port_positions {
                    trunk_ys.push(py);
                }
                trunk_ys.sort_unstable();
                trunk_ys.dedup();
                // Surface pipe at each connection y, plus PTG entry/exit
                // tiles between adjacent connections (±1 from each anchor).
                for &y in &trunk_ys {
                    row_occupied.insert((lane.x, y));
                }
                for pair in trunk_ys.windows(2) {
                    let (y0, y1) = (pair[0], pair[1]);
                    if y1 - y0 > 1 {
                        row_occupied.insert((lane.x, y0 + 1)); // PTG input
                    }
                    if y1 - y0 > 2 {
                        row_occupied.insert((lane.x, y1 - 1)); // PTG output
                    }
                }
                // Horizontal port connections: just the port tile and its
                // immediate neighbours (PTG entry/exit).
                let all_ports = lane.fluid_port_positions.iter()
                    .chain(lane.fluid_output_port_positions.iter());
                for &(_ri, port_x, port_y) in all_ports {
                    row_occupied.insert((port_x, port_y));
                    row_occupied.insert((lane.x, port_y)); // trunk side
                    // PTG entry/exit tiles (±1 from each anchor)
                    let (lo, hi) = if port_x < lane.x {
                        (port_x, lane.x)
                    } else {
                        (lane.x, port_x)
                    };
                    if hi - lo > 1 {
                        row_occupied.insert((lo + 1, port_y));
                    }
                    if hi - lo > 2 {
                        row_occupied.insert((hi - 1, port_y));
                    }
                }
            }
            let pole_strategy = if machines_for_poles.is_empty() { "empty" } else { "rows" };
            pole_entities = place_poles(&machines_for_poles, &row_occupied);
            crate::trace::emit(crate::trace::TraceEvent::PolesPlaced {
                count: pole_entities.len(),
                strategy: pole_strategy.to_string(),
            });
            crate::trace::emit(crate::trace::TraceEvent::PhaseComplete {
                phase: "poles_placed".into(),
                entity_count: pole_entities.len(),
            });
        }

        // Combine row entities with pre-placed poles so the router treats
        // pole tiles as hard obstacles from the start.
        let mut row_entities_with_poles = cur_row_entities.clone();
        row_entities_with_poles.extend(pole_entities.clone());

        // Route bus lanes
        let use_ghost = is_ghost_routing_forced()
            || std::env::var("FUCKTORIO_GHOST_ROUTING")
                .is_ok_and(|v| v == "1");

        if use_ghost {
            #[cfg(not(target_arch = "wasm32"))]
            let t_ghost = std::time::Instant::now();
            let ghost_result = crate::bus::ghost_router::route_bus_ghost(
                &cur_lanes,
                &cur_row_spans,
                cur_total_height,
                actual_bw,
                max_belt_tier,
                solver_result,
                &cur_families,
                &cur_row_entities,
            )?;
            bus_entities = ghost_result.entities;
            max_y = ghost_result.max_y;
            merge_max_x = ghost_result.merge_max_x;
            regions = ghost_result.regions;
            ghost_warnings.extend(ghost_result.warnings);
            #[cfg(not(target_arch = "wasm32"))]
            crate::trace::emit(crate::trace::TraceEvent::PhaseTime {
                phase: "ghost_routing".to_string(),
                duration_ms: t_ghost.elapsed().as_millis() as u64,
            });
            break; // ghost routing is single-pass — no retry loop needed
        }

        #[cfg(not(target_arch = "wasm32"))]
        let t_route_bus = std::time::Instant::now();
        let mut dropped: Vec<crate::bus::bus_router::DroppedBridge> = Vec::new();
        let (be, my, mx, rg) = route_bus(
            &cur_lanes,
            &cur_row_spans,
            cur_total_height,
            actual_bw,
            max_belt_tier,
            solver_result,
            &cur_families,
            &row_entities_with_poles,
            &mut dropped,
        )?;
        bus_entities = be;
        max_y = my;
        merge_max_x = mx;
        regions = rg;
        #[cfg(not(target_arch = "wasm32"))]
        crate::trace::emit(crate::trace::TraceEvent::PhaseTime {
            phase: format!("route_bus_attempt_{}", attempt),
            duration_ms: t_route_bus.elapsed().as_millis() as u64,
        });

        // Emit dropped-bridge trace events regardless of retry outcome so
        // the debug panel can see them even if we eventually succeed.
        for db in &dropped {
            crate::trace::emit(crate::trace::TraceEvent::BridgeDropped {
                trunk_item: db.trunk_item.clone(),
                trunk_x: db.trunk_x,
                range_start: db.range.0,
                range_end: db.range.1,
                colliding_tap_y: db.colliding_tap_y(),
            });
        }

        if dropped.is_empty() {
            break;
        }

        if attempt >= MAX_BRIDGE_RETRIES {
            crate::trace::emit(crate::trace::TraceEvent::BridgeRetryExhausted {
                final_dropped_count: dropped.len(),
                max_retries: MAX_BRIDGE_RETRIES,
            });
            break;
        }

        // Translate each dropped bridge into an extra_gap_after_row update.
        // For each drop, find the row whose input_belt_y or output_belt_y
        // equals the colliding tap y, then add a gap AFTER the previous row
        // to push the colliding row down by one tile. If the colliding row
        // is row 0 there's no predecessor, so the drop is unresolvable here
        // and we lose progress for that one (other drops may still help).
        let mut gap_updates: usize = 0;
        for db in &dropped {
            let colliding_y = db.colliding_tap_y();
            let row_idx_opt = cur_row_spans.iter().position(|rs| {
                rs.input_belt_y.contains(&colliding_y) || rs.output_belt_y == colliding_y
            });
            if let Some(row_idx) = row_idx_opt {
                if row_idx > 0 {
                    let target = row_idx - 1;
                    let cur = extra_gaps.entry(target).or_insert(0);
                    *cur += 1;
                    gap_updates += 1;
                }
            }
        }

        attempt += 1;
        crate::trace::emit(crate::trace::TraceEvent::BridgeRetry {
            attempt,
            dropped_count: dropped.len(),
            extra_gap_updates: gap_updates,
        });

        if gap_updates == 0 {
            // Nothing actionable — further retries can't make progress.
            crate::trace::emit(crate::trace::TraceEvent::BridgeRetryExhausted {
                final_dropped_count: dropped.len(),
                max_retries: MAX_BRIDGE_RETRIES,
            });
            break;
        }
    }

    // Only row_entities, row_width, and families are read past this point;
    // row_spans, total_height, and lanes are consumed inside the retry loop.
    let row_entities = cur_row_entities;
    let row_width = cur_row_width;
    let families = cur_families;
    crate::trace::emit(crate::trace::TraceEvent::PhaseComplete {
        phase: "bus_routed".into(),
        entity_count: bus_entities.len(),
    });
    emit_inter_row_bands(&cur_row_spans, &cur_lanes);
    if crate::trace::is_active() {
        let mut snap_entities = row_entities.clone();
        snap_entities.extend(bus_entities.clone());
        crate::trace::emit(crate::trace::TraceEvent::PhaseSnapshot {
            phase: "bus_routed".into(),
            entities: snap_entities,
            width: row_width.max(actual_bw).max(merge_max_x),
            height: max_y,
        });
    }

    // Remove row entities that overlap with bus splitters
    let splitter_names: FxHashSet<&str> = ["splitter", "fast-splitter", "express-splitter"]
        .iter()
        .copied()
        .collect();
    let mut bus_occupied: FxHashSet<(i32, i32)> = FxHashSet::default();
    for ent in &bus_entities {
        if splitter_names.contains(ent.name.as_str()) {
            bus_occupied.insert((ent.x, ent.y));
            if matches!(ent.direction, EntityDirection::West | EntityDirection::East) {
                bus_occupied.insert((ent.x, ent.y + 1));
            } else {
                bus_occupied.insert((ent.x + 1, ent.y));
            }
        }
    }
    let row_entities: Vec<PlacedEntity> = if bus_occupied.is_empty() {
        row_entities
    } else {
        row_entities.into_iter().filter(|e| !bus_occupied.contains(&(e.x, e.y))).collect()
    };

    let width = row_width.max(actual_bw).max(merge_max_x);

    // Emit a post-routing snapshot showing poles already placed before routing.
    if crate::trace::is_active() {
        let mut snap_entities = row_entities.clone();
        snap_entities.extend(bus_entities.clone());
        snap_entities.extend(pole_entities.clone());
        crate::trace::emit(crate::trace::TraceEvent::PhaseSnapshot {
            phase: "poles_placed".into(),
            entities: snap_entities,
            width,
            height: max_y,
        });
    }

    // Check for missing balancer templates and collect warnings
    let mut warnings = ghost_warnings;
    let templates = crate::bus::balancer_library::balancer_templates();
    for fam in &families {
        let (n, m) = (fam.shape.0 as u32, fam.shape.1 as u32);
        let has_direct = templates.contains_key(&(n, m));
        let has_decomp = (1..=n).rev().any(|g| {
            n % g == 0 && m % g == 0 && templates.contains_key(&(n / g, m / g))
        });
        if !has_direct && !has_decomp {
            warnings.push(format!(
                "No {}→{} balancer template for {}; producer outputs are disconnected",
                n, m, fam.item
            ));
        }
    }

    // Combine all entities: row_entities + bus_entities + pole_entities
    let mut all_entities = Vec::new();
    all_entities.extend(row_entities);
    all_entities.extend(bus_entities);
    all_entities.extend(pole_entities);

    Ok(LayoutResult {
        entities: all_entities,
        width,
        height: max_y,
        warnings,
        regions,
        trace: None,
    })
}

/// Traced variant of [`build_bus_layout`].
///
/// Collects structured trace events through all pipeline phases and returns
/// them in `LayoutResult.trace`. Zero overhead when using the non-traced entry point.
pub fn build_bus_layout_traced(
    solver_result: &SolverResult,
    max_belt_tier: Option<&str>,
) -> Result<LayoutResult, String> {
    let _guard = crate::trace::start_trace();
    let mut result = build_bus_layout(solver_result, max_belt_tier)?;
    result.trace = Some(crate::trace::drain_events());
    Ok(result)
}

/// Estimate bus width before full lane planning.
fn estimate_bus_width(solver_result: &SolverResult) -> i32 {
    // Count external solid inputs
    let n_external = solver_result
        .external_inputs
        .iter()
        .filter(|f| !f.is_fluid)
        .count() as i32;

    // Count intermediate items (items produced and consumed internally)
    let mut produced = FxHashSet::default();
    let mut consumed = FxHashSet::default();

    for m in &solver_result.machines {
        for out in &m.outputs {
            if !out.is_fluid {
                produced.insert(out.item.clone());
            }
        }
        for inp in &m.inputs {
            if !inp.is_fluid {
                consumed.insert(inp.item.clone());
            }
        }
    }

    let n_intermediate = produced.intersection(&consumed).count() as i32;
    let n_lanes = n_external + n_intermediate;
    (2).max(n_lanes * 2 + 1)
}

/// Compute extra gaps needed for balancer blocks.
fn compute_extra_gaps(families: &[LaneFamily]) -> FxHashMap<usize, i32> {
    let mut extra: FxHashMap<usize, i32> = FxHashMap::default();

    for fam in families {
        if fam.producer_rows.is_empty() {
            continue;
        }

        let n_producers = fam.shape.0;
        // Get template height from balancer library
        let (n, m) = (fam.shape.0 as u32, fam.shape.1 as u32);
        let templates = crate::bus::balancer_library::balancer_templates();
        let template_height = templates.get(&(n, m)).map(|t| t.height as i32)
            .or_else(|| {
                // Decomposition: find divisor g where (n/g, m/g) has a template.
                (1..=n).rev().find_map(|g| {
                    if n % g == 0 && m % g == 0 {
                        templates.get(&(n / g, m / g)).map(|t| t.height as i32)
                    } else {
                        None
                    }
                })
            })
            .unwrap_or(3);

        let needed = if n_producers == 1 {
            (template_height - 3).max(0)
        } else {
            (template_height - 2).max(0)
        };

        if needed == 0 {
            continue;
        }

        let last_producer = *fam.producer_rows.iter().max().unwrap();
        extra
            .entry(last_producer)
            .and_modify(|v| *v = (*v).max(needed))
            .or_insert(needed);
    }

    extra
}

/// Route all bus lanes and place belt entities.
fn route_bus(
    lanes: &[BusLane],
    row_spans: &[RowSpan],
    total_height: i32,
    bw: i32,
    max_belt_tier: Option<&str>,
    solver_result: &SolverResult,
    families: &[LaneFamily],
    row_entities: &[PlacedEntity],
    dropped_bridges: &mut Vec<DroppedBridge>,
) -> Result<(Vec<PlacedEntity>, i32, i32, Vec<crate::models::LayoutRegion>), String> {
    let mut entities: Vec<PlacedEntity> = Vec::new();
    let mut max_y = total_height;
    let mut merge_max_x = 0;

    // SAT crossing zones: trunk-only approach. SAT determines how trunks
    // handle crossings (UG bridges). The A* routes tap-offs normally (underground).
    // SAT zones have forced-empty tiles at tap_y so trunks bridge around them.
    #[cfg(not(target_arch = "wasm32"))]
    let sat_start = std::time::Instant::now();
    let (solved_crossings, mut crossing_tiles) =
        extract_and_solve_crossings(lanes, max_belt_tier);
    #[cfg(not(target_arch = "wasm32"))]
    crate::trace::emit(crate::trace::TraceEvent::PhaseTime {
        phase: "sat_crossing_zones".to_string(),
        duration_ms: sat_start.elapsed().as_millis() as u64,
    });
    for sc in &solved_crossings {
        crate::trace::emit(crate::trace::TraceEvent::CrossingZoneSolved {
            x: sc.zone.x,
            y: sc.zone.y,
            width: sc.zone.width,
            height: sc.zone.height,
            solve_time_us: sc.solution.stats.solve_time_us,
        });
        entities.extend(sc.solution.entities.clone());
    }

    #[cfg(not(target_arch = "wasm32"))]
    let negotiate_start = std::time::Instant::now();
    let routed_paths = negotiate_and_route(
        lanes,
        row_spans,
        total_height,
        bw,
        row_entities,
        solver_result,
        families,
        max_belt_tier,
    );
    #[cfg(not(target_arch = "wasm32"))]
    crate::trace::emit(crate::trace::TraceEvent::PhaseTime {
        phase: "negotiate_astar".to_string(),
        duration_ms: negotiate_start.elapsed().as_millis() as u64,
    });

    // Stamp N-to-M balancer blocks
    for fam in families {
        let balancer_ents = stamp_family_balancer(fam, max_belt_tier)
            .map_err(|e| format!("balancer stamp failed for family {:?}: {}", fam.shape, e))?;
        let template_found = !balancer_ents.is_empty();
        crate::trace::emit(crate::trace::TraceEvent::BalancerStamped {
            item: fam.item.clone(),
            shape: fam.shape,
            y_start: fam.balancer_y_start,
            y_end: fam.balancer_y_end,
            template_found,
        });
        entities.extend(balancer_ents);

        let path_ents = render_family_input_paths(
            fam,
            row_spans,
            crate::common::belt_entity_for_rate(fam.total_rate, max_belt_tier),
            Some(&routed_paths),
            bw,
        )
        .map_err(|e| format!("render family input paths failed for family {:?}: {}", fam.shape, e))?;
        entities.extend(path_ents);
    }

    // Build tap-off tile set from routed paths: positions where the A*
    // gave priority to tap-offs. Trunks must bridge around these.
    let mut tapoff_tiles: FxHashSet<(i32, i32)> = FxHashSet::default();
    for (key, path) in &routed_paths {
        if key.starts_with("tap:") {
            for &(px, py) in path {
                tapoff_tiles.insert((px, py));
            }
        }
    }

    // Collect splitter stamp right-half positions BEFORE routing, so we can
    // remove conflicting SAT crossing zones and adjust crossing_tiles.
    let mut splitter_stamp_tiles: FxHashSet<(i32, i32)> = FxHashSet::default();
    for lane in lanes {
        if lane.is_fluid || lane.tap_off_ys.len() <= 1 {
            continue;
        }
        let last_tap = lane.tap_off_ys.iter().copied().max();
        for &ty in &lane.tap_off_ys {
            if Some(ty) != last_tap {
                splitter_stamp_tiles.insert((lane.x + 1, ty - 1));
                splitter_stamp_tiles.insert((lane.x + 1, ty));
            }
        }
    }

    // Find SAT crossing zone segments that conflict with splitter stamps,
    // remove their entities, and strip their tiles from crossing_tiles so
    // the foreign_trunk_skip mechanism can bridge there instead.
    if !splitter_stamp_tiles.is_empty() {
        let conflicting_segs: FxHashSet<String> = entities.iter()
            .filter(|e| splitter_stamp_tiles.contains(&(e.x, e.y)))
            .filter_map(|e| e.segment_id.as_ref())
            .filter(|sid| sid.starts_with("crossing:"))
            .cloned()
            .collect();
        if !conflicting_segs.is_empty() {
            // Emit a trace event for each conflict before removing
            for e in entities.iter()
                .filter(|e| splitter_stamp_tiles.contains(&(e.x, e.y)))
                .filter(|e| matches!(&e.segment_id, Some(sid) if sid.starts_with("crossing:")))
            {
                if let Some(sid) = &e.segment_id {
                    crate::trace::emit(crate::trace::TraceEvent::CrossingZoneConflict {
                        segment_id: sid.clone(),
                        conflict_x: e.x,
                        conflict_y: e.y,
                    });
                }
            }
            // Remove conflicting crossing zone entities
            entities.retain(|e| {
                !matches!(&e.segment_id, Some(sid) if conflicting_segs.contains(sid))
            });
            // Rebuild crossing_tiles from remaining crossing entities so
            // the foreign_trunk_skip mechanism can bridge the gap instead.
            let remaining_tiles: FxHashSet<(i32, i32)> = entities.iter()
                .filter(|e| matches!(&e.segment_id, Some(sid) if sid.starts_with("crossing:")))
                .map(|e| (e.x, e.y))
                .collect();
            crossing_tiles = crate::bus::bus_router::CrossingTileSet::from_tiles(remaining_tiles);
        }
    }

    // Route each lane, skipping tiles owned by SAT crossing zones
    // and tiles claimed by A* tap-offs.
    #[cfg(not(target_arch = "wasm32"))]
    let route_lanes_start = std::time::Instant::now();
    for lane in lanes {
        let entity_count_before = entities.len();
        route_lane(&mut entities, lane, lanes, row_spans, bw, max_belt_tier, Some(&routed_paths), &crossing_tiles, &tapoff_tiles, dropped_bridges);
        let new_entities = entities.len() - entity_count_before;
        let has_tapoffs = !lane.tap_off_ys.is_empty();
        crate::trace::emit(crate::trace::TraceEvent::LaneRouted {
            item: lane.item.clone(),
            x: lane.x,
            is_fluid: lane.is_fluid,
            trunk_segments: new_entities,
            tapoffs: if has_tapoffs { lane.tap_off_ys.len() } else { 0 },
        });
    }
    #[cfg(not(target_arch = "wasm32"))]
    crate::trace::emit(crate::trace::TraceEvent::PhaseTime {
        phase: "route_all_lanes".to_string(),
        duration_ms: route_lanes_start.elapsed().as_millis() as u64,
    });

    // Remove non-SAT entities at SAT entity positions (SAT entities win).
    if !crossing_tiles.is_empty() {
        entities.retain(|e| {
            if !crossing_tiles.has_entity(&(e.x, e.y)) {
                return true;
            }
            matches!(&e.segment_id, Some(sid) if sid.starts_with("crossing:"))
        });
    }

    // Merge split lanes if needed
    let mut item_lane_groups: FxHashMap<String, Vec<&BusLane>> = FxHashMap::default();
    for lane in lanes {
        if !lane.is_fluid {
            item_lane_groups
                .entry(lane.item.clone())
                .or_default()
                .push(lane);
        }
    }

    for (_item, group) in item_lane_groups.iter() {
        if group.len() <= 1 {
            continue;
        }
        // Skip merger if all lanes have consumers
        if group.iter().all(|ln| !ln.consumer_rows.is_empty()) {
            continue;
        }
        let group_lanes: Vec<BusLane> = group.iter().map(|&l| l.clone()).collect();
        let (merger_ents, merger_end_y) =
            place_merger_block(&group_lanes, row_spans, max_y, &entities, max_belt_tier);
        crate::trace::emit(crate::trace::TraceEvent::MergerBlockPlaced {
            item: group_lanes[0].item.clone(),
            lanes: group_lanes.len(),
            block_y: max_y,
            block_height: merger_end_y - max_y,
        });
        entities.extend(merger_ents);
        max_y = max_y.max(merger_end_y);
    }

    // Merge output belts for final products
    let output_items: FxHashSet<String> = solver_result
        .external_outputs
        .iter()
        .filter(|ext| !ext.is_fluid)
        .map(|ext| ext.item.clone())
        .collect();

    for item in output_items {
        let output_rows: Vec<usize> = row_spans
            .iter()
            .enumerate()
            .filter(|(_, rs)| rs.spec.outputs.iter().any(|o| o.item == item && !o.is_fluid))
            .map(|(i, _)| i)
            .collect();

        if !output_rows.is_empty() {
            let (merge_ents, merge_end_y, item_merge_x) =
                merge_output_rows(&output_rows, &item, row_spans, max_y, max_belt_tier);
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

    // Build LayoutRegion metadata from solved crossing zones.
    let regions: Vec<crate::models::LayoutRegion> = solved_crossings
        .iter()
        .map(|sc| {
            let inputs: Vec<String> = sc.zone.boundaries.iter()
                .filter(|b| b.is_input)
                .map(|b| b.item.clone())
                .collect();
            let outputs: Vec<String> = sc.zone.boundaries.iter()
                .filter(|b| !b.is_input)
                .map(|b| b.item.clone())
                .collect();

            // Convert each boundary to a PortSpec relative to the zone's top-left.
            // Boundary tiles are INSIDE the zone (0 <= lx < width, 0 <= ly < height)
            // and sit on one of the four edge rows/columns. We classify by which
            // edge they're nearest to, using the belt direction as a tiebreaker for
            // corner tiles.
            let ports: Vec<crate::models::PortSpec> = sc.zone.boundaries.iter()
                .filter_map(|b| {
                    let lx = (b.x - sc.zone.x) as u32;
                    let ly = (b.y - sc.zone.y) as u32;
                    let w = sc.zone.width;
                    let h = sc.zone.height;
                    if lx >= w || ly >= h {
                        return None;
                    }
                    let io = if b.is_input {
                        crate::models::PortIo::Input
                    } else {
                        crate::models::PortIo::Output
                    };
                    // A boundary tile is on the edge of the zone. Determine which
                    // edge by checking whether it's in the top/bottom row or
                    // left/right column. Use the belt flow direction to break ties.
                    let on_north = ly == 0;
                    let on_south = ly == h - 1;
                    let on_west  = lx == 0;
                    let on_east  = lx == w - 1;

                    let edge = match (on_north, on_south, on_west, on_east) {
                        (true, false, false, false) => crate::models::PortEdge::N,
                        (false, true, false, false) => crate::models::PortEdge::S,
                        (false, false, true, false) => crate::models::PortEdge::W,
                        (false, false, false, true) => crate::models::PortEdge::E,
                        // Corner or centre tile: use belt direction to classify.
                        _ => {
                            use crate::models::EntityDirection;
                            match b.direction {
                                EntityDirection::North => crate::models::PortEdge::N,
                                EntityDirection::South => crate::models::PortEdge::S,
                                EntityDirection::West  => crate::models::PortEdge::W,
                                EntityDirection::East  => crate::models::PortEdge::E,
                            }
                        }
                    };

                    let offset = match edge {
                        crate::models::PortEdge::N | crate::models::PortEdge::S => lx,
                        crate::models::PortEdge::W | crate::models::PortEdge::E => ly,
                    };

                    Some(crate::models::PortSpec { edge, offset, io, item: None, direction: None })
                })
                .collect();

            let region = crate::models::LayoutRegion {
                kind: "crossing_zone".to_string(),
                x: sc.zone.x,
                y: sc.zone.y,
                width: sc.zone.width as i32,
                height: sc.zone.height as i32,
                inputs,
                outputs,
                ports,
                variables: sc.solution.stats.variables,
                clauses: sc.solution.stats.clauses,
                solve_time_us: sc.solution.stats.solve_time_us,
            };
            #[cfg(not(target_arch = "wasm32"))]
            crate::zone_cache::record_zone(&region, None);
            region
        })
        .collect();

    Ok((entities, max_y, merge_max_x, regions))
}

/// Place medium electric poles for power coverage.
///
/// Strategy: one horizontal pole line per machine row. Within a line, poles
/// are placed by greedy forward sweep — for each machine not yet covered, we
/// choose the rightmost pole position that still covers it, then advance past
/// every machine the new pole reaches. This guarantees edge machines are
/// covered (which a fixed-stride approach cannot) while still producing
/// regularly-spaced poles.
///
/// Pole y for a row is `machine_row_y - 1` (one tile above the machine tops).
/// With a 3-tile supply range that covers machine centers one tile below the
/// pole line comfortably. The tile above the machine row is typically the
/// inserter row, which has gaps every ~3 tiles between inserters — the probe
/// finds those gaps.
///
/// Connectivity is guaranteed by construction:
/// - Within a line: consecutive pole x-distance <= 6 < `WIRE_REACH` (9).
/// - Between lines: row cycle (row height + gap) is typically ~7 tiles <
///   wire-reach, so pole lines above consecutive rows connect vertically.
///
/// The old greedy + centroid-bridge implementation produced clumpy, order-
/// dependent output; this approach is deterministic, regular, and matches the
/// row-based structure of the bus layout.
fn place_poles(
    machines: &[(i32, i32, i32)],
    occupied: &FxHashSet<(i32, i32)>,
) -> Vec<PlacedEntity> {
    /// Supply range of a medium-electric-pole (Chebyshev, tiles).
    const POLE_RANGE: i32 = 3;
    /// Max X offset to probe when the ideal pole position is occupied.
    const POLE_PROBE_X: i32 = 3;

    if machines.is_empty() {
        return Vec::new();
    }

    // Group by (top_y, size). Rows of different-sized machines get their own
    // pole lines because the pole y needs to match the machine footprint.
    let mut by_row: FxHashMap<(i32, i32), Vec<i32>> = FxHashMap::default();
    for &(cx, top_y, sz) in machines {
        by_row.entry((top_y, sz)).or_default().push(cx);
    }
    for xs in by_row.values_mut() {
        xs.sort_unstable();
    }

    // Process rows top-to-bottom for determinism.
    let mut keys: Vec<(i32, i32)> = by_row.keys().copied().collect();
    keys.sort_unstable();

    let mut entities: Vec<PlacedEntity> = Vec::new();
    let mut placed: FxHashSet<(i32, i32)> = FxHashSet::default();

    for key in keys {
        let (top_y, _sz) = key;
        let cxs = &by_row[&key];
        let pole_y = top_y - 1; // one tile above the machine footprint
        if pole_y < 0 {
            continue;
        }

        let mut i = 0;
        while i < cxs.len() {
            // Aim for the rightmost position that still covers cxs[i] — this
            // maximises forward reach and keeps the line sparse. Probing
            // searches nearby tiles if the ideal one is occupied, always
            // staying within POLE_RANGE of the target machine.
            let target_cx = cxs[i];
            let ideal_px = target_cx + POLE_RANGE;
            let mut placed_x: Option<i32> = None;
            for d in 0..=POLE_PROBE_X {
                let offsets: &[i32] = if d == 0 { &[0] } else { &[-d, d] };
                for &off in offsets {
                    let px = ideal_px + off;
                    if (px - target_cx).abs() > POLE_RANGE {
                        continue; // stepped outside range of the target machine
                    }
                    if occupied.contains(&(px, pole_y)) || placed.contains(&(px, pole_y)) {
                        continue;
                    }
                    placed_x = Some(px);
                    break;
                }
                if placed_x.is_some() {
                    break;
                }
            }

            match placed_x {
                Some(px) => {
                    entities.push(make_pole(px, pole_y));
                    placed.insert((px, pole_y));
                    // Advance past every machine this pole covers.
                    i += 1;
                    while i < cxs.len() && (cxs[i] - px).abs() <= POLE_RANGE {
                        i += 1;
                    }
                }
                None => {
                    // Couldn't place a pole covering cxs[i]. Skip it to avoid an
                    // infinite loop — power validator will flag the gap.
                    i += 1;
                }
            }
        }
    }

    repair_pole_connectivity(&mut entities, &placed, occupied);
    entities
}

/// After the row lines are placed, bridge any remaining disconnected pole
/// clusters. This only fires when two machine rows are further apart in Y
/// than `WIRE_REACH` (e.g. oil-refinery row above a chemical-plant row with
/// a pipe-routing gap between them). We walk intermediate poles down a
/// free column between the two nearest clusters.
fn repair_pole_connectivity(
    entities: &mut Vec<PlacedEntity>,
    placed: &FxHashSet<(i32, i32)>,
    occupied: &FxHashSet<(i32, i32)>,
) {
    const WIRE_REACH: i32 = 9;

    let mut all_occupied: FxHashSet<(i32, i32)> = occupied.iter().copied().collect();
    for &p in placed {
        all_occupied.insert(p);
    }

    for _ in 0..20 {
        let positions: Vec<(i32, i32)> = entities.iter().map(|e| (e.x, e.y)).collect();
        if positions.len() <= 1 {
            return;
        }

        // Union-find under Chebyshev distance <= WIRE_REACH.
        let n = positions.len();
        let mut parent: Vec<usize> = (0..n).collect();
        fn find(p: &mut [usize], mut x: usize) -> usize {
            while p[x] != x {
                p[x] = p[p[x]];
                x = p[x];
            }
            x
        }
        for i in 0..n {
            for j in (i + 1)..n {
                let dx = (positions[i].0 - positions[j].0).abs();
                let dy = (positions[i].1 - positions[j].1).abs();
                if dx.max(dy) <= WIRE_REACH {
                    let ri = find(&mut parent, i);
                    let rj = find(&mut parent, j);
                    if ri != rj {
                        parent[ri] = rj;
                    }
                }
            }
        }

        // Group by root component.
        let mut by_comp: FxHashMap<usize, Vec<(i32, i32)>> = FxHashMap::default();
        for (idx, &pos) in positions.iter().enumerate() {
            let root = find(&mut parent, idx);
            by_comp.entry(root).or_default().push(pos);
        }
        if by_comp.len() == 1 {
            return;
        }

        // Find the closest inter-component pole pair.
        let comps: Vec<&Vec<(i32, i32)>> = by_comp.values().collect();
        let mut best: Option<((i32, i32), (i32, i32), i32)> = None;
        for a in 0..comps.len() {
            for b in (a + 1)..comps.len() {
                for &pa in comps[a] {
                    for &pb in comps[b] {
                        let d = (pa.0 - pb.0).abs().max((pa.1 - pb.1).abs());
                        if best.is_none_or(|(_, _, bd)| d < bd) {
                            best = Some((pa, pb, d));
                        }
                    }
                }
            }
        }
        let Some((pa, pb, _)) = best else {
            return;
        };

        // Pick a midpoint and walk outward in a small neighbourhood looking
        // for a free tile to place a bridge pole.
        let mid = ((pa.0 + pb.0) / 2, (pa.1 + pb.1) / 2);
        let mut bridge: Option<(i32, i32)> = None;
        'scan: for r in 0i32..=6 {
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx.abs() != r && dy.abs() != r {
                        continue; // only examine the ring at radius r
                    }
                    let p = (mid.0 + dx, mid.1 + dy);
                    if all_occupied.contains(&p) {
                        continue;
                    }
                    // Must be within wire-reach of pa or pb for it to actually bridge.
                    let near_a = (p.0 - pa.0).abs().max((p.1 - pa.1).abs()) <= WIRE_REACH;
                    let near_b = (p.0 - pb.0).abs().max((p.1 - pb.1).abs()) <= WIRE_REACH;
                    if near_a || near_b {
                        bridge = Some(p);
                        break 'scan;
                    }
                }
            }
        }

        let Some(p) = bridge else { return };
        entities.push(make_pole(p.0, p.1));
        all_occupied.insert(p);
    }
}


/// Create a pole entity at the given position.
fn emit_inter_row_bands(row_spans: &[RowSpan], lanes: &[BusLane]) {
    if row_spans.len() < 2 {
        return;
    }
    let lane_extents: Vec<(i32, i32)> = lanes
        .iter()
        .map(|l| {
            let mut y_min = l.source_y;
            let mut y_max = l.source_y;
            for &ty in &l.tap_off_ys {
                y_min = y_min.min(ty);
                y_max = y_max.max(ty);
            }
            for &cr in &l.consumer_rows {
                if let Some(rs) = row_spans.get(cr) {
                    y_min = y_min.min(rs.y_start);
                    y_max = y_max.max(rs.y_end - 1);
                }
            }
            (y_min, y_max)
        })
        .collect();

    for i in 0..row_spans.len() - 1 {
        let upper = &row_spans[i];
        let lower = &row_spans[i + 1];
        // y_end is exclusive, so y_end is the first tile of the gap.
        let band_y_start = upper.y_end;
        let band_y_end = lower.y_start - 1;
        if band_y_end < band_y_start {
            continue;
        }
        let mut trunk_count = 0usize;
        let mut items: FxHashSet<&str> = FxHashSet::default();
        for (lane, &(y_min, y_max)) in lanes.iter().zip(lane_extents.iter()) {
            if y_min <= band_y_start && y_max >= band_y_end {
                trunk_count += 1;
                items.insert(lane.item.as_str());
            }
        }
        crate::trace::emit(crate::trace::TraceEvent::InterRowBand {
            upper_row_idx: i,
            lower_row_idx: i + 1,
            band_y_start,
            band_y_end,
            gap_height: band_y_end - band_y_start + 1,
            trunk_count,
            distinct_items: items.len(),
        });
    }
}

fn make_pole(x: i32, y: i32) -> PlacedEntity {
    PlacedEntity {
        name: "medium-electric-pole".to_string(),
        x,
        y,
        direction: EntityDirection::North,
        recipe: None,
        io_type: None,
        carries: None,
        mirror: false,
        segment_id: Some("pole".to_string()),
        ..Default::default()
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_bus_width_empty() {
        let sr = SolverResult {
            machines: vec![],
            external_inputs: vec![],
            external_outputs: vec![],
            dependency_order: vec![],
        };
        let bw = estimate_bus_width(&sr);
        assert!(bw >= 2);
    }

    #[test]
    fn test_compute_extra_gaps_empty() {
        let extras = compute_extra_gaps(&[]);
        assert!(extras.is_empty());
    }

    #[test]
    fn test_build_bus_layout_iron_gear_wheel_has_machines() {
        use crate::solver::solve;
        use rustc_hash::FxHashSet;

        let available_inputs: FxHashSet<String> = ["iron-ore"]
            .iter()
            .map(|s| s.to_string())
            .collect();

        let solver_result = solve("iron-gear-wheel", 10.0, &available_inputs, "assembling-machine-2")
            .expect("solver should succeed for iron-gear-wheel");

        let layout = build_bus_layout(&solver_result, None)
            .expect("build_bus_layout should succeed");

        // Must have more than just power poles
        let assembling_count = layout.entities.iter()
            .filter(|e| e.name.contains("assembling-machine"))
            .count();

        assert!(
            assembling_count > 0,
            "Expected at least one assembling-machine in layout, got entities: {:?}",
            layout.entities.iter().map(|e| &e.name).collect::<Vec<_>>()
        );

        // Layout should have non-trivial dimensions
        assert!(layout.width > 0, "Layout width should be > 0");
        assert!(layout.height > 0, "Layout height should be > 0");
    }

    #[test]
    fn test_iron_gear_wheel_layout_has_machines() {
        use crate::solver::solve;
        use rustc_hash::FxHashSet;

        let result = solve(
            "iron-gear-wheel",
            10.0,
            &FxHashSet::default(),
            "assembling-machine-3",
        )
        .expect("solver should succeed for iron-gear-wheel");

        let layout = build_bus_layout(&result, None);
        assert!(layout.is_ok(), "build_bus_layout should return Ok, got: {:?}", layout.err());

        let layout = layout.unwrap();

        // Must contain at least one assembling machine
        let has_assembling = layout.entities.iter()
            .any(|e| e.name.starts_with("assembling-machine"));
        assert!(
            has_assembling,
            "Expected at least one assembling-machine entity; found: {:?}",
            layout.entities.iter().map(|e| &e.name).collect::<Vec<_>>()
        );

        // Must contain at least one transport belt (the bus layout places belts)
        let has_transport_belt = layout.entities.iter()
            .any(|e| e.name == "transport-belt");
        assert!(
            has_transport_belt,
            "Expected at least one transport-belt entity"
        );

        // Layout dimensions must be positive
        assert!(layout.width > 0, "Layout width must be > 0, got {}", layout.width);
        assert!(layout.height > 0, "Layout height must be > 0, got {}", layout.height);
    }

    #[test]
    fn test_ecircuit_yellow_belt_no_structural_errors() {
        // Regression: forcing yellow belts on electronic-circuit from ores
        // used to cause UG reach violations (negotiated A* used hardcoded
        // max_reach=8 instead of tier-aware value).
        use crate::solver::solve;
        use rustc_hash::FxHashSet;

        let inputs: FxHashSet<String> = ["iron-ore", "copper-ore"]
            .iter().map(|s| s.to_string()).collect();

        let sr = solve("electronic-circuit", 10.0, &inputs, "assembling-machine-3")
            .expect("solve");
        let layout = build_bus_layout(&sr, Some("transport-belt"))
            .expect("layout");

        // Check for entity overlaps (the main bug this test catches).
        // Full validation is skipped because the validator has a known hang
        // on certain belt topologies.
        let mut positions: std::collections::HashSet<(i32, i32)> = std::collections::HashSet::new();
        let mut overlaps = Vec::new();
        for e in &layout.entities {
            if !positions.insert((e.x, e.y)) {
                overlaps.push(format!("({},{}) {}", e.x, e.y, e.name));
            }
        }
        assert!(overlaps.is_empty(), "Entity overlaps: {}", overlaps.join("; "));

    }

    #[test]
    fn test_ecircuit_yellow_validation_checks() {
        use crate::solver::solve;
        use crate::validate::{Severity, belt_structural, belt_flow};
        use rustc_hash::FxHashSet;

        let inputs: FxHashSet<String> = ["iron-ore", "copper-ore"]
            .iter().map(|s| s.to_string()).collect();
        let sr = solve("electronic-circuit", 10.0, &inputs, "assembling-machine-3")
            .expect("solve");
        let layout = build_bus_layout(&sr, Some("transport-belt"))
            .expect("layout");

        // Run individual validators (full validate() hangs on some topologies).
        // Run individual validators (full validate() still slow on large layouts).
        let mut all_issues = Vec::new();
        all_issues.extend(belt_structural::check_belt_throughput(&layout));
        all_issues.extend(belt_structural::check_belt_dead_ends(&layout));
        all_issues.extend(belt_structural::check_belt_item_isolation(&layout));
        all_issues.extend(belt_structural::check_belt_loops(&layout));
        all_issues.extend(belt_flow::check_belt_junctions(&layout));
        all_issues.extend(belt_flow::check_underground_belt_pairs(&layout));
        all_issues.extend(belt_flow::check_belt_connectivity(&layout, Some(&sr)));
        all_issues.extend(belt_flow::check_belt_flow_path(&layout, Some(&sr), crate::validate::LayoutStyle::Bus));

        let errors: Vec<_> = all_issues.iter()
            .filter(|i| i.severity == Severity::Error)
            .collect();
        assert!(errors.is_empty(), "Expected 0 errors, got {}: {}", errors.len(),
            errors.iter().map(|i| i.message.as_str()).collect::<Vec<_>>().join("; "));
    }

    #[test]
    fn test_ecircuit_10s_from_ore_am1_yellow() {
        // Regression: 10/s e-circuit from ore with AM1 + yellow belt.
        // This is the case where copper-cable trunk columns (x=13-17) block
        // the iron-plate feeder that needs to cross them to reach the descent
        // column at x=6. With y_constraint: None + goal_on_obstacle: true,
        // the feeder should route around the trunks.
        use crate::solver::solve;
        use crate::trace::TraceEvent;
        use crate::validate::{Severity, belt_structural, belt_flow};
        use rustc_hash::FxHashSet;

        let inputs: FxHashSet<String> = ["iron-ore", "copper-ore"]
            .iter().map(|s| s.to_string()).collect();
        let sr = solve("electronic-circuit", 10.0, &inputs, "assembling-machine-1")
            .expect("solve");

        let layout = build_bus_layout_traced(&sr, Some("transport-belt"))
            .expect("layout");

        // Check for route failures
        let events = layout.trace.clone().expect("trace");

        // Print lane layout for debugging
        for e in &events {
            match e {
                TraceEvent::LanesPlanned { lanes, bus_width, .. } => {
                    eprintln!("Bus width: {}", bus_width);
                    for l in lanes {
                        eprintln!("  Lane: {:30} x={:2} src_y={:3} taps={:?} prod={:?} family={:?} fluid={}",
                            l.item, l.x, l.source_y, l.tap_off_ys, l.producer_row, l.family_id, l.is_fluid);
                    }
                }
                TraceEvent::BalancerStamped { item, shape, y_start, y_end, template_found } => {
                    eprintln!("  Balancer: {} {:?} y={}-{} found={}", item, shape, y_start, y_end, template_found);
                }
                _ => {}
            }
        }

        let failures: Vec<_> = events.iter()
            .filter_map(|e| match e {
                TraceEvent::RouteFailure { spec_key, item, from_x, from_y, to_x, to_y } => {
                    eprintln!("  RouteFailure: {} {} ({},{}) -> ({},{})", spec_key, item, from_x, from_y, to_x, to_y);
                    Some(format!("{}: {}", spec_key, item))
                }
                _ => None,
            })
            .collect();
        assert!(failures.is_empty(), "Route failures: {}", failures.join("; "));

        // No overlaps — but first diagnose
        let mut positions: std::collections::HashSet<(i32, i32)> = std::collections::HashSet::new();
        let mut overlaps = Vec::new();
        for e in &layout.entities {
            if !positions.insert((e.x, e.y)) {
                overlaps.push((e.x, e.y, e.name.clone()));
            }
        }
        if !overlaps.is_empty() {
            eprintln!("=== Overlap entities ===");
            for &(ox, oy, _) in &overlaps {
                for e in &layout.entities {
                    if e.x == ox && e.y == oy {
                        eprintln!("  ({},{}) {:40} dir={:?} carries={:?} seg={:?}",
                            e.x, e.y, e.name, e.direction, e.carries, e.segment_id);
                    }
                }
            }
            eprintln!("\n=== Nearby entities (±3) ===");
            for &(ox, oy, _) in &overlaps {
                for e in &layout.entities {
                    if (e.x - ox).abs() <= 3 && (e.y - oy).abs() <= 3 {
                        eprintln!("  ({},{}) {:40} dir={:?} carries={:?}", e.x, e.y, e.name, e.direction, e.carries);
                    }
                }
                eprintln!();
            }
            // Show route failures and balancer stamps
            for e in &events {
                match e {
                    TraceEvent::RouteFailure { spec_key, item, from_x, from_y, to_x, to_y } => {
                        eprintln!("  RouteFailure: {} {} ({},{}) -> ({},{})", spec_key, item, from_x, from_y, to_x, to_y);
                    }
                    TraceEvent::BalancerStamped { item, shape, y_start, y_end, template_found } => {
                        eprintln!("  BalancerStamped: {} {:?} y={}-{} found={}", item, shape, y_start, y_end, template_found);
                    }
                    _ => {}
                }
            }
        }
        assert!(overlaps.is_empty(), "Entity overlaps at: {}",
            overlaps.iter().map(|(x,y,n)| format!("({},{}) {}", x, y, n)).collect::<Vec<_>>().join("; "));

        // Full belt validation
        let mut all_issues = Vec::new();
        all_issues.extend(belt_structural::check_belt_dead_ends(&layout));
        all_issues.extend(belt_structural::check_belt_item_isolation(&layout));
        all_issues.extend(belt_structural::check_belt_loops(&layout));
        all_issues.extend(belt_flow::check_underground_belt_pairs(&layout));
        all_issues.extend(belt_flow::check_belt_connectivity(&layout, Some(&sr)));
        all_issues.extend(belt_flow::check_belt_flow_path(&layout, Some(&sr), crate::validate::LayoutStyle::Bus));

        let errors: Vec<_> = all_issues.iter()
            .filter(|i| i.severity == Severity::Error)
            .collect();

        // Diagnose dead-end belts
        if !errors.is_empty() {
            for err in &errors {
                if err.message.contains("no receiver") {
                    // Extract the belt position from the error message
                    eprintln!("\n=== Diagnosing: {} ===", err.message);
                    if let (Some(bx), Some(by)) = (err.x, err.y) {
                        // Find the belt entity
                        for e in &layout.entities {
                            if e.x == bx && e.y == by {
                                eprintln!("  Belt: ({},{}) {:40} dir={:?} carries={:?} seg={:?}", e.x, e.y, e.name, e.direction, e.carries, e.segment_id);
                            }
                        }
                        // Check receiver position
                        let rx = bx - 1;
                        let ry = by;
                        eprintln!("  Receiver position ({},{}):", rx, ry);
                        let mut found = false;
                        for e in &layout.entities {
                            if e.x == rx && e.y == ry {
                                eprintln!("    ({},{}) {:40} dir={:?} carries={:?} seg={:?}", e.x, e.y, e.name, e.direction, e.carries, e.segment_id);
                                found = true;
                            }
                        }
                        if !found {
                            eprintln!("    NOTHING at ({},{})", rx, ry);
                        }
                        // Show ±2 context
                        eprintln!("  Context (±2):");
                        for e in &layout.entities {
                            if (e.x - bx).abs() <= 2 && (e.y - by).abs() <= 2 {
                                eprintln!("    ({},{}) {:40} dir={:?} carries={:?}", e.x, e.y, e.name, e.direction, e.carries);
                            }
                        }
                    }
                }
            }
        }

        assert!(errors.is_empty(), "Expected 0 errors, got {}: {}",
            errors.len(), errors.iter().map(|i| i.message.as_str()).collect::<Vec<_>>().join("; "));
    }

    #[test]
    fn test_ecircuit_am1_tapoffs() {
        // Regression: 5/s electronic-circuit with AM1 + yellow belt had a gap
        // at y=14 where the copper-cable feeder couldn't cross the iron-ore trunk.
        use crate::solver::solve;
        use rustc_hash::FxHashSet;

        let inputs: FxHashSet<String> = [
            "copper-plate", "steel-plate", "stone", "coal",
            "water", "crude-oil", "iron-ore", "copper-ore",
        ].iter().map(|s| s.to_string()).collect();

        let sr = solve("electronic-circuit", 5.0, &inputs, "assembling-machine-1")
            .expect("solve");
        let layout = build_bus_layout(&sr, Some("transport-belt"))
            .expect("layout");

        // Should have no warnings
        assert!(layout.warnings.is_empty(), "Unexpected warnings: {:?}", layout.warnings);

        // Check that every feeder-carrying belt in the bus zone (x < bus_width)
        // is connected — no isolated single belts. We verify by checking that
        // the layout has a reasonable entity count (feeder paths add entities).
        let belt_count = layout.entities.iter()
            .filter(|e| e.name.contains("belt"))
            .count();
        assert!(belt_count > 50, "Expected >50 belt entities, got {}", belt_count);

        // Check no entity overlaps
        let mut positions: std::collections::HashSet<(i32, i32)> = std::collections::HashSet::new();
        let mut overlaps = Vec::new();
        for e in &layout.entities {
            if !positions.insert((e.x, e.y)) {
                overlaps.push(format!("({},{}) {}", e.x, e.y, e.name));
            }
        }
        assert!(overlaps.is_empty(), "Entity overlaps: {}", overlaps.join("; "));
    }

    #[test]
    fn test_ecircuit_10s_yellow_from_plates_ug_valid() {
        // Regression: 10/s electronic-circuit from plates with AM1 + yellow belts had an
        // iron-plate tap-off crossing a 5-lane copper-cable bus with a span of 6,
        // exceeding yellow UG belt's max reach of 4 (max distance = 5).
        // The underground belt validation must catch any span violation or unpaired UG.
        use crate::solver::solve;
        use crate::validate::belt_flow;
        use crate::validate::Severity;
        use rustc_hash::FxHashSet;

        let inputs: FxHashSet<String> = ["iron-plate", "copper-plate"]
            .iter().map(|s| s.to_string()).collect();

        let sr = solve("electronic-circuit", 10.0, &inputs, "assembling-machine-1")
            .expect("solve");
        let layout = build_bus_layout(&sr, Some("transport-belt"))
            .expect("layout");

        // Check entity overlaps — a UG input placed on an occupied trunk tile is the
        // routing bug this test guards against.
        let mut positions: std::collections::HashMap<(i32, i32), &str> = std::collections::HashMap::new();
        let mut overlaps: Vec<String> = Vec::new();
        for e in &layout.entities {
            if let Some(prev) = positions.insert((e.x, e.y), &e.name) {
                overlaps.push(format!("({},{}) {} vs {}", e.x, e.y, prev, e.name));
            }
        }
        assert!(
            overlaps.is_empty(),
            "Entity overlaps detected (UG input placed on occupied tile): {}",
            overlaps.join("; ")
        );

        let ug_issues = belt_flow::check_underground_belt_pairs(&layout);
        let errors: Vec<_> = ug_issues.iter()
            .filter(|i| i.severity == Severity::Error)
            .collect();
        assert!(
            errors.is_empty(),
            "Underground belt validation errors: {}",
            errors.iter().map(|i| i.message.as_str()).collect::<Vec<_>>().join("; ")
        );
    }

    #[test]
    fn test_iron_gear_wheel_red_belt() {
        use crate::solver::solve;
        use rustc_hash::FxHashSet;

        let sr = solve(
            "iron-gear-wheel",
            10.0,
            &FxHashSet::default(),
            "assembling-machine-2",
        )
        .expect("solver should succeed for iron-gear-wheel");

        let layout = build_bus_layout(&sr, Some("fast-transport-belt"))
            .expect("build_bus_layout should succeed with red belt");

        assert!(!layout.entities.is_empty(), "Layout should have entities");
        assert!(layout.width > 0, "Layout width should be > 0");
        assert!(layout.height > 0, "Layout height should be > 0");
    }

    #[test]
    fn test_ecircuit_10s_blue_belt() {
        use crate::solver::solve;
        use rustc_hash::FxHashSet;

        let inputs: FxHashSet<String> = ["iron-ore", "copper-ore"]
            .iter()
            .map(|s| s.to_string())
            .collect();

        let sr = solve("electronic-circuit", 10.0, &inputs, "assembling-machine-2")
            .expect("solver should succeed for electronic-circuit");

        let layout = build_bus_layout(&sr, Some("express-transport-belt"))
            .expect("build_bus_layout should succeed with blue belt");

        assert!(!layout.entities.is_empty(), "Layout should have entities");
        assert!(layout.width > 0, "Layout width should be > 0");
        assert!(layout.height > 0, "Layout height should be > 0");
    }

    #[test]
    #[ignore] // manual investigation — run with --ignored --nocapture
    fn test_ecircuit_20s_yellow_sat_zones() {
        // Investigative test for larger layouts. Asserts no warnings
        // (missing balancer templates = disconnected producers = broken layout).
        use crate::solver::solve;
        use rustc_hash::FxHashSet;

        use crate::validate::underground::check_underground_belt_pairs;
        use crate::validate::belt_flow::{
            check_belt_dead_ends, check_belt_item_isolation, check_belt_loops,
            check_belt_junctions,
        };
        use crate::validate::Severity;

        let inputs: FxHashSet<String> = ["iron-plate", "copper-plate"]
            .iter().map(|s| s.to_string()).collect();
        for machine in &["assembling-machine-1", "assembling-machine-3"] {
            let sr = solve("electronic-circuit", 20.0, &inputs, machine)
                .expect("solve");
            let layout = build_bus_layout(&sr, Some("transport-belt"))
                .unwrap_or_else(|e| panic!("layout with {}: {}", machine, e));

            assert!(layout.warnings.is_empty(),
                "{}: warnings: {:?}", machine, layout.warnings);

            // Check overlaps — report both entities at each position
            let mut tile_map: std::collections::HashMap<(i32,i32), Vec<String>> = std::collections::HashMap::new();
            for e in &layout.entities {
                tile_map.entry((e.x, e.y)).or_default()
                    .push(format!("{} {:?} seg={:?}", e.name, e.direction, e.segment_id));
            }
            let overlaps: Vec<_> = tile_map.iter()
                .filter(|(_, v)| v.len() > 1)
                .map(|((x,y), v)| format!("({},{}) {:?}", x, y, v))
                .collect();
            if !overlaps.is_empty() {
                eprintln!("{}: {} overlaps:", machine, overlaps.len());
                for o in &overlaps { eprintln!("  {}", o); }
            }
            assert!(overlaps.is_empty(), "{}: {} overlaps", machine, overlaps.len());

            // Full validation
            let mut all_errors = Vec::new();
            for issue in check_underground_belt_pairs(&layout) {
                if issue.severity == Severity::Error { all_errors.push(issue.message); }
            }
            for issue in check_belt_dead_ends(&layout) {
                if issue.severity == Severity::Error { all_errors.push(issue.message); }
            }
            for issue in check_belt_item_isolation(&layout) {
                if issue.severity == Severity::Error { all_errors.push(issue.message); }
            }
            for issue in check_belt_loops(&layout) {
                if issue.severity == Severity::Error { all_errors.push(issue.message); }
            }
            for issue in check_belt_junctions(&layout) {
                if issue.severity == Severity::Error { all_errors.push(issue.message); }
            }

            eprintln!("{}: {} entities, {} regions, {} errors",
                machine, layout.entities.len(), layout.regions.len(), all_errors.len());
            for e in &all_errors {
                eprintln!("  {}", e);
            }
            assert!(all_errors.is_empty(),
                "{}: {} validation errors", machine, all_errors.len());
        }
    }

    #[test]
    fn test_ecircuit_10s_asm1_yellow_from_plates() {
        // Regression: 10/s electronic-circuit with AM1 + yellow belt from plates
        // had missing (3,5) balancer template causing disconnected feeders.
        use crate::solver::solve;
        use rustc_hash::FxHashSet;

        let inputs: FxHashSet<String> = [
            "iron-plate", "copper-plate", "steel-plate", "stone", "coal",
            "water", "crude-oil", "iron-ore", "copper-ore",
        ].iter().map(|s| s.to_string()).collect();

        let sr = solve("electronic-circuit", 10.0, &inputs, "assembling-machine-1")
            .expect("solve");
        let layout = build_bus_layout(&sr, Some("transport-belt"))
            .expect("layout");

        // Should have no warnings about missing templates
        assert!(
            layout.warnings.is_empty(),
            "Expected no warnings, got: {:?}",
            layout.warnings
        );

        // Layout should have a reasonable number of entities.
        // After lane consolidation (fewer external input lanes), ~886 entities.
        // Was ~1048 with 1-lane-per-consumer; ~897 when feeders were broken.
        assert!(
            layout.entities.len() > 800,
            "Expected >800 entities (full layout with feeders), got {}",
            layout.entities.len()
        );

        // Full validation — dead-ends, loops, isolation, etc.
        use crate::validate::belt_flow::{
            check_belt_dead_ends, check_belt_item_isolation, check_belt_loops,
            check_belt_junctions,
        };
        use crate::validate::underground::check_underground_belt_pairs;
        use crate::validate::Severity;
        let mut all_errors = Vec::new();
        for issue in check_underground_belt_pairs(&layout) {
            if issue.severity == Severity::Error { all_errors.push(issue.message.clone()); }
        }
        for issue in check_belt_dead_ends(&layout) {
            if issue.severity == Severity::Error { all_errors.push(issue.message.clone()); }
        }
        for issue in check_belt_item_isolation(&layout) {
            if issue.severity == Severity::Error { all_errors.push(issue.message.clone()); }
        }
        for issue in check_belt_loops(&layout) {
            if issue.severity == Severity::Error { all_errors.push(issue.message.clone()); }
        }
        for issue in check_belt_junctions(&layout) {
            if issue.severity == Severity::Error { all_errors.push(issue.message.clone()); }
        }
        if !all_errors.is_empty() {
            eprintln!("{} validation errors:", all_errors.len());
            for e in &all_errors { eprintln!("  {}", e); }
        }
        assert!(all_errors.is_empty(), "{} validation errors", all_errors.len());
    }

    #[test]
    fn test_ecircuit_10s_from_plates_only() {
        // Matches web UI: electronic-circuit 10/s with just iron-plate + copper-plate inputs
        use crate::solver::solve;
        use crate::validate::belt_flow::check_belt_dead_ends;
        use crate::validate::Severity;
        use rustc_hash::FxHashSet;

        let inputs: FxHashSet<String> = ["iron-plate", "copper-plate"]
            .iter().map(|s| s.to_string()).collect();
        let sr = solve("electronic-circuit", 10.0, &inputs, "assembling-machine-1")
            .expect("solve");
        let layout = build_bus_layout(&sr, Some("transport-belt"))
            .expect("layout");
        eprintln!("{} entities, {} warnings", layout.entities.len(), layout.warnings.len());
        // Dump entities at x=9 and x=10,11
        for e in &layout.entities {
            if e.x >= 9 && e.x <= 11 {
                eprintln!("  ({},{}) {} {:?} carries={:?} io={:?} seg={:?}",
                    e.x, e.y, e.name, e.direction, e.carries, e.io_type, e.segment_id);
            }
        }
        let errors: Vec<_> = check_belt_dead_ends(&layout)
            .into_iter()
            .filter(|i| i.severity == Severity::Error)
            .collect();
        if !errors.is_empty() {
            eprintln!("{} dead-end errors:", errors.len());
            for e in &errors { eprintln!("  {}", e.message); }
        }
        assert!(errors.is_empty(), "{} dead-end errors", errors.len());
    }

    #[test]
    fn test_ecircuit_20s_from_plates_splitter_tapoffs() {
        // 20/s e-circuit from plates — matches the web app's default
        use crate::solver::solve;
        use crate::validate::belt_flow::{check_belt_dead_ends, check_belt_item_isolation};
        use crate::validate::underground::check_underground_belt_pairs;
        use crate::validate::Severity;
        use rustc_hash::FxHashSet;

        let inputs: FxHashSet<String> = ["iron-plate", "copper-plate"]
            .iter().map(|s| s.to_string()).collect();
        let sr = solve("electronic-circuit", 20.0, &inputs, "assembling-machine-1")
            .expect("solve");
        let layout = build_bus_layout(&sr, Some("transport-belt"))
            .expect("layout");

        // Dump entities near y=39 and y=48 for debugging
        eprintln!("{} entities, {} warnings", layout.entities.len(), layout.warnings.len());
        for e in &layout.entities {
            if (e.x <= 5 && e.y >= 36 && e.y <= 42)
                || (e.x <= 5 && e.y >= 45 && e.y <= 50)
            {
                eprintln!("  ({},{}) {} {:?} carries={:?} io={:?} seg={:?}",
                    e.x, e.y, e.name, e.direction, e.carries, e.io_type, e.segment_id);
            }
        }

        // Check for orphaned UG outputs (no matching input)
        let ug_errors: Vec<_> = check_underground_belt_pairs(&layout)
            .into_iter()
            .filter(|i| i.severity == Severity::Error)
            .collect();
        if !ug_errors.is_empty() {
            eprintln!("{} UG pair errors:", ug_errors.len());
            for e in &ug_errors { eprintln!("  {}", e.message); }
        }

        let dead_end_errors: Vec<_> = check_belt_dead_ends(&layout)
            .into_iter()
            .filter(|i| i.severity == Severity::Error)
            .collect();
        if !dead_end_errors.is_empty() {
            eprintln!("{} dead-end errors:", dead_end_errors.len());
            for e in &dead_end_errors { eprintln!("  {}", e.message); }
        }

        let isolation_errors: Vec<_> = check_belt_item_isolation(&layout)
            .into_iter()
            .filter(|i| i.severity == Severity::Error)
            .collect();
        if !isolation_errors.is_empty() {
            eprintln!("{} isolation errors:", isolation_errors.len());
            for e in &isolation_errors { eprintln!("  {}", e.message); }
        }

        let total_errors = ug_errors.len() + dead_end_errors.len() + isolation_errors.len();
        assert!(total_errors == 0, "{} validation errors", total_errors);
    }

    #[test]
    fn test_plastic_bar_layout() {
        // plastic-bar has fluid input (petroleum-gas) — tests fluid lane routing
        use crate::solver::solve;
        use crate::validate::belt_flow::check_belt_dead_ends;
        use crate::validate::Severity;
        use rustc_hash::FxHashSet;

        let inputs: FxHashSet<String> = [
            "iron-plate", "copper-plate", "steel-plate", "stone", "coal",
            "water", "crude-oil", "iron-ore", "copper-ore",
        ].iter().map(|s| s.to_string()).collect();
        let sr = solve("plastic-bar", 10.0, &inputs, "assembling-machine-2")
            .expect("solve");
        let layout = build_bus_layout(&sr, Some("transport-belt"))
            .expect("layout");
        assert!(!layout.entities.is_empty(), "Layout should have entities");
        let errors: Vec<_> = check_belt_dead_ends(&layout)
            .into_iter()
            .filter(|i| i.severity == Severity::Error)
            .collect();
        assert!(errors.is_empty(), "{} dead-end errors in plastic-bar layout", errors.len());
    }

    #[test]
    fn test_iron_gear_wheel_20s() {
        // Higher rate = more rows, tests row splitting at scale
        use crate::solver::solve;
        use rustc_hash::FxHashSet;

        let inputs: FxHashSet<String> = [
            "iron-plate", "copper-plate", "steel-plate", "stone", "coal",
            "water", "crude-oil", "iron-ore", "copper-ore",
        ].iter().map(|s| s.to_string()).collect();
        let sr = solve("iron-gear-wheel", 20.0, &inputs, "assembling-machine-2")
            .expect("solve");
        let layout = build_bus_layout(&sr, Some("transport-belt"))
            .expect("layout");
        assert!(!layout.entities.is_empty(), "Layout should have entities");
    }

    #[test]
    fn test_single_machine_low_rate() {
        // Edge case: very low rate → 1 machine
        use crate::solver::solve;
        use rustc_hash::FxHashSet;

        let inputs: FxHashSet<String> = [
            "iron-plate", "copper-plate", "steel-plate", "stone", "coal",
            "water", "crude-oil", "iron-ore", "copper-ore",
        ].iter().map(|s| s.to_string()).collect();
        let sr = solve("iron-gear-wheel", 1.0, &inputs, "assembling-machine-1")
            .expect("solve");
        let layout = build_bus_layout(&sr, Some("transport-belt"))
            .expect("layout");
        assert!(!layout.entities.is_empty(), "Layout should have entities");
        // Should have very few machines
        let machine_count = layout.entities.iter()
            .filter(|e| e.name.starts_with("assembling-machine"))
            .count();
        assert!(machine_count <= 3, "Expected few machines at 1/s, got {}", machine_count);
    }

    #[test]
    #[ignore = "slow: advanced-circuit layout takes 2+ min, run explicitly with --ignored"]
    fn test_advanced_circuit_from_plates() {
        // Deep chain: advanced-circuit needs electronic-circuit + copper-cable
        // + plastic-bar as intermediates. May have validation warnings but
        // should not panic.
        use crate::solver::solve;
        use rustc_hash::FxHashSet;

        let inputs: FxHashSet<String> = ["iron-plate", "copper-plate"]
            .iter().map(|s| s.to_string()).collect();
        let sr = solve("advanced-circuit", 5.0, &inputs, "assembling-machine-1");
        if let Ok(sr) = sr {
            // May fail to layout due to complexity — just verify no panic
            let _ = build_bus_layout(&sr, Some("transport-belt"));
        }
    }

    #[test]
    fn test_traced_layout_produces_events() {
        use crate::solver::solve;
        use crate::trace::TraceEvent;
        use rustc_hash::FxHashSet;

        let solver_result = solve(
            "iron-gear-wheel",
            10.0,
            &FxHashSet::default(),
            "assembling-machine-3",
        )
        .expect("solver should succeed");

        let layout = build_bus_layout_traced(&solver_result, None)
            .expect("traced layout should succeed");

        let events = layout.trace.expect("trace should be populated");

        // Should have events from all major phases
        let has_solver_completed = events.iter().any(|e| matches!(e, TraceEvent::SolverCompleted { .. }));
        let has_rows_placed = events.iter().any(|e| matches!(e, TraceEvent::RowsPlaced { .. }));
        let has_lanes_planned = events.iter().any(|e| matches!(e, TraceEvent::LanesPlanned { .. }));
        let has_lane_routed = events.iter().any(|e| matches!(e, TraceEvent::LaneRouted { .. }));
        let has_poles_placed = events.iter().any(|e| matches!(e, TraceEvent::PolesPlaced { .. }));

        assert!(has_solver_completed, "expected SolverCompleted event");
        assert!(has_rows_placed, "expected RowsPlaced event, got {} events: {:?}", events.len(), events.iter().map(|e| match e {
            TraceEvent::RowsPlaced { .. } => "RowsPlaced",
            TraceEvent::LanesPlanned { .. } => "LanesPlanned",
            TraceEvent::LaneRouted { .. } => "LaneRouted",
            TraceEvent::PolesPlaced { .. } => "PolesPlaced",
            TraceEvent::BalancerStamped { .. } => "BalancerStamped",
            TraceEvent::MergerBlockPlaced { .. } => "MergerBlockPlaced",
            TraceEvent::OutputMerged { .. } => "OutputMerged",
            TraceEvent::CrossingZoneSolved { .. } => "CrossingZoneSolved",
            _ => "other",
        }).collect::<Vec<_>>());
        assert!(has_lanes_planned, "expected LanesPlanned event");
        assert!(has_lane_routed, "expected LaneRouted event");
        assert!(has_poles_placed, "expected PolesPlaced event");

        // Non-traced layout should have no events
        let plain_layout = build_bus_layout(&solver_result, None)
            .expect("plain layout should succeed");
        assert!(plain_layout.trace.is_none(), "non-traced layout should have no trace");
    }

    /// Timing breakdown for the ecircuit layout — run manually to diagnose perf.
    #[test]
    #[ignore = "diagnostic: prints phase timing, not a correctness test"]
    fn test_ecircuit_timing_breakdown() {
        use crate::solver::solve;
        use crate::trace::TraceEvent;
        use rustc_hash::FxHashSet;

        let inputs: FxHashSet<String> = ["iron-plate", "copper-plate"]
            .iter().map(|s| s.to_string()).collect();
        let sr = solve("electronic-circuit", 10.0, &inputs, "assembling-machine-1")
            .expect("solve");

        let t0 = std::time::Instant::now();
        let layout = build_bus_layout_traced(&sr, Some("transport-belt"))
            .expect("layout");
        let total_ms = t0.elapsed().as_millis();

        let events = layout.trace.unwrap_or_default();

        eprintln!("\n=== ecircuit 10/s timing breakdown (total: {}ms) ===", total_ms);
        for ev in &events {
            match ev {
                TraceEvent::PhaseTime { phase, duration_ms } => {
                    eprintln!("  {:30} {:5}ms  ({:.0}%)", phase, duration_ms,
                        100.0 * *duration_ms as f64 / total_ms as f64);
                }
                TraceEvent::CrossingZoneSolved { x, y, width, height, solve_time_us } => {
                    eprintln!("  SAT zone ({},{}) {}x{}: {}µs", x, y, width, height, solve_time_us);
                }
                TraceEvent::NegotiateComplete { specs, iterations, duration_ms } => {
                    eprintln!("  negotiate: {} specs, {} iters, {}ms", specs, iterations, duration_ms);
                }
                _ => {}
            }
        }
    }
}
