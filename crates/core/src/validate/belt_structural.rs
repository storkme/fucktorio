//! Belt-structural validation checks.
//!
//! Port of the belt-related checks from `src/validate.py`:
//! - `check_belt_loops`
//! - `check_belt_dead_ends`
//! - `check_belt_item_isolation`
//! - `check_belt_inserter_conflict`
//! - `check_belt_throughput`
//! - `check_lane_throughput` (with `compute_lane_rates`)
//! - `check_output_belt_coverage`

use std::collections::VecDeque;

use rustc_hash::{FxHashMap, FxHashSet};

use crate::common::{belt_throughput, dir_to_vec, inserter_target_lane, lane_capacity, machine_size};
use crate::models::{EntityDirection, LayoutResult, PlacedEntity, SolverResult};

use super::{Severity, ValidationIssue};

// ---------------------------------------------------------------------------
// Entity-set helpers
// ---------------------------------------------------------------------------

const SURFACE_BELT_ENTITIES: &[&str] =
    &["transport-belt", "fast-transport-belt", "express-transport-belt"];
const UG_BELT_ENTITIES: &[&str] = &[
    "underground-belt",
    "fast-underground-belt",
    "express-underground-belt",
];
const SPLITTER_ENTITIES: &[&str] = &["splitter", "fast-splitter", "express-splitter"];
const INSERTER_ENTITIES: &[&str] =
    &["inserter", "long-handed-inserter", "fast-inserter", "stack-inserter"];
const MACHINE_ENTITIES: &[&str] = &[
    "assembling-machine-1",
    "assembling-machine-2",
    "assembling-machine-3",
    "chemical-plant",
    "electric-furnace",
    "oil-refinery",
];

fn is_surface_belt(name: &str) -> bool {
    SURFACE_BELT_ENTITIES.contains(&name)
}
fn is_ug_belt(name: &str) -> bool {
    UG_BELT_ENTITIES.contains(&name)
}
fn is_splitter(name: &str) -> bool {
    SPLITTER_ENTITIES.contains(&name)
}
fn is_belt_entity(name: &str) -> bool {
    is_surface_belt(name) || is_ug_belt(name) || is_splitter(name)
}
fn is_inserter(name: &str) -> bool {
    INSERTER_ENTITIES.contains(&name)
}
fn is_machine(name: &str) -> bool {
    MACHINE_ENTITIES.contains(&name)
}

fn inserter_reach(name: &str) -> i32 {
    if name == "long-handed-inserter" { 2 } else { 1 }
}

