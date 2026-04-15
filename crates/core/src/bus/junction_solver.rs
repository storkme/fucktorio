//! Junction solver: region-growth outer loop + strategy framework.
//!
//! The caller hands us a crossing tile and the set of specs that cross
//! at that tile (usually two, from `classify_crossing`). We build a
//! `GrowingRegion` around the crossing, then iterate: try each
//! registered `JunctionStrategy` on the current region, and if none
//! succeed, walk each participating spec's path one step outward and
//! try again. The loop terminates on success, on a tile-count cap, or
//! when every frontier is exhausted.
//!
//! `GrowingRegion` tracks two things per spec:
//! - a **frontier** `(start_idx, end_idx)` into the spec's routed path,
//!   representing the range of path tiles currently in the region,
//! - and a cached tile set so "is this tile in the region?" is O(1).
//!
//! Each iteration of `grow()` advances both ends of each frontier by
//! one step. The bbox is the tight enclosing rectangle of the accumulated
//! tile set. Tiles inside the bbox that belong to non-participating
//! specs' paths are marked forbidden — strategies must avoid them. If
//! a future strategy wants to also rewrite one of those specs, it can
//! be promoted to participating in a later pass (not yet implemented —
//! single-pass for the scaffold).
//!
//! Strategies consume a `Junction` snapshot (the existing
//! `bus::junction::Junction` type, which is already documented and is
//! the long-term template input). `GrowingRegion::to_junction` builds
//! the snapshot on each strategy call. Strategies also receive the
//! initial crossing tile, the current growth iteration, and references
//! to `routed_paths` + `hard_obstacles` in a context struct so new
//! fields can be added without breaking every impl.
//!
//! This module intentionally knows nothing about `BeltSpec` or any
//! ghost-router internals — strategies live in `ghost_router.rs` where
//! those types are in scope and can drive the existing templates.

use rustc_hash::{FxHashMap, FxHashSet};

use crate::bus::junction::{BeltTier, Junction, Rect, SpecCrossing};
use crate::bus::region_walker::{walk_affected, AffectedPath, ShadowView, WalkResult};
use crate::models::{EntityDirection, PlacedEntity, PortPoint};
use crate::trace::{self, TraceEvent};

/// Growth budget. Small on purpose — this runs per crossing tile and
/// bad inputs shouldn't melt the pipeline. Revisit once templates that
/// exploit growth are in place.
pub const MAX_GROWTH_ITERS: usize = 5;
/// Hard cap on region size. 8×8 = 64 tiles is roughly the largest
/// junction any per-tile template could reasonably stamp. Bigger than
/// this and we're in spec-run overlap territory (Sample C/D/E in the
/// RFP) which needs a different solver.
pub const MAX_REGION_TILES: usize = 64;

/// Mutable state threaded through the growth loop. Not consumed by
/// strategies directly — they see a `Junction` snapshot built via
/// `to_junction`.
pub struct GrowingRegion {
    /// The crossing tile that seeded this region. Kept for trace events
    /// and strategies that want to know where the "original" problem was.
    pub initial_tile: (i32, i32),
    /// Spec keys whose paths are in the region and may be rewritten.
    pub participating: Vec<String>,
    /// Non-participating spec keys whose paths intersect the current
    /// bbox. Their tiles are in `forbidden_tiles` and strategies must
    /// treat them as obstacles.
    pub encountered: Vec<String>,
    /// All tiles currently in the region (union of every participating
    /// spec's frontier range).
    pub tiles: FxHashSet<(i32, i32)>,
    /// Tiles in the bbox that a strategy must not place belts on —
    /// either non-participating spec paths or hard obstacles (machines,
    /// poles, row template belts, etc.) that the caller passed in.
    pub forbidden_tiles: FxHashSet<(i32, i32)>,
    /// Tight enclosing rectangle around `tiles`.
    pub bbox: Rect,
    /// Per-spec path-index range currently included. Inclusive on both
    /// ends.
    frontiers: FxHashMap<String, (usize, usize)>,
}

