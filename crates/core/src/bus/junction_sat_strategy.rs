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

use crate::bus::junction::{BeltTier, Rect, SpecOrigin};
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

/// Walk the feeder chain backward from `spec_entry` until the next
/// upstream feeder lies outside `bbox` (or there is no feeder). Returns
/// the final in-bbox tile and the physical flow direction at that tile.
///
/// At each step we consult `physical_feeder_hit` for the tile we're on;
/// if it points to a Permanent/belt-like entity inside the bbox, we
/// advance to that entity's tile and inherit its direction. The loop
/// terminates when:
///   - the spec tile has no physical feeder (orphan);
///   - the feeder's tile is outside the bbox (perimeter crossing);
///   - we'd re-enter a tile we've already visited (cycle safety).
///
/// Generalizes the prior single-step `interior_input_boundary`: with a
/// chain of in-bbox belts + a splitter feeder, the boundary keeps
/// moving back until it sits at the real perimeter-crossing tile rather
/// than the first-spec-path tile.
fn walk_entry_to_perimeter(
    spec_entry: (i32, i32),
    spec_dir: EntityDirection,
    bbox: &Rect,
    placed_entities: &[PlacedEntity],
) -> ((i32, i32), EntityDirection) {
    let mut current = spec_entry;
    // Initial direction: whatever the physical feeder at the spec entry
    // says, falling back to the spec's own direction if there's no
    // feeder. Mirrors the old `physical_feeder_direction` override.
    let mut current_dir = physical_feeder_direction(current, placed_entities)
        .unwrap_or(spec_dir);
    let mut visited: FxHashSet<(i32, i32)> = FxHashSet::default();
    visited.insert(current);
    loop {
        let Some(hit) = physical_feeder_hit(current, placed_entities) else {
            return (current, current_dir);
        };
        if !bbox.contains(hit.entity_tile.0, hit.entity_tile.1) {
            return (current, current_dir);
        }
        if !visited.insert(hit.entity_tile) {
            return (current, current_dir);
        }
        current = hit.entity_tile;
        current_dir = hit.entity_direction;
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

/// Synthesize SAT boundaries from the in-bbox splitters' actual
/// wiring. When the bbox absorbs a splitter, we can't just mark its
/// tiles as `forbidden` and move on — the splitter is an active flow
/// device, and without telling SAT about its inputs/outputs SAT will
/// happily invent routings that bypass the splitter (e.g. UG-tunneling
/// underneath the trunk's splitter tile), producing "satisfied"
/// solutions that don't match physics.
///
/// For each splitter tile inside `bbox`, we examine the tiles
/// immediately upstream (input side) and downstream (output side) of
/// that lane, and emit interior `ZoneBoundary`s for the connections
/// that are actually wired up in `placed_entities`:
///
/// - **Interior OUTPUT** at the splitter tile, direction = splitter's
///   direction, if the input-side neighbor has a feeder entity
///   carrying the splitter's item. This forces SAT to route the
///   in-zone flow *into* the splitter rather than bypassing it.
///
/// - **Interior INPUT** at the splitter tile, direction = splitter's
///   direction, if the output-side neighbor is wired with a
///   belt-like entity carrying a compatible item. This forces SAT
///   to honor the splitter's output — the downstream tile must
///   carry the splitter's item.
///
/// Unconnected lane-sides get no boundary (per the
/// "*if they are connected when we absorb them*" rule): we don't
/// invent a feeder or consumer where the real topology has none.
fn splitter_topology_boundaries(
    placed_entities: &[PlacedEntity],
    bbox: &Rect,
) -> Vec<ZoneBoundary> {
    let mut out = Vec::new();
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

            // Input side: any belt-like entity at `input_nb` that
            // outputs onto `(sx,sy)` carrying `item`. Surface belts,
            // splitters, UG-outs all count (each can emit). We
            // require a matching item to avoid tying together
            // flow-paths of different items — SAT models items
            // independently.
            let input_wired = placed_entities.iter().any(|n| {
                if n.carries.as_deref() != Some(item) {
                    return false;
                }
                if is_ug_belt(&n.name) && n.io_type.as_deref() != Some("output") {
                    return false; // UG-in doesn't emit onto surface
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
                out.push(ZoneBoundary {
                    x: sx,
                    y: sy,
                    direction: e.direction,
                    item: item.to_string(),
                    is_input: false, // splitter's input-side = zone OUT
                    interior: true,
                });
            }

            // Output side: any belt-like entity at `output_nb`
            // carrying a compatible item. We don't filter by the
            // receiver's facing — any belt accepts input from any
            // side (sideloads are permissible on surface belts; the
            // perpendicular-UG-in rule is enforced separately by the
            // encoder's interior-input arm).
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
            if output_wired {
                out.push(ZoneBoundary {
                    x: sx,
                    y: sy,
                    direction: e.direction,
                    item: item.to_string(),
                    is_input: true, // splitter's output-side = zone IN
                    interior: true,
                });
            }
        }
    }
    out
}

/// Synthesize SAT boundaries from absorbed **surface belts** whose
/// physical feeder or target lies outside the bbox. Complements
/// `splitter_topology_boundaries` for the belt-permissive case: even
/// though SAT is free to re-stamp surface belts, the item flow
/// crossing the bbox perimeter is a fixed boundary condition (the
/// upstream splitter / balancer / trunk outside the bbox will keep
/// feeding these tiles regardless of what SAT does inside).
///
/// Emits perimeter-style boundaries (`interior: false`) at the belt's
/// tile, with the belt's direction and item:
///
/// - **IN** if `physical_feeder_hit(T)` returns a feeder whose tile is
///   outside the bbox. Flow enters the zone here from outside.
///
/// - **OUT** if `T + dir_delta(belt.dir)` is outside the bbox. Flow
///   leaves the zone here to an outside consumer.
///
/// Belts with a fully-internal feeder+target contribute nothing (SAT
/// has full freedom over that interior region). Belts whose tile is
/// outside the bbox are skipped — only absorbed belts contribute
/// topology, per the "we only map inputs/outputs of the belts if they
/// are connected when we absorb them" rule.
///
/// Splitters are handled by `splitter_topology_boundaries` and skipped
/// here. UG-in/out are skipped for now (grey area — their tunnels
/// jump through non-adjacent tiles, which the perimeter-crossing check
/// doesn't model cleanly).
fn belt_topology_boundaries(
    placed_entities: &[PlacedEntity],
    bbox: &Rect,
) -> Vec<ZoneBoundary> {
    let mut out = Vec::new();
    for e in placed_entities {
        if !is_surface_belt(&e.name) {
            continue;
        }
        let Some(item) = e.carries.as_deref() else {
            continue;
        };
        if !bbox.contains(e.x, e.y) {
            continue;
        }
        let (dx, dy) = dir_delta(e.direction);

        // IN: flow enters the bbox at this belt if its physical feeder
        // sits outside the bbox. A belt with no feeder at all is an
        // "orphan" — don't invent an input where no physical source
        // connects (keeps us from treating broken ghost-stamped belts
        // like (5,10) iron-east @ iter1 as entries when their feeder
        // chain is silently broken by a currently-mis-stamped tile).
        if let Some(hit) = physical_feeder_hit((e.x, e.y), placed_entities) {
            if !bbox.contains(hit.entity_tile.0, hit.entity_tile.1) {
                out.push(ZoneBoundary {
                    x: e.x,
                    y: e.y,
                    direction: e.direction,
                    item: item.to_string(),
                    is_input: true,
                    interior: false,
                });
            }
        }

        // OUT: flow leaves the bbox if the belt's immediate output
        // tile lies outside the bbox.
        let target = (e.x + dx, e.y + dy);
        if !bbox.contains(target.0, target.1) {
            out.push(ZoneBoundary {
                x: e.x,
                y: e.y,
                direction: e.direction,
                item: item.to_string(),
                is_input: false,
                interior: false,
            });
        }
    }
    out
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
            // Walk the feeder chain backward from the spec's entry until
            // we leave the bbox. Interior iff the final tile sits in
            // forbidden (e.g. splitter body); perimeter otherwise.
            let (entry_tile, entry_direction) = walk_entry_to_perimeter(
                (spec.entry.x, spec.entry.y),
                spec.entry.direction,
                &ctx.junction.bbox,
                ctx.placed_entities,
            );
            let entry_interior = ctx.junction.forbidden.contains(&entry_tile);
            boundaries.push(ZoneBoundary {
                x: entry_tile.0,
                y: entry_tile.1,
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

        // Tag each accumulated boundary with its origin so snapshots
        // can distinguish participating/encountered spec-derived
        // boundaries from splitter-topology and belt-topology synthesis.
        // We zip after the spec loop completes (each spec contributes
        // two consecutive boundaries: IN then OUT).
        let mut origins: Vec<String> = ctx
            .junction
            .specs
            .iter()
            .flat_map(|s| {
                let o = match s.origin {
                    SpecOrigin::Participating => "participating",
                    SpecOrigin::Encountered => "encountered",
                };
                [o.to_string(), o.to_string()]
            })
            .collect();
        debug_assert_eq!(origins.len(), boundaries.len());

        // Splitter topology: for every splitter whose tile is inside
        // the bbox, read its actual wiring in `placed_entities` and
        // emit interior boundaries for the connected lanes.
        let splitter_bounds =
            splitter_topology_boundaries(ctx.placed_entities, &ctx.junction.bbox);
        for _ in &splitter_bounds {
            origins.push("splitter".to_string());
        }
        boundaries.extend(splitter_bounds);

        // Belt topology: surface belts whose feeder/target crosses the
        // bbox perimeter. Complements splitter topology for the
        // belt-permissive case — catches balancer-output and trunk
        // belts that flow into/out of the bbox but aren't on any
        // spec's routed path.
        let belt_bounds =
            belt_topology_boundaries(ctx.placed_entities, &ctx.junction.bbox);
        for _ in &belt_bounds {
            origins.push("belt".to_string());
        }
        boundaries.extend(belt_bounds);

        // Dedup by (x, y, direction, item, is_input). When the spec
        // walk lands on a splitter tile and splitter-topology emits
        // the same boundary, or when a belt-topology boundary matches
        // a walked spec boundary, we'd otherwise hand SAT duplicate
        // clauses. Kept-boundary wins its origin (first in order —
        // specs, then splitter, then belt — so splitter/belt origins
        // only show up for boundaries the specs didn't produce).
        // EntityDirection isn't Hash; encode as u8 for the dedup key.
        let dir_idx = |d: EntityDirection| -> u8 {
            match d {
                EntityDirection::North => 0,
                EntityDirection::East => 1,
                EntityDirection::South => 2,
                EntityDirection::West => 3,
            }
        };
        let mut seen: FxHashSet<(i32, i32, u8, String, bool)> = FxHashSet::default();
        let mut keep_idx: Vec<bool> = Vec::with_capacity(boundaries.len());
        for b in &boundaries {
            let key = (b.x, b.y, dir_idx(b.direction), b.item.clone(), b.is_input);
            keep_idx.push(seen.insert(key));
        }
        let (boundaries, origins): (Vec<_>, Vec<_>) = boundaries
            .into_iter()
            .zip(origins)
            .zip(keep_idx)
            .filter_map(|((b, o), keep)| if keep { Some((b, o)) } else { None })
            .unzip();

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

        let (entities_opt, stats) = solve_crossing_zone_with_stats(&zone, max_reach, belt_name);
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

    #[test]
    fn test_walk_entry_from_splitter() {
        // South-facing splitter at (1,9)+(2,9). Bbox is 3×3 from (2,9),
        // so (2,9) is inside the bbox. Spec entry at (2,10) flowing East;
        // the walk should step one tile upstream to the splitter body
        // (2,9) with the splitter's south direction.
        let splitter = make_splitter(1, 9, EntityDirection::South);
        let placed = vec![splitter];
        let bbox = Rect { x: 2, y: 9, w: 3, h: 3 };
        let (tile, dir) = walk_entry_to_perimeter(
            (2, 10),
            EntityDirection::East,
            &bbox,
            &placed,
        );
        assert_eq!(tile, (2, 9));
        assert_eq!(dir, EntityDirection::South);
    }

    #[test]
    fn test_walk_entry_external_splitter_stops_at_spec() {
        // Splitter wholly OUTSIDE bbox; the first upstream feeder we
        // reach from the spec entry falls outside bbox immediately, so
        // the walk stays at the spec entry (but with physical feeder
        // direction override if present).
        let splitter = make_splitter(1, 9, EntityDirection::South);
        let placed = vec![splitter];
        let bbox = Rect { x: 3, y: 9, w: 3, h: 3 };
        let (tile, dir) = walk_entry_to_perimeter(
            (2, 10),
            EntityDirection::East,
            &bbox,
            &placed,
        );
        // (2,10) itself is outside bbox, but walk doesn't check that —
        // it checks whether the *feeder tile* sits outside bbox. The
        // feeder (2,9) is outside bbox, so we stop at (2,10) with the
        // feeder's direction (South, from physical_feeder_direction
        // override).
        assert_eq!(tile, (2, 10));
        assert_eq!(dir, EntityDirection::South);
    }

    #[test]
    fn test_walk_entry_multi_step_belt_chain() {
        // Chain of belts (3,8) -> (3,9) -> (3,10), all south. Bbox
        // covers (3,9) and (3,10) but not (3,8). Spec entry (3,10):
        // walk should step back to (3,9) — feeder at (3,8) is outside
        // bbox so that's where we stop.
        let b1 = make_belt(3, 8, EntityDirection::South, "copper");
        let b2 = make_belt(3, 9, EntityDirection::South, "copper");
        let b3 = make_belt(3, 10, EntityDirection::South, "copper");
        let placed = vec![b1, b2, b3];
        let bbox = Rect { x: 2, y: 9, w: 3, h: 3 };
        let (tile, dir) = walk_entry_to_perimeter(
            (3, 10),
            EntityDirection::South,
            &bbox,
            &placed,
        );
        assert_eq!(tile, (3, 9));
        assert_eq!(dir, EntityDirection::South);
    }

    #[test]
    fn test_walk_entry_belt_feeder() {
        // Single east-facing belt at (5,5) feeding (6,5). Walk from
        // (6,5) with spec dir South: feeder at (5,5) is inside bbox so
        // step back; feeder for (5,5) is outside bbox so stop.
        let belt = make_belt(5, 5, EntityDirection::East, "iron-plate");
        let placed = vec![belt];
        let bbox = Rect { x: 5, y: 5, w: 3, h: 3 };
        let (tile, dir) = walk_entry_to_perimeter(
            (6, 5),
            EntityDirection::South,
            &bbox,
            &placed,
        );
        assert_eq!(tile, (5, 5));
        assert_eq!(dir, EntityDirection::East);
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

    #[test]
    fn test_splitter_topology_both_lanes_wired() {
        // South-facing splitter at (1,9)/(2,9). Feeders above at (1,8)
        // and (2,8), consumers below at (1,10) and (2,10). Expect 4
        // synthetic boundaries: IN+OUT on each splitter tile.
        let placed = vec![
            make_splitter_at(1, 9, EntityDirection::South, "iron-plate"),
            make_surface_belt(1, 8, EntityDirection::South, "iron-plate"),
            make_surface_belt(2, 8, EntityDirection::South, "iron-plate"),
            make_surface_belt(1, 10, EntityDirection::South, "iron-plate"),
            make_surface_belt(2, 10, EntityDirection::East, "iron-plate"),
        ];
        let bbox = Rect { x: 1, y: 8, w: 5, h: 5 };
        let bounds = splitter_topology_boundaries(&placed, &bbox);
        assert_eq!(bounds.len(), 4, "expected 4 boundaries, got {bounds:#?}");
        // 2 IN (downstream wired) + 2 OUT (upstream wired)
        let ins = bounds.iter().filter(|b| b.is_input).count();
        let outs = bounds.iter().filter(|b| !b.is_input).count();
        assert_eq!(ins, 2);
        assert_eq!(outs, 2);
        assert!(bounds.iter().all(|b| b.interior));
        assert!(bounds.iter().all(|b| b.direction == EntityDirection::South));
        assert!(bounds.iter().all(|b| b.item == "iron-plate"));
    }

    #[test]
    fn test_splitter_topology_one_lane_unwired() {
        // Lane 2 input side (2,8) has no entity; lane 2 output (2,10)
        // is still wired. Expect 3 boundaries: lane 1 IN+OUT, lane 2 IN.
        let placed = vec![
            make_splitter_at(1, 9, EntityDirection::South, "iron-plate"),
            make_surface_belt(1, 8, EntityDirection::South, "iron-plate"),
            make_surface_belt(1, 10, EntityDirection::South, "iron-plate"),
            make_surface_belt(2, 10, EntityDirection::East, "iron-plate"),
        ];
        let bbox = Rect { x: 1, y: 8, w: 5, h: 5 };
        let bounds = splitter_topology_boundaries(&placed, &bbox);
        assert_eq!(bounds.len(), 3, "expected 3 boundaries, got {bounds:#?}");
        // Lane 1 (splitter tile (1,9)): IN + OUT
        assert_eq!(
            bounds.iter().filter(|b| (b.x, b.y) == (1, 9)).count(),
            2
        );
        // Lane 2 (splitter tile (2,9)): just IN (output side wired)
        let lane2: Vec<_> = bounds.iter().filter(|b| (b.x, b.y) == (2, 9)).collect();
        assert_eq!(lane2.len(), 1);
        assert!(lane2[0].is_input);
    }

    #[test]
    fn test_splitter_topology_outside_bbox() {
        // Splitter wholly outside bbox → 0 boundaries.
        let placed = vec![
            make_splitter_at(1, 9, EntityDirection::South, "iron-plate"),
            make_surface_belt(1, 8, EntityDirection::South, "iron-plate"),
            make_surface_belt(1, 10, EntityDirection::South, "iron-plate"),
        ];
        let bbox = Rect { x: 20, y: 20, w: 3, h: 3 };
        let bounds = splitter_topology_boundaries(&placed, &bbox);
        assert!(bounds.is_empty());
    }

    #[test]
    fn test_splitter_topology_straddling_edge() {
        // Bbox covers (2,9) but not (1,9). Only lane 2 should
        // contribute boundaries.
        let placed = vec![
            make_splitter_at(1, 9, EntityDirection::South, "iron-plate"),
            make_surface_belt(1, 8, EntityDirection::South, "iron-plate"),
            make_surface_belt(2, 8, EntityDirection::South, "iron-plate"),
            make_surface_belt(1, 10, EntityDirection::South, "iron-plate"),
            make_surface_belt(2, 10, EntityDirection::East, "iron-plate"),
        ];
        let bbox = Rect { x: 2, y: 8, w: 3, h: 5 };
        let bounds = splitter_topology_boundaries(&placed, &bbox);
        assert_eq!(bounds.len(), 2);
        assert!(bounds.iter().all(|b| (b.x, b.y) == (2, 9)));
    }

    #[test]
    fn test_splitter_topology_item_mismatch_skipped() {
        // Feeder at (1,8) carries a different item — don't emit an
        // input boundary. Output side still OK (same item).
        let placed = vec![
            make_splitter_at(1, 9, EntityDirection::South, "iron-plate"),
            make_surface_belt(1, 8, EntityDirection::South, "copper-plate"), // wrong item
            make_surface_belt(1, 10, EntityDirection::South, "iron-plate"),
        ];
        let bbox = Rect { x: 1, y: 8, w: 5, h: 5 };
        let bounds = splitter_topology_boundaries(&placed, &bbox);
        // Only lane 1 output is wired. Expect 1 IN boundary at (1,9).
        assert_eq!(bounds.len(), 1);
        assert!(bounds[0].is_input);
        assert_eq!((bounds[0].x, bounds[0].y), (1, 9));
    }

    // -- Belt topology helpers ----------------------------------------------

    #[test]
    fn test_belt_topology_feeder_outside_emits_in() {
        // Surface belt at (3,9) South feeder at (3,8), which is OUT of
        // the 3×3 bbox (2,9..4,11). Expect IN (3,9) South.
        let placed = vec![
            make_surface_belt(3, 8, EntityDirection::South, "copper-cable"), // feeder outside bbox
            make_surface_belt(3, 9, EntityDirection::South, "copper-cable"),
            make_surface_belt(3, 10, EntityDirection::South, "copper-cable"), // target inside bbox
        ];
        let bbox = Rect { x: 2, y: 9, w: 3, h: 3 };
        let bounds = belt_topology_boundaries(&placed, &bbox);
        assert_eq!(bounds.len(), 1, "got {bounds:#?}");
        let b = &bounds[0];
        assert_eq!((b.x, b.y), (3, 9));
        assert_eq!(b.direction, EntityDirection::South);
        assert_eq!(b.item, "copper-cable");
        assert!(b.is_input);
        assert!(!b.interior);
    }

    #[test]
    fn test_belt_topology_target_outside_emits_out() {
        // Belt at (3,11) South: target (3,12) is outside bbox. Expect OUT.
        let placed = vec![
            make_surface_belt(3, 10, EntityDirection::South, "copper-cable"),
            make_surface_belt(3, 11, EntityDirection::South, "copper-cable"),
            // (3,12) outside bbox; no entity needed there
        ];
        let bbox = Rect { x: 2, y: 9, w: 3, h: 3 };
        let bounds = belt_topology_boundaries(&placed, &bbox);
        // (3,10) is interior (feeder/target both in bbox)
        // (3,11) emits OUT; and its feeder (3,10) is in bbox so no IN.
        let outs: Vec<_> = bounds.iter().filter(|b| !b.is_input && (b.x, b.y) == (3, 11)).collect();
        assert_eq!(outs.len(), 1, "got {bounds:#?}");
        assert_eq!(outs[0].direction, EntityDirection::South);
    }

    #[test]
    fn test_belt_topology_fully_internal_skipped() {
        // Belt chain entirely inside bbox with feeder+target also inside
        // should contribute no boundaries.
        let placed = vec![
            make_surface_belt(2, 9, EntityDirection::South, "iron-plate"),
            make_surface_belt(2, 10, EntityDirection::South, "iron-plate"),
            make_surface_belt(2, 11, EntityDirection::South, "iron-plate"),
        ];
        // (2,9) feeder is outside bbox, (2,11) target is outside: those
        // two endpoints emit. The middle (2,10) is fully internal.
        let bbox = Rect { x: 2, y: 9, w: 3, h: 3 };
        let bounds = belt_topology_boundaries(&placed, &bbox);
        let middle: Vec<_> = bounds.iter().filter(|b| (b.x, b.y) == (2, 10)).collect();
        assert!(middle.is_empty(), "middle belt should emit nothing, got {bounds:#?}");
    }

    #[test]
    fn test_belt_topology_orphan_no_feeder_skipped() {
        // Belt with no physical feeder (orphan) should NOT emit IN —
        // we don't want to invent entries for broken stamps.
        let placed = vec![
            make_surface_belt(5, 10, EntityDirection::East, "iron-plate"), // no feeder
        ];
        let bbox = Rect { x: 3, y: 9, w: 3, h: 3 };
        let bounds = belt_topology_boundaries(&placed, &bbox);
        let ins: Vec<_> = bounds.iter().filter(|b| b.is_input && (b.x, b.y) == (5, 10)).collect();
        assert!(ins.is_empty(), "orphan belt should not emit IN, got {bounds:#?}");
    }

    #[test]
    fn test_belt_topology_skips_splitters() {
        // Splitters are handled by splitter_topology_boundaries; this
        // helper must not emit for them.
        let placed = vec![
            make_splitter_at(3, 9, EntityDirection::South, "copper-cable"),
        ];
        let bbox = Rect { x: 2, y: 9, w: 3, h: 3 };
        let bounds = belt_topology_boundaries(&placed, &bbox);
        assert!(bounds.is_empty(), "splitter tiles must be skipped, got {bounds:#?}");
    }

    #[test]
    fn test_belt_topology_tier2_scenario() {
        // Tier-2 electronic-circuit 3×3 iter1 bbox (2,9..4,11).
        // Copper flow at column 3 & 4 enters from the north (splitter
        // at (3,8)+(4,8) outside bbox), exits south at (3,11) and east
        // at (4,11). This is what the user's trace reported as buggy.
        let placed = vec![
            // Splitter outside bbox (feeds columns 3 & 4)
            make_splitter_at(3, 8, EntityDirection::South, "copper-cable"),
            // Balancer output belts inside bbox
            make_surface_belt(3, 9, EntityDirection::South, "copper-cable"),
            make_surface_belt(4, 9, EntityDirection::South, "copper-cable"),
            // Trunk belts crossing the zone
            make_surface_belt(3, 10, EntityDirection::South, "copper-cable"),
            make_surface_belt(4, 10, EntityDirection::South, "copper-cable"),
            make_surface_belt(3, 11, EntityDirection::South, "copper-cable"),
            make_surface_belt(4, 11, EntityDirection::East, "copper-cable"),
        ];
        let bbox = Rect { x: 2, y: 9, w: 3, h: 3 };
        let bounds = belt_topology_boundaries(&placed, &bbox);
        let ins: Vec<_> = bounds.iter().filter(|b| b.is_input).collect();
        let outs: Vec<_> = bounds.iter().filter(|b| !b.is_input).collect();
        // Expected: IN (3,9) S, IN (4,9) S, OUT (3,11) S, OUT (4,11) E
        assert_eq!(ins.len(), 2, "ins={ins:#?}");
        assert_eq!(outs.len(), 2, "outs={outs:#?}");
        assert!(ins.iter().any(|b| (b.x, b.y) == (3, 9) && b.direction == EntityDirection::South));
        assert!(ins.iter().any(|b| (b.x, b.y) == (4, 9) && b.direction == EntityDirection::South));
        assert!(outs.iter().any(|b| (b.x, b.y) == (3, 11) && b.direction == EntityDirection::South));
        assert!(outs.iter().any(|b| (b.x, b.y) == (4, 11) && b.direction == EntityDirection::East));
    }
}