fn ug_to_surface_tier(ug: &str) -> &'static str {
    match ug {
        "underground-belt" => "transport-belt",
        "fast-underground-belt" => "fast-transport-belt",
        "express-underground-belt" => "express-transport-belt",
        _ => "transport-belt",
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Build a map `(x, y) → EntityDirection` for all belt entities (expanding
/// splitters to both tiles).
fn belt_dir_map(entities: &[PlacedEntity]) -> FxHashMap<(i32, i32), EntityDirection> {
    let mut map = FxHashMap::default();
    for e in entities {
        if !is_belt_entity(&e.name) {
            continue;
        }
        map.insert((e.x, e.y), e.direction);
        if is_splitter(&e.name) {
            map.insert(splitter_second_tile(e), e.direction);
        }
    }
    map
}

/// Second tile occupied by a splitter (perpendicular to flow).
fn splitter_second_tile(e: &PlacedEntity) -> (i32, i32) {
    match e.direction {
        EntityDirection::North | EntityDirection::South => (e.x + 1, e.y),
        _ => (e.x, e.y + 1),
    }
}

/// All tiles occupied by belt entities (expanding splitters).
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

/// Build underground belt pair map: entry ↔ exit (bidirectional).
fn build_ug_pairs(entities: &[PlacedEntity]) -> FxHashMap<(i32, i32), (i32, i32)> {
    let ug_inputs: Vec<&PlacedEntity> = entities
        .iter()
        .filter(|e| is_ug_belt(&e.name) && e.io_type.as_deref() == Some("input"))
        .collect();
    let ug_outputs: Vec<&PlacedEntity> = entities
        .iter()
        .filter(|e| is_ug_belt(&e.name) && e.io_type.as_deref() == Some("output"))
        .collect();

    let mut pairs = FxHashMap::default();
    let mut used_outputs: FxHashSet<(i32, i32)> = FxHashSet::default();

    for inp in &ug_inputs {
        let (dx, dy) = dir_to_vec(inp.direction);
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

        if let Some(out) = best_out {
            pairs.insert((inp.x, inp.y), (out.x, out.y));
            pairs.insert((out.x, out.y), (inp.x, inp.y));
            used_outputs.insert((out.x, out.y));
        }
    }

    pairs
}

/// All tiles occupied by machines.
fn build_machine_tile_set(entities: &[PlacedEntity]) -> FxHashSet<(i32, i32)> {
    let mut tiles = FxHashSet::default();
    for e in entities {
        if is_machine(&e.name) {
            let size = machine_size(&e.name) as i32;
            for dx in 0..size {
                for dy in 0..size {
                    tiles.insert((e.x + dx, e.y + dy));
                }
            }
        }
    }
    tiles
}

// ---------------------------------------------------------------------------
// check_belt_loops
// ---------------------------------------------------------------------------

/// Check for belt loops (cycles where items circulate forever).
pub fn check_belt_loops(layout: &LayoutResult) -> Vec<ValidationIssue> {
    let bdm = belt_dir_map(&layout.entities);

    let mut issues = Vec::new();
    let mut confirmed: FxHashSet<(i32, i32)> = FxHashSet::default();
    let mut reported_loops: FxHashSet<Vec<(i32, i32)>> = FxHashSet::default();

    for &start in bdm.keys() {
        if confirmed.contains(&start) {
            continue;
        }

        let mut visited_order: Vec<(i32, i32)> = Vec::new();
        let mut visited_set: FxHashSet<(i32, i32)> = FxHashSet::default();
        let mut cur = start;

        while bdm.contains_key(&cur) && !visited_set.contains(&cur) {
            visited_set.insert(cur);
            visited_order.push(cur);
            let d = bdm[&cur];
            let (dx, dy) = dir_to_vec(d);
            cur = (cur.0 + dx, cur.1 + dy);
        }

        if visited_set.contains(&cur) {
            let cycle_start_idx = visited_order.iter().position(|&t| t == cur).unwrap_or(0);
            let mut loop_tiles: Vec<(i32, i32)> = visited_order[cycle_start_idx..].to_vec();
            loop_tiles.sort();

            if !reported_loops.contains(&loop_tiles) {
                reported_loops.insert(loop_tiles.clone());
                let rep = loop_tiles[0];
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
// check_belt_dead_ends
// ---------------------------------------------------------------------------

/// Check for surface belts whose output tile is empty and interior to the layout.
///
/// Belts flowing off the layout edge are OK (external I/O points).
pub fn check_belt_dead_ends(layout: &LayoutResult) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    // Tiles that can receive belt output: belts, splitters, inserter pickups.
    let mut receiver_tiles: FxHashSet<(i32, i32)> = FxHashSet::default();
    for e in &layout.entities {
        if is_belt_entity(&e.name) {
            receiver_tiles.insert((e.x, e.y));
            if is_splitter(&e.name) {
                let dir = e.direction;
                if dir == EntityDirection::North || dir == EntityDirection::South {
                    receiver_tiles.insert((e.x + 1, e.y));
                } else {
                    receiver_tiles.insert((e.x, e.y + 1));
                }
            }
        } else if is_inserter(&e.name) {
            let d = dir_to_vec(e.direction);
            let reach = inserter_reach(&e.name);
            receiver_tiles.insert((e.x - d.0 * reach, e.y - d.1 * reach));
        }
    }

    let mut belt_at: FxHashMap<(i32, i32), &PlacedEntity> = FxHashMap::default();
    for e in &layout.entities {
        if is_surface_belt(&e.name) {
            belt_at.insert((e.x, e.y), e);
        }
    }

    let mut pickup_tiles: FxHashSet<(i32, i32)> = FxHashSet::default();
    for e in &layout.entities {
        if is_inserter(&e.name) {
            let d = dir_to_vec(e.direction);
            let reach = inserter_reach(&e.name);
            pickup_tiles.insert((e.x - d.0 * reach, e.y - d.1 * reach));
        }
    }

    let w = layout.width;
    let h = layout.height;

    for e in &layout.entities {
        if !is_surface_belt(&e.name) {
            continue;
        }
        let d = dir_to_vec(e.direction);
        let out_x = e.x + d.0;
        let out_y = e.y + d.1;
        if out_x < 0 || out_x >= w || out_y < 0 || out_y >= h {
            continue;
        }
        if receiver_tiles.contains(&(out_x, out_y)) {
            continue;
        }
        if chain_has_pickup((e.x, e.y), e.direction, &belt_at, &pickup_tiles) {
            continue;
        }
        issues.push(ValidationIssue::with_pos(
            Severity::Error,
            "belt-dead-end",
            format!(
                "Belt at ({},{}) facing {:?} has no receiver at output tile ({},{}) \
                 — items accumulate with nowhere to go",
                e.x,
                e.y,
                e.direction,
                out_x,
                out_y
            ),
            e.x,
            e.y,
        ));
    }

    issues
}

/// Walk upstream from `tail_tile` along same-direction belts; return true if
/// any tile is an inserter pickup (items from that inserter drain the chain).
fn chain_has_pickup(
    tail_tile: (i32, i32),
    direction: EntityDirection,
    belt_at: &FxHashMap<(i32, i32), &PlacedEntity>,
    pickup_tiles: &FxHashSet<(i32, i32)>,
) -> bool {
    let d = dir_to_vec(direction);
    let mut cur = tail_tile;
    let mut visited: FxHashSet<(i32, i32)> = FxHashSet::default();
    for _ in 0..200 {
        if !visited.insert(cur) {
            break;
        }
        if pickup_tiles.contains(&cur) {
            return true;
        }
        let upstream = (cur.0 - d.0, cur.1 - d.1);
        match belt_at.get(&upstream) {
            Some(b) if b.direction == direction => cur = upstream,
            _ => break,
        }
    }
    false
}

// ---------------------------------------------------------------------------
// check_belt_item_isolation
// ---------------------------------------------------------------------------

/// Check that belts carrying different items don't feed into each other.
pub fn check_belt_item_isolation(layout: &LayoutResult) -> Vec<ValidationIssue> {
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

    let mut issues = Vec::new();
    let mut seen: FxHashSet<((i32, i32), (i32, i32))> = FxHashSet::default();

    for (&(ax, ay), &ad) in &belt_dir {
        // UG belt entries carry items underground, not to the adjacent surface tile.
        if ug_inputs.contains(&(ax, ay)) {
            continue;
        }
        let (dx, dy) = dir_to_vec(ad);
        let b = (ax + dx, ay + dy);
        if !belt_dir.contains_key(&b) {
            continue;
        }
        let ac = belt_carry.get(&(ax, ay)).and_then(|c| c.as_deref());
        let bc = belt_carry.get(&b).and_then(|c| c.as_deref());
        if let (Some(ac), Some(bc)) = (ac, bc) {
            if ac != bc && seen.insert(((ax, ay), b)) {
                issues.push(ValidationIssue::with_pos(
                    Severity::Error,
                    "belt-item-isolation",
                    format!(
                        "Belt at ({},{}) carries {} but feeds into ({},{}) which carries {}",
                        ax, ay, ac, b.0, b.1, bc
                    ),
                    ax,
                    ay,
                ));
            }
        }
    }

    issues
}

// ---------------------------------------------------------------------------
// check_belt_inserter_conflict
// ---------------------------------------------------------------------------

/// Check that multiple inserters don't drop different items onto the same belt tile.
pub fn check_belt_inserter_conflict(layout: &LayoutResult) -> Vec<ValidationIssue> {
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
        let dv = dir_to_vec(e.direction);
        let reach = inserter_reach(&e.name);
        let drop = (e.x + dv.0 * reach, e.y + dv.1 * reach);
        if belt_tiles.contains(&drop) {
            drop_map.entry(drop).or_default().push(carries);
        }
    }

    let mut issues = Vec::new();
    for ((bx, by), items) in &drop_map {
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
                *bx,
                *by,
            ));
        }
    }

    issues
}

// ---------------------------------------------------------------------------
// check_belt_throughput
// ---------------------------------------------------------------------------

/// Check that belt tiles don't have multiple overlapping route entities.
pub fn check_belt_throughput(layout: &LayoutResult) -> Vec<ValidationIssue> {
    let mut tile_counts: FxHashMap<(i32, i32), usize> = FxHashMap::default();
    let mut tile_names: FxHashMap<(i32, i32), &str> = FxHashMap::default();

    for e in &layout.entities {
        if !is_belt_entity(&e.name) {
            continue;
        }
        *tile_counts.entry((e.x, e.y)).or_insert(0) += 1;
        tile_names.insert((e.x, e.y), &e.name);
    }

    let mut issues = Vec::new();
    for ((x, y), &count) in &tile_counts {
        if count > 1 {
            let belt_name = tile_names[&(*x, *y)];
            let max_throughput = belt_throughput(belt_name);
            issues.push(ValidationIssue::with_pos(
                Severity::Warning,
                "belt-throughput",
                format!(
                    "Belt at ({},{}): {} overlapping routes on {} (max {}/s)",
                    x, y, count, belt_name, max_throughput
                ),
                *x,
                *y,
            ));
        }
    }

    issues
}

// ---------------------------------------------------------------------------
// check_output_belt_coverage
// ---------------------------------------------------------------------------

/// Check that every machine with solid outputs has an output inserter with a belt.
pub fn check_output_belt_coverage(
    layout: &LayoutResult,
    solver_result: Option<&SolverResult>,
) -> Vec<ValidationIssue> {
    let mut fluid_output_recipes: FxHashSet<String> = FxHashSet::default();
    if let Some(sr) = solver_result {
        for spec in &sr.machines {
            if !spec.outputs.iter().any(|f| !f.is_fluid) {
                fluid_output_recipes.insert(spec.recipe.clone());
            }
        }
    }

    let machine_tiles = build_machine_tile_set(&layout.entities);
    let belt_tiles = build_belt_tile_set(&layout.entities);

    let mut issues = Vec::new();
    let mut checked: FxHashSet<(i32, i32)> = FxHashSet::default();

    for e in &layout.entities {
        if !is_machine(&e.name) || !checked.insert((e.x, e.y)) {
            continue;
        }
        if let Some(recipe) = &e.recipe {
            if fluid_output_recipes.contains(recipe) {
                continue;
            }
        }

        let size = machine_size(&e.name) as i32;
        let my_tiles: FxHashSet<(i32, i32)> =
            (0..size).flat_map(|dx| (0..size).map(move |dy| (e.x + dx, e.y + dy))).collect();

        let has_output_belt = layout.entities.iter().any(|ins| {
            if !is_inserter(&ins.name) {
                return false;
            }
            let dv = dir_to_vec(ins.direction);
            let reach = inserter_reach(&ins.name);
            let pickup = (ins.x - dv.0 * reach, ins.y - dv.1 * reach);
            let drop = (ins.x + dv.0 * reach, ins.y + dv.1 * reach);
            my_tiles.contains(&pickup) && !machine_tiles.contains(&drop) && belt_tiles.contains(&drop)
        });

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
// compute_lane_rates + check_lane_throughput
// ---------------------------------------------------------------------------

/// Classify feeder type for a belt tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FeedType {
    Straight,
    SideloadLeft,
    SideloadRight,
}

/// For each belt tile, classify all upstream feeders and their feed type.
fn classify_belt_feeders(
    belt_dir_map: &FxHashMap<(i32, i32), EntityDirection>,
    ug_output_tiles: &FxHashSet<(i32, i32)>,
) -> FxHashMap<(i32, i32), Vec<((i32, i32), FeedType)>> {
    let mut feeders: FxHashMap<(i32, i32), Vec<((i32, i32), FeedType)>> = FxHashMap::default();

    for (&(bx, by), &belt_d) in belt_dir_map {
        // UG outputs receive rates from the tunnel; their in-degree is handled separately.
        if ug_output_tiles.contains(&(bx, by)) {
            continue;
        }
        let (bdx, bdy) = dir_to_vec(belt_d);
        let (left_dx, left_dy) = (-bdy, bdx);

        let mut tile_feeders: Vec<((i32, i32), FeedType)> = Vec::new();
        for (ddx, ddy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let (nx, ny) = (bx + ddx, by + ddy);
            let nd = match belt_dir_map.get(&(nx, ny)) {
                Some(&d) => d,
                None => continue,
            };
            let (ndx, ndy) = dir_to_vec(nd);
            if (nx + ndx, ny + ndy) != (bx, by) {
                continue;
            }
            if nd == belt_d {
                tile_feeders.push(((nx, ny), FeedType::Straight));
            } else {
                let dot = (nx - bx) * left_dx + (ny - by) * left_dy;
                if dot > 0 {
                    tile_feeders.push(((nx, ny), FeedType::SideloadLeft));
                } else {
                    tile_feeders.push(((nx, ny), FeedType::SideloadRight));
                }
            }
        }
        if !tile_feeders.is_empty() {
            feeders.insert((bx, by), tile_feeders);
        }
    }

    feeders
}

/// Per-lane rates: `{ (x,y): (left_rate, right_rate) }`.
pub type LaneRates = FxHashMap<(i32, i32), (f64, f64)>;

/// Compute per-lane throughput for every surface belt / UG-output tile via
/// topological propagation of output-inserter injection rates.
pub fn compute_lane_rates(layout: &LayoutResult, solver_result: &SolverResult) -> LaneRates {
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
            match e.direction {
                EntityDirection::North | EntityDirection::South => {
                    belt_dir_map.insert((e.x + 1, e.y), e.direction);
                }
                _ => {
                    belt_dir_map.insert((e.x, e.y + 1), e.direction);
                }
            }
        }
    }

    if belt_dir_map.is_empty() {
        return FxHashMap::default();
    }

    let ug_pairs = build_ug_pairs(&layout.entities);
    for (&pos, &other) in &ug_pairs {
        if ug_input_dir.contains_key(&pos) {
            ug_output_to_input.insert(other, pos);
        }
    }

    let machine_tiles = build_machine_tile_set(&layout.entities);
    let mut machine_by_tile: FxHashMap<(i32, i32), (i32, i32)> = FxHashMap::default();
    let mut machine_entity_map: FxHashMap<(i32, i32), &PlacedEntity> = FxHashMap::default();
    for e in &layout.entities {
        if is_machine(&e.name) {
            machine_entity_map.insert((e.x, e.y), e);
            let size = machine_size(&e.name) as i32;
            for dx in 0..size {
                for dy in 0..size {
                    machine_by_tile.insert((e.x + dx, e.y + dy), (e.x, e.y));
                }
            }
        }
    }

    let recipe_to_spec: FxHashMap<&str, &_> =
        solver_result.machines.iter().map(|s| (s.recipe.as_str(), s)).collect();

    let mut belt_carries: FxHashMap<(i32, i32), Option<String>> = FxHashMap::default();
    for e in &layout.entities {
        if is_belt_entity(&e.name) {
            belt_carries.insert((e.x, e.y), e.carries.clone());
            if is_splitter(&e.name) {
                belt_carries.insert(splitter_second_tile(e), e.carries.clone());
            }
        }
    }

    // Seed lane injection rates from output inserters.
    let mut lane_injections: FxHashMap<(i32, i32), (f64, f64)> = FxHashMap::default();
    for ins in &layout.entities {
        if !is_inserter(&ins.name) {
            continue;
        }
        let dv = dir_to_vec(ins.direction);
        let reach = inserter_reach(&ins.name);
        let drop_pos = (ins.x + dv.0 * reach, ins.y + dv.1 * reach);
        let pickup_pos = (ins.x - dv.0 * reach, ins.y - dv.1 * reach);

        if !machine_tiles.contains(&pickup_pos) || !belt_dir_map.contains_key(&drop_pos) {
            continue;
        }
        let mpos = match machine_by_tile.get(&pickup_pos) {
            Some(&p) => p,
            None => continue,
        };
        let me = match machine_entity_map.get(&mpos) {
            Some(&e) => e,
            None => continue,
        };
        let recipe = match &me.recipe {
            Some(r) => r.as_str(),
            None => continue,
        };
        let spec = match recipe_to_spec.get(recipe) {
            Some(&s) => s,
            None => continue,
        };
        let carried_item = match belt_carries.get(&drop_pos).and_then(|c| c.as_deref()) {
            Some(i) => i,
            None => continue,
        };
        let rate = spec.outputs.iter().find(|o| o.item == carried_item).map(|o| o.rate).unwrap_or(0.0);
        if rate <= 0.0 {
            continue;
        }
        let belt_d = belt_dir_map[&drop_pos];
        let lane = inserter_target_lane(ins.x, ins.y, drop_pos.0, drop_pos.1, belt_d);
        let entry = lane_injections.entry(drop_pos).or_insert((0.0, 0.0));
        if lane == "left" { entry.0 += rate; } else { entry.1 += rate; }
    }

    let feeders = classify_belt_feeders(&belt_dir_map, &ug_output_tiles);

    let mut in_degree: FxHashMap<(i32, i32), i32> =
        belt_dir_map.keys().map(|&p| (p, 0)).collect();
    for (pos, tile_feeders) in &feeders {
        in_degree.insert(*pos, tile_feeders.len() as i32);
    }

    let mut lane_rates: FxHashMap<(i32, i32), (f64, f64)> = belt_dir_map
        .keys()
        .map(|&p| (p, lane_injections.get(&p).copied().unwrap_or((0.0, 0.0))))
        .collect();

    let mut splitter_sibling: FxHashMap<(i32, i32), (i32, i32)> = FxHashMap::default();
    for e in &layout.entities {
        if is_splitter(&e.name) {
            match e.direction {
                EntityDirection::North | EntityDirection::South => {
                    splitter_sibling.insert((e.x, e.y), (e.x + 1, e.y));
                    splitter_sibling.insert((e.x + 1, e.y), (e.x, e.y));
                }
                _ => {
                    splitter_sibling.insert((e.x, e.y), (e.x, e.y + 1));
                    splitter_sibling.insert((e.x, e.y + 1), (e.x, e.y));
                }
            }
        }
    }

    let mut queue: VecDeque<(i32, i32)> = in_degree
        .iter()
        .filter(|(_, &deg)| deg == 0)
        .map(|(&p, _)| p)
        .collect();

    let mut processed: FxHashSet<(i32, i32)> = FxHashSet::default();
    let mut splitter_input_ready: FxHashSet<(i32, i32)> = FxHashSet::default();

    while let Some(pos) = queue.pop_front() {
        if processed.contains(&pos) {
            continue;
        }

        // UG output: inherit rates from the belt that feeds the paired UG input.
        if ug_output_tiles.contains(&pos) {
            if let Some(&paired_input) = ug_output_to_input.get(&pos) {
                if let Some(&inp_d) = ug_input_dir.get(&paired_input) {
                    let (idx, idy) = dir_to_vec(inp_d);
                    let behind = (paired_input.0 - idx, paired_input.1 - idy);
                    if belt_dir_map.contains_key(&behind) && !processed.contains(&behind) {
                        queue.push_back(pos);
                        continue;
                    }
                    if let Some(&behind_rates) = lane_rates.get(&behind) {
                        let entry = lane_rates.entry(pos).or_insert((0.0, 0.0));
                        entry.0 += behind_rates.0;
                        entry.1 += behind_rates.1;
                    }
                }
            }
        }

        // Splitter tiles must wait for their sibling before redistributing 50/50.
        if let Some(&sib) = splitter_sibling.get(&pos) {
            if !processed.contains(&sib) {
                splitter_input_ready.insert(pos);
                if !splitter_input_ready.contains(&sib) {
                    continue;
                }
                let pos_rates = lane_rates.get(&pos).copied().unwrap_or((0.0, 0.0));
                let sib_rates = lane_rates.get(&sib).copied().unwrap_or((0.0, 0.0));
                let half_left = (pos_rates.0 + sib_rates.0) / 2.0;
                let half_right = (pos_rates.1 + sib_rates.1) / 2.0;
                lane_rates.insert(pos, (half_left, half_right));
                lane_rates.insert(sib, (half_left, half_right));
                for &tile in &[sib, pos] {
                    processed.insert(tile);
                    do_propagate(tile, &belt_dir_map, &mut lane_rates, &mut in_degree, &mut queue, &feeders);
                }
                continue;
            }
        }

        processed.insert(pos);
        do_propagate(pos, &belt_dir_map, &mut lane_rates, &mut in_degree, &mut queue, &feeders);
    }

    lane_rates
}

