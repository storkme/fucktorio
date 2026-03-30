//! Native A* pathfinding for Fucktorio.
//!
//! Faithful port of `_astar_path` from `src/routing/router.py` — item-aware,
//! lane-aware grid pathfinding with underground belt support.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use ordered_float::OrderedFloat;
use pyo3::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};

// ---------------------------------------------------------------------------
// Direction constants (matching Python's common.py)
// ---------------------------------------------------------------------------

const DIRECTIONS: [(i16, i16); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)]; // N, E, S, W

// EntityDirection values (Factorio 16-way, we use 4)
const DIR_NORTH: u8 = 0;
const DIR_EAST: u8 = 4;
const DIR_SOUTH: u8 = 8;
const DIR_WEST: u8 = 12;

/// Map EntityDirection u8 → (dx, dy)
fn dir_vec(d: u8) -> Option<(i16, i16)> {
    match d {
        DIR_NORTH => Some((0, -1)),
        DIR_EAST => Some((1, 0)),
        DIR_SOUTH => Some((0, 1)),
        DIR_WEST => Some((-1, 0)),
        _ => None,
    }
}

const UG_COST_MULTIPLIER: f32 = 5.0;

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Forced {
    dx: i8,
    dy: i8,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Lane {
    Left,
    Right,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct State {
    x: i16,
    y: i16,
    forced: Option<Forced>,
    lane: Option<Lane>,
}

// ---------------------------------------------------------------------------
// Priority queue entry (min-heap via reversed Ord)
// ---------------------------------------------------------------------------

struct QEntry {
    f: OrderedFloat<f32>,
    counter: u32,
    state: State,
}

impl PartialEq for QEntry {
    fn eq(&self, other: &Self) -> bool {
        self.f == other.f && self.counter == other.counter
    }
}
impl Eq for QEntry {}
impl PartialOrd for QEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for QEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .f
            .cmp(&self.f)
            .then(other.counter.cmp(&self.counter))
    }
}

// ---------------------------------------------------------------------------
// Heuristic
// ---------------------------------------------------------------------------

#[inline(always)]
fn manhattan(x: i16, y: i16, gx: i16, gy: i16) -> i16 {
    (x - gx).abs() + (y - gy).abs()
}

/// Heuristic: Manhattan distance to nearest goal.
#[inline]
fn h_single(x: i16, y: i16, gx: i16, gy: i16) -> f32 {
    manhattan(x, y, gx, gy) as f32
}

#[inline]
fn h_multi(x: i16, y: i16, goals: &[(i16, i16)]) -> f32 {
    let mut best = i16::MAX;
    for &(gx, gy) in goals {
        let d = manhattan(x, y, gx, gy);
        if d < best {
            best = d;
        }
    }
    best as f32
}

// ---------------------------------------------------------------------------
// Deviation from start→goal line
// ---------------------------------------------------------------------------

struct DeviationLine {
    sx: f32,
    sy: f32,
    line_dx: f32,
    line_dy: f32,
    line_len: f32,
}

impl DeviationLine {
    fn new(sx: i16, sy: i16, goals: &[(i16, i16)]) -> Self {
        let n = goals.len() as f32;
        let gcx: f32 = goals.iter().map(|g| g.0 as f32).sum::<f32>() / n;
        let gcy: f32 = goals.iter().map(|g| g.1 as f32).sum::<f32>() / n;
        let sxf = sx as f32;
        let syf = sy as f32;
        let ldx = gcx - sxf;
        let ldy = gcy - syf;
        let len = (ldx * ldx + ldy * ldy).sqrt().max(1.0);
        DeviationLine {
            sx: sxf,
            sy: syf,
            line_dx: ldx,
            line_dy: ldy,
            line_len: len,
        }
    }

    #[inline(always)]
    fn deviation(&self, x: i16, y: i16) -> f32 {
        let fx = x as f32 - self.sx;
        let fy = y as f32 - self.sy;
        (fx * self.line_dy - fy * self.line_dx).abs() / self.line_len
    }
}

// ---------------------------------------------------------------------------
// Contamination helpers
// ---------------------------------------------------------------------------

