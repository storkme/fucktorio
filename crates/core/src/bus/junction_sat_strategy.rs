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
use crate::trace::{self, BoundarySnapshot, ExternalFeederSnapshot, SatProposedEntity, TraceEvent};

/// A feeder/consumer tile candidate found adjacent to a spec entry/exit.
struct FeederHit {
    /// The tile of the Permanent entity that physically interacts with
    /// the boundary (for splitters, the specific one of two tiles).
    entity_tile: (i32, i32),
    /// The Permanent entity's facing direction.
    entity_direction: EntityDirection,
}

/// Knobs the outer loop tunes when asking SAT to solve a junction.
/// Strategies in priority order: the first config that satisfies AND
/// passes the walker wins. New cost dimensions (entity budget, specific
/// per-item constraints, …) should land here so the outer loop stays a
/// plain list of configs without ballooning into per-dimension strategy
/// structs.
#[derive(Debug, Clone, Copy)]
pub struct SatConstraints {
    /// Maximum number of underground-belt input tiles. Each UG-in
    /// pairs with exactly one UG-out, so this effectively caps the
    /// number of UG corridors in the zone.
    /// - `None`: unlimited — the original SAT behaviour.
    /// - `Some(0)`: surface-only. Forbids UG entirely.
    /// - `Some(k)`: at most `k` corridors. Used to spend the UG budget
    ///   only where surface routing is genuinely infeasible (real
    ///   crossings, tight turns), keeping everything else on the
    ///   surface.
    pub max_ug_ins: Option<u32>,
}

impl SatConstraints {
    /// Unrestricted — matches the original SAT behaviour.
    pub const fn unrestricted() -> Self {
        Self { max_ug_ins: None }
    }

    /// Hard-forbid underground-belt entities.
    pub const fn surface_only() -> Self {
        Self { max_ug_ins: Some(0) }
    }

    /// Cap the number of UG corridors at `n`.
    pub const fn max_ug_ins(n: u32) -> Self {
        Self { max_ug_ins: Some(n) }
    }
}

impl Default for SatConstraints {
    fn default() -> Self {
        Self::unrestricted()
    }
}

pub struct SatStrategy {
    name: &'static str,
    constraints: SatConstraints,
}

impl SatStrategy {
    /// Custom-named strategy with arbitrary constraints. Used when the
    /// pre-canned variants below aren't enough.
    pub const fn with(name: &'static str, constraints: SatConstraints) -> Self {
        Self { name, constraints }
    }

    /// Surface-only pass. Tried first so SAT only reaches for UG when
    /// it genuinely can't route on the surface.
    pub const fn surface_only() -> Self {
        Self::with("sat-surface", SatConstraints::surface_only())
    }

    /// Unrestricted pass. The original SAT behaviour — used as the
    /// fallback when budgeted passes fail.
    pub const fn unrestricted() -> Self {
        Self::with("sat", SatConstraints::unrestricted())
    }
}

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

