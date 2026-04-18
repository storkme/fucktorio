//! Bus layout orchestrator: rows + bus lanes + poles -> LayoutResult.
//!
//! Entry point: [`build_bus_layout`]. Calls `place_rows` to stack
//! assembly rows, `plan_bus_lanes` to decide which items need which
//! trunks, and `route_bus_ghost` to materialise every connecting belt
//! via the ghost-routing pipeline. See `docs/ghost-pipeline-contracts.md`
//! for the phase-by-phase invariants the router promises.

use rustc_hash::{FxHashMap, FxHashSet};

use crate::models::{EntityDirection, LayoutResult, PlacedEntity, SolverResult};
use crate::bus::lane_planner::{
    plan_bus_lanes, bus_width_for_lanes, BusLane, LaneFamily, MACHINE_ENTITIES,
};
use crate::bus::placer::{place_rows, RowSpan};

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

    // Pass 1: place rows with an estimated bus width, then plan lanes
    // so we know the real bus width and any balancer blocks that need
    // vertical gaps between producer rows.
    let temp_bw = estimate_bus_width(solver_result);
    #[cfg(not(target_arch = "wasm32"))]
    let t_place1 = std::time::Instant::now();
    let (row_entities_1, row_spans_1, _row_width_1, _total_height_1) = place_rows(
        &solver_result.machines,
        &solver_result.dependency_order,
        temp_bw,
        bus_header,
        max_belt_tier,
        Some(&final_output_items),
        None,
    );
    #[cfg(not(target_arch = "wasm32"))]
    crate::trace::emit(crate::trace::TraceEvent::PhaseTime {
        phase: "place_rows_1".to_string(),
        duration_ms: t_place1.elapsed().as_millis() as u64,
    });
    #[cfg(not(target_arch = "wasm32"))]
    let t_plan1 = std::time::Instant::now();
    let (lanes_1, families_1) = plan_bus_lanes(solver_result, &row_spans_1, max_belt_tier)?;
    #[cfg(not(target_arch = "wasm32"))]
    crate::trace::emit(crate::trace::TraceEvent::PhaseTime {
        phase: "plan_bus_lanes_1".to_string(),
        duration_ms: t_plan1.elapsed().as_millis() as u64,
    });
    let actual_bw = bus_width_for_lanes(&lanes_1);
    let extra_gaps = compute_extra_gaps(&families_1);

    // Pass 2: re-place rows with the real bus width + any extra gaps
    // needed to fit balancer blocks. Skipped only when nothing changed.
    let (row_entities, row_spans, row_width, total_height, lanes, families) =
        if actual_bw == temp_bw && extra_gaps.is_empty() {
            (row_entities_1, row_spans_1, _row_width_1, _total_height_1, lanes_1, families_1)
        } else {
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
            #[cfg(not(target_arch = "wasm32"))]
            crate::trace::emit(crate::trace::TraceEvent::PhaseTime {
                phase: "place_rows_2".to_string(),
                duration_ms: t_place2.elapsed().as_millis() as u64,
            });
            #[cfg(not(target_arch = "wasm32"))]
            let t_plan2 = std::time::Instant::now();
            let (nl, nf) = plan_bus_lanes(solver_result, &rs, max_belt_tier)?;
            #[cfg(not(target_arch = "wasm32"))]
            crate::trace::emit(crate::trace::TraceEvent::PhaseTime {
                phase: "plan_bus_lanes_2".to_string(),
                duration_ms: t_plan2.elapsed().as_millis() as u64,
            });
            (re, rs, rw, th, nl, nf)
        };

    crate::trace::emit(crate::trace::TraceEvent::PhaseComplete {
        phase: "rows_placed".into(),
        entity_count: row_entities.len(),
    });
    if crate::trace::is_active() {
        crate::trace::emit(crate::trace::TraceEvent::PhaseSnapshot {
            phase: "rows_placed".into(),
            entities: row_entities.clone(),
            width: row_width.max(actual_bw),
            height: total_height,
        });
    }
    crate::trace::emit(crate::trace::TraceEvent::PhaseComplete {
        phase: "lanes_planned".into(),
        entity_count: row_entities.len(),
    });
    if crate::trace::is_active() {
        crate::trace::emit(crate::trace::TraceEvent::PhaseSnapshot {
            phase: "lanes_planned".into(),
            entities: row_entities.clone(),
            width: row_width.max(actual_bw),
            height: total_height,
        });
    }

    // Place power poles from machine positions before routing so the router
    // sees them as hard obstacles. The occupied set reserves row-entity tiles
    // AND planned fluid-lane columns so poles don't land where the router
    // will later place pipe/PTG entities.
    let pole_entities: Vec<PlacedEntity> = {
        let mut row_occupied: FxHashSet<(i32, i32)> = FxHashSet::default();
        let mut machines_for_poles: Vec<(i32, i32, i32)> = Vec::new();
        for ent in &row_entities {
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
        for lane in &lanes {
            if !lane.is_fluid {
                continue;
            }
            let mut trunk_ys: Vec<i32> = vec![lane.source_y];
            trunk_ys.extend(lane.tap_off_ys.iter().copied());
            for &(_ri, _px, py) in &lane.fluid_output_port_positions {
                trunk_ys.push(py);
            }
            trunk_ys.sort_unstable();
            trunk_ys.dedup();
            for &y in &trunk_ys {
                row_occupied.insert((lane.x, y));
            }
            for pair in trunk_ys.windows(2) {
                let (y0, y1) = (pair[0], pair[1]);
                if y1 - y0 > 1 {
                    row_occupied.insert((lane.x, y0 + 1));
                }
                if y1 - y0 > 2 {
                    row_occupied.insert((lane.x, y1 - 1));
                }
            }
            let all_ports = lane.fluid_port_positions.iter()
                .chain(lane.fluid_output_port_positions.iter());
            for &(_ri, port_x, port_y) in all_ports {
                row_occupied.insert((port_x, port_y));
                row_occupied.insert((lane.x, port_y));
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
        let poles = place_poles(&machines_for_poles, &row_occupied);
        crate::trace::emit(crate::trace::TraceEvent::PolesPlaced {
            count: poles.len(),
            strategy: pole_strategy.to_string(),
        });
        crate::trace::emit(crate::trace::TraceEvent::PhaseComplete {
            phase: "poles_placed".into(),
            entity_count: poles.len(),
        });
        poles
    };

    // Route all connecting belts via the ghost routing pipeline.
    // See `docs/ghost-pipeline-contracts.md` for the phase-by-phase
    // contract and `ghost_router.rs` for the implementation.
    #[cfg(not(target_arch = "wasm32"))]
    let t_ghost = std::time::Instant::now();
    let ghost_result = crate::bus::ghost_router::route_bus_ghost(
        &lanes,
        &row_spans,
        total_height,
        actual_bw,
        max_belt_tier,
        solver_result,
        &families,
        &row_entities,
    )?;
    let bus_entities = ghost_result.entities;
    let max_y = ghost_result.max_y;
    let merge_max_x = ghost_result.merge_max_x;
    let regions = ghost_result.regions;
    let ghost_warnings = ghost_result.warnings;
    #[cfg(not(target_arch = "wasm32"))]
    crate::trace::emit(crate::trace::TraceEvent::PhaseTime {
        phase: "ghost_routing".to_string(),
        duration_ms: t_ghost.elapsed().as_millis() as u64,
    });
    crate::trace::emit(crate::trace::TraceEvent::PhaseComplete {
        phase: "bus_routed".into(),
        entity_count: bus_entities.len(),
    });
    emit_inter_row_bands(&row_spans, &lanes);
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

/// Streaming variant — mirrors every emitted `TraceEvent` to `on_event` as it
/// happens, and also returns them in `LayoutResult.trace` at the end. Used by
/// the web app to render pipeline progress live while the engine runs.
pub fn build_bus_layout_streaming(
    solver_result: &SolverResult,
    max_belt_tier: Option<&str>,
    mut on_event: Box<dyn FnMut(&crate::trace::TraceEvent)>,
) -> Result<LayoutResult, String> {
    let _collector_guard = crate::trace::start_trace();
    let _sink_guard = crate::trace::set_sink(Box::new(move |evt| on_event(evt)));
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

}
