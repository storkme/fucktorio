//! SAT-based junction strategy — wraps `crate::sat::solve_crossing_zone`
//! over a grown region.
//!
//! Only fires on regions that have grown past the initial single-tile
//! crossing: a 1×1 zone has entry==exit for every spec, which is not a
//! valid `CrossingZone`. Once the growth loop has walked each
//! participating spec's path at least one step outward, the bbox is
//! large enough that the spec entries and exits sit at distinct
//! boundary tiles, and the SAT encoder can route the interior.
//!
//! The mapping is mechanical:
//!
//! - `Junction.bbox`          → `CrossingZone { x, y, width, height }`
//! - `SpecCrossing.entry`     → `ZoneBoundary { ..., is_input: true }`
//! - `SpecCrossing.exit`      → `ZoneBoundary { ..., is_input: false }`
//! - `Junction.forbidden`     → `CrossingZone.forced_empty`
//!
//! Belt tier + UG max reach are picked from the dominant (highest-rank)
//! tier across the participating specs. If the region mixes tiers the
//! SAT solution uses the fastest belts everywhere — fine for
//! correctness, possibly wasteful for throughput-limited downstream
//! checks. Revisit if mixed-tier junctions turn out to be common.

use crate::bus::junction::{BeltTier, Rect};
use crate::bus::junction_solver::{JunctionSolution, JunctionStrategy, JunctionStrategyContext};
use crate::common::ug_max_reach;
use crate::sat::{solve_crossing_zone, CrossingZone, ZoneBoundary};

pub struct SatStrategy;

impl JunctionStrategy for SatStrategy {
    fn name(&self) -> &'static str {
        "sat"
    }

    fn try_solve(&self, ctx: &JunctionStrategyContext) -> Option<JunctionSolution> {
        // SAT cannot solve a 1-tile zone: entry and exit for each spec
        // would collapse to the same tile, which is not a valid
        // `CrossingZone`. Wait for the growth loop to expand the
        // frontier at least once.
        if ctx.region.tile_count() <= 1 {
            return None;
        }
        if ctx.junction.specs.is_empty() {
            return None;
        }

        // Dominant belt tier across participating specs. If a junction
        // carries both yellow and red specs we use red (faster) so the
        // solver has the widest UG reach to work with.
        let belt_tier: BeltTier = ctx
            .junction
            .specs
            .iter()
            .map(|s| s.belt_tier)
            .max_by_key(|t| t.rank())
            .unwrap_or(BeltTier::Yellow);
        let belt_name = belt_tier.belt_name();
        let max_reach = ug_max_reach(belt_name);

        // Two boundaries per spec — one input (entry), one output (exit).
        let mut boundaries: Vec<ZoneBoundary> =
            Vec::with_capacity(ctx.junction.specs.len() * 2);
        for spec in &ctx.junction.specs {
            boundaries.push(ZoneBoundary {
                x: spec.entry.x,
                y: spec.entry.y,
                direction: spec.entry.direction,
                item: spec.item.clone(),
                is_input: true,
            });
            boundaries.push(ZoneBoundary {
                x: spec.exit.x,
                y: spec.exit.y,
                direction: spec.exit.direction,
                item: spec.item.clone(),
                is_input: false,
            });
        }

        let forced_empty: Vec<(i32, i32)> =
            ctx.junction.forbidden.iter().copied().collect();

        let zone = CrossingZone {
            x: ctx.junction.bbox.x,
            y: ctx.junction.bbox.y,
            width: ctx.junction.bbox.w,
            height: ctx.junction.bbox.h,
            boundaries,
            forced_empty,
        };

        let solution = solve_crossing_zone(&zone, max_reach, belt_name)?;

        Some(JunctionSolution {
            entities: solution.entities,
            footprint: Rect {
                x: zone.x,
                y: zone.y,
                w: zone.width,
                h: zone.height,
            },
            strategy_name: self.name(),
        })
    }
}
