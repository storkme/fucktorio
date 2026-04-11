#![allow(clippy::too_many_arguments)]
//! PyO3 bindings for Fucktorio's native A* pathfinder.
//!
//! Thin adapter around `fucktorio_core::astar` — converts PyO3 arguments
//! into core types, calls the pure-Rust implementation, and converts
//! results back to Python.

use fucktorio_core::astar::{self, LaneSpec, RoutedLane};
use pyo3::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};

// ---------------------------------------------------------------------------
// Standalone A*
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

    astar::astar_path(
        &starts_set,
        &goals_set,
        &obstacles_set,
        max_extent,
        allow_underground,
        ug_max_reach,
        &bdm,
        oit.as_ref(),
    )
}

// ---------------------------------------------------------------------------
// Lane negotiation wrappers
// ---------------------------------------------------------------------------

/// Python-facing lane specification for the A* multi-lane negotiator.
///
/// Each `PyLaneSpec` describes one lane that needs routing: a unique ID, the
/// item it carries, waypoint sequence, and routing constraints. Passed from
/// Python into the Rust A* solver via [`crate::astar::negotiate_lanes`].
#[pyclass]
#[derive(Clone)]
struct PyLaneSpec {
    /// Unique lane identifier.
    #[pyo3(get, set)]
    id: u32,
    /// Item type index — distinguishes lanes carrying different items so the
    /// negotiator can enforce item separation.
    #[pyo3(get, set)]
    item_id: u16,
    /// Ordered waypoints the lane path must visit. The A* solver routes
    /// sequentially between consecutive waypoint pairs.
    #[pyo3(get, set)]
    waypoints: Vec<(i16, i16)>,
    /// Routing strategy selector (reserved for future use; always 0 currently).
    #[pyo3(get, set)]
    strategy: u8,
    /// Lane priority — higher values are routed first, giving them preferential
    /// access to shared corridor space.
    #[pyo3(get, set)]
    priority: u8,
    /// If set, constrains the lane's path to stay near this Y-coordinate
    /// (useful for keeping lanes horizontally aligned).
    #[pyo3(get, set)]
    y_constraint: Option<i16>,
    /// If set, constrains the lane's path to stay near this X-coordinate.
    #[pyo3(get, set)]
    x_constraint: Option<i16>,
    /// Preferred flow direction as `(dx, dy)` unit vector (e.g. `(0, 1)` for
    /// southbound). Guides the A* heuristic.
    #[pyo3(get, set)]
    flow_dir: Option<(i8, i8)>,
    /// When `true`, allows the lane's goal tile to overlap an obstacle tile.
    /// Used for edge cases where the destination is occupied but reachable.
    #[pyo3(get, set)]
    goal_on_obstacle: bool,
    /// Allowed vertical deviation from `y_constraint` (0 = exact, larger =
    /// more slack).
    #[pyo3(get, set)]
    y_tolerance: i16,
}

#[pymethods]
impl PyLaneSpec {
    #[new]
    #[pyo3(signature = (id, item_id, waypoints, strategy = 0, priority = 0, y_constraint = None, x_constraint = None, flow_dir = None, goal_on_obstacle = false, y_tolerance = 0))]
    fn new(
        id: u32,
        item_id: u16,
        waypoints: Vec<(i16, i16)>,
        strategy: u8,
        priority: u8,
        y_constraint: Option<i16>,
        x_constraint: Option<i16>,
        flow_dir: Option<(i8, i8)>,
        goal_on_obstacle: bool,
        y_tolerance: i16,
    ) -> Self {
        PyLaneSpec { id, item_id, waypoints, strategy, priority, y_constraint, x_constraint, flow_dir, goal_on_obstacle, y_tolerance }
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
        x_constraint: ps.x_constraint,
        flow_dir: ps.flow_dir,
        goal_on_obstacle: ps.goal_on_obstacle,
        y_tolerance: ps.y_tolerance,
        respect_extra_obstacles: false,

        own_trunk_x: None,


        forbid_ug_exit_to_goal: false,
    }).collect();

    let obs: FxHashSet<(i16, i16)> = obstacles.into_iter().collect();
    let extras: FxHashSet<(i16, i16)> = FxHashSet::default();

    let routed: Vec<RoutedLane> = astar::negotiate_lanes(
        &specs, &obs, &extras, max_iterations, max_extent,
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
