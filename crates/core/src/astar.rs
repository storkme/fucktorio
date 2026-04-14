//! Ghost-routing A* pathfinder.
//!
//! One entry point (`ghost_astar`) used by `bus::ghost_router` to route
//! every connecting belt: trunks, tap-offs, returns, feeders. Paths may
//! pass through existing belt tiles (hence "ghost") as long as those
//! tiles are not in the hard-obstacle set. Belt crossings are recorded
//! separately so the junction solver can pick them up in step 6a.
//!
//! A per-tile per-axis cost grid lets the caller (`ghost_router::negotiate_loop`)
//! bump costs between iterations to resolve same-axis conflicts.

use rustc_hash::{FxHashMap, FxHashSet};

/// Route a single belt spec with turn-penalty + per-axis cost A*.
///
/// Returns `Some((path, crossings))` where `path` is the tile sequence
/// from `start` to `goal` (both inclusive, Manhattan-adjacent steps) and
/// `crossings` is the subset of `path` that overlapped `existing_belts`.
/// Returns `None` if no path exists.
///
/// The solver:
/// - Treats `hard_obstacles` as impassable EXCEPT at the goal tile
///   (`goal_on_obstacle` semantics — lets specs terminate at a known
///   existing belt).
/// - Charges `turn_penalty` for every direction change.
/// - Adds the per-axis entry from `axis_cost_grid` for each step (the
///   `.0` component penalises vertical moves, `.1` horizontal).
pub fn ghost_astar(
    start: (i32, i32),
    goal: (i32, i32),
    hard_obstacles: &FxHashSet<(i32, i32)>,
    existing_belts: &FxHashSet<(i32, i32)>,
    width: i32,
    height: i32,
    turn_penalty: u32,
    axis_cost_grid: &FxHashMap<(i32, i32), (u32, u32)>,
) -> Option<(Vec<(i32, i32)>, Vec<(i32, i32)>)> {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;

    #[derive(Clone, Copy, Eq, PartialEq, Hash)]
    struct State {
        x: i32,
        y: i32,
        dir: i8, // -1 = unset, 0=E, 1=S, 2=W, 3=N
    }
    impl Ord for State {
        fn cmp(&self, other: &Self) -> std::cmp::Ordering {
            (self.x, self.y, self.dir).cmp(&(other.x, other.y, other.dir))
        }
    }
    impl PartialOrd for State {
        fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(other))
        }
    }

    const DIRS: [(i32, i32, i8); 4] = [(1, 0, 0), (0, 1, 1), (-1, 0, 2), (0, -1, 3)];
    let h = |x: i32, y: i32| -> u32 { ((x - goal.0).abs() + (y - goal.1).abs()) as u32 };

    let mut heap: BinaryHeap<Reverse<(u32, State)>> = BinaryHeap::new();
    let mut g_cost: FxHashMap<State, u32> = FxHashMap::default();
    let mut parent: FxHashMap<State, State> = FxHashMap::default();

    let start_state = State { x: start.0, y: start.1, dir: -1 };
    heap.push(Reverse((h(start.0, start.1), start_state)));
    g_cost.insert(start_state, 0);

    while let Some(Reverse((_, s))) = heap.pop() {
        if (s.x, s.y) == goal {
            // Reconstruct path
            let mut path = vec![(s.x, s.y)];
            let mut cur = s;
            while let Some(&p) = parent.get(&cur) {
                path.push((p.x, p.y));
                cur = p;
            }
            path.reverse();

            let crossings: Vec<(i32, i32)> = path
                .iter()
                .copied()
                .filter(|t| existing_belts.contains(t))
                .collect();
            return Some((path, crossings));
        }

        let cur_g = *g_cost.get(&s).unwrap_or(&u32::MAX);
        // Skip stale entries
        if let Some(&best) = g_cost.get(&s) {
            if cur_g > best {
                continue;
            }
        }

        for &(dx, dy, dir) in &DIRS {
            let nx = s.x + dx;
            let ny = s.y + dy;
            if nx < 0 || nx >= width || ny < 0 || ny >= height {
                continue;
            }
            // Hard obstacles block, but goal is always reachable (goal_on_obstacle)
            if hard_obstacles.contains(&(nx, ny)) && (nx, ny) != goal {
                continue;
            }
            let mut step: u32 = if s.dir == -1 || s.dir == dir {
                1
            } else {
                1 + turn_penalty
            };
            // Add per-axis negotiation penalty for stepping into this tile.
            // Vertical step (dy != 0) pays the vert penalty; horizontal pays horiz.
            if let Some(&(vp, hp)) = axis_cost_grid.get(&(nx, ny)) {
                if dy != 0 {
                    step += vp;
                } else if dx != 0 {
                    step += hp;
                }
            }
            let ng = cur_g + step;
            let ns = State { x: nx, y: ny, dir };
            if ng < g_cost.get(&ns).copied().unwrap_or(u32::MAX) {
                g_cost.insert(ns, ng);
                parent.insert(ns, s);
                heap.push(Reverse((ng + h(nx, ny), ns)));
            }
        }
    }
    None
}
