//! Bus layout engine: deterministic row-based factory layout with a main belt bus.
//!
//! Machines are grouped by recipe into rows. Items flow on parallel vertical trunk
//! belts (the "bus"). Each consuming row taps off its required items via underground
//! belt crossings. See `docs/layout-engine-deep-dive.md` for a full description.
//!
//! Entry point: [`layout::build_bus_layout`].
//!
//! Module map:
//! - [`layout`] — orchestrator: rows + bus lanes + poles → `LayoutResult`
//! - [`placer`] — stacks assembly rows vertically in dependency order
//! - [`templates`] — belt/inserter patterns stamped into each row
//! - [`bus_router`] — trunk placement, tap-offs, balancer families, output mergers
//! - [`balancer_library`] — pre-generated N-to-M balancer templates (do not edit manually)
//! - [`tapoff_search`] — brute-force search used only during template generation

pub mod balancer_library;
pub mod bus_router;
pub(crate) mod ghost_occupancy;
pub mod ghost_router;
pub mod layout;
pub mod placer;
pub mod plan;
pub mod tapoff_search;
pub mod templates;