impl GrowingRegion {
    /// Seed the region with one tile and a list of specs crossing it.
    /// Each spec's frontier starts collapsed at the index of `initial_tile`
    /// in that spec's routed path. Specs whose path doesn't contain the
    /// tile are silently skipped.
    pub fn from_crossing(
        initial_tile: (i32, i32),
        initial_specs: &[&str],
        routed_paths: &FxHashMap<String, Vec<(i32, i32)>>,
        hard_obstacles: &FxHashSet<(i32, i32)>,
        strict_obstacles: &FxHashSet<(i32, i32)>,
    ) -> Self {
        let mut tiles = FxHashSet::default();
        tiles.insert(initial_tile);
        let mut frontiers = FxHashMap::default();
        let mut participating = Vec::new();
        for &key in initial_specs {
            let Some(path) = routed_paths.get(key) else {
                continue;
            };
            let Some(idx) = path.iter().position(|&t| t == initial_tile) else {
                continue;
            };
            frontiers.insert(key.to_string(), (idx, idx));
            participating.push(key.to_string());
        }
        let bbox = Rect {
            x: initial_tile.0,
            y: initial_tile.1,
            w: 1,
            h: 1,
        };
        let mut region = Self {
            initial_tile,
            participating,
            encountered: Vec::new(),
            tiles,
            forbidden_tiles: FxHashSet::default(),
            bbox,
            frontiers,
        };
        region.refresh_forbidden(routed_paths, hard_obstacles, strict_obstacles);
        region
    }

    /// Advance each participating spec's frontier by one step in each
    /// direction along its path. Updates the bbox, the tile set, and
    /// the forbidden cache. Returns `true` if any new tile entered the
    /// region.
    pub fn grow(
        &mut self,
        routed_paths: &FxHashMap<String, Vec<(i32, i32)>>,
        hard_obstacles: &FxHashSet<(i32, i32)>,
        strict_obstacles: &FxHashSet<(i32, i32)>,
    ) -> bool {
        let mut added_any = false;
        let keys: Vec<String> = self.participating.clone();
        for key in &keys {
            let Some(path) = routed_paths.get(key) else {
                continue;
            };
            let Some(&(start, end)) = self.frontiers.get(key) else {
                continue;
            };
            let mut new_start = start;
            let mut new_end = end;
            if start > 0 {
                new_start = start - 1;
                let t = path[new_start];
                if self.tiles.insert(t) {
                    added_any = true;
                }
            }
            if end + 1 < path.len() {
                new_end = end + 1;
                let t = path[new_end];
                if self.tiles.insert(t) {
                    added_any = true;
                }
            }
            self.frontiers.insert(key.clone(), (new_start, new_end));
        }
        if added_any {
            self.recompute_bbox();
            self.refresh_forbidden(routed_paths, hard_obstacles, strict_obstacles);
        }
        added_any
    }

    /// Number of tiles currently in the region. Checked against
    /// `MAX_REGION_TILES` by the outer loop.
    pub fn tile_count(&self) -> usize {
        self.tiles.len()
    }

    fn recompute_bbox(&mut self) {
        let mut min_x = i32::MAX;
        let mut max_x = i32::MIN;
        let mut min_y = i32::MAX;
        let mut max_y = i32::MIN;
        for &(x, y) in &self.tiles {
            if x < min_x {
                min_x = x;
            }
            if x > max_x {
                max_x = x;
            }
            if y < min_y {
                min_y = y;
            }
            if y > max_y {
                max_y = y;
            }
        }
        self.bbox = Rect {
            x: min_x,
            y: min_y,
            w: (max_x - min_x + 1) as u32,
            h: (max_y - min_y + 1) as u32,
        };
    }