/// Propagate one tile's lane rates to its downstream neighbour and decrement in-degree.
fn do_propagate(
    tile: (i32, i32),
    belt_dir_map: &FxHashMap<(i32, i32), EntityDirection>,
    lane_rates: &mut FxHashMap<(i32, i32), (f64, f64)>,
    in_degree: &mut FxHashMap<(i32, i32), i32>,
    queue: &mut VecDeque<(i32, i32)>,
    feeders: &FxHashMap<(i32, i32), Vec<((i32, i32), FeedType)>>,
) {
    let d = match belt_dir_map.get(&tile) {
        Some(&d) => d,
        None => return,
    };
    let (ddx, ddy) = dir_to_vec(d);
    let downstream = (tile.0 + ddx, tile.1 + ddy);

    let downstream_d = match belt_dir_map.get(&downstream) {
        Some(&d) => d,
        None => return,
    };
    let (downstream_dx, downstream_dy) = dir_to_vec(downstream_d);
    let (left_dx, left_dy) = (-downstream_dy, downstream_dx);

    let my_rates = match lane_rates.get(&tile).copied() {
        Some(r) => r,
        None => return,
    };

    let ds_entry = lane_rates.entry(downstream).or_insert((0.0, 0.0));

    if d == downstream_d || (ddx, ddy) == (downstream_dx, downstream_dy) {
        ds_entry.0 += my_rates.0;
        ds_entry.1 += my_rates.1;
    } else {
        let behind_downstream = (downstream.0 - downstream_dx, downstream.1 - downstream_dy);
        if tile == behind_downstream {
            ds_entry.0 += my_rates.0;
            ds_entry.1 += my_rates.1;
        } else {
            let has_straight = feeders
                .get(&downstream)
                .map(|v| v.iter().any(|(_, ft)| *ft == FeedType::Straight))
                .unwrap_or(false);

            if has_straight {
                // Sideload: all items land on the near lane.
                let dot = (tile.0 - downstream.0) * left_dx + (tile.1 - downstream.1) * left_dy;
                let total = my_rates.0 + my_rates.1;
                if dot > 0 { ds_entry.0 += total; } else { ds_entry.1 += total; }
            } else {
                // 90-degree turn: cross > 0 → clockwise (left↔right swap).
                let cross = ddx * downstream_dy - ddy * downstream_dx;
                if cross > 0 {
                    ds_entry.1 += my_rates.0;
                    ds_entry.0 += my_rates.1;
                } else {
                    ds_entry.0 += my_rates.0;
                    ds_entry.1 += my_rates.1;
                }
            }
        }
    }

    let deg = in_degree.entry(downstream).or_insert(0);
    *deg -= 1;
    if *deg <= 0 {
        queue.push_back(downstream);
    }
}

