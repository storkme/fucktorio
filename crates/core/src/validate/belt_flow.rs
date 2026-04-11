//! Belt connectivity, flow paths, reachability, network topology, junctions.
//!
//! Port of the belt-check functions from `src/validate.py`:
//! - `check_belt_connectivity`
//! - `check_belt_flow_path`
//! - `check_belt_network_topology`
//! - `check_belt_junctions`
//! - `check_belt_flow_reachability`
//!   Plus underground-belt helpers used by those checks.

use std::collections::VecDeque;

use rustc_hash::{FxHashMap, FxHashSet};

use crate::common::{
    dir_to_vec, fluid_only_recipes, inserter_reach, inserter_target_lane, is_belt_entity,
    is_inserter, is_machine_entity, is_splitter, is_surface_belt, is_ug_belt,
    splitter_second_tile, ug_max_reach, ug_to_surface_tier, lane_capacity, machine_size,
    machine_tiles, LANE_LEFT,
};
use crate::models::{EntityDirection, LayoutResult, PlacedEntity, SolverResult};

use super::{LayoutStyle, Severity, ValidationIssue};

// ---------------------------------------------------------------------------
// Belt direction map (including splitter expansion)
// ---------------------------------------------------------------------------

fn belt_dir_map_from(entities: &[PlacedEntity]) -> FxHashMap<(i32, i32), EntityDirection> {
    belt_dir_map_filtered(entities, false)
}

fn belt_dir_map_filtered(entities: &[PlacedEntity], skip_balancers: bool) -> FxHashMap<(i32, i32), EntityDirection> {
    let mut bdm = FxHashMap::default();
    for e in entities {
        if !is_belt_entity(&e.name) {
            continue;
        }
        if skip_balancers {
            if let Some(ref seg) = e.segment_id {
                if seg.starts_with("balancer:") {
                    continue;
                }
            }
        }
        bdm.insert((e.x, e.y), e.direction);
        if is_splitter(&e.name) {
            let second = splitter_second_tile(e);
            bdm.insert(second, e.direction);
        }
    }
    bdm
}

// ---------------------------------------------------------------------------
// Belt tile set (including splitter expansion)
// ---------------------------------------------------------------------------

fn build_belt_tile_set(entities: &[PlacedEntity]) -> FxHashSet<(i32, i32)> {
    let mut tiles = FxHashSet::default();
    for e in entities {
        if is_belt_entity(&e.name) {
            tiles.insert((e.x, e.y));
            if is_splitter(&e.name) {
                tiles.insert(splitter_second_tile(e));
            }
        }
    }
    tiles
}

// ---------------------------------------------------------------------------
// Underground belt pair map
// ---------------------------------------------------------------------------

fn build_ug_pairs(layout: &LayoutResult) -> FxHashMap<(i32, i32), (i32, i32)> {
    let mut ug_inputs: Vec<&PlacedEntity> = Vec::new();
    let mut ug_outputs: Vec<&PlacedEntity> = Vec::new();
    for e in &layout.entities {
        if is_ug_belt(&e.name) {
            match e.io_type.as_deref() {
                Some("input") => ug_inputs.push(e),
                Some("output") => ug_outputs.push(e),
                _ => {}
            }
        }
    }

    let mut pairs: FxHashMap<(i32, i32), (i32, i32)> = FxHashMap::default();
    let mut used_outputs: FxHashSet<(i32, i32)> = FxHashSet::default();

    for inp in &ug_inputs {
        let (dx, dy) = dir_to_vec(inp.direction);
        let mut best_out: Option<&PlacedEntity> = None;
        let mut best_dist = i32::MAX;

        for out in &ug_outputs {
            if used_outputs.contains(&(out.x, out.y)) {
                continue;
            }
            if out.direction != inp.direction {
                continue;
            }
            let rx = out.x - inp.x;
            let ry = out.y - inp.y;
            let dist = if dx != 0 {
                if ry != 0 || (rx > 0) != (dx > 0) {
                    continue;
                }
                rx.abs()
            } else {
                if rx != 0 || (ry > 0) != (dy > 0) {
                    continue;
                }
                ry.abs()
            };
            if dist > 1 && dist < best_dist {
                best_dist = dist;
                best_out = Some(out);
            }
        }

        if let Some(out) = best_out {
            pairs.insert((inp.x, inp.y), (out.x, out.y));
            pairs.insert((out.x, out.y), (inp.x, inp.y));
            used_outputs.insert((out.x, out.y));
        }
    }
    pairs
}

// ---------------------------------------------------------------------------
// Splitter sibling map
// ---------------------------------------------------------------------------

fn build_splitter_siblings(layout: &LayoutResult) -> FxHashMap<(i32, i32), (i32, i32)> {
    let mut siblings: FxHashMap<(i32, i32), (i32, i32)> = FxHashMap::default();
    for e in &layout.entities {
        if !is_splitter(&e.name) {
            continue;
        }
        let second = splitter_second_tile(e);
        siblings.insert((e.x, e.y), second);
        siblings.insert(second, (e.x, e.y));
    }
    siblings
}

// ---------------------------------------------------------------------------
// BFS helpers
// ---------------------------------------------------------------------------

fn bfs_belt_reach(
    starts: &FxHashSet<(i32, i32)>,
    belt_tiles: &FxHashSet<(i32, i32)>,
    ug_pairs: Option<&FxHashMap<(i32, i32), (i32, i32)>>,
) -> FxHashSet<(i32, i32)> {
    let mut visited: FxHashSet<(i32, i32)> = FxHashSet::default();
    let mut queue: VecDeque<(i32, i32)> = VecDeque::new();
    for &s in starts {
        if visited.insert(s) {
            queue.push_back(s);
        }
    }
    while let Some((x, y)) = queue.pop_front() {
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let nb = (x + dx, y + dy);
            if belt_tiles.contains(&nb) && visited.insert(nb) {
                queue.push_back(nb);
            }
        }
        if let Some(pairs) = ug_pairs {
            if let Some(&paired) = pairs.get(&(x, y)) {
                if belt_tiles.contains(&paired) && visited.insert(paired) {
                    queue.push_back(paired);
                }
            }
        }
    }
    visited
}

fn bfs_belt_downstream(
    starts: &FxHashSet<(i32, i32)>,
    belt_dir_map: &FxHashMap<(i32, i32), EntityDirection>,
    ug_pairs: Option<&FxHashMap<(i32, i32), (i32, i32)>>,
    splitter_siblings: Option<&FxHashMap<(i32, i32), (i32, i32)>>,
) -> FxHashSet<(i32, i32)> {
    let mut visited: FxHashSet<(i32, i32)> = FxHashSet::default();
    let mut queue: VecDeque<(i32, i32)> = VecDeque::new();
    for &s in starts {
        if belt_dir_map.contains_key(&s) && visited.insert(s) {
            queue.push_back(s);
        }
    }
    while let Some((x, y)) = queue.pop_front() {
        if let Some(&d) = belt_dir_map.get(&(x, y)) {
            let (dx, dy) = dir_to_vec(d);
            let nb = (x + dx, y + dy);
            if belt_dir_map.contains_key(&nb) && visited.insert(nb) {
                queue.push_back(nb);
            }
        }
        if let Some(pairs) = ug_pairs {
            if let Some(&paired) = pairs.get(&(x, y)) {
                if belt_dir_map.contains_key(&paired) && visited.insert(paired) {
                    queue.push_back(paired);
                }
            }
        }
        if let Some(siblings) = splitter_siblings {
            if let Some(&sib) = siblings.get(&(x, y)) {
                if belt_dir_map.contains_key(&sib) && visited.insert(sib) {
                    queue.push_back(sib);
                }
            }
        }
    }
    visited
}

fn bfs_belt_upstream(
    starts: &FxHashSet<(i32, i32)>,
    belt_dir_map: &FxHashMap<(i32, i32), EntityDirection>,
    ug_pairs: Option<&FxHashMap<(i32, i32), (i32, i32)>>,
    splitter_siblings: Option<&FxHashMap<(i32, i32), (i32, i32)>>,
) -> FxHashSet<(i32, i32)> {
    let mut visited: FxHashSet<(i32, i32)> = FxHashSet::default();
    let mut queue: VecDeque<(i32, i32)> = VecDeque::new();
    for &s in starts {
        if belt_dir_map.contains_key(&s) && visited.insert(s) {
            queue.push_back(s);
        }
    }
    while let Some((x, y)) = queue.pop_front() {
        // Underground tunnel jump (reverse)
        if let Some(pairs) = ug_pairs {
            if let Some(&paired) = pairs.get(&(x, y)) {
                if belt_dir_map.contains_key(&paired) && visited.insert(paired) {
                    queue.push_back(paired);
                }
            }
        }
        // Splitter sibling
        if let Some(siblings) = splitter_siblings {
            if let Some(&sib) = siblings.get(&(x, y)) {
                if belt_dir_map.contains_key(&sib) && visited.insert(sib) {
                    queue.push_back(sib);
                }
            }
        }
        // Upstream neighbours: tiles whose direction points at (x, y)
        for (ddx, ddy) in [(1, 0i32), (-1, 0), (0, 1), (0, -1)] {
            let (nx, ny) = (x + ddx, y + ddy);
            if let Some(&nd) = belt_dir_map.get(&(nx, ny)) {
                let (ndx, ndy) = dir_to_vec(nd);
                if (nx + ndx, ny + ndy) == (x, y) && visited.insert((nx, ny)) {
                    queue.push_back((nx, ny));
                }
            }
        }
    }
    visited
}

// ---------------------------------------------------------------------------
// Machine tile helpers
// ---------------------------------------------------------------------------

fn build_machine_tile_set(layout: &LayoutResult) -> FxHashSet<(i32, i32)> {
    let mut tiles = FxHashSet::default();
    for e in &layout.entities {
        if is_machine_entity(&e.name) {
            let size = machine_size(&e.name);
            for t in machine_tiles(e.x, e.y, size) {
                tiles.insert(t);
            }
        }
    }
    tiles
}

/// Map each machine tile → machine origin `(e.x, e.y)`.
fn build_machine_by_tile(layout: &LayoutResult) -> FxHashMap<(i32, i32), (i32, i32)> {
    let mut by_tile = FxHashMap::default();
    for e in &layout.entities {
        if is_machine_entity(&e.name) {
            let size = machine_size(&e.name);
            for t in machine_tiles(e.x, e.y, size) {
                by_tile.insert(t, (e.x, e.y));
            }
        }
    }
    by_tile
}

// ---------------------------------------------------------------------------
// 1. check_belt_connectivity
// ---------------------------------------------------------------------------

pub fn check_belt_connectivity(
    layout: &LayoutResult,
    solver: Option<&SolverResult>,
) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    let fluid_only = fluid_only_recipes(solver);
    let belt_tiles = build_belt_tile_set(&layout.entities);
    let ug_pairs = build_ug_pairs(layout);
    let inserter_positions: FxHashSet<(i32, i32)> = layout
        .entities
        .iter()
        .filter(|e| is_inserter(&e.name))
        .map(|e| (e.x, e.y))
        .collect();

    if belt_tiles.is_empty() {
        let has_solid = layout.entities.iter().any(|e| {
            is_machine_entity(&e.name)
                && e.recipe
                    .as_deref()
                    .is_none_or(|r| !fluid_only.contains(r))
        });
        if has_solid {
            issues.push(ValidationIssue::new(
                Severity::Error,
                "belt-connectivity",
                "No belts in layout but machines require solid item transport",
            ));
        }
        return issues;
    }

    let mut checked: FxHashSet<(i32, i32)> = FxHashSet::default();
    for e in &layout.entities {
        if !is_machine_entity(&e.name) {
            continue;
        }
        if !checked.insert((e.x, e.y)) {
            continue;
        }
        if e.recipe.as_deref().is_some_and(|r| fluid_only.contains(r)) {
            continue;
        }

        let size = machine_size(&e.name) as i32;
        let my_tiles: FxHashSet<(i32, i32)> = (0..size)
            .flat_map(|dx| (0..size).map(move |dy| (e.x + dx, e.y + dy)))
            .collect();

        // Adjacent inserters
        let mut adjacent_inserters: Vec<(i32, i32)> = Vec::new();
        for dx in -1..=size {
            for dy in -1..=size {
                let pos = (e.x + dx, e.y + dy);
                if inserter_positions.contains(&pos) && !my_tiles.contains(&pos) {
                    adjacent_inserters.push(pos);
                }
            }
        }

        // Check if any inserter has a belt on its non-machine side
        let mut has_belt_connection = false;
        'outer: for (ix, iy) in &adjacent_inserters {
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let nb = (ix + dx, iy + dy);
                if belt_tiles.contains(&nb) && !my_tiles.contains(&nb) {
                    has_belt_connection = true;
                    break 'outer;
                }
            }
        }

        if !has_belt_connection {
            issues.push(ValidationIssue::with_pos(
                Severity::Error,
                "belt-connectivity",
                format!(
                    "{} at ({},{}): no inserter connects to a belt \
                     (inserters exist but none touch a belt tile)",
                    e.name, e.x, e.y
                ),
                e.x,
                e.y,
            ));
            continue;
        }

        // Collect starting belt tiles from inserters adjacent to this machine
        let mut start_belt_tiles: FxHashSet<(i32, i32)> = FxHashSet::default();
        for (ix, iy) in &adjacent_inserters {
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let nb = (ix + dx, iy + dy);
                if belt_tiles.contains(&nb) && !my_tiles.contains(&nb) {
                    start_belt_tiles.insert(nb);
                }
            }
        }

        let belt_network = bfs_belt_reach(&start_belt_tiles, &belt_tiles, Some(&ug_pairs));
        if belt_network.len() <= 1 {
            issues.push(ValidationIssue::with_pos(
                Severity::Error,
                "belt-connectivity",
                format!(
                    "{} at ({},{}): belt adjacent to inserter is isolated (single tile, not connected to anything)",
                    e.name, e.x, e.y
                ),
                e.x,
                e.y,
            ));
        }
    }

    issues
}

// ---------------------------------------------------------------------------
// 2. check_belt_flow_path
// ---------------------------------------------------------------------------

