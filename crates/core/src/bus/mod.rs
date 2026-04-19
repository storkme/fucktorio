//! Bus layout engine: deterministic row-based factory layout with a main belt bus.
//!
//! Machines are grouped by recipe into rows. Items flow on parallel vertical trunk
//! belts (the "bus"). Each consuming row taps off its required items via ghost-
//! routed A* crossings, with template/SAT junction solving for contested tiles.
//! See `docs/ghost-pipeline-contracts.md` for the phase-by-phase invariants.
//!
//! Entry point: [`layout::build_bus_layout`].
//!
//! Module map:
//! - [`layout`] — orchestrator: rows + lane planning + poles + ghost routing → `LayoutResult`
//! - [`placer`] — stacks assembly rows vertically in dependency order
//! - [`templates`] — belt/inserter patterns stamped into each row
//! - [`lane_planner`] — `BusLane` / `LaneFamily` types + `plan_bus_lanes` orchestration
//! - [`lane_order`] — left-to-right lane ordering optimiser
//! - [`balancer`] — N→M balancer block stamping (+ splitter/UG name helpers)
//! - [`balancer_library`] — pre-generated N-to-M balancer templates (do not edit manually)
//! - [`trunk_renderer`] — path → entity rendering, trunk segment slicing
//! - [`output_merger`] — final-product east-flowing output merger
//! - [`tapoff_search`] — brute-force search used only during template generation
//! - [`ghost_router`] — A* + negotiation loop that materialises every connecting belt
//! - [`ghost_occupancy`] — typed obstacle map shared between router phases
//! - [`junction_solver`] — region-growth outer loop for resolving contested crossings
//! - [`junction_sat_strategy`] — SAT-backed `JunctionStrategy` fallback
//! - [`junction`] — `Junction` snapshot type consumed by strategies

pub mod balancer;
pub mod balancer_library;
pub(crate) mod ghost_occupancy;
pub mod ghost_router;
pub(crate) mod junction;
pub mod junction_cost;
pub(crate) mod junction_sat_strategy;
pub(crate) mod junction_solver;
pub(crate) mod region_walker;
pub(crate) mod lane_order;
pub mod lane_planner;
pub mod layout;
pub mod output_merger;
pub mod placer;
pub mod tapoff_search;
pub mod templates;
pub mod trunk_renderer;