// ---------------------------------------------------------------------------
// check_lane_throughput
// ---------------------------------------------------------------------------

/// Check per-lane belt throughput using full lane simulation.
pub fn check_lane_throughput(
    layout: &LayoutResult,
    solver_result: Option<&SolverResult>,
) -> Vec<ValidationIssue> {
    let sr = match solver_result {
        Some(sr) => sr,
        None => return vec![],
    };

    let lane_rates = compute_lane_rates(layout, sr);
    if lane_rates.is_empty() {
        return vec![];
    }

    let mut belt_name_map: FxHashMap<(i32, i32), &str> = FxHashMap::default();
    for e in &layout.entities {
        if is_surface_belt(&e.name) {
            belt_name_map.insert((e.x, e.y), &e.name);
        } else if is_ug_belt(&e.name) && e.io_type.as_deref() == Some("output") {
            belt_name_map.insert((e.x, e.y), ug_to_surface_tier(&e.name));
        }
    }

    let mut issues = Vec::new();
    for ((x, y), (left, right)) in &lane_rates {
        let belt_name = belt_name_map.get(&(*x, *y)).copied().unwrap_or("transport-belt");
        let cap = lane_capacity(belt_name);
        for (lane_name, rate) in [("left", *left), ("right", *right)] {
            if rate > cap + 0.01 {
                issues.push(ValidationIssue::with_pos(
                    Severity::Error,
                    "lane-throughput",
                    format!(
                        "Belt at ({},{}): {} lane {:.1}/s exceeds {} per-lane capacity {}/s",
                        x, y, lane_name, rate, belt_name, cap
                    ),
                    *x,
                    *y,
                ));
            }
        }
    }

    issues
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ItemFlow, MachineSpec, PlacedEntity, SolverResult};

    fn make_entity(name: &str, x: i32, y: i32, dir: EntityDirection) -> PlacedEntity {
        PlacedEntity {
            name: name.to_string(),
            x,
            y,
            direction: dir,
            recipe: None,
            io_type: None,
            carries: None,
            mirror: false,
            segment_id: None,
        }
    }

    fn belt(x: i32, y: i32, dir: EntityDirection) -> PlacedEntity {
        make_entity("transport-belt", x, y, dir)
    }

    fn belt_carrying(x: i32, y: i32, dir: EntityDirection, item: &str) -> PlacedEntity {
        PlacedEntity { carries: Some(item.to_string()), ..belt(x, y, dir) }
    }

    fn inserter(x: i32, y: i32, dir: EntityDirection) -> PlacedEntity {
        make_entity("inserter", x, y, dir)
    }

    fn inserter_carrying(x: i32, y: i32, dir: EntityDirection, item: &str) -> PlacedEntity {
        PlacedEntity { carries: Some(item.to_string()), ..inserter(x, y, dir) }
    }

    fn machine(name: &str, x: i32, y: i32, recipe: &str) -> PlacedEntity {
        PlacedEntity {
            name: name.to_string(),
            x,
            y,
            direction: EntityDirection::North,
            recipe: Some(recipe.to_string()),
            io_type: None,
            carries: None,
            mirror: false,
            segment_id: None,
        }
    }

    fn layout(entities: Vec<PlacedEntity>) -> LayoutResult {
        LayoutResult { entities, width: 50, height: 50 }
    }

    // -----------------------------------------------------------------------
    // check_belt_loops
    // -----------------------------------------------------------------------

    #[test]
    fn belt_loop_no_loop_passes() {
        let lr = layout(vec![
            belt(0, 0, EntityDirection::East),
            belt(1, 0, EntityDirection::East),
            belt(2, 0, EntityDirection::East),
        ]);
        assert!(check_belt_loops(&lr).is_empty());
    }

    #[test]
    fn belt_loop_square_loop_detected() {
        let lr = layout(vec![
            belt(0, 0, EntityDirection::East),
            belt(1, 0, EntityDirection::South),
            belt(1, 1, EntityDirection::West),
            belt(0, 1, EntityDirection::North),
        ]);
        let issues = check_belt_loops(&lr);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].category, "belt-loop");
        assert_eq!(issues[0].severity, Severity::Error);
    }

    #[test]
    fn belt_loop_reported_once() {
        let lr = layout(vec![
            belt(0, 0, EntityDirection::East),
            belt(1, 0, EntityDirection::South),
            belt(1, 1, EntityDirection::West),
            belt(0, 1, EntityDirection::North),
        ]);
        let issues = check_belt_loops(&lr);
        assert_eq!(issues.len(), 1);
    }

    #[test]
    fn belt_loop_chain_leading_into_loop() {
        let lr = layout(vec![
            belt(-2, 0, EntityDirection::East),
            belt(-1, 0, EntityDirection::East),
            belt(0, 0, EntityDirection::East),
            belt(1, 0, EntityDirection::South),
            belt(1, 1, EntityDirection::West),
            belt(0, 1, EntityDirection::North),
        ]);
        let issues = check_belt_loops(&lr);
        assert_eq!(issues.len(), 1);
    }

    // -----------------------------------------------------------------------
    // check_belt_dead_ends
    // -----------------------------------------------------------------------

    #[test]
    fn dead_end_belt_flowing_off_edge_ok() {
        // Belt at x=49 facing East → output tile (50,0) is off-edge (width=50).
        let lr = layout(vec![
            belt(48, 0, EntityDirection::East),
            belt(49, 0, EntityDirection::East),
        ]);
        assert!(check_belt_dead_ends(&lr).is_empty());
    }

    #[test]
    fn dead_end_belt_into_another_belt_ok() {
        // Belt at (48,0) → (49,0) which has a belt → (50,0) off-edge.
        let lr = layout(vec![
            belt(48, 0, EntityDirection::East),
            belt(49, 0, EntityDirection::East),
        ]);
        assert!(check_belt_dead_ends(&lr).is_empty());
    }

    #[test]
    fn dead_end_isolated_belt_error() {
        let lr = layout(vec![belt(5, 5, EntityDirection::East)]);
        let issues = check_belt_dead_ends(&lr);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].category, "belt-dead-end");
        assert_eq!(issues[0].severity, Severity::Error);
    }

    #[test]
    fn dead_end_belt_into_inserter_pickup_ok() {
        // inserter at (2,0) facing EAST → pickup at (1,0); belt at (0,0) East → (1,0).
        let lr = layout(vec![
            belt(0, 0, EntityDirection::East),
            inserter(2, 0, EntityDirection::East),
        ]);
        assert!(check_belt_dead_ends(&lr).is_empty());
    }

    #[test]
    fn dead_end_belt_into_ug_input_ok() {
        let ug_input = PlacedEntity {
            name: "underground-belt".to_string(),
            x: 1,
            y: 0,
            direction: EntityDirection::East,
            io_type: Some("input".to_string()),
            recipe: None,
            carries: None,
            mirror: false,
            segment_id: None,
        };
        let lr = layout(vec![belt(0, 0, EntityDirection::East), ug_input]);
        assert!(check_belt_dead_ends(&lr).is_empty());
    }

    // -----------------------------------------------------------------------
    // check_belt_item_isolation
    // -----------------------------------------------------------------------

    #[test]
    fn item_isolation_same_item_ok() {
        let lr = layout(vec![
            belt_carrying(0, 0, EntityDirection::East, "iron-plate"),
            belt_carrying(1, 0, EntityDirection::East, "iron-plate"),
        ]);
        assert!(check_belt_item_isolation(&lr).is_empty());
    }

    #[test]
    fn item_isolation_different_items_error() {
        let lr = layout(vec![
            belt_carrying(0, 0, EntityDirection::East, "iron-plate"),
            belt_carrying(1, 0, EntityDirection::East, "copper-plate"),
        ]);
        let issues = check_belt_item_isolation(&lr);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].category, "belt-item-isolation");
        assert_eq!(issues[0].severity, Severity::Error);
    }

    #[test]
    fn item_isolation_ug_input_skipped() {
        let ug_input = PlacedEntity {
            name: "underground-belt".to_string(),
            x: 0,
            y: 0,
            direction: EntityDirection::East,
            io_type: Some("input".to_string()),
            carries: Some("iron-plate".to_string()),
            recipe: None,
            mirror: false,
            segment_id: None,
        };
        let lr = layout(vec![ug_input, belt_carrying(1, 0, EntityDirection::East, "copper-plate")]);
        assert!(check_belt_item_isolation(&lr).is_empty());
    }

    #[test]
    fn item_isolation_no_carry_no_error() {
        let lr = layout(vec![
            belt(0, 0, EntityDirection::East),
            belt_carrying(1, 0, EntityDirection::East, "copper-plate"),
        ]);
        assert!(check_belt_item_isolation(&lr).is_empty());
    }

    // -----------------------------------------------------------------------
    // check_belt_inserter_conflict
    // -----------------------------------------------------------------------

    #[test]
    fn inserter_conflict_same_item_ok() {
        let lr = layout(vec![
            belt(0, 5, EntityDirection::East),
            inserter_carrying(0, 4, EntityDirection::South, "iron-plate"),
            inserter_carrying(0, 6, EntityDirection::North, "iron-plate"),
        ]);
        assert!(check_belt_inserter_conflict(&lr).is_empty());
    }

    #[test]
    fn inserter_conflict_different_items_error() {
        let lr = layout(vec![
            belt(0, 5, EntityDirection::East),
            inserter_carrying(0, 4, EntityDirection::South, "iron-plate"),
            inserter_carrying(0, 6, EntityDirection::North, "copper-plate"),
        ]);
        let issues = check_belt_inserter_conflict(&lr);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].category, "belt-item-isolation");
        assert_eq!(issues[0].severity, Severity::Error);
    }

    #[test]
    fn inserter_conflict_no_carry_ignored() {
        let lr = layout(vec![
            belt(0, 5, EntityDirection::East),
            inserter(0, 4, EntityDirection::South),
            inserter_carrying(0, 6, EntityDirection::North, "iron-plate"),
        ]);
        assert!(check_belt_inserter_conflict(&lr).is_empty());
    }

    // -----------------------------------------------------------------------
    // check_belt_throughput
    // -----------------------------------------------------------------------

    #[test]
    fn throughput_no_overlap_ok() {
        let lr = layout(vec![
            belt(0, 0, EntityDirection::East),
            belt(1, 0, EntityDirection::East),
        ]);
        assert!(check_belt_throughput(&lr).is_empty());
    }

    #[test]
    fn throughput_overlapping_entities_warns() {
        let lr = layout(vec![
            belt(0, 0, EntityDirection::East),
            belt(0, 0, EntityDirection::South),
        ]);
        let issues = check_belt_throughput(&lr);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, Severity::Warning);
        assert_eq!(issues[0].category, "belt-throughput");
        assert!(issues[0].message.contains("2 overlapping"));
    }

    // -----------------------------------------------------------------------
    // check_output_belt_coverage
    // -----------------------------------------------------------------------

    #[test]
    fn output_coverage_machine_with_output_belt_ok() {
        // Machine at (0,0) 3×3; inserter at (1,3) facing South picks from (1,2) inside
        // machine and drops onto belt at (1,4).
        let lr = layout(vec![
            machine("assembling-machine-1", 0, 0, "iron-gear-wheel"),
            inserter(1, 3, EntityDirection::South),
            belt(1, 4, EntityDirection::East),
        ]);
        assert!(check_output_belt_coverage(&lr, None).is_empty());
    }

    #[test]
    fn output_coverage_machine_without_output_belt_error() {
        let lr = layout(vec![machine("assembling-machine-1", 0, 0, "iron-gear-wheel")]);
        let issues = check_output_belt_coverage(&lr, None);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].category, "output-belt");
        assert_eq!(issues[0].severity, Severity::Error);
    }

    #[test]
    fn output_coverage_fluid_output_recipe_skipped() {
        let sr = SolverResult {
            machines: vec![MachineSpec {
                entity: "assembling-machine-3".to_string(),
                recipe: "fluid-recipe".to_string(),
                count: 1.0,
                inputs: vec![],
                outputs: vec![ItemFlow { item: "water".to_string(), rate: 10.0, is_fluid: true }],
            }],
            external_inputs: vec![],
            external_outputs: vec![],
            dependency_order: vec![],
        };
        let lr = layout(vec![machine("assembling-machine-3", 0, 0, "fluid-recipe")]);
        assert!(check_output_belt_coverage(&lr, Some(&sr)).is_empty());
    }

    // -----------------------------------------------------------------------
    // check_lane_throughput
    // -----------------------------------------------------------------------

    #[test]
    fn lane_throughput_no_solver_result_returns_empty() {
        let lr = layout(vec![belt(0, 0, EntityDirection::East)]);
        assert!(check_lane_throughput(&lr, None).is_empty());
    }

    #[test]
    fn lane_throughput_single_inserter_within_capacity() {
        // One inserter at 2.5/s on yellow belt (7.5/s lane cap) — should pass.
        let sr = SolverResult {
            machines: vec![MachineSpec {
                entity: "assembling-machine-3".to_string(),
                recipe: "iron-gear-wheel".to_string(),
                count: 1.0,
                inputs: vec![ItemFlow { item: "iron-plate".to_string(), rate: 5.0, is_fluid: false }],
                outputs: vec![ItemFlow {
                    item: "iron-gear-wheel".to_string(),
                    rate: 2.5,
                    is_fluid: false,
                }],
            }],
            external_inputs: vec![],
            external_outputs: vec![],
            dependency_order: vec![],
        };
        let entities = vec![
            machine("assembling-machine-3", 3, 0, "iron-gear-wheel"),
            // inserter at (4,3) South: pickup at (4,2) inside machine, drop at (4,4)
            inserter(4, 3, EntityDirection::South),
            belt_carrying(4, 4, EntityDirection::East, "iron-gear-wheel"),
            belt_carrying(5, 4, EntityDirection::East, "iron-gear-wheel"),
        ];
        let lr = layout(entities);
        assert!(check_lane_throughput(&lr, Some(&sr)).is_empty());
    }

    #[test]
    fn lane_throughput_same_side_inserters_overload() {
        // Two machines each dropping 5.0/s on the same lane → 10.0/s > 7.5/s cap.
        let sr = SolverResult {
            machines: vec![MachineSpec {
                entity: "assembling-machine-3".to_string(),
                recipe: "iron-gear-wheel".to_string(),
                count: 2.0,
                inputs: vec![ItemFlow { item: "iron-plate".to_string(), rate: 5.0, is_fluid: false }],
                outputs: vec![ItemFlow {
                    item: "iron-gear-wheel".to_string(),
                    rate: 5.0,
                    is_fluid: false,
                }],
            }],
            external_inputs: vec![],
            external_outputs: vec![],
            dependency_order: vec![],
        };
        let mut entities = vec![
            machine("assembling-machine-3", 0, 0, "iron-gear-wheel"),
            machine("assembling-machine-3", 7, 0, "iron-gear-wheel"),
            inserter(1, 3, EntityDirection::South),
            inserter(8, 3, EntityDirection::South),
        ];
        for x in 1..=9 {
            entities.push(belt_carrying(x, 4, EntityDirection::East, "iron-gear-wheel"));
        }
        let lr = layout(entities);
        let issues = check_lane_throughput(&lr, Some(&sr));
        assert!(issues.iter().any(|i| i.category.contains("lane")));
    }
}