pub fn check_belt_flow_path(
    layout: &LayoutResult,
    solver: Option<&SolverResult>,
    style: LayoutStyle,
) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    let fluid_only = fluid_only_recipes(solver);
    let ug_pairs = build_ug_pairs(layout);
    let belt_tiles = build_belt_tile_set(&layout.entities);

    if belt_tiles.is_empty() {
        return issues;
    }

    let mut inserter_entities: Vec<&PlacedEntity> = Vec::new();
    let mut inserter_positions: FxHashSet<(i32, i32)> = FxHashSet::default();
    for e in &layout.entities {
        if is_inserter(&e.name) {
            inserter_entities.push(e);
            inserter_positions.insert((e.x, e.y));
        }
    }

    let all_machine_tiles = build_machine_tile_set(layout);

    // Classify inserters as input (drops into machine) or output (picks from machine)
    let mut input_inserter_positions: FxHashSet<(i32, i32)> = FxHashSet::default();
    let mut output_inserter_positions: FxHashSet<(i32, i32)> = FxHashSet::default();
    for ins in &inserter_entities {
        let (dx, dy) = dir_to_vec(ins.direction);
        let reach = inserter_reach(&ins.name);
        let drop_pos = (ins.x + dx * reach, ins.y + dy * reach);
        let pickup_pos = (ins.x - dx * reach, ins.y - dy * reach);
        if all_machine_tiles.contains(&drop_pos) {
            input_inserter_positions.insert((ins.x, ins.y));
        }
        if all_machine_tiles.contains(&pickup_pos) {
            output_inserter_positions.insert((ins.x, ins.y));
        }
    }

    // Layout boundary from belt positions
    let all_xs: Vec<i32> = belt_tiles.iter().map(|&(x, _)| x).collect();
    let all_ys: Vec<i32> = belt_tiles.iter().map(|&(_, y)| y).collect();
    let min_bx = *all_xs.iter().min().unwrap();
    let max_bx = *all_xs.iter().max().unwrap();
    let min_by = *all_ys.iter().min().unwrap();
    let max_by = *all_ys.iter().max().unwrap();

    let on_boundary = |bx: i32, by: i32| -> bool {
        bx == min_bx || bx == max_bx || by == min_by || by == max_by
    };
    let network_reaches_boundary = |network: &FxHashSet<(i32, i32)>| -> bool {
        network.len() >= 3 && network.iter().any(|&(bx, by)| on_boundary(bx, by))
    };

    // Recipes with solid outputs
    let mut solid_output_recipes: FxHashSet<String> = FxHashSet::default();
    if let Some(sr) = solver {
        for ms in &sr.machines {
            if ms.outputs.iter().any(|o| !o.is_fluid) {
                solid_output_recipes.insert(ms.recipe.clone());
            }
        }
    }

    let severity = if style == LayoutStyle::Spaghetti {
        Severity::Error
    } else {
        Severity::Warning
    };

    let mut checked: FxHashSet<(i32, i32)> = FxHashSet::default();
    let machine_entities: Vec<&PlacedEntity> = layout
        .entities
        .iter()
        .filter(|e| is_machine_entity(&e.name))
        .collect();

    for e in &machine_entities {
        if !checked.insert((e.x, e.y)) {
            continue;
        }
        if e.recipe.as_deref().is_some_and(|r| fluid_only.contains(r)) {
            continue;
        }

        let size = machine_size(&e.name) as i32;
        let my_tiles: FxHashSet<(i32, i32)> = (0..size)
            .flat_map(|dx| (0..size).map(move |dy| (e.x + dx, e.y + dy)))
            .collect();

        // Helper: belt tiles adjacent to this machine's inserters of a given type
        let belt_tiles_near_inserters = |target: &FxHashSet<(i32, i32)>| -> FxHashSet<(i32, i32)> {
            let mut result = FxHashSet::default();
            for dx in -1..=size {
                for dy in -1..=size {
                    let ipos = (e.x + dx, e.y + dy);
                    if !target.contains(&ipos) || my_tiles.contains(&ipos) {
                        continue;
                    }
                    for (ddx, ddy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                        let nb = (ipos.0 + ddx, ipos.1 + ddy);
                        if belt_tiles.contains(&nb) && !my_tiles.contains(&nb) {
                            result.insert(nb);
                        }
                    }
                }
            }
            result
        };

        // --- Input path check ---
        let input_belt_starts = belt_tiles_near_inserters(&input_inserter_positions);
        if !input_belt_starts.is_empty() {
            let network = bfs_belt_reach(&input_belt_starts, &belt_tiles, Some(&ug_pairs));
            let mut reaches_source = false;
            'outer: for &(bx, by) in &network {
                for (ddx, ddy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let adj = (bx + ddx, by + ddy);
                    if inserter_positions.contains(&adj)
                        && !my_tiles.contains(&adj)
                        && !input_inserter_positions.contains(&adj)
                    {
                        reaches_source = true;
                        break 'outer;
                    }
                    if inserter_positions.contains(&adj) && !my_tiles.contains(&adj) {
                        for (ddx2, ddy2) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                            let adj2 = (adj.0 + ddx2, adj.1 + ddy2);
                            if all_machine_tiles.contains(&adj2) && !my_tiles.contains(&adj2) {
                                reaches_source = true;
                                break 'outer;
                            }
                        }
                    }
                }
            }
            if !reaches_source && !network_reaches_boundary(&network) {
                issues.push(ValidationIssue::with_pos(
                    severity,
                    "belt-flow-path",
                    format!(
                        "{} at ({},{}): input belt network ({} tiles) \
                         doesn't reach any source (other machine or layout boundary)",
                        e.name,
                        e.x,
                        e.y,
                        network.len()
                    ),
                    e.x,
                    e.y,
                ));
            }
        }

        // --- Output path check ---
        let has_solid_output = solver.is_none_or(|_| {
            e.recipe
                .as_deref()
                .is_some_and(|r| solid_output_recipes.contains(r))
        });
        if !has_solid_output {
            continue;
        }
        let output_belt_starts = belt_tiles_near_inserters(&output_inserter_positions);
        if output_belt_starts.is_empty() {
            continue;
        }
        let network = bfs_belt_reach(&output_belt_starts, &belt_tiles, Some(&ug_pairs));

        let mut reaches_sink = false;
        'outer2: for &(bx, by) in &network {
            for (ddx, ddy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let adj = (bx + ddx, by + ddy);
                if input_inserter_positions.contains(&adj) && !my_tiles.contains(&adj) {
                    reaches_sink = true;
                    break 'outer2;
                }
            }
        }

        if !reaches_sink && !network_reaches_boundary(&network) {
            issues.push(ValidationIssue::with_pos(
                severity,
                "belt-flow-path",
                format!(
                    "{} at ({},{}): output belt network ({} tiles) \
                     doesn't reach any sink (other machine or layout boundary)",
                    e.name,
                    e.x,
                    e.y,
                    network.len()
                ),
                e.x,
                e.y,
            ));
        }
    }

    issues
}

// ---------------------------------------------------------------------------
// 3. check_belt_network_topology
// ---------------------------------------------------------------------------

pub fn check_belt_network_topology(
    layout: &LayoutResult,
    solver: Option<&SolverResult>,
) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    let sr = match solver {
        Some(s) => s,
        None => return issues,
    };

    // Build belt tile set with carries annotation, expanding splitters
    let mut belt_tiles: FxHashSet<(i32, i32)> = FxHashSet::default();
    let mut belt_carries: FxHashMap<(i32, i32), Option<String>> = FxHashMap::default();
    for e in &layout.entities {
        if is_belt_entity(&e.name) {
            belt_tiles.insert((e.x, e.y));
            belt_carries.insert((e.x, e.y), e.carries.clone());
            if is_splitter(&e.name) {
                let second = splitter_second_tile(e);
                belt_tiles.insert(second);
                belt_carries.insert(second, e.carries.clone());
            }
        }
    }
    if belt_tiles.is_empty() {
        return issues;
    }

    let machine_tiles_set = build_machine_tile_set(layout);
    let machine_by_tile = build_machine_by_tile(layout);

    // Per-machine belt tiles for input/output inserters
    let mut input_inserter_belt_tiles: FxHashMap<(i32, i32), Vec<(i32, i32)>> =
        FxHashMap::default();
    let mut output_inserter_belt_tiles: FxHashMap<(i32, i32), Vec<(i32, i32)>> =
        FxHashMap::default();

    for ins in &layout.entities {
        if !is_inserter(&ins.name) {
            continue;
        }
        let (dx, dy) = dir_to_vec(ins.direction);
        let reach = inserter_reach(&ins.name);
        let drop_pos = (ins.x + dx * reach, ins.y + dy * reach);
        let pickup_pos = (ins.x - dx * reach, ins.y - dy * reach);

        if machine_tiles_set.contains(&drop_pos) && belt_tiles.contains(&pickup_pos) {
            if let Some(&mpos) = machine_by_tile.get(&drop_pos) {
                input_inserter_belt_tiles
                    .entry(mpos)
                    .or_default()
                    .push(pickup_pos);
            }
        } else if machine_tiles_set.contains(&pickup_pos) && belt_tiles.contains(&drop_pos) {
            if let Some(&mpos) = machine_by_tile.get(&pickup_pos) {
                output_inserter_belt_tiles
                    .entry(mpos)
                    .or_default()
                    .push(drop_pos);
            }
        }
    }

    let ug_pairs = build_ug_pairs(layout);

    // Layout boundary
    let all_xs: Vec<i32> = belt_tiles.iter().map(|&(x, _)| x).collect();
    let all_ys: Vec<i32> = belt_tiles.iter().map(|&(_, y)| y).collect();
    let min_bx = *all_xs.iter().min().unwrap();
    let max_bx = *all_xs.iter().max().unwrap();
    let min_by = *all_ys.iter().min().unwrap();
    let max_by = *all_ys.iter().max().unwrap();

    let on_boundary = |(x, y): (i32, i32)| -> bool {
        x == min_bx || x == max_bx || y == min_by || y == max_by
    };

    // Group machines by recipe
    let mut recipe_machines: FxHashMap<&str, Vec<(i32, i32)>> = FxHashMap::default();
    for e in &layout.entities {
        if is_machine_entity(&e.name) {
            if let Some(r) = e.recipe.as_deref() {
                recipe_machines.entry(r).or_default().push((e.x, e.y));
            }
        }
    }

    let external_input_items: FxHashSet<&str> = sr
        .external_inputs
        .iter()
        .filter(|f| !f.is_fluid)
        .map(|f| f.item.as_str())
        .collect();
    let external_output_items: FxHashSet<&str> = sr
        .external_outputs
        .iter()
        .filter(|f| !f.is_fluid)
        .map(|f| f.item.as_str())
        .collect();

    // item → consumer recipes (external inputs)
    let mut item_to_consumer_recipes: FxHashMap<&str, FxHashSet<&str>> = FxHashMap::default();
    for spec in &sr.machines {
        for inp in &spec.inputs {
            if external_input_items.contains(inp.item.as_str()) && !inp.is_fluid {
                item_to_consumer_recipes
                    .entry(&inp.item)
                    .or_default()
                    .insert(&spec.recipe);
            }
        }
    }

    // item → producer recipes (external outputs)
    let mut item_to_producer_recipes: FxHashMap<&str, FxHashSet<&str>> = FxHashMap::default();
    for spec in &sr.machines {
        for out in &spec.outputs {
            if external_output_items.contains(out.item.as_str()) && !out.is_fluid {
                item_to_producer_recipes
                    .entry(&out.item)
                    .or_default()
                    .insert(&spec.recipe);
            }
        }
    }

    // Inner check function
    let mut check_network = |item: &str,
                              direction: &str,
                              belt_starts: &Vec<(i32, i32)>,
                              machine_list: &Vec<(i32, i32)>| {
        if belt_starts.is_empty() {
            return;
        }
        // Filter belt tiles to only those carrying this item
        let item_belt_tiles: FxHashSet<(i32, i32)> = belt_tiles
            .iter()
            .filter(|&&pos| belt_carries.get(&pos).and_then(|c| c.as_deref()) == Some(item))
            .copied()
            .collect();

        let starts_set: FxHashSet<(i32, i32)> = belt_starts.iter().copied().collect();
        let full_network = bfs_belt_reach(&starts_set, &item_belt_tiles, Some(&ug_pairs));

        // Check connectivity
        if belt_starts.len() > 1 {
            let first_set: FxHashSet<(i32, i32)> =
                std::iter::once(belt_starts[0]).collect();
            let first_network = bfs_belt_reach(&first_set, &item_belt_tiles, Some(&ug_pairs));
            let unreachable: Vec<(i32, i32)> = belt_starts[1..]
                .iter()
                .filter(|&&bt| !first_network.contains(&bt))
                .copied()
                .collect();
            if !unreachable.is_empty() {
                issues.push(ValidationIssue::new(
                    Severity::Error,
                    "belt-topology",
                    format!(
                        "{} {}: {} disconnected belt networks for {} machines \
                         (should be a single connected network)",
                        item,
                        direction,
                        unreachable.len() + 1,
                        machine_list.len()
                    ),
                ));
                return;
            }
        }

        let boundary_tiles: Vec<(i32, i32)> = full_network
            .iter()
            .filter(|&&t| on_boundary(t))
            .copied()
            .collect();

        if boundary_tiles.is_empty() {
            issues.push(ValidationIssue::new(
                Severity::Error,
                "belt-topology",
                format!(
                    "{} {}: belt network ({} tiles) doesn't reach layout boundary",
                    item,
                    direction,
                    full_network.len()
                ),
            ));
            return;
        }

        // Check boundary tiles are contiguous
        let boundary_set: FxHashSet<(i32, i32)> = boundary_tiles.iter().copied().collect();
        let mut bfs_visited: FxHashSet<(i32, i32)> = FxHashSet::default();
        let mut bfs_queue: VecDeque<(i32, i32)> = VecDeque::new();
        bfs_queue.push_back(boundary_tiles[0]);
        bfs_visited.insert(boundary_tiles[0]);
        while let Some((bx, by)) = bfs_queue.pop_front() {
            for (ddx, ddy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let nb = (bx + ddx, by + ddy);
                if boundary_set.contains(&nb) && bfs_visited.insert(nb) {
                    bfs_queue.push_back(nb);
                }
            }
        }
        if bfs_visited.len() < boundary_set.len() {
            issues.push(ValidationIssue::new(
                Severity::Warning,
                "belt-topology",
                format!(
                    "{} {}: belt network reaches layout boundary at multiple \
                     separate locations (ideally one contiguous entry/exit point)",
                    item, direction
                ),
            ));
        }
    };

    // Check input networks
    for (item, recipes) in &item_to_consumer_recipes {
        let mut input_belt_starts: Vec<(i32, i32)> = Vec::new();
        let mut consuming_machines: Vec<(i32, i32)> = Vec::new();
        for &recipe in recipes {
            for &mpos in recipe_machines.get(recipe).unwrap_or(&vec![]) {
                if let Some(bt_list) = input_inserter_belt_tiles.get(&mpos) {
                    let matched: Vec<(i32, i32)> = bt_list
                        .iter()
                        .filter(|&&pos| {
                            belt_carries
                                .get(&pos)
                                .and_then(|c| c.as_deref())
                                == Some(*item)
                        })
                        .copied()
                        .collect();
                    if !matched.is_empty() {
                        input_belt_starts.extend_from_slice(&matched);
                        consuming_machines.push(mpos);
                    }
                }
            }
        }
        check_network(item, "input", &input_belt_starts, &consuming_machines);
    }

    // Check output networks
    for (item, recipes) in &item_to_producer_recipes {
        let mut output_belt_starts: Vec<(i32, i32)> = Vec::new();
        let mut producing_machines: Vec<(i32, i32)> = Vec::new();
        for &recipe in recipes {
            for &mpos in recipe_machines.get(recipe).unwrap_or(&vec![]) {
                if let Some(bt_list) = output_inserter_belt_tiles.get(&mpos) {
                    let matched: Vec<(i32, i32)> = bt_list
                        .iter()
                        .filter(|&&pos| {
                            belt_carries
                                .get(&pos)
                                .and_then(|c| c.as_deref())
                                == Some(*item)
                        })
                        .copied()
                        .collect();
                    if !matched.is_empty() {
                        output_belt_starts.extend_from_slice(&matched);
                        producing_machines.push(mpos);
                    }
                }
            }
        }
        check_network(item, "output", &output_belt_starts, &producing_machines);
    }

    issues
}