/// Single-pass topology walk that discovers all SAT boundaries by classifying
/// every tile in the bbox as FREE (surface belt not in `forbidden`, SAT can
/// re-stamp) or FIXED (anything in `forbidden`: UG belts, splitters, machines,
/// etc.). Boundaries exist only where item flow crosses between a FREE tile
/// and the outside world or a FIXED tile. FIXED-to-FIXED and FIXED-to-outside
/// connections are pre-routed and invisible to SAT.
///
/// Replaces `walk_entry_to_perimeter` + `splitter_topology_boundaries` +
/// `belt_topology_boundaries` + the dedup step with a unified walk that
/// produces no duplicates by construction.
fn topology_boundaries(
    placed_entities: &[PlacedEntity],
    bbox: &Rect,
    forbidden: &FxHashSet<(i32, i32)>,
) -> Vec<ZoneBoundary> {
    let mut boundaries: Vec<ZoneBoundary> = Vec::new();

    // Phase 1: Walk every entity whose tile is inside bbox and FREE.
    for e in placed_entities {
        // Determine all tiles this entity occupies (1 for most, 2 for splitters).
        let entity_tiles: Vec<(i32, i32)> = if is_splitter(&e.name) {
            let (sx, sy) = splitter_second_tile(e);
            vec![(e.x, e.y), (sx, sy)]
        } else {
            vec![(e.x, e.y)]
        };

        for &(tx, ty) in &entity_tiles {
            if !bbox.contains(tx, ty) {
                continue;
            }
            // FREE = not in forbidden. FIXED tiles are skipped.
            if forbidden.contains(&(tx, ty)) {
                continue;
            }
            let Some(item) = e.carries.as_deref() else {
                continue;
            };

            // -- Output check: where does this belt/UG output? --
            let (dx, dy) = dir_delta(e.direction);
            let target = (tx + dx, ty + dy);

            if !bbox.contains(target.0, target.1) {
                // Target outside bbox: perimeter OUT.
                boundaries.push(ZoneBoundary {
                    x: tx,
                    y: ty,
                    direction: e.direction,
                    item: item.to_string(),
                    is_input: false,
                    interior: false,
                });
            } else if forbidden.contains(&target) {
                // Target is FIXED: interior OUT at the FIXED tile.
                boundaries.push(ZoneBoundary {
                    x: target.0,
                    y: target.1,
                    direction: e.direction,
                    item: item.to_string(),
                    is_input: false,
                    interior: true,
                });
            }
            // else: target is FREE, both SAT-routable → no boundary.

            // -- Input check: does anything feed this tile from outside/FIXED? --
            if let Some(hit) = physical_feeder_hit((tx, ty), placed_entities) {
                if !bbox.contains(hit.entity_tile.0, hit.entity_tile.1) {
                    // Feeder outside bbox: perimeter IN.
                    boundaries.push(ZoneBoundary {
                        x: tx,
                        y: ty,
                        direction: hit.entity_direction,
                        item: item.to_string(),
                        is_input: true,
                        interior: false,
                    });
                } else if forbidden.contains(&hit.entity_tile) {
                    // Feeder is FIXED: interior IN at the FIXED tile.
                    boundaries.push(ZoneBoundary {
                        x: hit.entity_tile.0,
                        y: hit.entity_tile.1,
                        direction: hit.entity_direction,
                        item: item.to_string(),
                        is_input: true,
                        interior: true,
                    });
                }
                // else: feeder is FREE → no boundary.
            }
        }
    }

    // Phase 2: Splitter topology. Splitters are FIXED (in forbidden) but
    // are active flow devices. For each splitter tile inside bbox, emit
    // interior boundaries for lanes connected to FREE tiles.
    for e in placed_entities {
        if !is_splitter(&e.name) {
            continue;
        }
        let Some(item) = e.carries.as_deref() else {
            continue;
        };
        let tiles = [(e.x, e.y), splitter_second_tile(e)];
        let (dx, dy) = dir_delta(e.direction);
        for &(sx, sy) in &tiles {
            if !bbox.contains(sx, sy) {
                continue;
            }
            let input_nb = (sx - dx, sy - dy);
            let output_nb = (sx + dx, sy + dy);

            // Input side: splitter receives from a FREE tile.
            let input_wired = placed_entities.iter().any(|n| {
                if n.carries.as_deref() != Some(item) {
                    return false;
                }
                if is_ug_belt(&n.name) && n.io_type.as_deref() != Some("output") {
                    return false;
                }
                if !(is_surface_belt(&n.name) || is_splitter(&n.name) || is_ug_belt(&n.name)) {
                    return false;
                }
                let (ndx, ndy) = dir_delta(n.direction);
                if is_splitter(&n.name) {
                    let (nsx, nsy) = splitter_second_tile(n);
                    (n.x + ndx, n.y + ndy) == (sx, sy)
                        || (nsx + ndx, nsy + ndy) == (sx, sy)
                } else {
                    (n.x == input_nb.0 && n.y == input_nb.1)
                        && (n.x + ndx, n.y + ndy) == (sx, sy)
                }
            });
            if input_wired {
                // Only emit if the neighbor is FREE (not FIXED-to-FIXED).
                if !forbidden.contains(&input_nb) {
                    boundaries.push(ZoneBoundary {
                        x: sx,
                        y: sy,
                        direction: e.direction,
                        item: item.to_string(),
                        is_input: false, // splitter input-side = zone OUT
                        interior: true,
                    });
                }
            }

            // Output side: splitter emits to a FREE tile.
            let output_wired = placed_entities.iter().any(|n| {
                if n.carries.as_deref() != Some(item) {
                    return false;
                }
                if !(is_surface_belt(&n.name) || is_splitter(&n.name) || is_ug_belt(&n.name)) {
                    return false;
                }
                if is_splitter(&n.name) {
                    let (nsx, nsy) = splitter_second_tile(n);
                    (n.x, n.y) == output_nb || (nsx, nsy) == output_nb
                } else {
                    (n.x, n.y) == output_nb
                }
            });
            if output_wired && !forbidden.contains(&output_nb) {
                boundaries.push(ZoneBoundary {
                    x: sx,
                    y: sy,
                    direction: e.direction,
                    item: item.to_string(),
                    is_input: true, // splitter output-side = zone IN
                    interior: true,
                });
            }
        }
    }

    boundaries
}