/// Check if a foreign belt points AT (tx, ty) from adjacent tile (ax, ay).
#[inline]
fn incoming_contamination(
    tx: i16,
    ty: i16,
    other_item_tiles: &FxHashSet<(i16, i16)>,
    belt_dir_map: &FxHashMap<(i16, i16), u8>,
) -> bool {
    for &(cdx, cdy) in &DIRECTIONS {
        let adj = (tx + cdx, ty + cdy);
        if other_item_tiles.contains(&adj) {
            if let Some(&adj_d) = belt_dir_map.get(&adj) {
                if let Some((dvx, dvy)) = dir_vec(adj_d) {
                    if dvx == -cdx && dvy == -cdy {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Check if any neighbor of (x, y) is in a foreign network (proximity).
#[inline]
fn proximity_check(x: i16, y: i16, other_item_tiles: &FxHashSet<(i16, i16)>) -> bool {
    for &(cdx, cdy) in &DIRECTIONS {
        if other_item_tiles.contains(&(x + cdx, y + cdy)) {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Normalize direction (for underground jumps producing non-unit deltas)
// ---------------------------------------------------------------------------

#[inline(always)]
fn sign(v: i16) -> i16 {
    if v > 0 {
        1
    } else if v < 0 {
        -1
    } else {
        0
    }
}

// ---------------------------------------------------------------------------
// Core A* implementation
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn astar_inner(
    start_set: &FxHashSet<(i16, i16)>,
    goals: &FxHashSet<(i16, i16)>,
    obstacles: &FxHashSet<(i16, i16)>,
    max_extent: i16,
    allow_underground: bool,
    ug_max_reach: i16,
    start_lane: Option<Lane>,
    goal_lane_check: Option<(i16, i16)>,
    belt_dir_map: &FxHashMap<(i16, i16), u8>,
    other_item_tiles: Option<&FxHashSet<(i16, i16)>>,
) -> Option<Vec<(i16, i16)>> {
    if start_set.is_empty() || goals.is_empty() {
        return None;
    }

    // Quick overlap check
    for s in start_set {
        if goals.contains(s) {
            return Some(vec![*s]);
        }
    }

    let goal_list: Vec<(i16, i16)> = goals.iter().copied().collect();
    let single_goal = if goal_list.len() == 1 {
        Some(goal_list[0])
    } else {
        None
    };
    // Deviation line from start centroid to goal center
    let scx = start_set.iter().map(|s| s.0 as f32).sum::<f32>() / start_set.len() as f32;
    let scy = start_set.iter().map(|s| s.1 as f32).sum::<f32>() / start_set.len() as f32;
    let dev = DeviationLine {
        sx: scx,
        sy: scy,
        line_dx: goal_list.iter().map(|g| g.0 as f32).sum::<f32>() / goal_list.len() as f32 - scx,
        line_dy: goal_list.iter().map(|g| g.1 as f32).sum::<f32>() / goal_list.len() as f32 - scy,
        line_len: 1.0, // recomputed below
    };
    let dev = DeviationLine {
        line_len: (dev.line_dx * dev.line_dx + dev.line_dy * dev.line_dy).sqrt().max(1.0),
        ..dev
    };
    let has_belt_dir_map = !belt_dir_map.is_empty();

    // Heuristic dispatch
    let heuristic = |x: i16, y: i16| -> f32 {
        if let Some((gx, gy)) = single_goal {
            h_single(x, y, gx, gy)
        } else {
            h_multi(x, y, &goal_list)
        }
    };

    let mut counter: u32 = 0;
    let mut open_set = BinaryHeap::new();
    let mut g_score: FxHashMap<State, f32> = FxHashMap::default();
    let mut parent: FxHashMap<State, State> = FxHashMap::default();

    // Seed open set with all start tiles
    for &(sx, sy) in start_set {
        if obstacles.contains(&(sx, sy)) {
            continue;
        }
        let initial = State {
            x: sx,
            y: sy,
            forced: None,
            lane: start_lane,
        };
        g_score.insert(initial, 0.0);
        open_set.push(QEntry {
            f: OrderedFloat(heuristic(sx, sy)),
            counter,
            state: initial,
        });
        counter += 1;
    }

    while let Some(QEntry { state, .. }) = open_set.pop() {
        let State {
            x: cx,
            y: cy,
            forced,
            lane,
        } = state;

        // Goal check (only when not forced)
        if goals.contains(&(cx, cy)) && forced.is_none() {
            if let Some((ins_dx, ins_dy)) = goal_lane_check {
                if let Some(cur_lane) = lane {
                    if let Some(&prev_state) = parent.get(&state) {
                        let pdx = sign(cx - prev_state.x);
                        let pdy = sign(cy - prev_state.y);
                        // Belt direction at goal = arrival direction
                        let left_dx = -pdy;
                        let left_dy = pdx;
                        let dot = ins_dx * left_dx + ins_dy * left_dy;
                        let needed = if dot > 0 { Lane::Left } else { Lane::Right };
                        if cur_lane != needed {
                            // Wrong lane — don't accept, keep searching
                            // (fall through to neighbor expansion)
                        } else {
                            return Some(reconstruct(state, &parent));
                        }
                    } else {
                        return Some(reconstruct(state, &parent));
                    }
                } else {
                    return Some(reconstruct(state, &parent));
                }
            } else {
                return Some(reconstruct(state, &parent));
            }
        }

        let cur_g = match g_score.get(&state) {
            Some(&g) => g,
            None => continue,
        };
        // Skip if we've already found a better path to this state
        // (stale entry in the priority queue)
        // This check is important: since we don't decrease-key, we may have
        // multiple entries for the same state; skip all but the best.
        // We can't just compare cur_g directly because of float precision,
        // but the g_score map always has the best known value.

        // --- Forced continuation ---
        if let Some(Forced { dx: fdx, dy: fdy }) = forced {
            let fdx16 = fdx as i16;
            let fdy16 = fdy as i16;
            let nx = cx + fdx16;
            let ny = cy + fdy16;

            // Prevent revisiting this position as normal tile
            let none_state = State {
                x: cx,
                y: cy,
                forced: None,
                lane,
            };
            let none_g = g_score.get(&none_state).copied();
            if none_g.is_none_or(|g| g > cur_g) {
                g_score.insert(none_state, cur_g);
            }

            if nx >= -10 && ny >= -10 && nx <= max_extent && ny <= max_extent
                && !obstacles.contains(&(nx, ny))
            {
                let mut forced_ok = true;
                if let Some(oit) = other_item_tiles {
                    // Outgoing contamination
                    if oit.contains(&(nx + fdx16, ny + fdy16)) {
                        forced_ok = false;
                    } else if has_belt_dir_map {
                        forced_ok = !incoming_contamination(nx, ny, oit, belt_dir_map);
                    }
                }
                if forced_ok {
                    let new_state = State {
                        x: nx,
                        y: ny,
                        forced: None,
                        lane,
                    };
                    let new_g = cur_g + 1.0;
                    let existing = g_score.get(&new_state).copied();
                    if existing.is_none_or(|g| g > new_g) {
                        g_score.insert(new_state, new_g);
                        parent.insert(new_state, state);
                        let f = new_g + heuristic(nx, ny);
                        open_set.push(QEntry {
                            f: OrderedFloat(f),
                            counter,
                            state: new_state,
                        });
                        counter += 1;
                    }
                }
            }
            continue; // No other moves when forced
        }

        // --- Normal surface moves ---
        for &(dx, dy) in &DIRECTIONS {
            let nx = cx + dx;
            let ny = cy + dy;

            if nx < -10 || ny < -10 || nx > max_extent || ny > max_extent {
                continue;
            }
            if obstacles.contains(&(nx, ny)) {
                continue;
            }

            // Item contamination checks
            if let Some(oit) = other_item_tiles {
                // Outgoing: belt at (nx,ny) pointing (dx,dy) → forward tile foreign?
                if oit.contains(&(nx + dx, ny + dy)) {
                    continue;
                }
                // Incoming: foreign belt points AT (nx,ny)?
                if has_belt_dir_map && incoming_contamination(nx, ny, oit, belt_dir_map) {
                    continue;
                }
            }

            let mut new_g = cur_g + 1.0;

            // Proximity penalty
            if let Some(oit) = other_item_tiles {
                if proximity_check(nx, ny, oit) {
                    new_g += 3.0;
                }
            }

            // Turn detection
            let mut is_turn = false;
            if let Some(&prev) = parent.get(&state) {
                let pdx = sign(cx - prev.x);
                let pdy = sign(cy - prev.y);
                if (dx, dy) != (pdx, pdy) {
                    new_g += 0.5;
                    is_turn = true;
                }
            }

            // Deviation penalty
            new_g += dev.deviation(nx, ny) * 0.1;

            // Lane transition logic (matches Python's if/elif exactly)
            let mut new_lane = lane;
            let sideloaded = if has_belt_dir_map {
                if let Some(&existing_dir) = belt_dir_map.get(&(nx, ny)) {
                    if let Some((edx, edy)) = dir_vec(existing_dir) {
                        let dot = dx * edx + dy * edy;
                        if dot == 0 {
                            // Sideload: near lane of receiver
                            let left_dx = -edy;
                            let left_dy = edx;
                            let rel_x = cx - nx;
                            let rel_y = cy - ny;
                            let side_dot = (rel_x as i32) * (left_dx as i32)
                                + (rel_y as i32) * (left_dy as i32);
                            new_lane = Some(if side_dot > 0 {
                                Lane::Left
                            } else {
                                Lane::Right
                            });
                            true
                        } else {
                            // Same/opposite direction — not a sideload
                            true // was in belt_dir_map, so skip turn logic
                        }
                    } else {
                        false
                    }
                } else {
                    false // (nx,ny) not in belt_dir_map
                }
            } else {
                false
            };
            if !sideloaded && is_turn && lane.is_some() {
                // Turn on our own path: lanes swap (left↔right)
                new_lane = Some(match lane.unwrap() {
                    Lane::Left => Lane::Right,
                    Lane::Right => Lane::Left,
                });
            }

            let new_state = State {
                x: nx,
                y: ny,
                forced: None,
                lane: new_lane,
            };

            let existing = g_score.get(&new_state).copied();
            if existing.is_some_and(|g| g <= new_g) {
                continue;
            }

            g_score.insert(new_state, new_g);
            parent.insert(new_state, state);
            let f = new_g + heuristic(nx, ny);
            open_set.push(QEntry {
                f: OrderedFloat(f),
                counter,
                state: new_state,
            });
            counter += 1;
        }

        // --- Underground jumps ---
        if allow_underground {
            for &(dx, dy) in &DIRECTIONS {
                for dist in 2..=(ug_max_reach + 1) {
                    let ex = cx + dx * dist;
                    let ey = cy + dy * dist;
                    if ex < -10 || ey < -10 || ex > max_extent || ey > max_extent {
                        break;
                    }
                    if obstacles.contains(&(ex, ey)) {
                        continue; // exit blocked, try further
                    }
                    if goals.contains(&(ex, ey)) {
                        continue; // don't land on goal
                    }
                    // Tile after exit must be free (for forced continuation)
                    if obstacles.contains(&(ex + dx, ey + dy)) {
                        continue;
                    }

                    // Item contamination at UG exit
                    if let Some(oit) = other_item_tiles {
                        let cont_tile = (ex + dx, ey + dy);
                        let after_cont = (ex + 2 * dx, ey + 2 * dy);
                        if oit.contains(&after_cont) {
                            continue;
                        }
                        if has_belt_dir_map {
                            let mut ug_contam = false;
                            for &tile in &[(ex, ey), cont_tile] {
                                if incoming_contamination(tile.0, tile.1, oit, belt_dir_map) {
                                    ug_contam = true;
                                    break;
                                }
                            }
                            if ug_contam {
                                continue;
                            }
                        }
                    }

                    let mut new_g =
                        cur_g + (dist as f32) * UG_COST_MULTIPLIER + dev.deviation(ex, ey) * 0.1;

                    // Perpendicular entry penalty + lane transition
                    let mut ug_lane = lane;
                    if let Some(&prev) = parent.get(&state) {
                        let pdx = sign(cx - prev.x);
                        let pdy = sign(cy - prev.y);
                        let dot = (pdx as i32) * (dx as i32) + (pdy as i32) * (dy as i32);
                        if dot == 0 {
                            // Perpendicular approach → sideload at UG entry
                            new_g += 10.0;
                            if ug_lane.is_some() {
                                let left_dx = -dy;
                                let left_dy = dx;
                                let rel_x = prev.x - cx;
                                let rel_y = prev.y - cy;
                                let side_dot = (rel_x as i32) * (left_dx as i32)
                                    + (rel_y as i32) * (left_dy as i32);
                                ug_lane = Some(if side_dot > 0 {
                                    Lane::Left
                                } else {
                                    Lane::Right
                                });
                            }
                        }
                    }

                    let new_state = State {
                        x: ex,
                        y: ey,
                        forced: Some(Forced {
                            dx: dx as i8,
                            dy: dy as i8,
                        }),
                        lane: ug_lane,
                    };
                    let existing = g_score.get(&new_state).copied();
                    if existing.is_some_and(|g| g <= new_g) {
                        continue;
                    }

                    g_score.insert(new_state, new_g);
                    parent.insert(new_state, state);
                    let f = new_g + heuristic(ex, ey);
                    open_set.push(QEntry {
                        f: OrderedFloat(f),
                        counter,
                        state: new_state,
                    });
                    counter += 1;
                }
            }
        }
    }

    None // No path found
}

/// Reconstruct path from goal state back to start.
fn reconstruct(goal: State, parent: &FxHashMap<State, State>) -> Vec<(i16, i16)> {
    let mut path = vec![(goal.x, goal.y)];
    let mut cur = goal;
    while let Some(&prev) = parent.get(&cur) {
        path.push((prev.x, prev.y));
        cur = prev;
    }
    path.reverse();
    path
}

// ---------------------------------------------------------------------------
// PyO3 wrapper
// ---------------------------------------------------------------------------

/// Convert Python lane string to Rust enum.
fn parse_lane(s: Option<&str>) -> Option<Lane> {
    match s {
        Some("left") => Some(Lane::Left),
        Some("right") => Some(Lane::Right),
        _ => None,
    }
}

#[pyfunction]
#[pyo3(signature = (
    starts,
    goals,
    obstacles,
    max_extent = 200,
    allow_underground = false,
    ug_max_reach = 4,
    start_lane = None,
    goal_lane_check = None,
    belt_dir_map = None,
    other_item_tiles = None,
))]
fn astar_path(
    starts: Vec<(i16, i16)>,
    goals: Vec<(i16, i16)>,
    obstacles: Vec<(i16, i16)>,
    max_extent: i16,
    allow_underground: bool,
    ug_max_reach: i16,
    start_lane: Option<&str>,
    goal_lane_check: Option<(i16, i16)>,
    belt_dir_map: Option<Vec<((i16, i16), u8)>>,
    other_item_tiles: Option<Vec<(i16, i16)>>,
) -> Option<Vec<(i16, i16)>> {
    let goals_set: FxHashSet<(i16, i16)> = goals.into_iter().collect();
    let obstacles_set: FxHashSet<(i16, i16)> = obstacles.into_iter().collect();
    let starts_set: FxHashSet<(i16, i16)> = starts.into_iter().collect();
    let bdm: FxHashMap<(i16, i16), u8> = belt_dir_map
        .map(|v| v.into_iter().collect())
        .unwrap_or_default();
    let oit: Option<FxHashSet<(i16, i16)>> =
        other_item_tiles.map(|v| v.into_iter().collect());

    astar_inner(
        &starts_set,
        &goals_set,
        &obstacles_set,
        max_extent,
        allow_underground,
        ug_max_reach,
        parse_lane(start_lane),
        goal_lane_check,
        &bdm,
        oit.as_ref(),
    )
}

/// Python module.
#[pymodule]
fn fucktorio_native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(astar_path, m)?)?;
    Ok(())
}