    fn refresh_forbidden(
        &mut self,
        routed_paths: &FxHashMap<String, Vec<(i32, i32)>>,
        hard_obstacles: &FxHashSet<(i32, i32)>,
        strict_obstacles: &FxHashSet<(i32, i32)>,
    ) {
        self.forbidden_tiles.clear();
        self.encountered.clear();
        // Walk every tile in the bbox and flag obstacles. Walking the
        // bbox rectangle (not just participating tiles) lets strategies
        // see the full geometry they have to work within. Frontier
        // endpoint tiles (the entry/exit ports for participating specs)
        // are exempted from the obstacle check: a tap-off path's first
        // tile may land on a Permanent splitter on the trunk column,
        // and forbidding it would make the SAT zone infeasible because
        // SAT requires its boundary ports to be free. Interior path
        // tiles are NOT exempted — if a previous strategy iteration
        // stamped a Permanent belt that happens to lie on this spec's
        // interior path, the new strategy must still treat it as an
        // obstacle so it doesn't double-stamp.
        let port_tiles: FxHashSet<(i32, i32)> = self
            .frontiers
            .iter()
            .filter_map(|(key, &(start, end))| {
                routed_paths.get(key).map(|p| (p, start, end))
            })
            .flat_map(|(p, start, end)| [p[start], p[end]])
            .collect();
        for y in self.bbox.y..self.bbox.y + self.bbox.h as i32 {
            for x in self.bbox.x..self.bbox.x + self.bbox.w as i32 {
                if port_tiles.contains(&(x, y)) {
                    continue;
                }
                if hard_obstacles.contains(&(x, y))
                    || strict_obstacles.contains(&(x, y))
                {
                    self.forbidden_tiles.insert((x, y));
                }
            }
        }
        let participating: FxHashSet<&str> = self
            .participating
            .iter()
            .map(|s| s.as_str())
            .collect();
        for (key, path) in routed_paths {
            if participating.contains(key.as_str()) {
                continue;
            }
            let mut encountered = false;
            for &(tx, ty) in path {
                if tx < self.bbox.x
                    || tx >= self.bbox.x + self.bbox.w as i32
                    || ty < self.bbox.y
                    || ty >= self.bbox.y + self.bbox.h as i32
                {
                    continue;
                }
                // Tiles that are *also* on a participating spec's
                // path are conflict tiles, not forbidden. The whole
                // point of growing the region is that the inner
                // solver gets to re-route participating tiles —
                // marking them forbidden would make any port sitting
                // on one infeasible (SAT rejects a zone with a port
                // at a forced-empty tile, which is exactly what
                // happens at the UG-out-axis-conflict shape).
                if self.tiles.contains(&(tx, ty)) {
                    encountered = true;
                    continue;
                }
                self.forbidden_tiles.insert((tx, ty));
                encountered = true;
            }
            if encountered && !self.encountered.iter().any(|k| k == key) {
                self.encountered.push(key.clone());
            }
        }
    }

    /// Promote any encountered spec whose *entire* routed path is already
    /// inside the current tile set. These specs are "fully engulfed" —
    /// every tile they occupy is within the zone bbox — and must be routed
    /// by the SAT solver or they become orphaned crossing specs.
    ///
    /// Typical case: a 1-tile trunk column that sits directly on the path
    /// of a longer tap. The tap's crossing zone grows to include that tile,
    /// which causes the trunk to appear in `encountered`. Without promotion
    /// the SAT doesn't know about the trunk and places an entity that
    /// conflicts with it.
    #[allow(dead_code)]
    pub fn promote_fully_enclosed(
        &mut self,
        routed_paths: &FxHashMap<String, Vec<(i32, i32)>>,
        hard_obstacles: &FxHashSet<(i32, i32)>,
        strict_obstacles: &FxHashSet<(i32, i32)>,
    ) {
        let to_promote: Vec<String> = self
            .encountered
            .iter()
            .filter(|key| {
                routed_paths
                    .get(key.as_str())
                    .is_some_and(|path| path.iter().all(|t| self.tiles.contains(t)))
            })
            .cloned()
            .collect();
        if to_promote.is_empty() {
            return;
        }
        for key in &to_promote {
            if let Some(path) = routed_paths.get(key.as_str()) {
                let start = 0;
                let end = path.len().saturating_sub(1);
                self.frontiers.insert(key.clone(), (start, end));
            }
            self.participating.push(key.clone());
        }
        self.encountered.retain(|k| !to_promote.contains(k));
        self.refresh_forbidden(routed_paths, hard_obstacles, strict_obstacles);
    }

