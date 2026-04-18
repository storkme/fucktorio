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

use rustc_hash::FxHashSet;

use crate::bus::junction::{BeltTier, Rect};
use crate::bus::junction_solver::{JunctionSolution, JunctionStrategy, JunctionStrategyContext};
use crate::common::{is_splitter, is_surface_belt, is_ug_belt, splitter_second_tile, ug_max_reach};
use crate::models::{EntityDirection, PlacedEntity};
use crate::sat::{solve_crossing_zone_with_stats, CrossingZone, ZoneBoundary};
use crate::trace::{self, BoundarySnapshot, ExternalFeederSnapshot, TraceEvent};

/// A feeder/consumer tile candidate found adjacent to a spec entry/exit.
struct FeederHit {
    /// The tile of the Permanent entity that physically interacts with
    /// the boundary (for splitters, the specific one of two tiles).
    entity_tile: (i32, i32),
    /// The Permanent entity's facing direction.
    entity_direction: EntityDirection,
}

pub struct SatStrategy;

/// Direction vector for N/E/S/W.
fn dir_delta(d: EntityDirection) -> (i32, i32) {
    match d {
        EntityDirection::North => (0, -1),
        EntityDirection::East => (1, 0),
        EntityDirection::South => (0, 1),
        EntityDirection::West => (-1, 0),
    }
}

/// Human-readable direction label for trace events.
fn dir_label(d: EntityDirection) -> String {
    match d {
        EntityDirection::North => "North",
        EntityDirection::East => "East",
        EntityDirection::South => "South",
        EntityDirection::West => "West",
    }
    .to_string()
}

/// Find any entity in `placed` whose output lands on `tile`, for use in
/// BoundarySnapshot.external_feeder.
fn find_external_feeder(
    tile: (i32, i32),
    placed: &[PlacedEntity],
) -> Option<ExternalFeederSnapshot> {
    for e in placed {
        if is_ug_belt(&e.name) && e.io_type.as_deref() == Some("input") {
            continue;
        }
        let emits = is_surface_belt(&e.name)
            || is_splitter(&e.name)
            || (is_ug_belt(&e.name) && e.io_type.as_deref() == Some("output"));
        if !emits {
            continue;
        }
        let (dx, dy) = dir_delta(e.direction);
        let lands = if is_splitter(&e.name) {
            let (s2x, s2y) = splitter_second_tile(e);
            (e.x + dx, e.y + dy) == tile || (s2x + dx, s2y + dy) == tile
        } else {
            (e.x + dx, e.y + dy) == tile
        };
        if lands {
            return Some(ExternalFeederSnapshot {
                entity_name: e.name.clone(),
                entity_x: e.x,
                entity_y: e.y,
                direction: dir_label(e.direction),
            });
        }
    }
    None
}


/// Physical flow direction at an entry-boundary tile.
///
/// The SAT encoder treats `ZoneBoundary.direction` as the direction items
/// flow *through* the boundary tile. For a straight-axis entry the spec's
/// desired exit direction coincides with the physical arrival axis, so
/// the spec-derived direction is correct. But when an external feeder
/// (a splitter, stamped belt, UG-out) dumps into the entry tile from a
/// non-native side, the physical flow direction is the feeder's output
/// direction, not the spec's axis. Without this override SAT forces the
/// tile to face the spec's axis, which mis-models the arrival and can
/// lock the solver into a sideload or produce UNSAT.
///
/// Returns `Some(feeder.direction)` if a feeder outputs onto `tile`,
/// else `None` (use the spec direction). Only belts, splitters, and
/// UG-outs are considered — UG-ins capture rather than emit, and other
/// entity types (inserters, machines) don't participate in belt flow.
fn physical_feeder_direction(
    tile: (i32, i32),
    placed_entities: &[PlacedEntity],
) -> Option<EntityDirection> {
    physical_feeder_hit(tile, placed_entities).map(|hit| hit.entity_direction)
}

