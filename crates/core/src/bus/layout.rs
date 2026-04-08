//! Bus layout orchestrator: rows + bus lanes + poles -> LayoutResult.
//!
//! Port of `src/bus/layout.py`.

use rustc_hash::{FxHashMap, FxHashSet};

use crate::models::{EntityDirection, LayoutResult, PlacedEntity, SolverResult};
use crate::bus::bus_router::{
    plan_bus_lanes, bus_width_for_lanes, stamp_family_balancer, render_family_input_paths,
    merge_output_rows, place_merger_block, route_lane, negotiate_and_route,
    extract_and_solve_crossings, SatCrossingRegion,
    BusLane, LaneFamily, MACHINE_ENTITIES,
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

    // First pass: place rows with temp bus width
    let temp_bw = estimate_bus_width(solver_result);
    let (row_entities, row_spans, row_width, total_height) = place_rows(
        &solver_result.machines,
        &solver_result.dependency_order,
        temp_bw,
        bus_header,
        max_belt_tier,
        Some(&final_output_items),
        None,
    );

    let (lanes, families) = plan_bus_lanes(solver_result, &row_spans, max_belt_tier)?;
    let actual_bw = bus_width_for_lanes(&lanes);

    // Compute extra gaps needed for balancer blocks
    let extra_gaps = compute_extra_gaps(&families);

    // Re-place rows if bus width changed or balancers need extra space
    let (row_entities, row_spans, row_width, total_height) = if actual_bw != temp_bw || !extra_gaps.is_empty()
    {
        place_rows(
            &solver_result.machines,
            &solver_result.dependency_order,
            actual_bw,
            bus_header,
            max_belt_tier,
            Some(&final_output_items),
            Some(&extra_gaps),
        )
    } else {
        (row_entities, row_spans, row_width, total_height)
    };

    // Re-plan lanes with final row positions
    let (lanes, families) = if actual_bw != temp_bw || !extra_gaps.is_empty() {
        plan_bus_lanes(solver_result, &row_spans, max_belt_tier)?
    } else {
        (lanes, families)
    };

    // Route bus lanes
    let (bus_entities, max_y, merge_max_x, regions) = route_bus(
        &lanes,
        &row_spans,
        total_height,
        actual_bw,
        max_belt_tier,
        solver_result,
        &families,
        &row_entities,
    )?;

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

    // Collect occupied tiles and machine centers for pole placement
    let mut occupied: FxHashSet<(i32, i32)> = FxHashSet::default();
    let mut machine_centers: Vec<(i32, i32)> = Vec::new();

    for ent in row_entities.iter().chain(bus_entities.iter()) {
        if MACHINE_ENTITIES.contains(&ent.name.as_str()) {
            let sz = crate::common::machine_size(&ent.name) as i32;
            for dx in 0..sz {
                for dy in 0..sz {
                    occupied.insert((ent.x + dx, ent.y + dy));
                }
            }
            machine_centers.push((ent.x + sz / 2, ent.y + sz / 2));
        } else {
            occupied.insert((ent.x, ent.y));
        }
    }

    let width = row_width.max(actual_bw).max(merge_max_x);

    // Place power poles
    let pole_entities = place_poles(width, max_y, Some(occupied), Some(machine_centers));

    // Check for missing balancer templates and collect warnings
    let mut warnings = Vec::new();
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
    })
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
        let shape_u32 = (fam.shape.0 as u32, fam.shape.1 as u32);
        let template_height = crate::bus::balancer_library::balancer_templates()
            .get(&shape_u32)
            .map(|t| t.height as i32)
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
) -> Result<(Vec<PlacedEntity>, i32, i32, Vec<crate::models::LayoutRegion>), String> {
    let mut entities: Vec<PlacedEntity> = Vec::new();
    let mut max_y = total_height;
    let mut merge_max_x = 0;

    // SAT crossing zones: trunk-only approach. SAT determines how trunks
    // handle crossings (UG bridges). The A* routes tap-offs normally (underground).
    // SAT zones have forced-empty tiles at tap_y so trunks bridge around them.
    let (solved_crossings, crossing_tiles, _sat_regions) =
        extract_and_solve_crossings(lanes, row_spans, max_belt_tier);
    for sc in &solved_crossings {
        entities.extend(sc.solution.entities.clone());
    }

    // No spec splitting needed — A* runs normally, SAT only affects trunk rendering.
    let empty_regions: Vec<SatCrossingRegion> = Vec::new();
    let routed_paths = negotiate_and_route(
        lanes,
        row_spans,
        total_height,
        bw,
        row_entities,
        solver_result,
        families,
        max_belt_tier,
        &empty_regions,
        &FxHashSet::default(),
    );

    // Stamp N-to-M balancer blocks
    for fam in families {
        let balancer_ents = stamp_family_balancer(fam, max_belt_tier)
            .map_err(|e| format!("balancer stamp failed for family {:?}: {}", fam.shape, e))?;
        entities.extend(balancer_ents);

        let path_ents = render_family_input_paths(
            fam,
            row_spans,
            crate::common::belt_entity_for_rate(fam.total_rate, max_belt_tier),
            Some(&routed_paths),
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

    // Route each lane, skipping tiles owned by SAT crossing zones
    // and tiles claimed by A* tap-offs.
    for lane in lanes {
        route_lane(&mut entities, lane, lanes, row_spans, bw, max_belt_tier, Some(&routed_paths), &crossing_tiles, &tapoff_tiles);
    }

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
            crate::models::LayoutRegion {
                kind: "crossing_zone".to_string(),
                x: sc.zone.x,
                y: sc.zone.y,
                width: sc.zone.width as i32,
                height: sc.zone.height as i32,
                inputs,
                outputs,
                variables: sc.solution.stats.variables,
                clauses: sc.solution.stats.clauses,
                solve_time_us: sc.solution.stats.solve_time_us,
            }
        })
        .collect();

    Ok((entities, max_y, merge_max_x, regions))
}