// ---------------------------------------------------------------------------
// 5. check_belt_junctions
// ---------------------------------------------------------------------------

pub fn check_belt_junctions(layout: &LayoutResult) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    let mut belt_dir: FxHashMap<(i32, i32), EntityDirection> = FxHashMap::default();
    let mut belt_carry: FxHashMap<(i32, i32), Option<String>> = FxHashMap::default();
    for e in &layout.entities {
        if is_belt_entity(&e.name) {
            belt_dir.insert((e.x, e.y), e.direction);
            belt_carry.insert((e.x, e.y), e.carries.clone());
            if is_splitter(&e.name) {
                let second = splitter_second_tile(e);
                belt_dir.insert(second, e.direction);
                belt_carry.insert(second, e.carries.clone());
            }
        }
    }

    for (&(x, y), &direction) in &belt_dir {
        let (dx, dy) = dir_to_vec(direction);

        for (nx, ny) in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)] {
            if !belt_dir.contains_key(&(nx, ny)) {
                continue;
            }
            // Only check same-item feeders
            if belt_carry.get(&(nx, ny)) != belt_carry.get(&(x, y)) {
                continue;
            }
            let nd = belt_dir[&(nx, ny)];
            let (ndx, ndy) = dir_to_vec(nd);
            // Does this neighbour point at (x, y)?
            if (nx + ndx, ny + ndy) != (x, y) {
                continue;
            }

            let is_perpendicular = ndx * dx + ndy * dy == 0;
            let is_from_behind = ndx == dx && ndy == dy;
            if is_from_behind {
                continue;
            }
            if !is_perpendicular {
                let is_head_on = ndx == -dx && ndy == -dy;
                issues.push(ValidationIssue::with_pos(
                    if is_head_on {
                        Severity::Error
                    } else {
                        Severity::Warning
                    },
                    "belt-junction",
                    if is_head_on {
                        format!("Belt at ({},{}) feeds HEAD-ON into ({},{})", nx, ny, x, y)
                    } else {
                        format!(
                            "Belt at ({},{}) feeds into ({},{}) from an invalid angle (not perpendicular)",
                            nx, ny, x, y
                        )
                    },
                    x,
                    y,
                ));
            }
        }
    }

    issues
}

// ---------------------------------------------------------------------------
// 6. check_belt_flow_reachability
// ---------------------------------------------------------------------------

pub fn check_belt_flow_reachability(
    layout: &LayoutResult,
    solver: Option<&SolverResult>,
    style: LayoutStyle,
) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    if solver.is_none() {
        return issues;
    }

    let fluid_only = fluid_only_recipes(solver);
    let belt_dir_map = belt_dir_map_from(&layout.entities);
    if belt_dir_map.is_empty() {
        return issues;
    }

    let ug_pairs = build_ug_pairs(layout);
    let splitter_siblings = build_splitter_siblings(layout);
    let machine_tiles_set = build_machine_tile_set(layout);
    let machine_by_tile = build_machine_by_tile(layout);

    let mut input_belt_tiles: FxHashSet<(i32, i32)> = FxHashSet::default();
    let mut output_belt_tiles: FxHashSet<(i32, i32)> = FxHashSet::default();
    let mut machine_input_belts: FxHashMap<(i32, i32), Vec<(i32, i32)>> = FxHashMap::default();
    let mut machine_output_belts: FxHashMap<(i32, i32), Vec<(i32, i32)>> = FxHashMap::default();

    for ins in &layout.entities {
        if !is_inserter(&ins.name) {
            continue;
        }
        let (dx, dy) = dir_to_vec(ins.direction);
        let reach = inserter_reach(&ins.name);
        let drop_pos = (ins.x + dx * reach, ins.y + dy * reach);
        let pickup_pos = (ins.x - dx * reach, ins.y - dy * reach);

        if machine_tiles_set.contains(&drop_pos) && belt_dir_map.contains_key(&pickup_pos) {
            if let Some(&mpos) = machine_by_tile.get(&drop_pos) {
                input_belt_tiles.insert(pickup_pos);
                machine_input_belts.entry(mpos).or_default().push(pickup_pos);
            }
        } else if machine_tiles_set.contains(&pickup_pos) && belt_dir_map.contains_key(&drop_pos) {
            if let Some(&mpos) = machine_by_tile.get(&pickup_pos) {
                output_belt_tiles.insert(drop_pos);
                machine_output_belts.entry(mpos).or_default().push(drop_pos);
            }
        }
    }

    // Boundary from belt positions
    let all_xs: Vec<i32> = belt_dir_map.keys().map(|&(x, _)| x).collect();
    let all_ys: Vec<i32> = belt_dir_map.keys().map(|&(_, y)| y).collect();
    let min_bx = *all_xs.iter().min().unwrap();
    let max_bx = *all_xs.iter().max().unwrap();
    let min_by = *all_ys.iter().min().unwrap();
    let max_by = *all_ys.iter().max().unwrap();

    let on_boundary = |(x, y): (i32, i32)| -> bool {
        x == min_bx || x == max_bx || y == min_by || y == max_by
    };

    let severity = if style == LayoutStyle::Spaghetti {
        Severity::Error
    } else {
        Severity::Warning
    };

    // Input check
    let mut checked: FxHashSet<(i32, i32)> = FxHashSet::default();
    for e in &layout.entities {
        if !is_machine_entity(&e.name) {
            continue;
        }
        let mpos = (e.x, e.y);
        if !checked.insert(mpos) {
            continue;
        }
        if e.recipe.as_deref().is_some_and(|r| fluid_only.contains(r)) {
            continue;
        }
        let belts = match machine_input_belts.get(&mpos) {
            Some(b) => b,
            None => continue,
        };
        let belt_set: FxHashSet<(i32, i32)> = belts.iter().copied().collect();
        let upstream = bfs_belt_upstream(
            &belt_set,
            &belt_dir_map,
            Some(&ug_pairs),
            Some(&splitter_siblings),
        );
        let upstream_beyond: FxHashSet<(i32, i32)> =
            upstream.difference(&belt_set).copied().collect();
        let reaches_source = upstream_beyond.iter().any(|&t| on_boundary(t))
            || upstream_beyond.intersection(&output_belt_tiles).next().is_some();
        if !reaches_source {
            issues.push(ValidationIssue::with_pos(
                severity,
                "belt-flow-reachability",
                format!(
                    "{} at ({},{}): items can't reach input \
                     (no upstream path from boundary or another machine's output)",
                    e.name, e.x, e.y
                ),
                e.x,
                e.y,
            ));
        }
    }

    // Output check
    checked.clear();
    for e in &layout.entities {
        if !is_machine_entity(&e.name) {
            continue;
        }
        let mpos = (e.x, e.y);
        if !checked.insert(mpos) {
            continue;
        }
        if e.recipe.as_deref().is_some_and(|r| fluid_only.contains(r)) {
            continue;
        }
        let belts = match machine_output_belts.get(&mpos) {
            Some(b) => b,
            None => continue,
        };
        let belt_set: FxHashSet<(i32, i32)> = belts.iter().copied().collect();
        let downstream = bfs_belt_downstream(
            &belt_set,
            &belt_dir_map,
            Some(&ug_pairs),
            Some(&splitter_siblings),
        );
        let downstream_beyond: FxHashSet<(i32, i32)> =
            downstream.difference(&belt_set).copied().collect();
        let reaches_sink = downstream_beyond.iter().any(|&t| on_boundary(t))
            || downstream_beyond
                .intersection(&input_belt_tiles)
                .next()
                .is_some();
        if !reaches_sink {
            issues.push(ValidationIssue::with_pos(
                severity,
                "belt-flow-reachability",
                format!(
                    "{} at ({},{}): items can't leave output \
                     (no downstream path to boundary or another machine's input)",
                    e.name, e.x, e.y
                ),
                e.x,
                e.y,
            ));
        }
    }

    issues
}

// ---------------------------------------------------------------------------
// 7. check_belt_throughput
// ---------------------------------------------------------------------------

pub fn check_belt_throughput(layout: &LayoutResult) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    let mut tile_counts: FxHashMap<(i32, i32), usize> = FxHashMap::default();
    let mut tile_names: FxHashMap<(i32, i32), &str> = FxHashMap::default();

    for e in &layout.entities {
        if is_belt_entity(&e.name) {
            let pos = (e.x, e.y);
            *tile_counts.entry(pos).or_insert(0) += 1;
            tile_names.insert(pos, &e.name);
        }
    }

    for (&pos, &count) in &tile_counts {
        if count > 1 {
            let belt_name = tile_names.get(&pos).copied().unwrap_or("transport-belt");
            let max_throughput = match belt_name {
                "transport-belt" | "underground-belt" => 15.0_f64,
                "fast-transport-belt" | "fast-underground-belt" => 30.0,
                "express-transport-belt" | "express-underground-belt" => 45.0,
                _ => 15.0,
            };
            issues.push(ValidationIssue::with_pos(
                Severity::Warning,
                "belt-throughput",
                format!(
                    "Belt at ({},{}): {} overlapping routes on {} (max {}/s)",
                    pos.0, pos.1, count, belt_name, max_throughput
                ),
                pos.0,
                pos.1,
            ));
        }
    }

    issues
}

// ---------------------------------------------------------------------------
// 8. check_output_belt_coverage
// ---------------------------------------------------------------------------

pub fn check_output_belt_coverage(
    layout: &LayoutResult,
    solver: Option<&SolverResult>,
) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    let mut fluid_output_recipes: FxHashSet<String> = FxHashSet::default();
    if let Some(sr) = solver {
        for spec in &sr.machines {
            if !spec.outputs.iter().any(|f| !f.is_fluid) {
                fluid_output_recipes.insert(spec.recipe.clone());
            }
        }
    }

    let machine_tiles_set = build_machine_tile_set(layout);
    let belt_tiles = build_belt_tile_set(&layout.entities);

    let mut checked: FxHashSet<(i32, i32)> = FxHashSet::default();
    for e in &layout.entities {
        if !is_machine_entity(&e.name) {
            continue;
        }
        if !checked.insert((e.x, e.y)) {
            continue;
        }
        if e.recipe
            .as_deref()
            .is_some_and(|r| fluid_output_recipes.contains(r))
        {
            continue;
        }

        let size = machine_size(&e.name) as i32;
        let my_tiles: FxHashSet<(i32, i32)> = (0..size)
            .flat_map(|dx| (0..size).map(move |dy| (e.x + dx, e.y + dy)))
            .collect();

        let mut has_output_belt = false;
        'outer: for ins in &layout.entities {
            if !is_inserter(&ins.name) {
                continue;
            }
            let (dx, dy) = dir_to_vec(ins.direction);
            let reach = inserter_reach(&ins.name);
            let (odx, ody) = (-dx, -dy);
            let pickup_pos = (ins.x + odx * reach, ins.y + ody * reach);
            let drop_pos = (ins.x + dx * reach, ins.y + dy * reach);

            if my_tiles.contains(&pickup_pos)
                && !machine_tiles_set.contains(&drop_pos)
                && belt_tiles.contains(&drop_pos)
            {
                has_output_belt = true;
                break 'outer;
            }
        }

        if !has_output_belt {
            issues.push(ValidationIssue::with_pos(
                Severity::Error,
                "output-belt",
                format!(
                    "{} at ({},{}): no output inserter has a belt at its drop position",
                    e.name, e.x, e.y
                ),
                e.x,
                e.y,
            ));
        }
    }

    issues
}

// ---------------------------------------------------------------------------
// 9. check_underground_belt_pairs
// ---------------------------------------------------------------------------

pub fn check_underground_belt_pairs(layout: &LayoutResult) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    let mut ug_inputs: Vec<&PlacedEntity> = Vec::new();
    let mut ug_outputs: Vec<&PlacedEntity> = Vec::new();
    let mut all_ug: Vec<&PlacedEntity> = Vec::new();
    for e in &layout.entities {
        if is_ug_belt(&e.name) {
            all_ug.push(e);
            match e.io_type.as_deref() {
                Some("input") => ug_inputs.push(e),
                Some("output") => ug_outputs.push(e),
                _ => {}
            }
        }
    }

    let mut used_outputs: FxHashSet<(i32, i32)> = FxHashSet::default();

    for inp in &ug_inputs {
        let (dx, dy) = dir_to_vec(inp.direction);
        let surface_tier = ug_to_surface_tier(&inp.name);
        let max_reach = ug_max_reach(surface_tier) as i32;

        let mut best_out: Option<&PlacedEntity> = None;
        let mut best_dist = i32::MAX;

        for out in &ug_outputs {
            if used_outputs.contains(&(out.x, out.y)) {
                continue;
            }
            if out.direction != inp.direction || out.name != inp.name {
                continue;
            }
            let rx = out.x - inp.x;
            let ry = out.y - inp.y;
            let dist = if dx != 0 {
                if ry != 0 || (rx > 0) != (dx > 0) {
                    continue;
                }
                rx.abs()
            } else {
                if rx != 0 || (ry > 0) != (dy > 0) {
                    continue;
                }
                ry.abs()
            };
            if dist > 1 && dist < best_dist {
                best_dist = dist;
                best_out = Some(out);
            }
        }

        if best_out.is_none() {
            issues.push(ValidationIssue::with_pos(
                Severity::Error,
                "underground-belt",
                format!(
                    "Unpaired underground belt input at ({},{}) facing {:?}: no matching output found",
                    inp.x, inp.y, inp.direction
                ),
                inp.x,
                inp.y,
            ));
            continue;
        }
        let out = best_out.unwrap();
        used_outputs.insert((out.x, out.y));

        if best_dist > max_reach + 1 {
            issues.push(ValidationIssue::with_pos(
                Severity::Error,
                "underground-belt",
                format!(
                    "Underground belt pair ({},{})->({},{}) distance {} exceeds max reach {} for {}",
                    inp.x, inp.y, out.x, out.y, best_dist, max_reach, surface_tier
                ),
                inp.x,
                inp.y,
            ));
        }

        // Check for intercepting UG belts
        for ug in &all_ug {
            if (ug.x, ug.y) == (inp.x, inp.y) || (ug.x, ug.y) == (out.x, out.y) {
                continue;
            }
            if ug.name != inp.name || ug.direction != inp.direction {
                continue;
            }
            let rx = ug.x - inp.x;
            let ry = ug.y - inp.y;
            let udist = if dx != 0 {
                if ry != 0 || (rx > 0) != (dx > 0) {
                    continue;
                }
                rx.abs()
            } else {
                if rx != 0 || (ry > 0) != (dy > 0) {
                    continue;
                }
                ry.abs()
            };
            if udist > 0 && udist < best_dist {
                issues.push(ValidationIssue::with_pos(
                    Severity::Warning,
                    "underground-belt",
                    format!(
                        "Underground belt at ({},{}) intercepts pair ({},{})->({},{})",
                        ug.x, ug.y, inp.x, inp.y, out.x, out.y
                    ),
                    ug.x,
                    ug.y,
                ));
            }
        }
    }

    // Unpaired outputs
    for out in &ug_outputs {
        if !used_outputs.contains(&(out.x, out.y)) {
            issues.push(ValidationIssue::with_pos(
                Severity::Error,
                "underground-belt",
                format!(
                    "Unpaired underground belt output at ({},{}) facing {:?}: no matching input found",
                    out.x, out.y, out.direction
                ),
                out.x,
                out.y,
            ));
        }
    }

    issues
}