impl JunctionStrategy for SatStrategy {
    fn name(&self) -> &'static str {
        self.name
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

        // Topology-based boundary discovery: walk every physical tile in the
        // bbox, classify as FREE (surface belt, SAT can re-stamp) or FIXED
        // (anything in forbidden), and emit boundaries wherever flow crosses
        // between FREE and outside/FIXED. No dedup needed — each boundary
        // position is unique by construction.
        let boundaries = topology_boundaries(
            ctx.placed_entities,
            &ctx.junction.bbox,
            &ctx.junction.forbidden,
        );
        let origins: Vec<String> = boundaries.iter().map(|_| "topology".to_string()).collect();

        let forced_empty: Vec<(i32, i32)> =
            ctx.junction.forbidden.iter().copied().collect();

        // Build snapshots: origin was tracked in parallel above.
        let boundary_snapshots: Vec<BoundarySnapshot> = boundaries
            .iter()
            .zip(&origins)
            .map(|(b, origin)| {
                let feeder = if b.is_input {
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
                    interior: b.interior,
                    spec_key: String::new(),
                    origin: origin.clone(),
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

        let (entities_opt, stats) = solve_crossing_zone_with_stats(
            &zone,
            max_reach,
            belt_name,
            self.constraints.max_ug_ins,
        );
        let satisfied = entities_opt.is_some();
        let entities_raw = entities_opt.as_ref().map(|e| e.len()).unwrap_or(0);
        let proposed_entities: Vec<SatProposedEntity> = entities_opt
            .as_ref()
            .map(|es| {
                es.iter()
                    .map(|e| SatProposedEntity {
                        x: e.x,
                        y: e.y,
                        name: e.name.clone(),
                        direction: dir_label(e.direction),
                        carries: e.carries.clone(),
                        io_type: e.io_type.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        trace::emit(TraceEvent::SatInvocation {
            seed_x,
            seed_y,
            iter,
            variant: ctx.growth_variant.to_string(),
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
            proposed_entities,
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

    // -- Splitter topology helpers ------------------------------------------

    fn make_surface_belt(x: i32, y: i32, dir: EntityDirection, item: &str) -> PlacedEntity {
        PlacedEntity {
            name: "fast-transport-belt".into(),
            x,
            y,
            direction: dir,
            carries: Some(item.into()),
            ..Default::default()
        }
    }

    fn make_splitter_at(x: i32, y: i32, dir: EntityDirection, item: &str) -> PlacedEntity {
        PlacedEntity {
            name: "fast-splitter".into(),
            x,
            y,
            direction: dir,
            carries: Some(item.into()),
            ..Default::default()
        }
    }

    // -- Topology boundary tests ------------------------------------------------

    #[test]
    fn test_topology_boundaries_ug_crossing_zone() {
        // Bbox: x:2-5, y:17-19 (4x3)
        //
        // y\x  2         3           4         5
        // 17   .         UG↓in(cc)   belt↓(cc) .
        // 18   belt→(ip) UG→in(ip)   belt↓(cc) UG→out(ip)
        // 19   .         UG↓out(cc)  belt→(cc) belt→(cc)
        //
        // FREE: (2,18), (4,17), (4,18), (4,19), (5,19) — surface belts
        // FIXED: (3,17), (3,18), (3,19), (5,18) — UG belts
        let bbox = Rect { x: 2, y: 17, w: 4, h: 3 };

        let placed = vec![
            // External feeders (outside bbox)
            make_surface_belt(1, 18, EntityDirection::East, "iron-plate"),  // feeds (2,18)
            make_surface_belt(4, 16, EntityDirection::South, "copper-cable"), // feeds (4,17)
            // Iron-plate path
            make_surface_belt(2, 18, EntityDirection::East, "iron-plate"),
            ug_in(3, 18, EntityDirection::East, "iron-plate"),
            ug_out(5, 18, EntityDirection::East, "iron-plate"),
            // Copper-cable UG tunnel (column 3)
            ug_in(3, 17, EntityDirection::South, "copper-cable"),
            ug_out(3, 19, EntityDirection::South, "copper-cable"),
            // Copper-cable surface chain (column 4 → 5)
            make_surface_belt(4, 17, EntityDirection::South, "copper-cable"),
            make_surface_belt(4, 18, EntityDirection::South, "copper-cable"),
            make_surface_belt(4, 19, EntityDirection::East, "copper-cable"),
            make_surface_belt(5, 19, EntityDirection::East, "copper-cable"),
        ];

        let forbidden: FxHashSet<(i32, i32)> = [
            (3, 17), (3, 18), (3, 19), (5, 18),
        ].into_iter().collect();

        let bounds = topology_boundaries(&placed, &bbox, &forbidden);

        let ins: Vec<_> = bounds.iter().filter(|b| b.is_input).collect();
        let outs: Vec<_> = bounds.iter().filter(|b| !b.is_input).collect();

        // Expected 4 boundaries:
        //   IN:  (2,18) East iron-plate perimeter, (4,17) South copper-cable perimeter
        //   OUT: (3,18) East iron-plate interior, (5,19) East copper-cable perimeter
        assert_eq!(ins.len(), 2, "IN boundaries: {ins:#?}");
        assert_eq!(outs.len(), 2, "OUT boundaries: {outs:#?}");

        // IN (2,18) East iron-plate, perimeter
        assert!(
            ins.iter().any(|b| (b.x, b.y) == (2, 18)
                && b.direction == EntityDirection::East
                && b.item == "iron-plate"
                && !b.interior),
            "missing perimeter IN (2,18) East iron-plate"
        );

        // IN (4,17) South copper-cable, perimeter
        assert!(
            ins.iter().any(|b| (b.x, b.y) == (4, 17)
                && b.direction == EntityDirection::South
                && b.item == "copper-cable"
                && !b.interior),
            "missing perimeter IN (4,17) South copper-cable"
        );

        // OUT (3,18) East iron-plate, interior (belt outputs to FIXED UG-in)
        assert!(
            outs.iter().any(|b| (b.x, b.y) == (3, 18)
                && b.direction == EntityDirection::East
                && b.item == "iron-plate"
                && b.interior),
            "missing interior OUT (3,18) East iron-plate"
        );

        // OUT (5,19) East copper-cable, perimeter
        assert!(
            outs.iter().any(|b| (b.x, b.y) == (5, 19)
                && b.direction == EntityDirection::East
                && b.item == "copper-cable"
                && !b.interior),
            "missing perimeter OUT (5,19) East copper-cable"
        );
    }
}