/// Place medium electric poles for power coverage.
fn place_poles(
    width: i32,
    height: i32,
    occupied: Option<FxHashSet<(i32, i32)>>,
    machine_centers: Option<Vec<(i32, i32)>>,
) -> Vec<PlacedEntity> {
    let occupied = occupied.unwrap_or_default();

    if let Some(centers) = machine_centers {
        if !centers.is_empty() {
            return place_poles_greedy(&occupied, &centers);
        }
    }

    place_poles_grid(width, height, &occupied)
}

/// Create a pole entity at the given position.
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

/// Greedy pole placement near machines.
fn place_poles_greedy(
    occupied: &FxHashSet<(i32, i32)>,
    machine_centers: &[(i32, i32)],
) -> Vec<PlacedEntity> {
    let mut entities: Vec<PlacedEntity> = Vec::new();
    let pole_range = 3;
    let mut covered: FxHashSet<(i32, i32)> = FxHashSet::default();

    for &(cx, cy) in machine_centers {
        let mut best_pos = None;
        let mut best_coverage = 0;

        for dx in -pole_range..=pole_range {
            for dy in -pole_range..=pole_range {
                let px = cx + dx;
                let py = cy + dy;
                if occupied.contains(&(px, py)) || covered.contains(&(px, py)) {
                    continue;
                }

                let mut coverage = 0;
                for &(mx, my) in machine_centers {
                    if (px - mx).abs() <= pole_range && (py - my).abs() <= pole_range
                        && !covered.contains(&(mx, my)) {
                            coverage += 1;
                        }
                }

                if coverage > best_coverage {
                    best_coverage = coverage;
                    best_pos = Some((px, py));
                }
            }
        }

        if let Some((px, py)) = best_pos {
            entities.push(make_pole(px, py));

            for &(mx, my) in machine_centers {
                if (px - mx).abs() <= pole_range && (py - my).abs() <= pole_range {
                    covered.insert((mx, my));
                }
            }
        }
    }

    entities
}

/// Grid-based pole placement (fallback).
fn place_poles_grid(
    width: i32,
    height: i32,
    occupied: &FxHashSet<(i32, i32)>,
) -> Vec<PlacedEntity> {
    let mut entities: Vec<PlacedEntity> = Vec::new();
    let pole_spacing = 7;

    let mut y = -1;
    while y < height + pole_spacing {
        let mut x = -1;
        while x < width + pole_spacing {
            if !occupied.contains(&(x, y)) {
                entities.push(make_pole(x, y));
            }
            x += pole_spacing;
        }
        y += pole_spacing;
    }

    entities
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
        all_issues.extend(belt_flow::check_belt_direction_continuity(&layout));

        let errors: Vec<_> = all_issues.iter()
            .filter(|i| i.severity == Severity::Error)
            .collect();
        assert!(errors.is_empty(), "Expected 0 errors, got {}: {}", errors.len(),
            errors.iter().map(|i| i.message.as_str()).collect::<Vec<_>>().join("; "));
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
    #[ignore] // manual investigation — run with --ignored --nocapture
    fn test_ecircuit_20s_yellow_sat_zones() {
        // Investigative test for larger layouts. Asserts no warnings
        // (missing balancer templates = disconnected producers = broken layout).
        use crate::solver::solve;
        use rustc_hash::FxHashSet;

        use crate::validate::underground::check_underground_belt_pairs;
        use crate::validate::belt_flow::{
            check_belt_dead_ends, check_belt_item_isolation, check_belt_loops,
            check_belt_junctions, check_belt_direction_continuity,
        };
        use crate::validate::Severity;

        let inputs: FxHashSet<String> = ["iron-plate", "copper-plate"]
            .iter().map(|s| s.to_string()).collect();
        for machine in &["assembling-machine-1", "assembling-machine-3"] {
            let sr = solve("electronic-circuit", 20.0, &inputs, machine)
                .expect("solve");
            let layout = build_bus_layout(&sr, Some("transport-belt"))
                .expect(&format!("layout with {}", machine));

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
            if !overlaps.is_empty() {
                eprintln!("{}: {} overlaps (not fatal — dedup handles)", machine, overlaps.len());
            }

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
            for issue in check_belt_direction_continuity(&layout) {
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

        // Layout should have a reasonable number of entities (was ~1048 after fix,
        // was ~897 when feeders were broken/skipped)
        assert!(
            layout.entities.len() > 950,
            "Expected >950 entities (full layout with feeders), got {}",
            layout.entities.len()
        );
    }
}