    /// Materialize a `Junction` snapshot suitable for strategy input.
    /// Entry/exit points for each participating spec are the first and
    /// last tiles of its current frontier range, with directions taken
    /// from the adjacent path step. For specs whose entire path is in
    /// the region, we fall back to the in-path step direction at the
    /// endpoint.
    pub fn to_junction(
        &self,
        routed_paths: &FxHashMap<String, Vec<(i32, i32)>>,
        spec_belt_tiers: &FxHashMap<String, BeltTier>,
        spec_items: &FxHashMap<String, String>,
        spec_exit_dirs: &FxHashMap<String, EntityDirection>,
    ) -> Junction {
        let mut specs: Vec<SpecCrossing> = Vec::with_capacity(self.participating.len());
        for key in &self.participating {
            let Some(path) = routed_paths.get(key) else {
                continue;
            };
            let Some(&(start, end)) = self.frontiers.get(key) else {
                continue;
            };
            let dir_hint = spec_exit_dirs.get(key).copied();
            let entry_dir = direction_at(path, start, dir_hint);
            let exit_dir = direction_at(path, end, dir_hint);
            let entry = PortPoint {
                x: path[start].0,
                y: path[start].1,
                direction: entry_dir,
            };
            let exit = PortPoint {
                x: path[end].0,
                y: path[end].1,
                direction: exit_dir,
            };
            let item = spec_items
                .get(key)
                .cloned()
                .unwrap_or_else(|| "?".to_string());
            let belt_tier = spec_belt_tiers
                .get(key)
                .copied()
                .unwrap_or(BeltTier::Yellow);
            specs.push(SpecCrossing {
                item,
                belt_tier,
                entry,
                exit,
            });
        }
        Junction {
            bbox: self.bbox,
            forbidden: self.forbidden_tiles.clone(),
            specs,
        }
    }
}

/// Direction of flow at path index `idx`. Looks at the next step when
/// possible, falling back to the previous step at the tail of the path.
/// `fallback` is used when the path is a single tile (no neighbours to
/// derive direction from) — callers should pass the spec's `exit_dir`.
fn direction_at(
    path: &[(i32, i32)],
    idx: usize,
    fallback: Option<EntityDirection>,
) -> EntityDirection {
    if idx + 1 < path.len() {
        let (x0, y0) = path[idx];
        let (x1, y1) = path[idx + 1];
        step_direction(x1 - x0, y1 - y0)
    } else if idx > 0 {
        let (x0, y0) = path[idx - 1];
        let (x1, y1) = path[idx];
        step_direction(x1 - x0, y1 - y0)
    } else {
        fallback.unwrap_or(EntityDirection::East)
    }
}

fn step_direction(dx: i32, dy: i32) -> EntityDirection {
    if dx > 0 {
        EntityDirection::East
    } else if dx < 0 {
        EntityDirection::West
    } else if dy > 0 {
        EntityDirection::South
    } else {
        EntityDirection::North
    }
}

/// A placed-entity list + the bbox it occupies. Returned by strategies
/// on success.
pub struct JunctionSolution {
    pub entities: Vec<PlacedEntity>,
    pub footprint: Rect,
    /// Kept for future trace instrumentation / diagnostics. Not yet
    /// consumed by the call site — the outer loop already emits a
    /// `JunctionSolved` trace event with the strategy name.
    #[allow(dead_code)]
    pub strategy_name: &'static str,
}