// ---------------------------------------------------------------------------
// 10. check_underground_belt_sideloading
// ---------------------------------------------------------------------------

pub fn check_underground_belt_sideloading(layout: &LayoutResult) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    let mut belt_dir: FxHashMap<(i32, i32), EntityDirection> = FxHashMap::default();
    for e in &layout.entities {
        if is_belt_entity(&e.name) {
            belt_dir.insert((e.x, e.y), e.direction);
            if is_splitter(&e.name) {
                belt_dir.insert(splitter_second_tile(e), e.direction);
            }
        }
    }

    for e in &layout.entities {
        if !is_ug_belt(&e.name) || e.io_type.as_deref() != Some("output") {
            continue;
        }
        let (dx, dy) = dir_to_vec(e.direction);
        let exit_tile = (e.x + dx, e.y + dy);
        if let Some(&target_dir) = belt_dir.get(&exit_tile) {
            let (tdx, tdy) = dir_to_vec(target_dir);
            let dot = dx * tdx + dy * tdy;
            if dot < 0 {
                issues.push(ValidationIssue::with_pos(
                    Severity::Error,
                    "underground-belt",
                    format!(
                        "Underground belt exit at ({},{}) facing {:?} collides head-on with belt at ({},{}) facing {:?}",
                        e.x, e.y, e.direction, exit_tile.0, exit_tile.1, target_dir
                    ),
                    e.x,
                    e.y,
                ));
            }
        }
    }

    issues
}

// ---------------------------------------------------------------------------
// 11. check_underground_belt_entry_sideload
// ---------------------------------------------------------------------------

pub fn check_underground_belt_entry_sideload(layout: &LayoutResult) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    let mut belt_dir: FxHashMap<(i32, i32), EntityDirection> = FxHashMap::default();
    let mut ug_inputs: Vec<&PlacedEntity> = Vec::new();

    for e in &layout.entities {
        if is_surface_belt(&e.name) || is_splitter(&e.name) {
            belt_dir.insert((e.x, e.y), e.direction);
            if is_splitter(&e.name) {
                belt_dir.insert(splitter_second_tile(e), e.direction);
            }
        } else if is_ug_belt(&e.name) {
            match e.io_type.as_deref() {
                Some("output") => {
                    belt_dir.insert((e.x, e.y), e.direction);
                }
                Some("input") => ug_inputs.push(e),
                _ => {}
            }
        }
    }

    for ug in &ug_inputs {
        let (ug_dx, ug_dy) = dir_to_vec(ug.direction);
        for (ndx, ndy) in [(0, -1), (0, 1), (-1, 0), (1, 0)] {
            let (nx, ny) = (ug.x + ndx, ug.y + ndy);
            if let Some(&n_dir) = belt_dir.get(&(nx, ny)) {
                let (n_dx, n_dy) = dir_to_vec(n_dir);
                if (nx + n_dx, ny + n_dy) != (ug.x, ug.y) {
                    continue;
                }
                let dot = n_dx * ug_dx + n_dy * ug_dy;
                if dot == 0 {
                    issues.push(ValidationIssue::with_pos(
                        Severity::Warning,
                        "underground-belt",
                        format!(
                            "Belt at ({},{}) facing {:?} sideloads into underground input at ({},{}) facing {:?} \
                             — only one lane loaded, must feed UG inputs straight",
                            nx, ny, n_dir, ug.x, ug.y, ug.direction
                        ),
                        ug.x,
                        ug.y,
                    ));
                }
            }
        }
    }

    issues
}

// ---------------------------------------------------------------------------
// 12. check_belt_dead_ends
// ---------------------------------------------------------------------------

pub fn check_belt_dead_ends(layout: &LayoutResult) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    // All tiles that can receive belt output
    let mut receiver_tiles: FxHashSet<(i32, i32)> = FxHashSet::default();
    for e in &layout.entities {
        if is_belt_entity(&e.name) {
            receiver_tiles.insert((e.x, e.y));
            if is_splitter(&e.name) {
                receiver_tiles.insert(splitter_second_tile(e));
            }
        } else if is_inserter(&e.name) {
            if let Some(d) = Some(dir_to_vec(e.direction)) {
                let reach: i32 = if e.name == "long-handed-inserter" { 2 } else { 1 };
                let pickup = (e.x - d.0 * reach, e.y - d.1 * reach);
                receiver_tiles.insert(pickup);
            }
        }
    }

    // Surface belts indexed by position
    let mut belt_at: FxHashMap<(i32, i32), &PlacedEntity> = FxHashMap::default();
    for e in &layout.entities {
        if is_surface_belt(&e.name) {
            belt_at.insert((e.x, e.y), e);
        }
    }

    // Inserter pickup tiles (where inserters take FROM belt)
    let mut pickup_tiles: FxHashSet<(i32, i32)> = FxHashSet::default();
    for e in &layout.entities {
        if is_inserter(&e.name) {
            let (dx, dy) = dir_to_vec(e.direction);
            let reach: i32 = if e.name == "long-handed-inserter" { 2 } else { 1 };
            pickup_tiles.insert((e.x - dx * reach, e.y - dy * reach));
        }
    }

    let w = layout.width;
    let h = layout.height;

    // Also check UG belt outputs — they emit items on the surface at their
    // output tile, so that tile must have a receiver.
    let mut ug_outputs: Vec<&PlacedEntity> = Vec::new();
    for e in &layout.entities {
        if is_ug_belt(&e.name) && e.io_type.as_deref() == Some("output") {
            ug_outputs.push(e);
        }
    }

    for e in &layout.entities {
        let is_surface = is_surface_belt(&e.name);
        let is_ug_out = is_ug_belt(&e.name) && e.io_type.as_deref() == Some("output");
        if !is_surface && !is_ug_out {
            continue;
        }
        // Skip balancer internals — they have intentional dead-end patterns
        if let Some(ref seg) = e.segment_id {
            if seg.starts_with("balancer:") {
                continue;
            }
        }
        let (dx, dy) = dir_to_vec(e.direction);
        let out_x = e.x + dx;
        let out_y = e.y + dy;
        // Flowing off layout edge is OK (external I/O)
        if out_x < 0 || out_x >= w || out_y < 0 || out_y >= h {
            continue;
        }
        if receiver_tiles.contains(&(out_x, out_y)) {
            continue;
        }
        // Walk upstream: if chain has an inserter pickup, slack is OK
        if is_surface && chain_has_pickup((e.x, e.y), e.direction, &belt_at, &pickup_tiles) {
            continue;
        }
        let kind = if is_ug_out { "UG output" } else { "Belt" };
        issues.push(ValidationIssue::with_pos(
            Severity::Error,
            "belt-dead-end",
            format!(
                "{} at ({},{}) facing {:?} has no receiver at output tile ({},{}) — items accumulate with nowhere to go",
                kind, e.x, e.y, e.direction, out_x, out_y
            ),
            e.x,
            e.y,
        ));
    }

    issues
}

fn chain_has_pickup(
    tail: (i32, i32),
    direction: EntityDirection,
    belt_at: &FxHashMap<(i32, i32), &PlacedEntity>,
    pickup_tiles: &FxHashSet<(i32, i32)>,
) -> bool {
    let (dx, dy) = dir_to_vec(direction);
    let mut cur = tail;
    let mut visited: FxHashSet<(i32, i32)> = FxHashSet::default();
    for _ in 0..200 {
        if !visited.insert(cur) {
            break;
        }
        if pickup_tiles.contains(&cur) {
            return true;
        }
        let upstream = (cur.0 - dx, cur.1 - dy);
        match belt_at.get(&upstream) {
            Some(up) if up.direction == direction => cur = upstream,
            _ => break,
        }
    }
    false
}

// ---------------------------------------------------------------------------
// 13. check_belt_loops
// ---------------------------------------------------------------------------

pub fn check_belt_loops(layout: &LayoutResult) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    // Skip balancer entities — balancer templates use intentional feedback loops
    // for even item distribution. These are pre-validated by the SAT generator.
    let belt_dir_map = belt_dir_map_filtered(&layout.entities, true);
    let mut confirmed: FxHashSet<(i32, i32)> = FxHashSet::default();
    let mut reported_loops: FxHashSet<Vec<(i32, i32)>> = FxHashSet::default();

    for &start in belt_dir_map.keys() {
        if confirmed.contains(&start) {
            continue;
        }
        let mut visited_order: Vec<(i32, i32)> = Vec::new();
        let mut visited_set: FxHashSet<(i32, i32)> = FxHashSet::default();
        let mut cur = start;

        while belt_dir_map.contains_key(&cur) && !visited_set.contains(&cur) {
            visited_set.insert(cur);
            visited_order.push(cur);
            let (dx, dy) = dir_to_vec(belt_dir_map[&cur]);
            cur = (cur.0 + dx, cur.1 + dy);
        }

        if visited_set.contains(&cur) {
            // Extract cycle
            let cycle_start_idx = visited_order.iter().position(|&t| t == cur).unwrap_or(0);
            let mut loop_tiles: Vec<(i32, i32)> = visited_order[cycle_start_idx..].to_vec();
            loop_tiles.sort();
            if !reported_loops.contains(&loop_tiles) {
                reported_loops.insert(loop_tiles.clone());
                let rep = *loop_tiles.iter().min().unwrap();
                issues.push(ValidationIssue::with_pos(
                    Severity::Error,
                    "belt-loop",
                    format!(
                        "Belt loop detected: {} tiles form a cycle near ({},{})",
                        loop_tiles.len(),
                        rep.0,
                        rep.1
                    ),
                    rep.0,
                    rep.1,
                ));
            }
        }
        confirmed.extend(visited_set);
    }

    issues
}

// ---------------------------------------------------------------------------
// 14. check_belt_item_isolation
// ---------------------------------------------------------------------------

pub fn check_belt_item_isolation(layout: &LayoutResult) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    let mut belt_dir: FxHashMap<(i32, i32), EntityDirection> = FxHashMap::default();
    let mut belt_carry: FxHashMap<(i32, i32), Option<String>> = FxHashMap::default();
    let mut ug_inputs: FxHashSet<(i32, i32)> = FxHashSet::default();

    for e in &layout.entities {
        if is_belt_entity(&e.name) {
            belt_dir.insert((e.x, e.y), e.direction);
            belt_carry.insert((e.x, e.y), e.carries.clone());
            if is_ug_belt(&e.name) && e.io_type.as_deref() == Some("input") {
                ug_inputs.insert((e.x, e.y));
            }
            if is_splitter(&e.name) {
                let second = splitter_second_tile(e);
                belt_dir.insert(second, e.direction);
                belt_carry.insert(second, e.carries.clone());
            }
        }
    }

    let mut seen: FxHashSet<((i32, i32), (i32, i32))> = FxHashSet::default();
    for (&(ax, ay), &ad) in &belt_dir {
        if ug_inputs.contains(&(ax, ay)) {
            continue;
        }
        let (dx, dy) = dir_to_vec(ad);
        let next = (ax + dx, ay + dy);
        if !belt_dir.contains_key(&next) {
            continue;
        }
        let ac = belt_carry.get(&(ax, ay)).and_then(|c| c.as_deref());
        let bc = belt_carry.get(&next).and_then(|c| c.as_deref());
        if let (Some(a_item), Some(b_item)) = (ac, bc) {
            if a_item != b_item {
                let pair = ((ax, ay), next);
                if seen.insert(pair) {
                    issues.push(ValidationIssue::with_pos(
                        Severity::Error,
                        "belt-item-isolation",
                        format!(
                            "Belt at ({},{}) carries {} but feeds into ({},{}) which carries {}",
                            ax, ay, a_item, next.0, next.1, b_item
                        ),
                        ax,
                        ay,
                    ));
                }
            }
        }
    }

    issues
}

// ---------------------------------------------------------------------------
// 15. check_belt_inserter_conflict
// ---------------------------------------------------------------------------

pub fn check_belt_inserter_conflict(layout: &LayoutResult) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    let belt_tiles = build_belt_tile_set(&layout.entities);
    let mut drop_map: FxHashMap<(i32, i32), Vec<String>> = FxHashMap::default();

    for e in &layout.entities {
        if !is_inserter(&e.name) {
            continue;
        }
        let carries = match &e.carries {
            Some(c) => c.clone(),
            None => continue,
        };
        let (dx, dy) = dir_to_vec(e.direction);
        let reach = inserter_reach(&e.name);
        let drop = (e.x + dx * reach, e.y + dy * reach);
        if belt_tiles.contains(&drop) {
            drop_map.entry(drop).or_default().push(carries);
        }
    }

    for (&(bx, by), items) in &drop_map {
        let unique: FxHashSet<&str> = items.iter().map(|s| s.as_str()).collect();
        if unique.len() >= 2 {
            let mut sorted: Vec<&str> = unique.into_iter().collect();
            sorted.sort();
            issues.push(ValidationIssue::with_pos(
                Severity::Error,
                "belt-item-isolation",
                format!(
                    "Belt at ({},{}): inserters drop conflicting items {:?} and {:?}",
                    bx, by, sorted[0], sorted[1]
                ),
                bx,
                by,
            ));
        }
    }

    issues
}

// ---------------------------------------------------------------------------
// 16. compute_lane_rates + check_lane_throughput
// ---------------------------------------------------------------------------

pub fn compute_lane_rates(
    layout: &LayoutResult,
    solver: Option<&SolverResult>,
) -> FxHashMap<(i32, i32), [f64; 2]> {
    compute_lane_rates_impl(layout, solver)
}