/// Find a Permanent entity (splitter / belt / UG-out) whose output lands
/// on `tile`. Returns the *specific* feeder tile and direction.
///
/// For splitters, returns the one of the two tiles that physically emits
/// onto `tile` (the tile from which `tile = feeder_tile + dir_delta(dir)`).
fn physical_feeder_hit(
    tile: (i32, i32),
    placed_entities: &[PlacedEntity],
) -> Option<FeederHit> {
    for e in placed_entities {
        // UG-ins consume; they don't emit onto the surface.
        if is_ug_belt(&e.name) && e.io_type.as_deref() == Some("input") {
            continue;
        }
        let emits = is_surface_belt(&e.name)
            || is_splitter(&e.name)
            || (is_ug_belt(&e.name) && e.io_type.as_deref() == Some("output"));
        if !emits {
            continue;
        }
        let (dx, dy) = dir_delta(e.direction);
        if is_splitter(&e.name) {
            let (sx, sy) = splitter_second_tile(e);
            if (e.x + dx, e.y + dy) == tile {
                return Some(FeederHit {
                    entity_tile: (e.x, e.y),
                    entity_direction: e.direction,
                });
            }
            if (sx + dx, sy + dy) == tile {
                return Some(FeederHit {
                    entity_tile: (sx, sy),
                    entity_direction: e.direction,
                });
            }
        } else if (e.x + dx, e.y + dy) == tile {
            return Some(FeederHit {
                entity_tile: (e.x, e.y),
                entity_direction: e.direction,
            });
        }
    }
    None
}

/// If a Permanent flow-source entity has a tile inside `bbox` AND in
/// `forbidden`, and that tile emits onto `spec_entry`, return
/// `(interior_tile, feeder.direction)`. Otherwise None — caller falls
/// back to the perimeter boundary model.
///
/// "Interior" means the boundary lives at the Permanent entity's tile
/// rather than at the first SAT-placeable tile downstream. Lets SAT route
/// the downstream tile freely instead of pinning it to the arrival axis.
fn interior_input_boundary(
    spec_entry: (i32, i32),
    bbox: &Rect,
    forbidden: &FxHashSet<(i32, i32)>,
    placed_entities: &[PlacedEntity],
) -> Option<((i32, i32), EntityDirection)> {
    let hit = physical_feeder_hit(spec_entry, placed_entities)?;
    let (tx, ty) = hit.entity_tile;
    if bbox.contains(tx, ty) && forbidden.contains(&(tx, ty)) {
        Some(((tx, ty), hit.entity_direction))
    } else {
        None
    }
}