/// Context passed to every strategy call. A struct so fields can be
/// added without breaking every `impl JunctionStrategy`.
pub struct JunctionStrategyContext<'a> {
    pub junction: &'a Junction,
    pub region: &'a GrowingRegion,
    /// Current region-growth iteration. 0 = initial single-tile
    /// crossing. Strategies that want cheap-first, expensive-later
    /// escalation read this; the scaffold wrapper ignores it.
    #[allow(dead_code)]
    pub growth_iter: usize,
    pub routed_paths: &'a FxHashMap<String, Vec<(i32, i32)>>,
    pub hard_obstacles: &'a FxHashSet<(i32, i32)>,
    /// Tiles outside the narrow `hard_obstacles` set that strategies
    /// stamping interior belts (currently SAT) must also avoid: trunk
    /// columns, tap-off splitters, prior template output, row-template
    /// belts. Built from `Occupancy::snapshot_junction_obstacles`. The
    /// perpendicular-template strategy ignores this — it relies on the
    /// `release_for_pertile_template` path to clear trunks/tap-offs out
    /// of its 1×3 footprint and would refuse to fire if it saw them as
    /// obstacles.
    #[allow(dead_code)]
    pub strict_obstacles: &'a FxHashSet<(i32, i32)>,
    /// Entities already placed in Steps 2-5 (row templates, splitter
    /// stamps, balancer blocks, ghost-routed belts). Strategies that
    /// place UG inputs consult this to detect perpendicular sideloads
    /// from splitters or belts whose flow would drop items into the UG
    /// input tile from the wrong side — these sources live in
    /// `placed_entities` but never enter `routed_paths`.
    pub placed_entities: &'a [crate::models::PlacedEntity],
    /// Tiles holding a Permanent / Template / RowEntity / HardObstacle
    /// claim whose segment id is NOT `trunk:*` or `tapoff:*`. These are
    /// the claims `release_for_pertile_template` refuses to clear, so
    /// the perpendicular-template strategy must treat them as obstacles
    /// even though the comment on `strict_obstacles` says the strategy
    /// relies on release-in-footprint for trunk/tapoff cleanup. Computed
    /// once per `solve_crossing` call by the caller.
    pub unreleasable_obstacles: &'a FxHashSet<(i32, i32)>,
}

/// A strategy that attempts to produce a `JunctionSolution` for a
/// given region state. Return `None` to pass to the next strategy or
/// the next growth iteration.
pub trait JunctionStrategy {
    fn name(&self) -> &'static str;
    fn try_solve(&self, ctx: &JunctionStrategyContext) -> Option<JunctionSolution>;
}