pub fn check_lane_throughput(
    layout: &LayoutResult,
    solver: Option<&SolverResult>,
) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    let lane_rates = compute_lane_rates_impl(layout, solver);
    if lane_rates.is_empty() {
        return issues;
    }

    let mut belt_name_map: FxHashMap<(i32, i32), &str> = FxHashMap::default();
    for e in &layout.entities {
        if is_surface_belt(&e.name) {
            belt_name_map.insert((e.x, e.y), &e.name);
        } else if is_ug_belt(&e.name) && e.io_type.as_deref() == Some("output") {
            belt_name_map.insert((e.x, e.y), ug_to_surface_tier(&e.name));
        }
    }

    for (&pos, &[left, right]) in &lane_rates {
        let belt_name = belt_name_map.get(&pos).copied().unwrap_or("transport-belt");
        let cap = lane_capacity(belt_name);
        for (lane_name, rate) in [("left", left), ("right", right)] {
            if rate > cap + 0.01 {
                issues.push(ValidationIssue::with_pos(
                    Severity::Error,
                    "lane-throughput",
                    format!(
                        "Belt at ({},{}): {} lane {:.1}/s exceeds {} per-lane capacity {}/s",
                        pos.0, pos.1, lane_name, rate, belt_name, cap
                    ),
                    pos.0,
                    pos.1,
                ));
            }
        }
    }

    issues
}

fn compute_lane_rates_impl(
    layout: &LayoutResult,
    solver: Option<&SolverResult>,
) -> FxHashMap<(i32, i32), [f64; 2]> {
    let sr = match solver {
        Some(s) => s,
        None => return FxHashMap::default(),
    };

    let mut belt_dir_map: FxHashMap<(i32, i32), EntityDirection> = FxHashMap::default();
    let mut ug_output_tiles: FxHashSet<(i32, i32)> = FxHashSet::default();
    let mut ug_output_to_input: FxHashMap<(i32, i32), (i32, i32)> = FxHashMap::default();
    let mut ug_input_dir: FxHashMap<(i32, i32), EntityDirection> = FxHashMap::default();

    for e in &layout.entities {
        if is_surface_belt(&e.name) {
            belt_dir_map.insert((e.x, e.y), e.direction);
        } else if is_ug_belt(&e.name) {
            match e.io_type.as_deref() {
                Some("output") => {
                    belt_dir_map.insert((e.x, e.y), e.direction);
                    ug_output_tiles.insert((e.x, e.y));
                }
                Some("input") => {
                    ug_input_dir.insert((e.x, e.y), e.direction);
                }
                _ => {}
            }
        } else if is_splitter(&e.name) {
            belt_dir_map.insert((e.x, e.y), e.direction);
            let second = splitter_second_tile(e);
            belt_dir_map.insert(second, e.direction);
        }
    }
    if belt_dir_map.is_empty() {
        return FxHashMap::default();
    }

    let ug_pairs = build_ug_pairs(layout);
    for (&(ix, iy), &(ox, oy)) in &ug_pairs {
        if ug_input_dir.contains_key(&(ix, iy)) {
            ug_output_to_input.insert((ox, oy), (ix, iy));
        }
    }

    let machine_tiles_set = build_machine_tile_set(layout);
    let machine_by_tile = build_machine_by_tile(layout);

    let mut belt_carries: FxHashMap<(i32, i32), Option<String>> = FxHashMap::default();
    for e in &layout.entities {
        if is_belt_entity(&e.name) {
            belt_carries.insert((e.x, e.y), e.carries.clone());
            if is_splitter(&e.name) {
                belt_carries.insert(splitter_second_tile(e), e.carries.clone());
            }
        }
    }

    let recipe_to_spec: FxHashMap<&str, &crate::models::MachineSpec> = sr
        .machines
        .iter()
        .map(|s| (s.recipe.as_str(), s))
        .collect();
    let mut machine_entity: FxHashMap<(i32, i32), &PlacedEntity> = FxHashMap::default();
    for e in &layout.entities {
        if is_machine_entity(&e.name) {
            machine_entity.insert((e.x, e.y), e);
        }
    }

    let mut lane_injections: FxHashMap<(i32, i32), [f64; 2]> = FxHashMap::default();
    for ins in &layout.entities {
        if !is_inserter(&ins.name) {
            continue;
        }
        let (dx, dy) = dir_to_vec(ins.direction);
        let reach = inserter_reach(&ins.name);
        let drop_pos = (ins.x + dx * reach, ins.y + dy * reach);
        let pickup_pos = (ins.x - dx * reach, ins.y - dy * reach);
        if !machine_tiles_set.contains(&pickup_pos) || !belt_dir_map.contains_key(&drop_pos) {
            continue;
        }
        let mpos = match machine_by_tile.get(&pickup_pos) {
            Some(&p) => p,
            None => continue,
        };
        let me = match machine_entity.get(&mpos) {
            Some(e) => e,
            None => continue,
        };
        let spec = match me.recipe.as_deref().and_then(|r| recipe_to_spec.get(r)) {
            Some(s) => *s,
            None => continue,
        };
        let carried_item = match belt_carries.get(&drop_pos).and_then(|c| c.as_deref()) {
            Some(i) => i,
            None => continue,
        };
        let rate = spec
            .outputs
            .iter()
            .find(|o| o.item == carried_item)
            .map(|o| o.rate)
            .unwrap_or(0.0);
        if rate <= 0.0 {
            continue;
        }
        let belt_d = belt_dir_map[&drop_pos];
        let lane = inserter_target_lane(ins.x, ins.y, drop_pos.0, drop_pos.1, belt_d);
        let entry = lane_injections.entry(drop_pos).or_insert([0.0, 0.0]);
        if lane == LANE_LEFT {
            entry[0] += rate;
        } else {
            entry[1] += rate;
        }
    }

    // Build feeder map
    let mut feeders: FxHashMap<(i32, i32), Vec<((i32, i32), u8)>> = FxHashMap::default();
    for (&(bx, by), &belt_d) in &belt_dir_map {
        if ug_output_tiles.contains(&(bx, by)) {
            continue;
        }
        let (left_dx, left_dy) = { let (bdx, bdy) = dir_to_vec(belt_d); (-bdy, bdx) };
        let mut tile_feeders: Vec<((i32, i32), u8)> = Vec::new();
        for (ddx, ddy) in [(1, 0i32), (-1, 0), (0, 1), (0, -1)] {
            let (nx, ny) = (bx + ddx, by + ddy);
            if let Some(&nd) = belt_dir_map.get(&(nx, ny)) {
                let (ndx, ndy) = dir_to_vec(nd);
                if (nx + ndx, ny + ndy) != (bx, by) {
                    continue;
                }
                let feed_type = if nd == belt_d {
                    0u8
                } else {
                    let dot = (nx - bx) * left_dx + (ny - by) * left_dy;
                    if dot > 0 { 1u8 } else { 2u8 }
                };
                tile_feeders.push(((nx, ny), feed_type));
            }
        }
        if !tile_feeders.is_empty() {
            feeders.insert((bx, by), tile_feeders);
        }
    }

    let mut splitter_sibling: FxHashMap<(i32, i32), (i32, i32)> = FxHashMap::default();
    for e in &layout.entities {
        if is_splitter(&e.name) {
            let second = splitter_second_tile(e);
            splitter_sibling.insert((e.x, e.y), second);
            splitter_sibling.insert(second, (e.x, e.y));
        }
    }

    let mut in_degree: FxHashMap<(i32, i32), i32> =
        belt_dir_map.keys().map(|&p| (p, 0)).collect();
    for (&pos, tile_feeders) in &feeders {
        in_degree.insert(pos, tile_feeders.len() as i32);
    }
    // Unify splitter pair in-degrees: both tiles share the sum of both sides'
    // feeders. Without this, a 1→2 splitter whose "empty" side has no feeders
    // enters the queue immediately (in_degree=0) and exhausts retries waiting
    // for its sibling, propagating with stale [0,0] rates. See belt-flow bug
    // where tier2 copper-cable trunks fed from a single-input splitter all
    // delivered 0/s to downstream rows.
    let mut visited_pairs: FxHashSet<((i32, i32), (i32, i32))> = FxHashSet::default();
    for (&a, &b) in &splitter_sibling {
        let key = if a < b { (a, b) } else { (b, a) };
        if !visited_pairs.insert(key) {
            continue;
        }
        let total = in_degree.get(&a).copied().unwrap_or(0)
            + in_degree.get(&b).copied().unwrap_or(0);
        in_degree.insert(a, total);
        in_degree.insert(b, total);
    }

    // Virtual dependency: each UG-output inherits its rate from the surface
    // tile "behind" its paired UG-input. Track those dependencies as an
    // additional in_degree bump so UG-outputs don't dequeue before their
    // source is ready. `behind_to_ug_outputs` lets us decrement the UG-output's
    // counter once the "behind" tile is processed.
    let mut behind_to_ug_outputs: FxHashMap<(i32, i32), Vec<(i32, i32)>> =
        FxHashMap::default();
    for &ug_out in &ug_output_tiles {
        if let Some(&paired_input) = ug_output_to_input.get(&ug_out) {
            if let Some(&inp_d) = ug_input_dir.get(&paired_input) {
                let (idx, idy) = dir_to_vec(inp_d);
                let behind = (paired_input.0 - idx, paired_input.1 - idy);
                if belt_dir_map.contains_key(&behind) && behind != ug_out {
                    behind_to_ug_outputs
                        .entry(behind)
                        .or_default()
                        .push(ug_out);
                    *in_degree.entry(ug_out).or_insert(0) += 1;
                }
            }
        }
    }

    let mut lane_rates: FxHashMap<(i32, i32), [f64; 2]> = belt_dir_map
        .keys()
        .map(|&p| (p, lane_injections.get(&p).copied().unwrap_or([0.0, 0.0])))
        .collect();

    // Seed graph-source belts that carry external input items. External inputs
    // come from outside the layout and have no upstream producer in the belt
    // graph — without this seeding, rate propagation starts at 0 and every
    // downstream consumer of an external input is incorrectly flagged as
    // starved. We distribute each item's total external rate across its source
    // tiles so the validator sees the actual per-belt flow the layout engine
    // intended.
    let external_rates: FxHashMap<&str, f64> = sr
        .external_inputs
        .iter()
        .filter(|f| !f.is_fluid)
        .map(|f| (f.item.as_str(), f.rate))
        .collect();
    if !external_rates.is_empty() {
        // First pass: group source tiles by the item they carry. A "source" is a
        // belt tile that has no upstream feeder in the surface belt graph. We
        // include UG outputs here too: although they inherit rate via the topo
        // sort's UG special case, that inheritance relies on the "behind the UG
        // input" surface tile being correctly seeded — for external inputs it's
        // simpler and safer to seed every graph source independently.
        let mut sources_by_item: FxHashMap<&str, Vec<(i32, i32)>> = FxHashMap::default();
        for &pos in belt_dir_map.keys() {
            if feeders.contains_key(&pos) {
                continue; // has upstream feeders, not a source
            }
            if let Some(Some(item)) = belt_carries.get(&pos) {
                if external_rates.contains_key(item.as_str()) {
                    sources_by_item
                        .entry(external_rates.get_key_value(item.as_str()).unwrap().0)
                        .or_default()
                        .push(pos);
                }
            }
        }
        // Second pass: seed each source tile with its share of the total rate,
        // split evenly across the belt's two lanes.
        for (item, sources) in &sources_by_item {
            let total = external_rates[item];
            let per_tile = total / sources.len() as f64;
            for &pos in sources {
                let entry = lane_rates.entry(pos).or_insert([0.0, 0.0]);
                entry[0] += per_tile / 2.0;
                entry[1] += per_tile / 2.0;
            }
        }
    }

    let mut processed: FxHashSet<(i32, i32)> = FxHashSet::default();
    let mut splitter_input_ready: FxHashSet<(i32, i32)> = FxHashSet::default();
    let mut queue: VecDeque<(i32, i32)> = in_degree
        .iter()
        .filter(|(_, &d)| d == 0)
        .map(|(&p, _)| p)
        .collect();

    // Splitter sibling waits are still retry-driven (the pair is serialized,
    // so whichever half dequeues second triggers the averaging path).
    let mut splitter_retries: FxHashMap<(i32, i32), u32> = FxHashMap::default();
    const MAX_RETRIES: u32 = 3;

    // Helper: after marking `tile` processed, decrement in_degree for any
    // UG-output tiles that depended on `tile` as their "behind" source, and
    // enqueue them if ready.
    let notify_ug_deps = |tile: (i32, i32),
                          in_degree: &mut FxHashMap<(i32, i32), i32>,
                          queue: &mut VecDeque<(i32, i32)>| {
        if let Some(deps) = behind_to_ug_outputs.get(&tile) {
            for &ug_out in deps {
                let d = in_degree.entry(ug_out).or_insert(0);
                *d -= 1;
                if *d <= 0 {
                    queue.push_back(ug_out);
                }
            }
        }
    };

    while let Some(pos) = queue.pop_front() {
        if processed.contains(&pos) {
            continue;
        }

        // Underground output: inherit from behind paired input. The topo-sort
        // dependency is encoded via `behind_to_ug_outputs` — this tile won't
        // be dequeued until `behind` has been processed, so we can read its
        // rates directly.
        if ug_output_tiles.contains(&pos) {
            if let Some(&paired_input) = ug_output_to_input.get(&pos) {
                if let Some(&inp_d) = ug_input_dir.get(&paired_input) {
                    let (idx, idy) = dir_to_vec(inp_d);
                    let behind = (paired_input.0 - idx, paired_input.1 - idy);
                    if let Some(&behind_rates) = lane_rates.get(&behind) {
                        let rates = lane_rates.entry(pos).or_insert([0.0, 0.0]);
                        rates[0] += behind_rates[0];
                        rates[1] += behind_rates[1];
                    }
                }
            }
        }

        // Splitter: wait for sibling
        if let Some(&sib) = splitter_sibling.get(&pos) {
            if !processed.contains(&sib) {
                splitter_input_ready.insert(pos);
                if !splitter_input_ready.contains(&sib) {
                    let retry = splitter_retries.entry(pos).or_insert(0);
                    if *retry < MAX_RETRIES {
                        *retry += 1;
                        queue.push_back(pos);
                        continue;
                    }
                    // Gave up waiting for sibling — mark processed with current
                    // rates and skip averaging to avoid silently wrong numbers.
                    processed.insert(pos);
                    do_propagate(pos, &belt_dir_map, &feeders, &splitter_sibling, &mut in_degree, &mut queue, &mut lane_rates);
                    notify_ug_deps(pos, &mut in_degree, &mut queue);
                    continue;
                } else {
                    let pos_rates = lane_rates.get(&pos).copied().unwrap_or([0.0, 0.0]);
                    let sib_rates = lane_rates.get(&sib).copied().unwrap_or([0.0, 0.0]);
                    let total_l = pos_rates[0] + sib_rates[0];
                    let total_r = pos_rates[1] + sib_rates[1];
                    for &tile in &[pos, sib] {
                        let r = lane_rates.entry(tile).or_insert([0.0, 0.0]);
                        r[0] = total_l / 2.0;
                        r[1] = total_r / 2.0;
                    }
                    for &tile in &[sib, pos] {
                        processed.insert(tile);
                        do_propagate(tile, &belt_dir_map, &feeders, &splitter_sibling, &mut in_degree, &mut queue, &mut lane_rates);
                        notify_ug_deps(tile, &mut in_degree, &mut queue);
                    }
                    continue;
                }
            }
        }

        processed.insert(pos);
        do_propagate(pos, &belt_dir_map, &feeders, &splitter_sibling, &mut in_degree, &mut queue, &mut lane_rates);
        notify_ug_deps(pos, &mut in_degree, &mut queue);
    }

    lane_rates
}