/// Symmetric to `interior_input_boundary`: if the tile immediately past
/// `spec_exit` in direction `spec_exit_dir` (i.e. the next tile downstream
/// of the zone exit) is inside `bbox` AND in `forbidden` AND is occupied
/// by a Permanent entity, the boundary can move to that interior tile.
///
/// Direction stays as `spec_exit_dir`: the boundary tile's "output axis"
/// is whatever direction items would continue moving in once they reach
/// the Permanent consumer.
fn interior_output_boundary(
    spec_exit: (i32, i32),
    spec_exit_dir: EntityDirection,
    bbox: &Rect,
    forbidden: &FxHashSet<(i32, i32)>,
    placed_entities: &[PlacedEntity],
) -> Option<((i32, i32), EntityDirection)> {
    let (dx, dy) = dir_delta(spec_exit_dir);
    let consumer = (spec_exit.0 + dx, spec_exit.1 + dy);
    if !bbox.contains(consumer.0, consumer.1) || !forbidden.contains(&consumer) {
        return None;
    }
    // Confirm a Permanent entity actually occupies `consumer`.
    let occupied = placed_entities.iter().any(|e| {
        if is_splitter(&e.name) {
            let (sx, sy) = splitter_second_tile(e);
            (e.x, e.y) == consumer || (sx, sy) == consumer
        } else {
            (e.x, e.y) == consumer
        }
    });
    if occupied {
        Some((consumer, spec_exit_dir))
    } else {
        None
    }
}

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
        //
        // Boundary positioning has three modes:
        //   1. **Interior** — a Permanent flow-source/consumer has a tile
        //      inside the bbox AND in `forbidden`. The boundary lives at
        //      that interior tile; SAT places no entity there but
        //      constrains the adjacent in-zone tile to receive/send the
        //      flow. Frees the in-zone tile to face any direction SAT
        //      finds convenient.
        //   2. **Perimeter + arrival override** — feeder is OUTSIDE the
        //      bbox and dumps into the spec's entry tile. The boundary
        //      stays at the spec's tile but `direction` is overridden to
        //      the feeder's output direction so SAT models the arrival
        //      axis correctly.
        //   3. **Perimeter (default)** — straight-axis crossing. Boundary
        //      at the spec's tile with the spec's direction.
        let mut boundaries: Vec<ZoneBoundary> =
            Vec::with_capacity(ctx.junction.specs.len() * 2);
        for spec in &ctx.junction.specs {
            let (entry_x, entry_y, entry_direction, entry_interior) =
                match interior_input_boundary(
                    (spec.entry.x, spec.entry.y),
                    &ctx.junction.bbox,
                    &ctx.junction.forbidden,
                    ctx.placed_entities,
                ) {
                    Some(((ix, iy), dir)) => (ix, iy, dir, true),
                    None => {
                        let dir = physical_feeder_direction(
                            (spec.entry.x, spec.entry.y),
                            ctx.placed_entities,
                        )
                        .unwrap_or(spec.entry.direction);
                        (spec.entry.x, spec.entry.y, dir, false)
                    }
                };
            boundaries.push(ZoneBoundary {
                x: entry_x,
                y: entry_y,
                direction: entry_direction,
                item: spec.item.clone(),
                is_input: true,
                interior: entry_interior,
            });

            let (exit_x, exit_y, exit_direction, exit_interior) =
                match interior_output_boundary(
                    (spec.exit.x, spec.exit.y),
                    spec.exit.direction,
                    &ctx.junction.bbox,
                    &ctx.junction.forbidden,
                    ctx.placed_entities,
                ) {
                    Some(((ix, iy), dir)) => (ix, iy, dir, true),
                    None => (spec.exit.x, spec.exit.y, spec.exit.direction, false),
                };
            boundaries.push(ZoneBoundary {
                x: exit_x,
                y: exit_y,
                direction: exit_direction,
                item: spec.item.clone(),
                is_input: false,
                interior: exit_interior,
            });
        }

        let forced_empty: Vec<(i32, i32)> =
            ctx.junction.forbidden.iter().copied().collect();

        // Build a snapshot view of the boundaries for the trace event —
        // mirrors the junction_solver snapshot format so CLI replay
        // tools can use the same rendering for both.
        let boundary_snapshots: Vec<BoundarySnapshot> = boundaries
            .iter()
            .zip(ctx.junction.specs.iter().flat_map(|s| {
                [
                    (s, true),
                    (s, false),
                ]
            }))
            .map(|(b, (_, is_input))| {
                let feeder = if is_input {
                    find_external_feeder((b.x, b.y), ctx.placed_entities)
                } else {
                    None
                };
                BoundarySnapshot {
                    x: b.x,
                    y: b.y,
                    direction: dir_label(b.direction),
                    item: b.item.clone(),
                    is_input: b.is_input,
                    spec_key: String::new(),
                    external_feeder: feeder,
                }
            })
            .collect();

        let zone = CrossingZone {
            x: ctx.junction.bbox.x,
            y: ctx.junction.bbox.y,
            width: ctx.junction.bbox.w,
            height: ctx.junction.bbox.h,
            boundaries: boundaries.clone(),
            forced_empty: forced_empty.clone(),
        };

        let (seed_x, seed_y) = ctx.region.initial_tile;
        let iter = ctx.growth_iter;

        let (entities_opt, stats) = solve_crossing_zone_with_stats(&zone, max_reach, belt_name);
        let satisfied = entities_opt.is_some();
        let entities_raw = entities_opt.as_ref().map(|e| e.len()).unwrap_or(0);
        trace::emit(TraceEvent::SatInvocation {
            seed_x,
            seed_y,
            iter,
            zone_x: zone.x,
            zone_y: zone.y,
            zone_w: zone.width,
            zone_h: zone.height,
            boundaries: boundary_snapshots,
            forced_empty,
            belt_tier: belt_name.to_string(),
            max_reach,
            satisfied,
            variables: stats.variables,
            clauses: stats.clauses,
            solve_time_us: stats.solve_time_us,
            entities_raw,
        });
        let entities = entities_opt?;

        let pruned = prune_dangling_sat_entities(
            entities,
            &boundaries,
            max_reach,
            zone.x,
            zone.y,
        );

        Some(JunctionSolution {
            entities: pruned,
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

// ---------------------------------------------------------------------------
// Dangling belt pruning
// ---------------------------------------------------------------------------

fn opposite(dir: EntityDirection) -> EntityDirection {
    match dir {
        EntityDirection::North => EntityDirection::South,
        EntityDirection::East  => EntityDirection::West,
        EntityDirection::South => EntityDirection::North,
        EntityDirection::West  => EntityDirection::East,
    }
}

/// Remove SAT-placed belt entities that are not on any path from an input
/// boundary to an output boundary.  Orphaned tiles arise from near-miss SAT
/// assignments where a variable is set true but the resulting entity is
/// unreachable in the final flow graph.
///
/// Algorithm: downstream BFS from all input boundaries ∩ upstream BFS from
/// all output boundaries.  Keep only entities in both reachable sets.
///
/// For interior boundaries the boundary tile itself is `forced_empty`
/// (no SAT entity), so the BFS seeds from the in-zone *neighbor* — the
/// tile the encoder's interior arm actually constrains. For an interior
/// input the neighbor is `boundary + dir_delta(direction)`; for an
/// interior output it's `boundary + dir_delta(opposite(direction))`.
fn prune_dangling_sat_entities(
    entities: Vec<PlacedEntity>,
    boundaries: &[ZoneBoundary],
    max_reach: u32,
    zone_x: i32,
    zone_y: i32,
) -> Vec<PlacedEntity> {
    use std::collections::{HashMap, VecDeque};

    let by_tile: HashMap<(i32, i32), usize> = entities
        .iter()
        .enumerate()
        .map(|(i, e)| ((e.x, e.y), i))
        .collect();

    // Map a boundary to the actual in-zone tile that holds the SAT
    // entity feeding (for inputs) or sinking (for outputs) the spec's
    // flow. Perimeter boundaries: that's the boundary tile itself.
    // Interior boundaries: the in-zone neighbor along the flow axis.
    let bfs_start = |b: &ZoneBoundary| -> (i32, i32) {
        if b.interior {
            let (dx, dy) = if b.is_input {
                dir_delta(b.direction)
            } else {
                dir_delta(opposite(b.direction))
            };
            (b.x + dx, b.y + dy)
        } else {
            (b.x, b.y)
        }
    };

    // ---- downstream BFS (input → output direction) ----

    let mut reachable_from_input: FxHashSet<(i32, i32)> = FxHashSet::default();
    let mut queue: VecDeque<(i32, i32)> = VecDeque::new();

    for b in boundaries.iter().filter(|b| b.is_input) {
        let t = bfs_start(b);
        if reachable_from_input.insert(t) {
            queue.push_back(t);
        }
    }

    while let Some(t) = queue.pop_front() {
        let Some(&idx) = by_tile.get(&t) else { continue };
        let e = &entities[idx];
        let next_tiles = next_downstream(&entities, &by_tile, e, max_reach);
        for n in next_tiles {
            if reachable_from_input.insert(n) {
                queue.push_back(n);
            }
        }
    }

    // ---- upstream BFS (output → input direction) ----

    let mut reachable_to_output: FxHashSet<(i32, i32)> = FxHashSet::default();

    for b in boundaries.iter().filter(|b| !b.is_input) {
        let t = bfs_start(b);
        if reachable_to_output.insert(t) {
            queue.push_back(t);
        }
    }

    while let Some(t) = queue.pop_front() {
        let Some(&idx) = by_tile.get(&t) else { continue };
        let e = &entities[idx];
        let prev_tiles = next_upstream(&entities, &by_tile, e, max_reach);
        for n in prev_tiles {
            if reachable_to_output.insert(n) {
                queue.push_back(n);
            }
        }
    }

    // ---- keep intersection ----

    let total = entities.len();
    let pruned: Vec<PlacedEntity> = entities
        .into_iter()
        .filter(|e| {
            let t = (e.x, e.y);
            reachable_from_input.contains(&t) && reachable_to_output.contains(&t)
        })
        .collect();
    let kept = pruned.len();

    if kept < total {
        trace::emit(trace::TraceEvent::SatPruned { zone_x, zone_y, total, kept });
    }

    pruned
}

/// Tiles reachable downstream from entity `e` in one step (or one UG pair).
fn next_downstream(
    entities: &[PlacedEntity],
    by_tile: &std::collections::HashMap<(i32, i32), usize>,
    e: &PlacedEntity,
    max_reach: u32,
) -> Vec<(i32, i32)> {
    match e.io_type.as_deref() {
        Some("input") => {
            // UG-in: scan forward up to max_reach tiles to find the paired UG-out.
            let (dx, dy) = dir_delta(e.direction);
            let mut results = Vec::new();
            for dist in 1..=max_reach as i32 {
                let nx = e.x + dx * dist;
                let ny = e.y + dy * dist;
                if let Some(&ni) = by_tile.get(&(nx, ny)) {
                    let n = &entities[ni];
                    if n.io_type.as_deref() == Some("output") && n.direction == e.direction {
                        results.push((nx, ny));
                        break;
                    }
                }
            }
            results
        }
        _ => {
            // Belt or UG-out: next tile in output direction.
            let (dx, dy) = dir_delta(e.direction);
            vec![(e.x + dx, e.y + dy)]
        }
    }
}

/// Tiles reachable upstream from entity `e` in one step (or one UG pair).
fn next_upstream(
    entities: &[PlacedEntity],
    by_tile: &std::collections::HashMap<(i32, i32), usize>,
    e: &PlacedEntity,
    max_reach: u32,
) -> Vec<(i32, i32)> {
    match e.io_type.as_deref() {
        Some("output") => {
            // UG-out: scan backward to find the paired UG-in.
            let (dx, dy) = dir_delta(opposite(e.direction));
            let mut results = Vec::new();
            for dist in 1..=max_reach as i32 {
                let nx = e.x + dx * dist;
                let ny = e.y + dy * dist;
                if let Some(&ni) = by_tile.get(&(nx, ny)) {
                    let n = &entities[ni];
                    if n.io_type.as_deref() == Some("input") && n.direction == e.direction {
                        results.push((nx, ny));
                        break;
                    }
                }
            }
            results
        }
        _ => {
            // Belt or UG-in: the tile that outputs toward us.
            // Check all 4 neighbors; keep those whose entity outputs into `e`.
            let mut results = Vec::new();
            for &dir in &[
                EntityDirection::North,
                EntityDirection::East,
                EntityDirection::South,
                EntityDirection::West,
            ] {
                let (dx, dy) = dir_delta(dir);
                let nx = e.x + dx;
                let ny = e.y + dy;
                if let Some(&ni) = by_tile.get(&(nx, ny)) {
                    let n = &entities[ni];
                    // n must output in direction opposite(dir) to feed into e
                    if n.io_type.as_deref() != Some("input") && n.direction == opposite(dir) {
                        results.push((nx, ny));
                    }
                }
            }
            results
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::EntityDirection;

    fn make_belt(x: i32, y: i32, dir: EntityDirection, item: &str) -> PlacedEntity {
        PlacedEntity {
            name: "transport-belt".into(),
            x,
            y,
            direction: dir,
            carries: Some(item.into()),
            ..Default::default()
        }
    }

    #[test]
    fn test_prune_removes_orphan_belt() {
        // Layout: input at (0,0) East, output at (2,0) East.
        // Valid path: (0,0)→(1,0)→(2,0) all facing East.
        // Orphan: (1,1) facing East — not connected to anything.
        let entities = vec![
            make_belt(0, 0, EntityDirection::East, "iron-plate"),
            make_belt(1, 0, EntityDirection::East, "iron-plate"),
            make_belt(2, 0, EntityDirection::East, "iron-plate"),
            make_belt(1, 1, EntityDirection::East, "iron-plate"), // orphan
        ];
        let boundaries = vec![
            ZoneBoundary { x: 0, y: 0, direction: EntityDirection::East, item: "iron-plate".into(), is_input: true, interior: false },
            ZoneBoundary { x: 2, y: 0, direction: EntityDirection::East, item: "iron-plate".into(), is_input: false, interior: false },
        ];
        let result = prune_dangling_sat_entities(entities, &boundaries, 4, 0, 0);
        assert_eq!(result.len(), 3, "orphan at (1,1) should be pruned");
        assert!(result.iter().all(|e| e.y == 0), "only y=0 row survives");
    }

    #[test]
    fn test_prune_keeps_full_path() {
        // Single straight path, nothing to prune.
        let entities = vec![
            make_belt(0, 0, EntityDirection::East, "copper-plate"),
            make_belt(1, 0, EntityDirection::East, "copper-plate"),
        ];
        let boundaries = vec![
            ZoneBoundary { x: 0, y: 0, direction: EntityDirection::East, item: "copper-plate".into(), is_input: true, interior: false },
            ZoneBoundary { x: 1, y: 0, direction: EntityDirection::East, item: "copper-plate".into(), is_input: false, interior: false },
        ];
        let result = prune_dangling_sat_entities(entities, &boundaries, 4, 0, 0);
        assert_eq!(result.len(), 2, "full path should be untouched");
    }

    // -- Interior-boundary helpers ------------------------------------------

    fn make_splitter(x: i32, y: i32, dir: EntityDirection) -> PlacedEntity {
        PlacedEntity {
            name: "splitter".into(),
            x,
            y,
            direction: dir,
            ..Default::default()
        }
    }

    #[test]
    fn test_interior_input_boundary_from_splitter() {
        // South-facing splitter at (1,9)+(2,9). Bbox is 3×3 from (2,9),
        // so (2,9) is inside the bbox AND in forbidden. The spec's entry
        // is at (2,10) (the tile the splitter feeds into). The interior
        // boundary should land at (2,9) dir=South.
        let splitter = make_splitter(1, 9, EntityDirection::South);
        let placed = vec![splitter];
        let bbox = Rect { x: 2, y: 9, w: 3, h: 3 };
        let mut forbidden = FxHashSet::default();
        forbidden.insert((2, 9));

        let result = interior_input_boundary(
            (2, 10),
            &bbox,
            &forbidden,
            &placed,
        );
        assert_eq!(result, Some(((2, 9), EntityDirection::South)));
    }

    #[test]
    fn test_interior_input_boundary_external_splitter_returns_none() {
        // Splitter wholly OUTSIDE bbox: bbox starts at (3,9), splitter
        // at (1,9)+(2,9). Spec entry at (2,10) — but (2,10) is also
        // outside the bbox here, so the splitter feed lands externally.
        // The Permanent feeder tile (1,9) and (2,9) are NOT in forbidden.
        // Expected: None (fall back to perimeter model).
        let splitter = make_splitter(1, 9, EntityDirection::South);
        let placed = vec![splitter];
        let bbox = Rect { x: 3, y: 9, w: 3, h: 3 };
        let forbidden = FxHashSet::default();

        let result = interior_input_boundary(
            (2, 10),
            &bbox,
            &forbidden,
            &placed,
        );
        assert_eq!(result, None);
    }

    #[test]
    fn test_interior_input_boundary_belt_feeder() {
        // Single-tile belt feeder (not a splitter): an east-facing belt
        // at (5,5) emits onto (6,5). Bbox covers (5,5)+forbidden.
        let belt = make_belt(5, 5, EntityDirection::East, "iron-plate");
        let placed = vec![belt];
        let bbox = Rect { x: 5, y: 5, w: 3, h: 3 };
        let mut forbidden = FxHashSet::default();
        forbidden.insert((5, 5));

        let result = interior_input_boundary(
            (6, 5),
            &bbox,
            &forbidden,
            &placed,
        );
        assert_eq!(result, Some(((5, 5), EntityDirection::East)));
    }

    #[test]
    fn test_interior_output_boundary_from_consumer() {
        // Spec exit at (5,5) flowing East. The next tile (6,5) is in
        // bbox AND forbidden AND occupied by some Permanent entity
        // (a splitter for example). Boundary moves to (6,5) dir=East.
        let consumer = make_splitter(6, 5, EntityDirection::South);
        let placed = vec![consumer];
        let bbox = Rect { x: 5, y: 5, w: 3, h: 3 };
        let mut forbidden = FxHashSet::default();
        forbidden.insert((6, 5));

        let result = interior_output_boundary(
            (5, 5),
            EntityDirection::East,
            &bbox,
            &forbidden,
            &placed,
        );
        assert_eq!(result, Some(((6, 5), EntityDirection::East)));
    }

    #[test]
    fn test_interior_output_boundary_no_consumer() {
        // Spec exit at (5,5) East. Next tile (6,5) is in forbidden
        // (e.g. a tap-off underground passage), but no Permanent entity
        // sits there. Expected: None — we don't want to invent a
        // consumer where none exists.
        let placed: Vec<PlacedEntity> = vec![];
        let bbox = Rect { x: 5, y: 5, w: 3, h: 3 };
        let mut forbidden = FxHashSet::default();
        forbidden.insert((6, 5));

        let result = interior_output_boundary(
            (5, 5),
            EntityDirection::East,
            &bbox,
            &forbidden,
            &placed,
        );
        assert_eq!(result, None);
    }

    // -- Prune behaviour with interior boundaries ---------------------------

    fn ug_in(x: i32, y: i32, dir: EntityDirection, item: &str) -> PlacedEntity {
        PlacedEntity {
            name: "fast-underground-belt".into(),
            x,
            y,
            direction: dir,
            carries: Some(item.into()),
            io_type: Some("input".into()),
            ..Default::default()
        }
    }

    fn ug_out(x: i32, y: i32, dir: EntityDirection, item: &str) -> PlacedEntity {
        PlacedEntity {
            name: "fast-underground-belt".into(),
            x,
            y,
            direction: dir,
            carries: Some(item.into()),
            io_type: Some("output".into()),
            ..Default::default()
        }
    }

    /// Reproduces the iter-2 tier2 SAT solution and pipes it through
    /// `prune_dangling_sat_entities` exactly as the strategy does.
    /// The boundaries are interior on both inputs (iron-plate at (2,9),
    /// copper-cable at (3,9)) — their tiles are forced_empty, so a
    /// naive BFS that starts at `(b.x, b.y)` never advances and ALL
    /// entities get pruned even though they form valid input→output
    /// flows via UG corridors. The fix is to seed the BFS from the
    /// in-zone neighbour for interior boundaries.
    #[test]
    fn test_prune_keeps_interior_boundary_paths() {
        let entities = vec![
            ug_in(2, 10, EntityDirection::East, "iron-plate"),
            ug_out(5, 10, EntityDirection::East, "iron-plate"),
            ug_in(3, 10, EntityDirection::South, "copper-cable"),
            ug_out(3, 12, EntityDirection::South, "copper-cable"),
        ];
        let boundaries = vec![
            ZoneBoundary {
                x: 3, y: 9,
                direction: EntityDirection::South,
                item: "copper-cable".into(),
                is_input: true,
                interior: true,
            },
            ZoneBoundary {
                x: 3, y: 12,
                direction: EntityDirection::South,
                item: "copper-cable".into(),
                is_input: false,
                interior: false,
            },
            ZoneBoundary {
                x: 2, y: 9,
                direction: EntityDirection::South,
                item: "iron-plate".into(),
                is_input: true,
                interior: true,
            },
            ZoneBoundary {
                x: 5, y: 10,
                direction: EntityDirection::East,
                item: "iron-plate".into(),
                is_input: false,
                interior: false,
            },
        ];

        let pruned = prune_dangling_sat_entities(entities.clone(), &boundaries, 6, 1, 8);
        // All 4 UG endpoints must survive — they're the SAT-resolved
        // crossing for both specs and form valid input→output paths.
        assert_eq!(
            pruned.len(),
            4,
            "interior-boundary specs should retain their UG endpoints; got {pruned:#?}"
        );
    }
}
