//! Native A* pathfinding for Fucktorio.
//!
//! Faithful port of `_astar_path` from `src/routing/router.py` — item-aware,
//! grid pathfinding with underground belt support.

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
// State — (x, y, forced direction for UG exit continuation)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Forced {
    dx: i8,
    dy: i8,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct State {
    x: i16,
    y: i16,
    forced: Option<Forced>,
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
    belt_dir_map: &FxHashMap<(i16, i16), u8>,
    other_item_tiles: Option<&FxHashSet<(i16, i16)>>,
    congestion: Option<&CongestionGrid>,
    hard_block_perp_ug: bool,
    y_constraint: Option<i16>,
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
    let sc = (scx as i16, scy as i16);
    let dev = DeviationLine::new(sc.0, sc.1, &goal_list);

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
        let initial = State { x: sx, y: sy, forced: None };
        g_score.insert(initial, 0.0);
        open_set.push(QEntry {
            f: OrderedFloat(heuristic(sx, sy)),
            counter,
            state: initial,
        });
        counter += 1;
    }

    while let Some(QEntry { state, .. }) = open_set.pop() {
        let State { x: cx, y: cy, forced } = state;

        // Goal check (only when not forced)
        if goals.contains(&(cx, cy)) && forced.is_none() {
            return Some(reconstruct(state, &parent));
        }

        let cur_g = match g_score.get(&state) {
            Some(&g) => g,
            None => continue,
        };

        // --- Forced continuation (UG exit tile) ---
        if let Some(Forced { dx: fdx, dy: fdy }) = forced {
            let fdx16 = fdx as i16;
            let fdy16 = fdy as i16;
            let nx = cx + fdx16;
            let ny = cy + fdy16;

            // Prevent revisiting this position as normal tile
            let none_state = State { x: cx, y: cy, forced: None };
            let none_g = g_score.get(&none_state).copied();
            if none_g.is_none_or(|g| g > cur_g) {
                g_score.insert(none_state, cur_g);
            }

            if nx >= -10 && ny >= -10 && nx <= max_extent && ny <= max_extent
                && !obstacles.contains(&(nx, ny))
            {
                let mut forced_ok = true;
                if let Some(oit) = other_item_tiles {
                    if oit.contains(&(nx + fdx16, ny + fdy16)) {
                        forced_ok = false;
                    } else if has_belt_dir_map {
                        forced_ok = !incoming_contamination(nx, ny, oit, belt_dir_map);
                    }
                }
                if forced_ok {
                    let new_state = State { x: nx, y: ny, forced: None };
                    let new_g = cur_g + 1.0;
                    let existing = g_score.get(&new_state).copied();
                    if existing.is_none_or(|g| g > new_g) {
                        g_score.insert(new_state, new_g);
                        parent.insert(new_state, state);
                        let f = new_g + heuristic(nx, ny);
                        open_set.push(QEntry { f: OrderedFloat(f), counter, state: new_state });
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
            // y_constraint: only explore tiles on the constrained row
            if let Some(yc) = y_constraint {
                if ny != yc {
                    continue;
                }
            }
            if obstacles.contains(&(nx, ny)) {
                continue;
            }

            // Item contamination checks
            if let Some(oit) = other_item_tiles {
                if oit.contains(&(nx + dx, ny + dy)) {
                    continue;
                }
                if has_belt_dir_map && incoming_contamination(nx, ny, oit, belt_dir_map) {
                    continue;
                }
            }

            let mut new_g = cur_g + 1.0;

            // Congestion cost from negotiation grid
            if let Some(cg) = congestion {
                new_g += cg.cost_at(nx, ny) - 1.0;
            }

            // Proximity penalty
            if let Some(oit) = other_item_tiles {
                if proximity_check(nx, ny, oit) {
                    new_g += 3.0;
                }
            }

            // Turn cost
            if let Some(&prev) = parent.get(&state) {
                let pdx = sign(cx - prev.x);
                let pdy = sign(cy - prev.y);
                if (dx, dy) != (pdx, pdy) {
                    new_g += 0.5;
                }
            }

            // Deviation penalty
            new_g += dev.deviation(nx, ny) * 0.1;

            let new_state = State { x: nx, y: ny, forced: None };

            let existing = g_score.get(&new_state).copied();
            if existing.is_some_and(|g| g <= new_g) {
                continue;
            }

            g_score.insert(new_state, new_g);
            parent.insert(new_state, state);
            let f = new_g + heuristic(nx, ny);
            open_set.push(QEntry { f: OrderedFloat(f), counter, state: new_state });
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
                    // y_constraint: UG exit must land on the constrained row
                    if let Some(yc) = y_constraint {
                        if ey != yc {
                            continue;
                        }
                    }
                    if obstacles.contains(&(ex, ey)) {
                        continue;
                    }
                    if goals.contains(&(ex, ey)) {
                        continue;
                    }
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

                    // Congestion cost at UG exit
                    if let Some(cg) = congestion {
                        new_g += cg.cost_at(ex, ey) - 1.0;
                    }

                    // Perpendicular entry: hard block or soft penalty
                    if let Some(&prev) = parent.get(&state) {
                        let pdx = sign(cx - prev.x);
                        let pdy = sign(cy - prev.y);
                        let dot = (pdx as i32) * (dx as i32) + (pdy as i32) * (dy as i32);
                        if dot == 0 {
                            if hard_block_perp_ug {
                                continue; // sideloading onto UG input only loads far lane
                            }
                            new_g += 10.0;
                        }
                    }

                    let new_state = State {
                        x: ex,
                        y: ey,
                        forced: Some(Forced { dx: dx as i8, dy: dy as i8 }),
                    };
                    let existing = g_score.get(&new_state).copied();
                    if existing.is_some_and(|g| g <= new_g) {
                        continue;
                    }

                    g_score.insert(new_state, new_g);
                    parent.insert(new_state, state);
                    let f = new_g + heuristic(ex, ey);
                    open_set.push(QEntry { f: OrderedFloat(f), counter, state: new_state });
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

#[pyfunction]
#[pyo3(signature = (
    starts,
    goals,
    obstacles,
    max_extent = 200,
    allow_underground = false,
    ug_max_reach = 4,
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
        &bdm,
        oit.as_ref(),
        None,   // no congestion grid for standalone A*
        false,  // soft perpendicular UG penalty
        None,   // no y constraint
    )
}

// ===========================================================================
// Lane-first negotiated congestion routing
// ===========================================================================

/// Specification for a lane to be routed.
#[derive(Clone)]
struct LaneSpec {
    id: u32,
    item_id: u16,
    /// Waypoints the lane must pass through, in order.
    /// For bus: [(x, source_y), (x, sink_y)] for vertical,
    ///          [(x_from, y), (x_to, y)] for horizontal.
    waypoints: Vec<(i16, i16)>,
    /// Routing strategy: 0 = axis-aligned (bus), 1 = A* free-form, 2 = bus A* (hard perp block)
    strategy: u8,
    /// Higher priority lanes are harder to rip up.
    priority: u8,
    /// If set, constrain A* routing to only explore tiles at this y-coordinate.
    /// Used for bus horizontal demands (Phase 1) to prevent vertical detours.
    y_constraint: Option<i16>,
}

/// A routed lane: the resolved path through the grid.
#[derive(Clone)]
struct RoutedLane {
    id: u32,
    item_id: u16,
    /// Tile path from source to sink.
    path: Vec<(i16, i16)>,
    /// Direction at each path tile (Factorio direction constant).
    directions: Vec<u8>,
    /// Tiles where this lane crosses a lane carrying a different item.
    /// These will need underground belt resolution in the renderer.
    crossings: Vec<(i16, i16)>,
}

/// Congestion cost grid with PathFinder-style history escalation.
struct CongestionGrid {
    /// Base cost per tile (e.g. obstacles get f32::INFINITY).
    base_cost: FxHashMap<(i16, i16), f32>,
    /// Accumulated history cost from past iterations.
    history_cost: FxHashMap<(i16, i16), f32>,
    /// Number of lanes claiming this tile in the current iteration.
    present_demand: FxHashMap<(i16, i16), u16>,
    /// Which item_id occupies each tile in the current iteration.
    /// Used to distinguish same-item overlap (OK) from cross-item conflict.
    tile_item: FxHashMap<(i16, i16), u16>,
    /// Multiplier for history accumulation.
    history_factor: f32,
    /// Cost penalty per additional demand on a tile.
    present_factor: f32,
}

impl CongestionGrid {
    fn new(history_factor: f32, present_factor: f32) -> Self {
        CongestionGrid {
            base_cost: FxHashMap::default(),
            history_cost: FxHashMap::default(),
            present_demand: FxHashMap::default(),
            tile_item: FxHashMap::default(),
            history_factor,
            present_factor,
        }
    }

    /// Total cost of using tile (x, y).
    #[inline]
    fn cost_at(&self, x: i16, y: i16) -> f32 {
        let base = self.base_cost.get(&(x, y)).copied().unwrap_or(1.0);
        let history = self.history_cost.get(&(x, y)).copied().unwrap_or(0.0);
        let demand = *self.present_demand.get(&(x, y)).unwrap_or(&0) as f32;
        base + history + demand * self.present_factor
    }

    /// Mark tile as claimed by a lane in the current iteration.
    fn claim(&mut self, x: i16, y: i16, item_id: u16) {
        *self.present_demand.entry((x, y)).or_insert(0) += 1;
        self.tile_item.insert((x, y), item_id);
    }

    /// Clear per-iteration state for a new round.
    fn release_all(&mut self) {
        self.present_demand.clear();
        self.tile_item.clear();
    }

    /// After an iteration, escalate history on tiles with cross-item conflicts.
    fn escalate(&mut self) {
        // Conflicts are tiles with demand > 1 where different items compete.
        // We track this via the demand count — same-item overlaps are less
        // of a concern (lanes carrying the same item can share a belt).
        for (&pos, &demand) in &self.present_demand {
            if demand > 1 {
                let h = self.history_cost.entry(pos).or_insert(0.0);
                *h += self.history_factor * (demand - 1) as f32;
            }
        }
    }

    /// Count tiles with demand > 1 (conflicts).
    fn conflict_count(&self) -> u32 {
        self.present_demand.values().filter(|&&d| d > 1).count() as u32
    }

    /// Set a tile as a fixed obstacle (infinite base cost).
    fn set_obstacle(&mut self, x: i16, y: i16) {
        self.base_cost.insert((x, y), f32::INFINITY);
    }
}

// ---------------------------------------------------------------------------
// Axis-aligned lane routing (for bus layout)
// ---------------------------------------------------------------------------

/// Direction constant for a movement vector.
fn vec_to_dir(dx: i16, dy: i16) -> u8 {
    match (dx.signum(), dy.signum()) {
        (0, -1) => DIR_NORTH,
        (1, 0) => DIR_EAST,
        (0, 1) => DIR_SOUTH,
        (-1, 0) => DIR_WEST,
        _ => DIR_SOUTH, // default
    }
}

/// Route an axis-aligned path between waypoints.
/// The path goes horizontal then vertical (or vice versa) between each
/// pair of waypoints, choosing whichever direction is needed.
fn route_axis_aligned(
    spec: &LaneSpec,
    grid: &CongestionGrid,
    obstacles: &FxHashSet<(i16, i16)>,
) -> Option<(Vec<(i16, i16)>, Vec<u8>)> {
    if spec.waypoints.len() < 2 {
        return None;
    }

    let mut path: Vec<(i16, i16)> = Vec::new();
    let mut dirs: Vec<u8> = Vec::new();

    for seg in spec.waypoints.windows(2) {
        let (sx, sy) = seg[0];
        let (ex, ey) = seg[1];

        // Generate tiles for this segment.
        // Route vertical first, then horizontal (standard bus pattern).
        let mut cur_x = sx;
        let mut cur_y = sy;

        // Skip first tile if it was already added by previous segment
        let skip_first = !path.is_empty() && path.last() == Some(&(sx, sy));

        // Vertical segment
        if cur_y != ey {
            let dy: i16 = if ey > cur_y { 1 } else { -1 };
            let dir = vec_to_dir(0, dy);
            while cur_y != ey {
                if !skip_first || (cur_x, cur_y) != (sx, sy) {
                    path.push((cur_x, cur_y));
                    dirs.push(dir);
                }
                cur_y += dy;
            }
        }

        // Horizontal segment
        if cur_x != ex {
            let dx: i16 = if ex > cur_x { 1 } else { -1 };
            let dir = vec_to_dir(dx, 0);
            while cur_x != ex {
                if !skip_first || (cur_x, cur_y) != (sx, sy) {
                    path.push((cur_x, cur_y));
                    dirs.push(dir);
                }
                cur_x += dx;
            }
        }

        // Add final tile
        if !path.is_empty() && path.last() != Some(&(ex, ey)) {
            // Direction continues from last segment
            let last_dir = dirs.last().copied().unwrap_or(DIR_SOUTH);
            path.push((ex, ey));
            dirs.push(last_dir);
        } else if path.is_empty() {
            path.push((ex, ey));
            dirs.push(DIR_SOUTH);
        }
    }

    // Add the start tile if missing
    if path.is_empty() || path[0] != spec.waypoints[0] {
        let wp0 = spec.waypoints[0];
        path.insert(0, wp0);
        dirs.insert(0, dirs.first().copied().unwrap_or(DIR_SOUTH));
    }

    Some((path, dirs))
}

/// Route a lane using the cost-aware A* (for spaghetti / free-form).
fn route_astar(
    spec: &LaneSpec,
    grid: &CongestionGrid,
    obstacles: &FxHashSet<(i16, i16)>,
    max_extent: i16,
    allow_underground: bool,
    ug_max_reach: i16,
    hard_block_perp_ug: bool,
) -> Option<(Vec<(i16, i16)>, Vec<u8>)> {
    if spec.waypoints.len() < 2 {
        return None;
    }

    let start = spec.waypoints[0];
    let goal = *spec.waypoints.last().unwrap();

    let starts_set: FxHashSet<(i16, i16)> = [start].into_iter().collect();
    let goals_set: FxHashSet<(i16, i16)> = [goal].into_iter().collect();

    // Build obstacle set: fixed obstacles + high-cost tiles from grid
    let mut obs = obstacles.clone();
    for (&pos, &cost) in &grid.base_cost {
        if cost >= f32::INFINITY {
            obs.insert(pos);
        }
    }

    let bdm = FxHashMap::default();
    let path = astar_inner(
        &starts_set,
        &goals_set,
        &obs,
        max_extent,
        allow_underground,
        ug_max_reach,
        &bdm,
        None,
        Some(grid),
        hard_block_perp_ug,
        spec.y_constraint,
    )?;

    // Compute directions from path
    let mut dirs = Vec::with_capacity(path.len());
    for i in 0..path.len() {
        if i + 1 < path.len() {
            let dx = (path[i + 1].0 - path[i].0).signum();
            let dy = (path[i + 1].1 - path[i].1).signum();
            dirs.push(vec_to_dir(dx, dy));
        } else if i > 0 {
            dirs.push(*dirs.last().unwrap_or(&DIR_SOUTH));
        } else {
            dirs.push(DIR_SOUTH);
        }
    }

    Some((path, dirs))
}

// ---------------------------------------------------------------------------
// Negotiation loop
// ---------------------------------------------------------------------------

/// Detect crossings: tiles where lanes carrying different items overlap.
fn find_crossings(lanes: &[RoutedLane]) -> Vec<(i16, i16, u32, u32)> {
    // Build tile → (lane_id, item_id) map
    let mut tile_owners: FxHashMap<(i16, i16), Vec<(u32, u16)>> = FxHashMap::default();
    for lane in lanes {
        for &pos in &lane.path {
            tile_owners.entry(pos).or_default().push((lane.id, lane.item_id));
        }
    }

    let mut crossings = Vec::new();
    for (&pos, owners) in &tile_owners {
        if owners.len() > 1 {
            // Check if different items are present
            let first_item = owners[0].1;
            for &(lane_id, item_id) in &owners[1..] {
                if item_id != first_item {
                    crossings.push((pos.0, pos.1, owners[0].0, lane_id));
                }
            }
        }
    }
    crossings
}

/// Run the PathFinder negotiation loop.
fn negotiate_inner(
    specs: &[LaneSpec],
    obstacles: &FxHashSet<(i16, i16)>,
    max_iterations: u16,
    max_extent: i16,
    allow_underground: bool,
    ug_max_reach: i16,
    history_factor: f32,
    present_factor: f32,
) -> Vec<RoutedLane> {
    let mut grid = CongestionGrid::new(history_factor, present_factor);

    // Seed grid with fixed obstacles
    for &(x, y) in obstacles {
        grid.set_obstacle(x, y);
    }

    let mut best_lanes: Vec<RoutedLane> = Vec::new();
    let mut best_conflicts = u32::MAX;

    for _iteration in 0..max_iterations {
        grid.release_all();
        let mut lanes: Vec<RoutedLane> = Vec::with_capacity(specs.len());

        // Route each lane
        // Sort by priority descending — high priority routes first (gets best tiles)
        let mut order: Vec<usize> = (0..specs.len()).collect();
        order.sort_by(|&a, &b| specs[b].priority.cmp(&specs[a].priority));

        for &idx in &order {
            let spec = &specs[idx];
            let result = match spec.strategy {
                0 => route_axis_aligned(spec, &grid, obstacles),
                1 => route_astar(spec, &grid, obstacles, max_extent, allow_underground, ug_max_reach, false),
                2 => route_astar(spec, &grid, obstacles, max_extent, true, ug_max_reach, true),
                _ => None,
            };

            if let Some((path, directions)) = result {
                // Claim tiles in the grid
                for &pos in &path {
                    grid.claim(pos.0, pos.1, spec.item_id);
                }

                lanes.push(RoutedLane {
                    id: spec.id,
                    item_id: spec.item_id,
                    path,
                    directions,
                    crossings: Vec::new(), // filled after all lanes routed
                });
            } else {
                // Failed to route — add empty lane
                lanes.push(RoutedLane {
                    id: spec.id,
                    item_id: spec.item_id,
                    path: Vec::new(),
                    directions: Vec::new(),
                    crossings: Vec::new(),
                });
            }
        }

        // Detect crossings and mark them on lanes
        let crossing_list = find_crossings(&lanes);
        for lane in &mut lanes {
            lane.crossings.clear();
        }
        let lane_by_id: FxHashMap<u32, usize> = lanes.iter().enumerate()
            .map(|(i, l)| (l.id, i))
            .collect();
        for &(x, y, id1, id2) in &crossing_list {
            if let Some(&idx) = lane_by_id.get(&id1) {
                lanes[idx].crossings.push((x, y));
            }
            if let Some(&idx) = lane_by_id.get(&id2) {
                lanes[idx].crossings.push((x, y));
            }
        }

        let conflicts = grid.conflict_count();
        if conflicts < best_conflicts {
            best_conflicts = conflicts;
            best_lanes = lanes.clone();
        }

        if conflicts == 0 {
            break; // converged — no same-tile conflicts
        }

        grid.escalate();
    }

    best_lanes
}

// ---------------------------------------------------------------------------
// PyO3 wrappers for negotiation
// ---------------------------------------------------------------------------

/// Python-facing lane specification.
#[pyclass]
#[derive(Clone)]
struct PyLaneSpec {
    #[pyo3(get, set)]
    id: u32,
    #[pyo3(get, set)]
    item_id: u16,
    #[pyo3(get, set)]
    waypoints: Vec<(i16, i16)>,
    #[pyo3(get, set)]
    strategy: u8,
    #[pyo3(get, set)]
    priority: u8,
    #[pyo3(get, set)]
    y_constraint: Option<i16>,
}

#[pymethods]
impl PyLaneSpec {
    #[new]
    #[pyo3(signature = (id, item_id, waypoints, strategy = 0, priority = 0, y_constraint = None))]
    fn new(id: u32, item_id: u16, waypoints: Vec<(i16, i16)>, strategy: u8, priority: u8, y_constraint: Option<i16>) -> Self {
        PyLaneSpec { id, item_id, waypoints, strategy, priority, y_constraint }
    }
}

/// Python-facing routed lane result.
#[pyclass]
#[derive(Clone)]
struct PyRoutedLane {
    #[pyo3(get)]
    id: u32,
    #[pyo3(get)]
    item_id: u16,
    #[pyo3(get)]
    path: Vec<(i16, i16)>,
    #[pyo3(get)]
    directions: Vec<u8>,
    #[pyo3(get)]
    crossings: Vec<(i16, i16)>,
}

#[pyfunction]
#[pyo3(signature = (
    lane_specs,
    obstacles,
    max_extent = 200,
    max_iterations = 10,
    allow_underground = false,
    ug_max_reach = 4,
    history_factor = 1.0,
    present_factor = 1.5,
))]
fn negotiate_lanes(
    lane_specs: Vec<PyLaneSpec>,
    obstacles: Vec<(i16, i16)>,
    max_extent: i16,
    max_iterations: u16,
    allow_underground: bool,
    ug_max_reach: i16,
    history_factor: f32,
    present_factor: f32,
) -> Vec<PyRoutedLane> {
    let specs: Vec<LaneSpec> = lane_specs.iter().map(|ps| LaneSpec {
        id: ps.id,
        item_id: ps.item_id,
        waypoints: ps.waypoints.clone(),
        strategy: ps.strategy,
        priority: ps.priority,
        y_constraint: ps.y_constraint,
    }).collect();

    let obs: FxHashSet<(i16, i16)> = obstacles.into_iter().collect();

    let routed = negotiate_inner(
        &specs, &obs, max_iterations, max_extent,
        allow_underground, ug_max_reach, history_factor, present_factor,
    );

    routed.into_iter().map(|r| PyRoutedLane {
        id: r.id,
        item_id: r.item_id,
        path: r.path,
        directions: r.directions,
        crossings: r.crossings,
    }).collect()
}

/// Python module.
#[pymodule]
fn fucktorio_native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(astar_path, m)?)?;
    m.add_function(wrap_pyfunction!(negotiate_lanes, m)?)?;
    m.add_class::<PyLaneSpec>()?;
    m.add_class::<PyRoutedLane>()?;
    Ok(())
}