fn do_propagate(
    tile: (i32, i32),
    belt_dir_map: &FxHashMap<(i32, i32), EntityDirection>,
    feeders: &FxHashMap<(i32, i32), Vec<((i32, i32), u8)>>,
    splitter_sibling: &FxHashMap<(i32, i32), (i32, i32)>,
    in_degree: &mut FxHashMap<(i32, i32), i32>,
    queue: &mut VecDeque<(i32, i32)>,
    lane_rates: &mut FxHashMap<(i32, i32), [f64; 2]>,
) {
    let d = match belt_dir_map.get(&tile) {
        Some(&d) => d,
        None => return,
    };
    let (ddx, ddy) = dir_to_vec(d);
    let downstream = (tile.0 + ddx, tile.1 + ddy);
    if !belt_dir_map.contains_key(&downstream) {
        return;
    }

    let my_rates = *lane_rates.get(&tile).unwrap_or(&[0.0, 0.0]);
    let ds_d = belt_dir_map[&downstream];
    let (downstream_dx, downstream_dy) = dir_to_vec(ds_d);
    let (left_dx, left_dy) = (-downstream_dy, downstream_dx);

    let ds_rates = lane_rates.entry(downstream).or_insert([0.0, 0.0]);

    if d == ds_d {
        // Straight continuation
        ds_rates[0] += my_rates[0];
        ds_rates[1] += my_rates[1];
    } else {
        let behind_downstream = (downstream.0 - downstream_dx, downstream.1 - downstream_dy);
        if tile == behind_downstream {
            ds_rates[0] += my_rates[0];
            ds_rates[1] += my_rates[1];
        } else {
            // Check if sideload (downstream has a straight feeder) or turn
            let ds_feeders = feeders.get(&downstream);
            let has_straight =
                ds_feeders.is_some_and(|fs| fs.iter().any(|(_, ft)| *ft == 0));

            if has_straight {
                // Sideload: all items go onto the near lane
                let rel_x = tile.0 - downstream.0;
                let rel_y = tile.1 - downstream.1;
                let dot = rel_x * left_dx + rel_y * left_dy;
                let total = my_rates[0] + my_rates[1];
                if dot > 0 {
                    ds_rates[0] += total;
                } else {
                    ds_rates[1] += total;
                }
            } else {
                // 90-degree turn: CW or CCW
                let cross = ddx * downstream_dy - ddy * downstream_dx;
                if cross > 0 {
                    // Clockwise
                    ds_rates[1] += my_rates[0];
                    ds_rates[0] += my_rates[1];
                } else {
                    // Counter-clockwise
                    ds_rates[0] += my_rates[0];
                    ds_rates[1] += my_rates[1];
                }
            }
        }
    }

    let deg = in_degree.entry(downstream).or_insert(0);
    *deg -= 1;
    let ready = *deg <= 0;
    if ready {
        queue.push_back(downstream);
    }
    // If downstream is one half of a splitter, also decrement its sibling so
    // both tiles reach zero in lockstep (see in_degree unification above).
    if let Some(&sib) = splitter_sibling.get(&downstream) {
        let sib_deg = in_degree.entry(sib).or_insert(0);
        *sib_deg -= 1;
        if *sib_deg <= 0 {
            queue.push_back(sib);
        }
    }
}

// ---------------------------------------------------------------------------
/// Check that the belt rate arriving at each consumer's input inserter pickup
/// point meets the machine's required input rate.
///
/// Uses the same lane rate propagation as `check_lane_throughput` (topological
/// sort with splitter 50/50 handling) but instead of checking capacity, checks
/// that the delivered rate matches what the machine needs.
pub fn check_input_rate_delivery(
    layout: &LayoutResult,
    solver: Option<&SolverResult>,
) -> Vec<ValidationIssue> {
    let sr = match solver {
        Some(s) => s,
        None => return Vec::new(),
    };

    let lane_rates = compute_lane_rates_impl(layout, solver);
    if lane_rates.is_empty() {
        return Vec::new();
    }

    let machine_tiles_set = build_machine_tile_set(layout);
    let machine_by_tile = build_machine_by_tile(layout);

    let recipe_to_spec: FxHashMap<&str, &crate::models::MachineSpec> = sr
        .machines
        .iter()
        .map(|s| (s.recipe.as_str(), s))
        .collect();
    let mut machine_entity: FxHashMap<(i32, i32), &PlacedEntity> = FxHashMap::default();
    for e in &layout.entities {
        if is_machine_entity(&e.name) {
            machine_entity.insert((e.x, e.y), e);
        }
    }

    let mut belt_carries: FxHashMap<(i32, i32), Option<String>> = FxHashMap::default();
    for e in &layout.entities {
        if is_belt_entity(&e.name) {
            belt_carries.insert((e.x, e.y), e.carries.clone());
            if is_splitter(&e.name) {
                belt_carries.insert(splitter_second_tile(e), e.carries.clone());
            }
        }
    }

    let mut issues = Vec::new();

    // First pass: collect input inserters and count how many feed each (machine, item) pair.
    struct InputInserter {
        pickup_pos: (i32, i32),
        machine_pos: (i32, i32),
        carried_item: String,
    }
    let mut inserters: Vec<InputInserter> = Vec::new();
    let mut inserter_count: FxHashMap<((i32, i32), String), usize> = FxHashMap::default();

    for ins in &layout.entities {
        if !is_inserter(&ins.name) {
            continue;
        }
        let (dx, dy) = dir_to_vec(ins.direction);
        let reach = inserter_reach(&ins.name);
        let drop_pos = (ins.x + dx * reach, ins.y + dy * reach);
        let pickup_pos = (ins.x - dx * reach, ins.y - dy * reach);

        if !machine_tiles_set.contains(&drop_pos) {
            continue;
        }
        let mpos = match machine_by_tile.get(&drop_pos) {
            Some(&p) => p,
            None => continue,
        };
        let carried_item = match belt_carries.get(&pickup_pos).and_then(|c| c.as_deref()) {
            Some(i) => i.to_string(),
            None => continue,
        };
        *inserter_count.entry((mpos, carried_item.clone())).or_insert(0) += 1;
        inserters.push(InputInserter {
            pickup_pos,
            machine_pos: mpos,
            carried_item,
        });
    }

    // Second pass: check each inserter's available rate vs its share of the required rate.
    for ins in &inserters {
        let me = match machine_entity.get(&ins.machine_pos) {
            Some(e) => e,
            None => continue,
        };
        let spec = match me.recipe.as_deref().and_then(|r| recipe_to_spec.get(r)) {
            Some(s) => *s,
            None => continue,
        };
        // spec.inputs[].rate is the per-machine input rate at full utilization.
        // The layout places ceil(spec.count) physical machines, each running at
        // spec.count / ceil(spec.count) utilization — scale the required rate
        // accordingly or the check is too strict by up to 10× when the solver
        // needs a fractional machine (e.g. sulfuric-acid at 5/s wants only 0.1
        // machines but the physical machine runs at 10% speed).
        let effective_count = spec.count.ceil().max(1.0);
        let utilization = (spec.count / effective_count).min(1.0);
        let required_rate = spec
            .inputs
            .iter()
            .find(|i| i.item == ins.carried_item)
            .map(|i| i.rate * utilization)
            .unwrap_or(0.0);
        if required_rate <= 0.0 {
            continue;
        }
        let count = inserter_count.get(&(ins.machine_pos, ins.carried_item.clone())).copied().unwrap_or(1);
        let per_inserter_rate = required_rate / count as f64;

        let available = match lane_rates.get(&ins.pickup_pos) {
            Some(&[left, right]) => left + right,
            None => 0.0,
        };

        if available < per_inserter_rate - 0.01 {
            issues.push(ValidationIssue::with_pos(
                Severity::Warning,
                "input-rate-delivery",
                format!(
                    "Input belt at ({},{}) delivers {:.1}/s but machine needs {:.1}/s of {} (across {} inserter{})",
                    ins.pickup_pos.0, ins.pickup_pos.1, available, required_rate, ins.carried_item,
                    count, if count > 1 { "s" } else { "" }
                ),
                ins.pickup_pos.0,
                ins.pickup_pos.1,
            ));
        }
    }

    issues
}

// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{EntityDirection, ItemFlow, LayoutResult, MachineSpec, PlacedEntity,
                       SolverResult};

    fn belt(x: i32, y: i32, dir: EntityDirection) -> PlacedEntity {
        PlacedEntity {
            name: "transport-belt".to_string(),
            x,
            y,
            direction: dir,
            recipe: None,
            io_type: None,
            carries: None,
            mirror: false,
            segment_id: None,
            ..Default::default()
        }
    }

    fn belt_carries(x: i32, y: i32, dir: EntityDirection, item: &str) -> PlacedEntity {
        PlacedEntity {
            name: "transport-belt".to_string(),
            x,
            y,
            direction: dir,
            recipe: None,
            io_type: None,
            carries: Some(item.to_string()),
            mirror: false,
            segment_id: None,
            ..Default::default()
        }
    }

    fn inserter(x: i32, y: i32, dir: EntityDirection) -> PlacedEntity {
        PlacedEntity {
            name: "inserter".to_string(),
            x,
            y,
            direction: dir,
            recipe: None,
            io_type: None,
            carries: None,
            mirror: false,
            segment_id: None,
            ..Default::default()
        }
    }

    fn machine(x: i32, y: i32, recipe: &str) -> PlacedEntity {
        PlacedEntity {
            name: "assembling-machine-1".to_string(),
            x,
            y,
            direction: EntityDirection::North,
            recipe: Some(recipe.to_string()),
            io_type: None,
            carries: None,
            mirror: false,
            segment_id: None,
            ..Default::default()
        }
    }

    fn ug_belt(x: i32, y: i32, dir: EntityDirection, io_type: &str) -> PlacedEntity {
        PlacedEntity {
            name: "underground-belt".to_string(),
            x,
            y,
            direction: dir,
            recipe: None,
            io_type: Some(io_type.to_string()),
            carries: None,
            mirror: false,
            segment_id: None,
            ..Default::default()
        }
    }

    fn simple_solver(input_rate: f64, output_rate: f64) -> SolverResult {
        SolverResult {
            machines: vec![MachineSpec {
                entity: "assembling-machine-3".to_string(),
                recipe: "iron-gear-wheel".to_string(),
                count: 1.0,
                inputs: vec![ItemFlow {
                    item: "iron-plate".to_string(),
                    rate: input_rate,
                    is_fluid: false,
                }],
                outputs: vec![ItemFlow {
                    item: "iron-gear-wheel".to_string(),
                    rate: output_rate,
                    is_fluid: false,
                }],
            }],
            external_inputs: vec![ItemFlow {
                item: "iron-plate".to_string(),
                rate: input_rate,
                is_fluid: false,
            }],
            external_outputs: vec![ItemFlow {
                item: "iron-gear-wheel".to_string(),
                rate: output_rate,
                is_fluid: false,
            }],
            dependency_order: vec!["iron-gear-wheel".to_string()],
        }
    }

    // --- belt_dir_map_from ---

    #[test]
    fn belt_dir_map_surface_belt() {
        let e = belt(3, 5, EntityDirection::East);
        let map = belt_dir_map_from(&[e]);
        assert_eq!(map.get(&(3, 5)), Some(&EntityDirection::East));
    }

    #[test]
    fn belt_dir_map_splitter_expands() {
        let sp = PlacedEntity {
            name: "splitter".to_string(),
            x: 2,
            y: 4,
            direction: EntityDirection::North,
            ..Default::default()
        };
        let map = belt_dir_map_from(&[sp]);
        assert!(map.contains_key(&(2, 4)));
        assert!(map.contains_key(&(3, 4))); // second tile (North/South → x+1)
    }

    // --- build_ug_pairs ---

    #[test]
    fn ug_pairs_basic_east() {
        let layout = LayoutResult {
            entities: vec![
                ug_belt(0, 0, EntityDirection::East, "input"),
                ug_belt(3, 0, EntityDirection::East, "output"),
            ],
            width: 10,
            height: 10,
            ..Default::default()
        };
        let pairs = build_ug_pairs(&layout);
        assert_eq!(pairs.get(&(0, 0)), Some(&(3, 0)));
        assert_eq!(pairs.get(&(3, 0)), Some(&(0, 0)));
    }

    #[test]
    fn ug_pairs_no_match_different_direction() {
        let layout = LayoutResult {
            entities: vec![
                ug_belt(0, 0, EntityDirection::East, "input"),
                ug_belt(3, 0, EntityDirection::West, "output"),
            ],
            width: 10,
            height: 10,
            ..Default::default()
        };
        let pairs = build_ug_pairs(&layout);
        assert!(pairs.is_empty());
    }

    // --- bfs_belt_reach ---

    #[test]
    fn bfs_belt_reach_connected() {
        let tiles: FxHashSet<(i32, i32)> =
            [(0, 0), (1, 0), (2, 0)].iter().copied().collect();
        let starts: FxHashSet<(i32, i32)> = [(0, 0)].iter().copied().collect();
        let reached = bfs_belt_reach(&starts, &tiles, None);
        assert_eq!(reached.len(), 3);
    }

    #[test]
    fn bfs_belt_reach_with_ug_jump() {
        let tiles: FxHashSet<(i32, i32)> =
            [(0, 0), (5, 0)].iter().copied().collect();
        let mut ug: FxHashMap<(i32, i32), (i32, i32)> = FxHashMap::default();
        ug.insert((0, 0), (5, 0));
        ug.insert((5, 0), (0, 0));
        let starts: FxHashSet<(i32, i32)> = [(0, 0)].iter().copied().collect();
        let reached = bfs_belt_reach(&starts, &tiles, Some(&ug));
        assert_eq!(reached.len(), 2);
    }

    // --- check_belt_connectivity ---

    #[test]
    fn belt_connectivity_inserter_with_belt_ok() {
        // 3x3 machine at (0,0), inserter at (1,-1) SOUTH, belt at (1,-2) extended
        let lr = LayoutResult {
            entities: vec![
                machine(0, 0, "iron-gear-wheel"),
                inserter(1, -1, EntityDirection::South),
                belt_carries(1, -2, EntityDirection::East, "iron-plate"),
                belt_carries(2, -2, EntityDirection::East, "iron-plate"),
            ],
            width: 20,
            height: 20,
            ..Default::default()
        };
        let issues = check_belt_connectivity(&lr, None);
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        assert!(errors.is_empty(), "unexpected errors: {:?}", errors);
    }

    #[test]
    fn belt_connectivity_no_belts_with_machine_error() {
        let lr = LayoutResult {
            entities: vec![machine(0, 0, "iron-gear-wheel")],
            width: 10,
            height: 10,
            ..Default::default()
        };
        let issues = check_belt_connectivity(&lr, None);
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        assert!(!errors.is_empty());
    }

    #[test]
    fn belt_connectivity_inserter_without_belt_error() {
        let lr = LayoutResult {
            entities: vec![
                machine(0, 0, "iron-gear-wheel"),
                inserter(1, -1, EntityDirection::South),
            ],
            width: 10,
            height: 10,
            ..Default::default()
        };
        let issues = check_belt_connectivity(&lr, None);
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        assert!(!errors.is_empty());
        assert_eq!(errors[0].category, "belt-connectivity");
    }

    #[test]
    fn belt_connectivity_isolated_single_belt_error() {
        let lr = LayoutResult {
            entities: vec![
                machine(0, 0, "iron-gear-wheel"),
                inserter(1, -1, EntityDirection::South),
                belt(1, -2, EntityDirection::East),
            ],
            width: 10,
            height: 10,
            ..Default::default()
        };
        let issues = check_belt_connectivity(&lr, None);
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        assert!(!errors.is_empty());
        assert!(errors[0].message.contains("isolated"));
    }

    // --- check_belt_flow_path ---

    #[test]
    fn belt_flow_path_connected_to_boundary_ok() {
        let lr = LayoutResult {
            entities: vec![
                machine(5, 5, "iron-gear-wheel"),
                inserter(6, 4, EntityDirection::South),
                belt(6, 3, EntityDirection::East),
                belt(5, 3, EntityDirection::East),
                belt(4, 3, EntityDirection::East),
                belt(3, 3, EntityDirection::East),
            ],
            width: 20,
            height: 20,
            ..Default::default()
        };
        let issues = check_belt_flow_path(&lr, None, LayoutStyle::Spaghetti);
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        assert!(errors.is_empty(), "unexpected errors: {:?}", errors);
    }

    #[test]
    fn belt_flow_path_disconnected_input_error() {
        let lr = LayoutResult {
            entities: vec![
                machine(10, 10, "iron-gear-wheel"),
                inserter(11, 9, EntityDirection::South),
                belt(11, 8, EntityDirection::East),
                belt(12, 8, EntityDirection::East),
                // Push boundary far
                belt(0, 0, EntityDirection::East),
                belt(30, 30, EntityDirection::East),
            ],
            width: 50,
            height: 50,
            ..Default::default()
        };
        let issues = check_belt_flow_path(&lr, None, LayoutStyle::Spaghetti);
        let errors: Vec<_> = issues
            .iter()
            .filter(|i| i.severity == Severity::Error && i.category == "belt-flow-path")
            .collect();
        assert_eq!(errors.len(), 1);
    }

    // --- check_belt_throughput ---

    #[test]
    fn belt_throughput_no_overlap_ok() {
        let lr = LayoutResult {
            entities: vec![
                belt(0, 0, EntityDirection::East),
                belt(1, 0, EntityDirection::East),
            ],
            width: 10,
            height: 10,
            ..Default::default()
        };
        assert!(check_belt_throughput(&lr).is_empty());
    }

    #[test]
    fn belt_throughput_overlapping_warning() {
        let lr = LayoutResult {
            entities: vec![
                belt(0, 0, EntityDirection::East),
                belt(0, 0, EntityDirection::South),
            ],
            width: 10,
            height: 10,
            ..Default::default()
        };
        let issues = check_belt_throughput(&lr);
        let warnings: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Warning).collect();
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].category, "belt-throughput");
        assert!(warnings[0].message.contains("2 overlapping"));
    }

    // --- check_belt_junctions ---

    #[test]
    fn belt_junctions_head_on_is_error() {
        let lr = LayoutResult {
            entities: vec![
                belt_carries(0, 0, EntityDirection::East, "iron-plate"),
                belt_carries(1, 0, EntityDirection::West, "iron-plate"),
            ],
            width: 10,
            height: 10,
            ..Default::default()
        };
        let issues = check_belt_junctions(&lr);
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        assert!(!errors.is_empty());
        assert!(errors.iter().any(|e| e.message.contains("HEAD-ON")));
    }

    #[test]
    fn belt_junctions_perpendicular_sideload_ok() {
        let lr = LayoutResult {
            entities: vec![
                belt_carries(0, 0, EntityDirection::East, "iron-plate"),
                belt_carries(0, 1, EntityDirection::North, "iron-plate"),
            ],
            width: 10,
            height: 10,
            ..Default::default()
        };
        let issues = check_belt_junctions(&lr);
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        assert!(errors.is_empty());
    }

    #[test]
    fn belt_junctions_same_direction_ok() {
        let lr = LayoutResult {
            entities: vec![
                belt_carries(0, 0, EntityDirection::East, "iron-plate"),
                belt_carries(1, 0, EntityDirection::East, "iron-plate"),
            ],
            width: 10,
            height: 10,
            ..Default::default()
        };
        assert!(check_belt_junctions(&lr).is_empty());
    }

    #[test]
    fn belt_junctions_different_items_not_checked() {
        let lr = LayoutResult {
            entities: vec![
                belt_carries(0, 0, EntityDirection::East, "iron-plate"),
                belt_carries(1, 0, EntityDirection::West, "copper-plate"),
            ],
            width: 10,
            height: 10,
            ..Default::default()
        };
        assert!(check_belt_junctions(&lr).is_empty());
    }

    // --- check_belt_flow_reachability ---

    #[test]
    fn flow_reachability_straight_east_ok() {
        let sr = simple_solver(5.0, 2.5);
        let mut entities = vec![
            PlacedEntity {
                name: "assembling-machine-3".to_string(),
                x: 3,
                y: 0,
                direction: EntityDirection::North,
                recipe: Some("iron-gear-wheel".to_string()),
                ..Default::default()
            },
            inserter(4, -1, EntityDirection::South),
        ];
        for x in 0..5 {
            entities.push(belt(x, -2, EntityDirection::East));
        }
        let lr = LayoutResult {
            entities,
            width: 20,
            height: 20,
            ..Default::default()
        };
        let issues = check_belt_flow_reachability(&lr, Some(&sr), LayoutStyle::Spaghetti);
        let errors: Vec<_> = issues
            .iter()
            .filter(|i| i.message.contains("can't reach input"))
            .collect();
        assert!(errors.is_empty(), "unexpected errors: {:?}", errors);
    }

    #[test]
    fn flow_reachability_reversed_belt_fails() {
        let sr = simple_solver(5.0, 2.5);
        let mut entities = vec![
            PlacedEntity {
                name: "assembling-machine-3".to_string(),
                x: 3,
                y: 0,
                direction: EntityDirection::North,
                recipe: Some("iron-gear-wheel".to_string()),
                ..Default::default()
            },
            inserter(4, -1, EntityDirection::South),
        ];
        for x in 0..5 {
            entities.push(belt(x, -2, EntityDirection::West)); // reversed
        }
        let lr = LayoutResult {
            entities,
            width: 20,
            height: 20,
            ..Default::default()
        };
        let issues = check_belt_flow_reachability(&lr, Some(&sr), LayoutStyle::Spaghetti);
        let errors: Vec<_> = issues
            .iter()
            .filter(|i| i.message.contains("can't reach input"))
            .collect();
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn flow_reachability_output_dead_end_fails() {
        let sr = simple_solver(5.0, 2.5);
        let mut entities = vec![
            PlacedEntity {
                name: "assembling-machine-3".to_string(),
                x: 3,
                y: 0,
                direction: EntityDirection::North,
                recipe: Some("iron-gear-wheel".to_string()),
                ..Default::default()
            },
            inserter(4, -1, EntityDirection::South),
        ];
        for x in 0..5 {
            entities.push(belt(x, -2, EntityDirection::East));
        }
        // Output inserter drops onto a NORTH-facing belt (dead-end)
        entities.push(inserter(4, 3, EntityDirection::South));
        entities.push(belt(4, 4, EntityDirection::North));
        let lr = LayoutResult {
            entities,
            width: 20,
            height: 20,
            ..Default::default()
        };
        let issues = check_belt_flow_reachability(&lr, Some(&sr), LayoutStyle::Spaghetti);
        let errors: Vec<_> = issues
            .iter()
            .filter(|i| i.message.contains("can't leave output"))
            .collect();
        assert_eq!(errors.len(), 1);
    }

    // --- check_belt_dead_ends: UG output ---

    #[test]
    fn ug_output_dead_end_detected() {
        // UG output facing South into empty space should be flagged
        let lr = LayoutResult {
            entities: vec![
                belt(0, 0, EntityDirection::South),
                ug_belt(0, 1, EntityDirection::South, "input"),
                ug_belt(0, 3, EntityDirection::South, "output"),
                // nothing at (0,4) — dead end
            ],
            width: 10,
            height: 10,
            ..Default::default()
        };
        let issues = check_belt_dead_ends(&lr);
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        assert_eq!(errors.len(), 1, "expected 1 dead-end for UG output into nothing");
        assert!(errors[0].message.contains("UG output"));
    }

    #[test]
    fn ug_output_with_receiver_ok() {
        // UG output into a surface belt at layout edge — no dead end
        let lr = LayoutResult {
            entities: vec![
                ug_belt(0, 0, EntityDirection::South, "input"),
                ug_belt(0, 2, EntityDirection::South, "output"),
                belt(0, 3, EntityDirection::South), // receives UG output, flows off edge
            ],
            width: 10,
            height: 4, // belt at y=3 flows to y=4 which is off-edge — OK
            ..Default::default()
        };
        let issues = check_belt_dead_ends(&lr);
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        assert!(errors.is_empty(), "UG output into belt should not be flagged");
    }

    // --- check_underground_belt_pairs ---

    #[test]
    fn ug_pairs_valid_east() {
        let lr = LayoutResult {
            entities: vec![
                ug_belt(0, 0, EntityDirection::East, "input"),
                ug_belt(3, 0, EntityDirection::East, "output"),
            ],
            width: 10,
            height: 10,
            ..Default::default()
        };
        assert!(check_underground_belt_pairs(&lr).is_empty());
    }

    #[test]
    fn ug_pairs_unpaired_input_error() {
        let lr = LayoutResult {
            entities: vec![ug_belt(0, 0, EntityDirection::East, "input")],
            width: 10,
            height: 10,
            ..Default::default()
        };
        let issues = check_underground_belt_pairs(&lr);
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("Unpaired"));
    }

    #[test]
    fn ug_pairs_unpaired_output_error() {
        let lr = LayoutResult {
            entities: vec![ug_belt(5, 0, EntityDirection::East, "output")],
            width: 10,
            height: 10,
            ..Default::default()
        };
        let issues = check_underground_belt_pairs(&lr);
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("Unpaired"));
    }

    #[test]
    fn ug_pairs_over_range_error() {
        // transport-belt max reach 4, distance 6 → error
        let lr = LayoutResult {
            entities: vec![
                ug_belt(0, 0, EntityDirection::East, "input"),
                ug_belt(6, 0, EntityDirection::East, "output"),
            ],
            width: 20,
            height: 10,
            ..Default::default()
        };
        let issues = check_underground_belt_pairs(&lr);
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        assert!(errors.iter().any(|e| e.message.contains("exceeds max reach")));
    }

    #[test]
    fn ug_pairs_at_max_range_ok() {
        let lr = LayoutResult {
            entities: vec![
                ug_belt(0, 0, EntityDirection::East, "input"),
                ug_belt(4, 0, EntityDirection::East, "output"),
            ],
            width: 10,
            height: 10,
            ..Default::default()
        };
        let issues = check_underground_belt_pairs(&lr);
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        assert!(errors.is_empty());
    }

    #[test]
    fn ug_pairs_wrong_direction_not_paired() {
        let lr = LayoutResult {
            entities: vec![
                ug_belt(0, 0, EntityDirection::East, "input"),
                ug_belt(3, 0, EntityDirection::West, "output"),
            ],
            width: 10,
            height: 10,
            ..Default::default()
        };
        let issues = check_underground_belt_pairs(&lr);
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn ug_pairs_intercepting_warning() {
        let lr = LayoutResult {
            entities: vec![
                ug_belt(0, 0, EntityDirection::East, "input"),
                ug_belt(2, 0, EntityDirection::East, "input"),
                ug_belt(3, 0, EntityDirection::East, "output"),
            ],
            width: 10,
            height: 10,
            ..Default::default()
        };
        let issues = check_underground_belt_pairs(&lr);
        let warnings: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Warning).collect();
        assert!(warnings.iter().any(|w| w.message.contains("intercepts")));
    }

    // --- check_underground_belt_sideloading ---

    #[test]
    fn ug_sideload_same_direction_ok() {
        let lr = LayoutResult {
            entities: vec![
                ug_belt(0, 0, EntityDirection::East, "output"),
                belt(1, 0, EntityDirection::East),
            ],
            width: 10,
            height: 10,
            ..Default::default()
        };
        assert!(check_underground_belt_sideloading(&lr).is_empty());
    }

    #[test]
    fn ug_sideload_head_on_error() {
        let lr = LayoutResult {
            entities: vec![
                ug_belt(0, 0, EntityDirection::East, "output"),
                belt(1, 0, EntityDirection::West),
            ],
            width: 10,
            height: 10,
            ..Default::default()
        };
        let issues = check_underground_belt_sideloading(&lr);
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("head-on"));
    }

    #[test]
    fn ug_sideload_perpendicular_ok() {
        let lr = LayoutResult {
            entities: vec![
                ug_belt(0, 0, EntityDirection::East, "output"),
                belt(1, 0, EntityDirection::North),
            ],
            width: 10,
            height: 10,
            ..Default::default()
        };
        let issues = check_underground_belt_sideloading(&lr);
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        assert!(errors.is_empty());
    }

    #[test]
    fn ug_sideload_input_ignored() {
        let lr = LayoutResult {
            entities: vec![
                ug_belt(0, 0, EntityDirection::East, "input"),
                belt(1, 0, EntityDirection::West),
            ],
            width: 10,
            height: 10,
            ..Default::default()
        };
        assert!(check_underground_belt_sideloading(&lr).is_empty());
    }

    // --- check_belt_loops ---

    #[test]
    fn belt_loops_no_cycle_ok() {
        let lr = LayoutResult {
            entities: vec![
                belt(0, 0, EntityDirection::East),
                belt(1, 0, EntityDirection::East),
                belt(2, 0, EntityDirection::East),
            ],
            width: 10,
            height: 10,
            ..Default::default()
        };
        assert!(check_belt_loops(&lr).is_empty());
    }

    #[test]
    fn belt_loops_cycle_detected() {
        // 2×2 cycle: (0,0)→E→(1,0)→S→(1,1)→W→(0,1)→N→(0,0)
        let lr = LayoutResult {
            entities: vec![
                belt(0, 0, EntityDirection::East),
                belt(1, 0, EntityDirection::South),
                belt(1, 1, EntityDirection::West),
                belt(0, 1, EntityDirection::North),
            ],
            width: 10,
            height: 10,
            ..Default::default()
        };
        let issues = check_belt_loops(&lr);
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        assert!(!errors.is_empty());
        assert!(errors[0].message.contains("loop"));
    }

    // --- check_belt_item_isolation ---

    #[test]
    fn belt_item_isolation_same_item_ok() {
        let lr = LayoutResult {
            entities: vec![
                belt_carries(0, 0, EntityDirection::East, "iron-plate"),
                belt_carries(1, 0, EntityDirection::East, "iron-plate"),
            ],
            width: 10,
            height: 10,
            ..Default::default()
        };
        assert!(check_belt_item_isolation(&lr).is_empty());
    }

    #[test]
    fn belt_item_isolation_different_items_error() {
        let lr = LayoutResult {
            entities: vec![
                belt_carries(0, 0, EntityDirection::East, "iron-plate"),
                belt_carries(1, 0, EntityDirection::East, "copper-plate"),
            ],
            width: 10,
            height: 10,
            ..Default::default()
        };
        let issues = check_belt_item_isolation(&lr);
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        assert!(!errors.is_empty());
        assert!(errors[0].message.contains("iron-plate"));
        assert!(errors[0].message.contains("copper-plate"));
    }

    // --- check_belt_inserter_conflict ---

    #[test]
    fn inserter_conflict_same_item_ok() {
        let belt_e = PlacedEntity {
            name: "transport-belt".to_string(),
            x: 0,
            y: 1,
            direction: EntityDirection::East,
            carries: Some("iron-plate".to_string()),
            ..Default::default()
        };
        let ins = PlacedEntity {
            name: "inserter".to_string(),
            x: 0,
            y: 0,
            direction: EntityDirection::South,
            carries: Some("iron-plate".to_string()),
            ..Default::default()
        };
        let lr = LayoutResult {
            entities: vec![belt_e, ins],
            width: 10,
            height: 10,
            ..Default::default()
        };
        assert!(check_belt_inserter_conflict(&lr).is_empty());
    }

    #[test]
    fn inserter_conflict_different_items_error() {
        // Two inserters dropping onto the same belt tile with different items
        let belt_e = PlacedEntity {
            name: "transport-belt".to_string(),
            x: 0,
            y: 1,
            direction: EntityDirection::East,
            carries: Some("iron-plate".to_string()),
            ..Default::default()
        };
        let ins1 = PlacedEntity {
            name: "inserter".to_string(),
            x: 0,
            y: 0,
            direction: EntityDirection::South,
            carries: Some("iron-plate".to_string()),
            ..Default::default()
        };
        let ins2 = PlacedEntity {
            name: "inserter".to_string(),
            x: 1,
            y: 1,
            direction: EntityDirection::West,
            carries: Some("copper-plate".to_string()),
            ..Default::default()
        };
        let lr = LayoutResult {
            entities: vec![belt_e, ins1, ins2],
            width: 10,
            height: 10,
            ..Default::default()
        };
        let issues = check_belt_inserter_conflict(&lr);
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        assert!(!errors.is_empty());
    }

    // --- check_lane_throughput ---

    #[test]
    fn lane_throughput_single_inserter_within_capacity() {
        let sr = SolverResult {
            machines: vec![MachineSpec {
                entity: "assembling-machine-3".to_string(),
                recipe: "iron-gear-wheel".to_string(),
                count: 1.0,
                inputs: vec![ItemFlow {
                    item: "iron-plate".to_string(),
                    rate: 5.0,
                    is_fluid: false,
                }],
                outputs: vec![ItemFlow {
                    item: "iron-gear-wheel".to_string(),
                    rate: 2.5,
                    is_fluid: false,
                }],
            }],
            external_inputs: vec![ItemFlow {
                item: "iron-plate".to_string(),
                rate: 5.0,
                is_fluid: false,
            }],
            external_outputs: vec![ItemFlow {
                item: "iron-gear-wheel".to_string(),
                rate: 2.5,
                is_fluid: false,
            }],
            dependency_order: vec!["iron-gear-wheel".to_string()],
        };

        let entities = vec![
            PlacedEntity {
                name: "assembling-machine-3".to_string(),
                x: 3,
                y: 0,
                direction: EntityDirection::North,
                recipe: Some("iron-gear-wheel".to_string()),
                ..Default::default()
            },
            PlacedEntity {
                name: "inserter".to_string(),
                x: 4,
                y: 3,
                direction: EntityDirection::South,
                ..Default::default()
            },
            PlacedEntity {
                name: "transport-belt".to_string(),
                x: 4,
                y: 4,
                direction: EntityDirection::East,
                carries: Some("iron-gear-wheel".to_string()),
                ..Default::default()
            },
            PlacedEntity {
                name: "transport-belt".to_string(),
                x: 5,
                y: 4,
                direction: EntityDirection::East,
                carries: Some("iron-gear-wheel".to_string()),
                ..Default::default()
            },
        ];
        let lr = LayoutResult {
            entities,
            width: 20,
            height: 20,
            ..Default::default()
        };
        let issues = check_lane_throughput(&lr, Some(&sr));
        assert!(issues.is_empty(), "unexpected issues: {:?}", issues);
    }

    #[test]
    fn lane_throughput_no_solver_returns_empty() {
        let lr = LayoutResult {
            entities: vec![],
            width: 10,
            height: 10,
            ..Default::default()
        };
        assert!(check_lane_throughput(&lr, None).is_empty());
    }

    // --- check_input_rate_delivery ---

    #[test]
    fn input_rate_delivery_no_solver_returns_empty() {
        let lr = LayoutResult {
            entities: vec![],
            width: 10,
            height: 10,
            ..Default::default()
        };
        assert!(check_input_rate_delivery(&lr, None).is_empty());
    }

    #[test]
    fn input_rate_delivery_sufficient_rate_ok() {
        // Two machines: producer (iron-plate) → belt chain → consumer (iron-gear-wheel).
        // Producer outputs 5/s iron-plate via inserter onto belt.
        // Consumer needs 5/s iron-plate — exactly matched.
        let sr = SolverResult {
            machines: vec![
                MachineSpec {
                    entity: "electric-furnace".to_string(),
                    recipe: "iron-plate".to_string(),
                    count: 1.0,
                    inputs: vec![ItemFlow {
                        item: "iron-ore".to_string(),
                        rate: 5.0,
                        is_fluid: false,
                    }],
                    outputs: vec![ItemFlow {
                        item: "iron-plate".to_string(),
                        rate: 5.0,
                        is_fluid: false,
                    }],
                },
                MachineSpec {
                    entity: "assembling-machine-3".to_string(),
                    recipe: "iron-gear-wheel".to_string(),
                    count: 1.0,
                    inputs: vec![ItemFlow {
                        item: "iron-plate".to_string(),
                        rate: 5.0,
                        is_fluid: false,
                    }],
                    outputs: vec![ItemFlow {
                        item: "iron-gear-wheel".to_string(),
                        rate: 2.5,
                        is_fluid: false,
                    }],
                },
            ],
            external_inputs: vec![ItemFlow {
                item: "iron-ore".to_string(),
                rate: 5.0,
                is_fluid: false,
            }],
            external_outputs: vec![ItemFlow {
                item: "iron-gear-wheel".to_string(),
                rate: 2.5,
                is_fluid: false,
            }],
            dependency_order: vec!["iron-plate".to_string(), "iron-gear-wheel".to_string()],
        };

        // Layout:
        //   Furnace at (0,0), output inserter at (1,3) South, drops onto belt at (1,4).
        //   Belt chain: (1,4) East → (2,4) East → (3,4) East.
        //   Assembler at (5,5), input inserter at (6,5) South picks from belt at (6,4) East.
        //   Wait — need the belt to flow from producer output to consumer input.
        //   Let's keep it simple: output inserter drops at (1,4), belt goes East to (3,4).
        //   But consumer inserter needs to pick from a belt tile in the chain.
        //
        // Simpler: producer at (0,0). Output inserter (1,3) South drops iron-plate at (1,4) East.
        // Belt (1,4)→(2,4)→(3,4) all East carrying iron-plate.
        // Consumer at (5,0). Input inserter at (6,3) South picks from belt at (6,4).
        // Hmm, the belt chain needs to reach (6,4).
        //
        // Even simpler: just one belt tile shared between output and input inserters.
        // Producer output inserter drops at (1,4). Consumer input inserter picks from (1,4).
        // But that's the same tile — inserter drops and picks from same belt.
        //
        // Simplest correct layout:
        // Output inserter at (1,3) South → drops onto belt (1,4) East
        // Belt chain (1,4) → (2,4) → (3,4) all East
        // Input inserter at (2,3) South ← picks from belt (2,4) East
        //   Wait, inserter at (2,3) South picks from (2,3-1)=(2,2) not (2,4).
        //   Inserter at (2,3) South: picks from (2,2), drops to (2,4). That's wrong direction.
        //   For input inserter: picks from belt, drops to machine.
        //   Inserter at (2,5) North: picks from (2,6), drops to (2,4). Picks from belt at (2,6).
        //
        // Let me use the standard bus template pattern:
        // Belt at y=0 East (input belt for assembler row).
        // Inserter at (1,1) South picks from belt at (1,0) drops to machine at (1,2).
        // Machine at (0,2) assembling-machine-3.
        // For the rate to be seeded, we need a PRODUCER output inserter dropping onto (1,0).
        // Producer machine at (0,-3), output inserter at (1,-1) South, drops at (1,0).
        // But the belt at (1,0) is EAST. Output inserter at (1,-1) South drops to (1,0). OK.
        // compute_lane_rates seeds 5/s at (1,0) left lane (inserter is inline with belt).
        // Input inserter at (1,1) South picks from (1,0). Needs 5/s. Available 5/s. OK.
        // Furnace at (0,-4), output inserter at (1,-1) South: drops to (1,0).
        // Single belt at (1,0) East (no chain → in_degree=0, seeded by injection).
        // Input inserter at (1,1) South: picks from (1,0), drops to (1,2).
        // Assembler at (0,2).
        let entities = vec![
            PlacedEntity {
                name: "electric-furnace".to_string(),
                x: 0,
                y: -4,
                direction: EntityDirection::North,
                recipe: Some("iron-plate".to_string()),
                ..Default::default()
            },
            PlacedEntity {
                name: "inserter".to_string(),
                x: 1,
                y: -1,
                direction: EntityDirection::South,
                carries: Some("iron-plate".to_string()),
                ..Default::default()
            },
            PlacedEntity {
                name: "transport-belt".to_string(),
                x: 1,
                y: 0,
                direction: EntityDirection::East,
                carries: Some("iron-plate".to_string()),
                ..Default::default()
            },
            PlacedEntity {
                name: "inserter".to_string(),
                x: 1,
                y: 1,
                direction: EntityDirection::South,
                carries: Some("iron-plate".to_string()),
                ..Default::default()
            },
            PlacedEntity {
                name: "assembling-machine-3".to_string(),
                x: 0,
                y: 2,
                direction: EntityDirection::North,
                recipe: Some("iron-gear-wheel".to_string()),
                ..Default::default()
            },
            PlacedEntity {
                name: "inserter".to_string(),
                x: 1,
                y: 5,
                direction: EntityDirection::South,
                carries: Some("iron-gear-wheel".to_string()),
                ..Default::default()
            },
            PlacedEntity {
                name: "transport-belt".to_string(),
                x: 1,
                y: 6,
                direction: EntityDirection::East,
                carries: Some("iron-gear-wheel".to_string()),
                ..Default::default()
            },
        ];
        let lr = LayoutResult {
            entities,
            width: 20,
            height: 20,
            ..Default::default()
        };
        let issues = check_input_rate_delivery(&lr, Some(&sr));
        assert!(issues.is_empty(), "unexpected issues: {:?}", issues);
    }

    #[test]
    fn input_rate_delivery_insufficient_rate_warns() {
        // Same layout but producer outputs 5/s, consumer needs 20/s.
        let sr = SolverResult {
            machines: vec![
                MachineSpec {
                    entity: "electric-furnace".to_string(),
                    recipe: "iron-plate".to_string(),
                    count: 1.0,
                    inputs: vec![ItemFlow {
                        item: "iron-ore".to_string(),
                        rate: 5.0,
                        is_fluid: false,
                    }],
                    outputs: vec![ItemFlow {
                        item: "iron-plate".to_string(),
                        rate: 5.0,
                        is_fluid: false,
                    }],
                },
                MachineSpec {
                    entity: "assembling-machine-3".to_string(),
                    recipe: "iron-gear-wheel".to_string(),
                    count: 1.0,
                    inputs: vec![ItemFlow {
                        item: "iron-plate".to_string(),
                        rate: 20.0,
                        is_fluid: false,
                    }],
                    outputs: vec![ItemFlow {
                        item: "iron-gear-wheel".to_string(),
                        rate: 10.0,
                        is_fluid: false,
                    }],
                },
            ],
            external_inputs: vec![],
            external_outputs: vec![ItemFlow {
                item: "iron-gear-wheel".to_string(),
                rate: 10.0,
                is_fluid: false,
            }],
            dependency_order: vec!["iron-plate".to_string(), "iron-gear-wheel".to_string()],
        };

        let entities = vec![
            PlacedEntity {
                name: "electric-furnace".to_string(),
                x: 0,
                y: -4,
                direction: EntityDirection::North,
                recipe: Some("iron-plate".to_string()),
                ..Default::default()
            },
            PlacedEntity {
                name: "inserter".to_string(),
                x: 1,
                y: -1,
                direction: EntityDirection::South,
                carries: Some("iron-plate".to_string()),
                ..Default::default()
            },
            PlacedEntity {
                name: "transport-belt".to_string(),
                x: 1,
                y: 0,
                direction: EntityDirection::East,
                carries: Some("iron-plate".to_string()),
                ..Default::default()
            },
            PlacedEntity {
                name: "inserter".to_string(),
                x: 1,
                y: 1,
                direction: EntityDirection::South,
                carries: Some("iron-plate".to_string()),
                ..Default::default()
            },
            PlacedEntity {
                name: "assembling-machine-3".to_string(),
                x: 0,
                y: 2,
                direction: EntityDirection::North,
                recipe: Some("iron-gear-wheel".to_string()),
                ..Default::default()
            },
            PlacedEntity {
                name: "inserter".to_string(),
                x: 1,
                y: 5,
                direction: EntityDirection::South,
                carries: Some("iron-gear-wheel".to_string()),
                ..Default::default()
            },
            PlacedEntity {
                name: "transport-belt".to_string(),
                x: 1,
                y: 6,
                direction: EntityDirection::East,
                carries: Some("iron-gear-wheel".to_string()),
                ..Default::default()
            },
        ];
        let lr = LayoutResult {
            entities,
            width: 20,
            height: 20,
            ..Default::default()
        };
        let issues = check_input_rate_delivery(&lr, Some(&sr));
        assert!(!issues.is_empty(), "expected warning for insufficient rate");
        assert!(issues.iter().any(|i| i.category == "input-rate-delivery"),
            "expected input-rate-delivery issue, got: {:?}", issues);
    }
}