/// Outer loop. Builds a `GrowingRegion` from the initial crossing tile
/// and iterates: try every strategy on the current region, grow, repeat.
/// Returns the first successful solution, or `None` if every strategy
/// failed within the growth budget.
#[allow(clippy::too_many_arguments)]
pub fn solve_crossing(
    initial_tile: (i32, i32),
    initial_specs: &[&str],
    routed_paths: &FxHashMap<String, Vec<(i32, i32)>>,
    hard_obstacles: &FxHashSet<(i32, i32)>,
    strict_obstacles: &FxHashSet<(i32, i32)>,
    unreleasable_obstacles: &FxHashSet<(i32, i32)>,
    spec_belt_tiers: &FxHashMap<String, BeltTier>,
    spec_items: &FxHashMap<String, String>,
    spec_exit_dirs: &FxHashMap<String, EntityDirection>,
    placed_entities: &[crate::models::PlacedEntity],
    strategies: &[&dyn JunctionStrategy],
    // All crossing tiles in the layout. Used to detect when a spec's
    // frontier exit lands on another unresolved crossing — in that case
    // the zone defers (grows one more step) so the solution exits beyond
    // all consecutive crossings rather than stopping mid-run.
    all_crossings: &FxHashSet<(i32, i32)>,
) -> Option<JunctionSolution> {
    let mut region = GrowingRegion::from_crossing(
        initial_tile,
        initial_specs,
        routed_paths,
        hard_obstacles,
        strict_obstacles,
    );

    for iter in 0..MAX_GROWTH_ITERS {
        let junction = region.to_junction(routed_paths, spec_belt_tiers, spec_items, spec_exit_dirs);
        let ctx = JunctionStrategyContext {
            junction: &junction,
            region: &region,
            growth_iter: iter,
            routed_paths,
            hard_obstacles,
            strict_obstacles,
            placed_entities,
            unreleasable_obstacles,
        };
        for strategy in strategies {
            if let Some(sol) = strategy.try_solve(&ctx) {
                // Deferred-exit check: if any participating spec currently
                // exits at another unresolved crossing tile (not the initial
                // tile), the solution would leave a consecutive crossing
                // unresolved. Grow one more step so the spec's frontier
                // extends past all consecutive crossings before we commit.
                let exits_at_crossing = ctx.junction.specs.iter().any(|s| {
                    let exit = (s.exit.x, s.exit.y);
                    exit != initial_tile && all_crossings.contains(&exit)
                });
                if exits_at_crossing {
                    break; // skip this iter's solution; fall through to grow
                }

                // Walker veto: reject solutions that would break a routed
                // path whose tiles touch the region's bbox (or its 1-tile
                // perimeter). Catches the tier2_electronic_circuit class
                // where SAT is locally valid but breaks a perpendicular
                // trunk the region was unaware of.
                let bbox = region.bbox;
                let released = bbox_tiles_set(bbox);
                let affected: Vec<AffectedPath<'_>> = routed_paths
                    .iter()
                    .filter_map(|(seg, tiles)| {
                        if tiles.iter().any(|&t| near_bbox(bbox, t)) {
                            let item = spec_items
                                .get(seg)
                                .map(|s| s.as_str())
                                .unwrap_or("");
                            Some(AffectedPath {
                                segment_id: seg.as_str(),
                                tiles: tiles.as_slice(),
                                item,
                            })
                        } else {
                            None
                        }
                    })
                    .collect();
                let shadow = ShadowView::build(placed_entities, &released, &sol.entities);
                if let WalkResult::Broken { breaks } = walk_affected(&affected, &shadow) {
                    if let Some(first) = breaks.first() {
                        trace::emit(TraceEvent::RegionWalkerVeto {
                            tile_x: initial_tile.0,
                            tile_y: initial_tile.1,
                            strategy: strategy.name().to_string(),
                            growth_iter: iter,
                            broken_segment: first.segment_id.clone(),
                            break_tile_x: first.tile.0,
                            break_tile_y: first.tile.1,
                            break_count: breaks.len(),
                        });
                    }
                    continue; // try the next strategy; if all fail, grow
                }

                trace::emit(TraceEvent::JunctionSolved {
                    tile_x: initial_tile.0,
                    tile_y: initial_tile.1,
                    strategy: strategy.name().to_string(),
                    growth_iter: iter,
                    region_tiles: region.tile_count(),
                });
                return Some(sol);
            }
        }

        if region.tile_count() >= MAX_REGION_TILES {
            trace::emit(TraceEvent::JunctionGrowthCapped {
                tile_x: initial_tile.0,
                tile_y: initial_tile.1,
                iters: iter,
                region_tiles: region.tile_count(),
                reason: "tile_cap".to_string(),
            });
            return None;
        }
        if !region.grow(routed_paths, hard_obstacles, strict_obstacles) {
            trace::emit(TraceEvent::JunctionGrowthCapped {
                tile_x: initial_tile.0,
                tile_y: initial_tile.1,
                iters: iter,
                region_tiles: region.tile_count(),
                reason: "frontier_exhausted".to_string(),
            });
            return None;
        }
    }

    trace::emit(TraceEvent::JunctionGrowthCapped {
        tile_x: initial_tile.0,
        tile_y: initial_tile.1,
        iters: MAX_GROWTH_ITERS,
        region_tiles: region.tile_count(),
        reason: "iter_cap".to_string(),
    });
    None
}

/// Every tile inside `bbox` (inclusive on the min side, exclusive on the
/// max side, per `Rect`'s convention). Used as the "released set" when
/// building a walker shadow view — a strategy's solution is authoritative
/// over every tile in its footprint.
fn bbox_tiles_set(bbox: Rect) -> FxHashSet<(i32, i32)> {
    let mut out = FxHashSet::default();
    for dy in 0..bbox.h as i32 {
        for dx in 0..bbox.w as i32 {
            out.insert((bbox.x + dx, bbox.y + dy));
        }
    }
    out
}

/// True iff `(x, y)` falls inside `bbox` expanded by one tile on each
/// side. The walker uses this to decide which routed paths to check —
/// anything one tile beyond the bbox can still interact with the
/// region's boundary entities (e.g. a sideload onto a UG input).
fn near_bbox(bbox: Rect, (x, y): (i32, i32)) -> bool {
    let min_x = bbox.x - 1;
    let min_y = bbox.y - 1;
    let max_x = bbox.x + bbox.w as i32; // inclusive upper bound with +1 perimeter
    let max_y = bbox.y + bbox.h as i32;
    x >= min_x && x <= max_x && y >= min_y && y <= max_y
}
