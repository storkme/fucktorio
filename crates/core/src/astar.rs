//! Native A* pathfinding for Fucktorio.
//!
//! Faithful port of `_astar_path` from `src/routing/router.py` — item-aware,
//! grid pathfinding with underground belt support.
//!
//! This is pure Rust with no Python/WASM dependencies so it can be reused
//! across pyo3-bindings (Python) and wasm-bindings (browser).

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use ordered_float::OrderedFloat;
use rustc_hash::{FxHashMap, FxHashSet};

// ---------------------------------------------------------------------------
// Direction constants (matching Python's common.py)
// ---------------------------------------------------------------------------

pub const DIRECTIONS: [(i16, i16); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)]; // N, E, S, W

// EntityDirection values (Factorio 16-way, we use 4)
pub const DIR_NORTH: u8 = 0;
pub const DIR_EAST: u8 = 4;
pub const DIR_SOUTH: u8 = 8;
pub const DIR_WEST: u8 = 12;

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
/// Discount applied to the UG cost when the jump direction is aligned
/// with the lane's `flow_dir`. With 0.3, an aligned UG at ~1.5/tile is
/// cheaper than a surface belt over a turn (turn penalty = 0.5), so the
/// router prefers straight tunnels over meandering surface routes but
/// still picks surface by default on an unobstructed line.
const UG_ALIGNED_DISCOUNT: f32 = 0.3;

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
pub fn astar_inner(
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
    x_constraint: Option<i16>,
    flow_dir: Option<(i8, i8)>,
    goal_on_obstacle: bool,
    y_tolerance: i16,
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

    // Seed open set with all start tiles.
    // When x_constraint is set (vertical trunk routing), allow starting on
    // obstacles — the trunk will immediately go underground.  Surface moves
    // FROM an obstacle tile are still blocked (line 354), so the only option
    // is a UG jump, which is the correct behavior.
    for &(sx, sy) in start_set {
        if obstacles.contains(&(sx, sy)) && x_constraint.is_none() {
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

        // Goal check: normally requires non-forced state (need continuation space).
        // Bus routing (hard_block_perp_ug): allow reaching goal while forced —
        // the template belt beyond the goal provides the continuation.
        if goals.contains(&(cx, cy)) && (forced.is_none() || hard_block_perp_ug) {
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
            // y_constraint: only explore tiles on the constrained row (±tolerance)
            if let Some(yc) = y_constraint {
                if (ny - yc).unsigned_abs() > y_tolerance as u16 {
                    continue;
                }
            }
            // x_constraint: only explore tiles on the constrained column
            if let Some(xc) = x_constraint {
                if nx != xc {
                    continue;
                }
            }
            // Allow reaching goal tiles on obstacles for constrained specs:
            // x_constrained (trunks) need to reach goals promoted by tap-offs,
            // y_constrained (feeders) need to reach goals on trunk tiles.
            // Also allowed when goal_on_obstacle flag is explicitly set (feeders
            // that are unconstrained but still need to reach trunk tile goals).
            // Unconstrained specs (mergers) must not place on obstacle tiles.
            let goal_on_obstacle_ok = (x_constraint.is_some() || y_constraint.is_some() || goal_on_obstacle) && goals.contains(&(nx, ny));
            if obstacles.contains(&(nx, ny)) && !goal_on_obstacle_ok {
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
                    // y_constraint: UG exit must land within tolerance of constrained row
                    if let Some(yc) = y_constraint {
                        if (ey - yc).unsigned_abs() > y_tolerance as u16 {
                            continue;
                        }
                    }
                    // x_constraint: UG exit must land on the constrained column
                    if let Some(xc) = x_constraint {
                        if ex != xc {
                            continue;
                        }
                    }
                    let landing_on_goal = goals.contains(&(ex, ey));
                    // UG exits cannot land on obstacles — even goal tiles.
                    // A UG exit entity would overlap with the obstacle entity.
                    // Surface moves CAN reach goal obstacles (checked separately).
                    if obstacles.contains(&(ex, ey)) {
                        continue;
                    }
                    if landing_on_goal && !hard_block_perp_ug {
                        // Spaghetti: skip UG jumps to goal (need continuation space)
                        continue;
                    }
                    // Bus routing: landing on goal is OK (template belts continue)
                    // Only check continuation obstacle when NOT landing on goal
                    if !landing_on_goal && obstacles.contains(&(ex + dx, ey + dy)) {
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

                    let ug_mult = if let Some((fx, fy)) = flow_dir {
                        let dot = (fx as i32) * (dx as i32) + (fy as i32) * (dy as i32);
                        if dot > 0 {
                            UG_COST_MULTIPLIER * UG_ALIGNED_DISCOUNT
                        } else {
                            UG_COST_MULTIPLIER
                        }
                    } else {
                        UG_COST_MULTIPLIER
                    };
                    let mut new_g =
                        cur_g + (dist as f32) * ug_mult + dev.deviation(ex, ey) * 0.1;

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

/// Convenience: standalone A* with no congestion grid / constraints.
#[allow(clippy::too_many_arguments)]
pub fn astar_path(
    starts: &FxHashSet<(i16, i16)>,
    goals: &FxHashSet<(i16, i16)>,
    obstacles: &FxHashSet<(i16, i16)>,
    max_extent: i16,
    allow_underground: bool,
    ug_max_reach: i16,
    belt_dir_map: &FxHashMap<(i16, i16), u8>,
    other_item_tiles: Option<&FxHashSet<(i16, i16)>>,
) -> Option<Vec<(i16, i16)>> {
    astar_inner(
        starts,
        goals,
        obstacles,
        max_extent,
        allow_underground,
        ug_max_reach,
        belt_dir_map,
        other_item_tiles,
        None,  // no congestion grid for standalone A*
        false, // soft perpendicular UG penalty
        None,  // no y constraint
        None,  // no x constraint
        None,  // no flow direction
        false, // no goal_on_obstacle
        0,     // no y tolerance
    )
}

// ===========================================================================
// Lane-first negotiated congestion routing
// ===========================================================================

/// Specification for a lane to be routed.
#[derive(Clone)]
pub struct LaneSpec {
    pub id: u32,
    pub item_id: u16,
    /// Waypoints the lane must pass through, in order.
    /// For bus: [(x, source_y), (x, sink_y)] for vertical,
    ///          [(x_from, y), (x_to, y)] for horizontal.
    pub waypoints: Vec<(i16, i16)>,
    /// Routing strategy: 0 = axis-aligned (bus), 1 = A* free-form, 2 = bus A* (hard perp block)
    pub strategy: u8,
    /// Higher priority lanes are harder to rip up.
    pub priority: u8,
    /// If set, constrain A* routing to only explore tiles at this y-coordinate.
    /// Used for bus horizontal demands (Phase 1) to prevent vertical detours.
    /// When y_tolerance > 0, the constraint is relaxed to allow ±y_tolerance deviation.
    pub y_constraint: Option<i16>,
    /// If set, constrain A* routing to only explore tiles at this x-coordinate.
    /// Used for bus vertical trunk demands to prevent horizontal detours.
    pub x_constraint: Option<i16>,
    /// When y_constraint is set, allows the A* to deviate ±y_tolerance rows from
    /// the constraint. This lets horizontal feeders route around wide trunk groups
    /// by making short vertical detours. Without tolerance, a y-constrained feeder
    /// can't cross a trunk group wider than the UG reach. Default: 0.
    pub y_tolerance: i16,
    /// Flow direction (dx, dy) of the lane. When set, underground jumps
    /// aligned with this direction get a cost discount so routing prefers
    /// straight tunnels over detours. Derived from waypoints in Python.
    pub flow_dir: Option<(i8, i8)>,
    /// If true, the A* is allowed to reach goal tiles that sit on obstacles.
    /// Used by feeders whose goal is on a trunk tile claimed by a higher-priority spec.
    pub goal_on_obstacle: bool,
}

/// A routed lane: the resolved path through the grid.
#[derive(Clone)]
pub struct RoutedLane {
    pub id: u32,
    pub item_id: u16,
    /// Tile path from source to sink.
    pub path: Vec<(i16, i16)>,
    /// Direction at each path tile (Factorio direction constant).
    pub directions: Vec<u8>,
    /// Tiles where this lane crosses a lane carrying a different item.
    /// These will need underground belt resolution in the renderer.
    pub crossings: Vec<(i16, i16)>,
}

/// Congestion cost grid with PathFinder-style history escalation.
pub struct CongestionGrid {
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
    pub fn new(history_factor: f32, present_factor: f32) -> Self {
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
    pub fn cost_at(&self, x: i16, y: i16) -> f32 {
        let base = self.base_cost.get(&(x, y)).copied().unwrap_or(1.0);
        let history = self.history_cost.get(&(x, y)).copied().unwrap_or(0.0);
        let demand = *self.present_demand.get(&(x, y)).unwrap_or(&0) as f32;
        base + history + demand * self.present_factor
    }

    /// Mark tile as claimed by a lane in the current iteration.
    pub fn claim(&mut self, x: i16, y: i16, item_id: u16) {
        *self.present_demand.entry((x, y)).or_insert(0) += 1;
        self.tile_item.insert((x, y), item_id);
    }

    /// Clear per-iteration state for a new round.
    pub fn release_all(&mut self) {
        self.present_demand.clear();
        self.tile_item.clear();
    }

    /// After an iteration, escalate history on tiles with cross-item conflicts.
    pub fn escalate(&mut self) {
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
    pub fn conflict_count(&self) -> u32 {
        self.present_demand.values().filter(|&&d| d > 1).count() as u32
    }

    /// Set a tile as a fixed obstacle (infinite base cost).
    pub fn set_obstacle(&mut self, x: i16, y: i16) {
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
    _grid: &CongestionGrid,
    _obstacles: &FxHashSet<(i16, i16)>,
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
        spec.x_constraint,
        spec.flow_dir,
        spec.goal_on_obstacle,
        spec.y_tolerance,
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
#[allow(clippy::too_many_arguments)]
pub fn negotiate_lanes(
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
    let mut stall_count: u32 = 0;
    // True once we've seen conflicts decrease from their initial post-first-iteration value.
    // Used to tighten the stall patience after convergence has begun.
    let mut initial_conflicts: Option<u32> = None;
    let mut had_improvement = false;

    // Track tiles promoted to obstacles at priority boundaries (cleared each iteration)
    let mut promoted: Vec<(i16, i16)> = Vec::new();

    for _iteration in 0..max_iterations {
        // Clear promotions from previous iteration
        for &pos in &promoted {
            grid.base_cost.remove(&pos);
        }
        promoted.clear();

        grid.release_all();
        let mut lanes: Vec<RoutedLane> = Vec::with_capacity(specs.len());

        // Route each lane
        // Sort by priority descending — high priority routes first (gets best tiles)
        let mut order: Vec<usize> = (0..specs.len()).collect();
        order.sort_by(|&a, &b| specs[b].priority.cmp(&specs[a].priority));

        let mut current_priority: Option<u8> = None;

        for &idx in &order {
            let spec = &specs[idx];

            // Priority-based hard claims: when we cross a priority boundary,
            // promote all previously-claimed tiles to obstacles.  This ensures
            // lower-priority specs (e.g. trunks) MUST go underground past
            // higher-priority specs (e.g. tap-offs) rather than relying on
            // cost escalation convergence.
            if let Some(prev) = current_priority {
                if spec.priority < prev {
                    let to_promote: Vec<(i16, i16)> = grid.present_demand
                        .iter()
                        .filter(|(_, &d)| d > 0)
                        .map(|(&pos, _)| pos)
                        .filter(|pos| !obstacles.contains(pos))
                        .collect();
                    for pos in to_promote {
                        grid.set_obstacle(pos.0, pos.1);
                        promoted.push(pos);
                    }
                }
            }
            current_priority = Some(spec.priority);

            // Compute a tight per-spec max_extent from the spec's waypoints plus a
            // generous detour buffer.  Unconstrained specs (feeders, bal Z-wraps) only
            // need to explore within their own waypoint bounding box ± detour room, not
            // the full global layout extent. Constrained specs (x_constraint /
            // y_constraint) limit their own search to one axis anyway, so this
            // tightening is most impactful for unconstrained A* calls.
            let spec_max_extent = if spec.x_constraint.is_some() || spec.y_constraint.is_some() {
                // Constrained: the other axis is already unlimited within max_extent; keep it
                max_extent
            } else {
                let wp_max_x = spec.waypoints.iter().map(|&(x, _)| x).max().unwrap_or(0);
                let wp_max_y = spec.waypoints.iter().map(|&(_, y)| y).max().unwrap_or(0);
                // Allow a ±20-tile detour window beyond the waypoint bounding box
                (wp_max_x + 20).max(wp_max_y + 20).min(max_extent)
            };

            let result = match spec.strategy {
                0 => route_axis_aligned(spec, &grid, obstacles),
                1 => route_astar(spec, &grid, obstacles, spec_max_extent, allow_underground, ug_max_reach, false),
                2 => route_astar(spec, &grid, obstacles, spec_max_extent, true, ug_max_reach, true),
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
            stall_count = 0;
            // Track whether we've improved below the initial conflict count
            // (the first iteration always improves from u32::MAX, so we record
            // the initial level on that iteration and only set had_improvement once
            // a subsequent iteration beats it).
            if let Some(init) = initial_conflicts {
                if conflicts < init {
                    had_improvement = true;
                }
            } else {
                // First iteration — record initial conflict level
                initial_conflicts = Some(conflicts);
            }
        } else {
            stall_count += 1;
        }

        if conflicts == 0 {
            break; // converged — no same-tile conflicts
        }

        // Early exit: if conflicts haven't decreased for several consecutive iterations,
        // further iterations are unlikely to help (routing has reached a local minimum).
        // We allow a longer patience before any improvement (to let history build up),
        // but once we've seen at least one improvement, a single stall is enough to stop.
        let stall_limit = if had_improvement { 1 } else { 3 };
        if stall_count >= stall_limit {
            break;
        }

        grid.escalate();
    }

    best_lanes
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn empty_bdm() -> FxHashMap<(i16, i16), u8> {
        FxHashMap::default()
    }

    fn set(coords: &[(i16, i16)]) -> FxHashSet<(i16, i16)> {
        coords.iter().copied().collect()
    }

    fn lane_spec(
        id: u32,
        item_id: u16,
        from: (i16, i16),
        to: (i16, i16),
        strategy: u8,
    ) -> LaneSpec {
        LaneSpec {
            id,
            item_id,
            waypoints: vec![from, to],
            strategy,
            priority: 0,
            y_constraint: None,
            x_constraint: None,
            flow_dir: None,
            goal_on_obstacle: false,
            y_tolerance: 0,
        }
    }

    // -----------------------------------------------------------------------
    // astar_path tests
    // -----------------------------------------------------------------------

    /// Simple straight-line path with no obstacles, east to west.
    #[test]
    fn test_astar_straight_line_horizontal() {
        let starts = set(&[(0, 0)]);
        let goals = set(&[(4, 0)]);
        let obstacles = set(&[]);
        let bdm = empty_bdm();

        let path = astar_path(&starts, &goals, &obstacles, 20, false, 4, &bdm, None);
        assert!(path.is_some(), "expected a path to be found");
        let p = path.unwrap();
        assert_eq!(p[0], (0, 0), "path should start at (0,0)");
        assert_eq!(*p.last().unwrap(), (4, 0), "path should end at (4,0)");
        // All tiles should have y == 0 for a straight horizontal route
        for &(_, y) in &p {
            assert_eq!(y, 0, "horizontal path should stay on row 0");
        }
    }

    /// Simple straight-line path going south (increasing y).
    #[test]
    fn test_astar_straight_line_vertical() {
        let starts = set(&[(2, 0)]);
        let goals = set(&[(2, 5)]);
        let obstacles = set(&[]);
        let bdm = empty_bdm();

        let path = astar_path(&starts, &goals, &obstacles, 20, false, 4, &bdm, None);
        assert!(path.is_some());
        let p = path.unwrap();
        assert_eq!(p[0], (2, 0));
        assert_eq!(*p.last().unwrap(), (2, 5));
        for &(x, _) in &p {
            assert_eq!(x, 2, "vertical path should stay on column 2");
        }
    }

    /// Start == goal: should return a trivial one-tile path.
    #[test]
    fn test_astar_start_equals_goal() {
        let starts = set(&[(3, 3)]);
        let goals = set(&[(3, 3)]);
        let obstacles = set(&[]);
        let bdm = empty_bdm();

        let path = astar_path(&starts, &goals, &obstacles, 20, false, 4, &bdm, None);
        assert!(path.is_some());
        let p = path.unwrap();
        assert_eq!(p, vec![(3, 3)]);
    }

    /// No path: goal is completely surrounded by obstacles.
    #[test]
    fn test_astar_blocked_goal_returns_none() {
        // Goal at (5,5), surrounded on all four sides.
        let starts = set(&[(0, 0)]);
        let goals = set(&[(5, 5)]);
        let obstacles = set(&[(4, 5), (6, 5), (5, 4), (5, 6)]);
        let bdm = empty_bdm();

        let path = astar_path(&starts, &goals, &obstacles, 20, false, 4, &bdm, None);
        assert!(path.is_none(), "goal is surrounded — no path should exist");
    }

    /// No path: goal is walled off on all sides, including the routes through
    /// negative coordinates (which the grid allows down to -10).
    #[test]
    fn test_astar_no_path_all_blocked() {
        // Build a full ring of obstacles around the goal (5,5):
        // a box from (3,3) to (7,7) as the wall, with goal at (5,5).
        let starts = set(&[(0, 0)]);
        let goals = set(&[(5, 5)]);
        let mut wall = Vec::new();
        for x in 3_i16..=7 {
            wall.push((x, 3));
            wall.push((x, 7));
        }
        for y in 4_i16..=6 {
            wall.push((3, y));
            wall.push((7, y));
        }
        let obstacles: FxHashSet<(i16, i16)> = wall.into_iter().collect();
        let bdm = empty_bdm();

        let path = astar_path(&starts, &goals, &obstacles, 20, false, 4, &bdm, None);
        assert!(path.is_none(), "walled-off goal should be unreachable");
    }

    /// Path around a single obstacle: must find L-shaped detour.
    #[test]
    fn test_astar_path_around_obstacle() {
        // Start (0,0) → goal (2,0), obstacle at (1,0) forces a detour via y=1
        let starts = set(&[(0, 0)]);
        let goals = set(&[(2, 0)]);
        let obstacles = set(&[(1, 0)]);
        let bdm = empty_bdm();

        let path = astar_path(&starts, &goals, &obstacles, 10, false, 4, &bdm, None);
        assert!(path.is_some(), "should find a path around the obstacle");
        let p = path.unwrap();
        assert_eq!(p[0], (0, 0));
        assert_eq!(*p.last().unwrap(), (2, 0));
        // The obstacle tile must not appear in the path
        assert!(!p.contains(&(1, 0)), "path must not pass through obstacle");
    }

    /// Underground belt: can reach a goal that is behind a wall of obstacles.
    /// The wall must span the full reachable y range (-10..=max_extent) to
    /// prevent a surface detour going around the ends.
    #[test]
    fn test_astar_underground_crosses_wall() {
        // Use a tight max_extent=5 and wall all of x=1 for y in -10..=5.
        // Start (0,0) → goal (3,0). The wall is one tile wide — underground
        // can jump distance 2 (min for UG) to land at x=2 and continue east.
        let starts = set(&[(0, 0)]);
        let goals = set(&[(3, 0)]);
        let max_extent: i16 = 5;
        let obstacles: FxHashSet<(i16, i16)> = ((-10)..=max_extent)
            .map(|y| (1_i16, y))
            .collect();
        let bdm = empty_bdm();

        // Without underground: should fail (wall covers full y range)
        let no_ug = astar_path(&starts, &goals, &obstacles, max_extent, false, 4, &bdm, None);
        assert!(no_ug.is_none(), "wall should block surface route");

        // With underground: should succeed (jump over the wall)
        let with_ug = astar_path(&starts, &goals, &obstacles, max_extent, true, 4, &bdm, None);
        assert!(with_ug.is_some(), "underground should be able to cross the wall");
        let p = with_ug.unwrap();
        assert_eq!(p[0], (0, 0));
        assert_eq!(*p.last().unwrap(), (3, 0));
    }

    /// Multiple starts: A* accepts a set of start tiles and picks the cheapest.
    #[test]
    fn test_astar_multiple_starts() {
        // Two starts: (0,0) is far, (3,0) is adjacent to goal (4,0).
        let starts = set(&[(0, 0), (3, 0)]);
        let goals = set(&[(4, 0)]);
        let obstacles = set(&[]);
        let bdm = empty_bdm();

        let path = astar_path(&starts, &goals, &obstacles, 20, false, 4, &bdm, None);
        assert!(path.is_some());
        let p = path.unwrap();
        // Nearest start is (3,0), so path length should be short
        assert!(p.len() <= 3, "should pick the closer start");
        assert_eq!(*p.last().unwrap(), (4, 0));
    }

    /// Other-item contamination: tiles belonging to a foreign belt network
    /// should be avoided.
    #[test]
    fn test_astar_other_item_avoidance() {
        // Route from (0,2) to (4,2). A foreign belt network occupies row y=2 at x=1,2,3.
        let starts = set(&[(0, 2)]);
        let goals = set(&[(4, 2)]);
        let obstacles = set(&[]);
        let other_items = set(&[(1, 2), (2, 2), (3, 2)]);
        let bdm = empty_bdm();

        let path = astar_path(&starts, &goals, &obstacles, 10, false, 4, &bdm, Some(&other_items));
        // Path must not pass through other_item tiles
        if let Some(p) = path {
            for tile in &p[1..p.len().saturating_sub(1)] {
                assert!(
                    !other_items.contains(tile),
                    "path should not pass through foreign item tiles, but found {:?}",
                    tile
                );
            }
        }
        // (It's OK if no path found — the point is it doesn't contaminate)
    }

    // -----------------------------------------------------------------------
    // negotiate_lanes tests
    // -----------------------------------------------------------------------

    /// Empty spec list returns empty result.
    #[test]
    fn test_negotiate_empty_specs() {
        let obstacles = set(&[]);
        let result = negotiate_lanes(&[], &obstacles, 5, 20, false, 4, 1.0, 1.0);
        assert!(result.is_empty());
    }

    /// Single axis-aligned lane, no conflicts — should route without crossings.
    #[test]
    fn test_negotiate_single_axis_aligned_no_conflict() {
        let spec = lane_spec(1, 10, (0, 0), (0, 5), 0 /* axis-aligned */);
        let obstacles = set(&[]);
        let result = negotiate_lanes(&[spec], &obstacles, 5, 20, false, 4, 1.0, 1.0);

        assert_eq!(result.len(), 1);
        let lane = &result[0];
        assert_eq!(lane.id, 1);
        assert_eq!(lane.item_id, 10);
        assert!(!lane.path.is_empty(), "lane should have a non-empty path");
        assert!(lane.crossings.is_empty(), "single lane has no crossings");
        assert_eq!(lane.path[0], (0, 0), "path should start at source");
        assert_eq!(*lane.path.last().unwrap(), (0, 5), "path should end at sink");
    }

    /// Single A* lane, no obstacles — should route and return path.
    #[test]
    fn test_negotiate_single_astar_lane() {
        let spec = lane_spec(2, 20, (0, 0), (4, 0), 1 /* A* */);
        let obstacles = set(&[]);
        let result = negotiate_lanes(&[spec], &obstacles, 5, 20, false, 4, 1.0, 1.0);

        assert_eq!(result.len(), 1);
        let lane = &result[0];
        assert!(!lane.path.is_empty());
        assert_eq!(lane.path[0], (0, 0));
        assert_eq!(*lane.path.last().unwrap(), (4, 0));
    }

    /// Two crossing lanes carrying different items — crossings should be detected.
    #[test]
    fn test_negotiate_two_crossing_lanes_detects_crossings() {
        // Lane A: item 1, travels east along y=3 from x=0 to x=6 (axis-aligned)
        // Lane B: item 2, travels south along x=3 from y=0 to y=6 (axis-aligned)
        // They cross at (3, 3).
        let spec_a = LaneSpec {
            id: 1,
            item_id: 1,
            waypoints: vec![(0, 3), (6, 3)],
            strategy: 0,
            priority: 1,
            y_constraint: None,
            x_constraint: None,
            flow_dir: None,
            goal_on_obstacle: false,
            y_tolerance: 0,
        };
        let spec_b = LaneSpec {
            id: 2,
            item_id: 2,
            waypoints: vec![(3, 0), (3, 6)],
            strategy: 0,
            priority: 0,
            x_constraint: None,
            y_constraint: None,
            flow_dir: None,
            goal_on_obstacle: false,
            y_tolerance: 0,
        };
        let obstacles = set(&[]);
        let result =
            negotiate_lanes(&[spec_a, spec_b], &obstacles, 10, 20, true, 4, 1.0, 1.0);

        assert_eq!(result.len(), 2);
        // At least one of the lanes should report a crossing at (3,3)
        let has_crossing = result
            .iter()
            .any(|l| l.crossings.contains(&(3, 3)));
        assert!(
            has_crossing,
            "expected crossing at (3,3) to be detected; crossings: {:?}",
            result.iter().map(|l| &l.crossings).collect::<Vec<_>>()
        );
    }

    /// Two same-item lanes on the same tiles — NOT a crossing (same item).
    #[test]
    fn test_negotiate_same_item_overlap_is_not_crossing() {
        let spec_a = LaneSpec {
            id: 1,
            item_id: 5,
            waypoints: vec![(0, 0), (4, 0)],
            strategy: 0,
            priority: 0,
            y_constraint: None,
            x_constraint: None,
            flow_dir: None,
            goal_on_obstacle: false,
            y_tolerance: 0,
        };
        let spec_b = LaneSpec {
            id: 2,
            item_id: 5, // same item
            waypoints: vec![(0, 0), (4, 0)],
            strategy: 0,
            priority: 0,
            y_constraint: None,
            x_constraint: None,
            flow_dir: None,
            goal_on_obstacle: false,
            y_tolerance: 0,
        };
        let obstacles = set(&[]);
        let result =
            negotiate_lanes(&[spec_a, spec_b], &obstacles, 5, 20, false, 4, 1.0, 1.0);

        assert_eq!(result.len(), 2);
        // No cross-item crossings
        for lane in &result {
            assert!(
                lane.crossings.is_empty(),
                "same-item overlap should not be a crossing, but got: {:?}",
                lane.crossings
            );
        }
    }

    /// Pre-blocked x columns via obstacles force the lane to detour or go underground.
    #[test]
    fn test_negotiate_blocked_column_forces_detour() {
        // A* lane from (0,0) to (4,0). Obstacle wall at x=2, blocking y=0.
        // The lane must route around it.
        let spec = LaneSpec {
            id: 1,
            item_id: 7,
            waypoints: vec![(0, 0), (4, 0)],
            strategy: 1,
            priority: 0,
            y_constraint: None,
            x_constraint: None,
            flow_dir: None,
            goal_on_obstacle: false,
            y_tolerance: 0,
        };
        let obstacles = set(&[(2, -1), (2, 0), (2, 1)]);
        let result =
            negotiate_lanes(&[spec], &obstacles, 5, 20, true, 4, 1.0, 1.0);

        assert_eq!(result.len(), 1);
        let lane = &result[0];
        assert!(
            !lane.path.is_empty(),
            "should find a path around the blocked column"
        );
        // Ensure obstacle tiles are not in the path
        for tile in &lane.path {
            assert!(
                !obstacles.contains(tile),
                "path should not include obstacle tile {:?}",
                tile
            );
        }
    }

    // -----------------------------------------------------------------------
    // CongestionGrid tests
    // -----------------------------------------------------------------------

    /// CongestionGrid cost escalates when tiles are claimed multiple times.
    #[test]
    fn test_congestion_grid_cost_escalation() {
        let mut grid = CongestionGrid::new(2.0, 1.0);

        // Base cost for fresh tile should be 1.0
        assert_eq!(grid.cost_at(0, 0), 1.0);

        // Claim twice by different items
        grid.claim(0, 0, 1);
        grid.claim(0, 0, 2);

        // Present demand = 2, present_factor = 1.0 → cost = 1.0 + 2*1.0 = 3.0
        assert_eq!(grid.cost_at(0, 0), 3.0);

        // After escalation, history should accumulate on contested tile
        grid.escalate();
        grid.release_all();

        // History: (demand=2 - 1) * history_factor=2.0 = 2.0 added
        // No present demand, so cost = 1.0 (base) + 2.0 (history) = 3.0
        assert_eq!(grid.cost_at(0, 0), 3.0);
    }

    /// Obstacle tile has infinite base cost.
    #[test]
    fn test_congestion_grid_obstacle_infinite_cost() {
        let mut grid = CongestionGrid::new(1.0, 1.0);
        grid.set_obstacle(5, 5);
        assert_eq!(grid.cost_at(5, 5), f32::INFINITY);
    }

    /// conflict_count returns the number of contested tiles.
    #[test]
    fn test_congestion_grid_conflict_count() {
        let mut grid = CongestionGrid::new(1.0, 1.0);

        // No conflicts initially
        assert_eq!(grid.conflict_count(), 0);

        // Claim one tile once — still no conflict
        grid.claim(1, 1, 1);
        assert_eq!(grid.conflict_count(), 0);

        // Claim same tile again — now it's a conflict
        grid.claim(1, 1, 2);
        assert_eq!(grid.conflict_count(), 1);

        // Release clears demand
        grid.release_all();
        assert_eq!(grid.conflict_count(), 0);
    }
}
