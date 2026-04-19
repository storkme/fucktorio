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

use crate::bus::junction::{BeltTier, Junction, Rect, SpecCrossing, SpecOrigin};
use crate::bus::region_walker::{walk_affected, AffectedPath, ShadowView, WalkResult};
use crate::common::{
    balancer_seg_is_simple, is_machine_entity, is_splitter, is_surface_belt, is_ug_belt,
    machine_size, machine_tiles, splitter_second_tile, tile_is_forbidden_kind,
};
use crate::models::{EntityDirection, PlacedEntity, PortPoint};
use crate::trace::{
    self, BoundarySnapshot, ExternalFeederSnapshot, ParticipatingSpec, StampedNeighbor,
    TraceEvent,
};

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
#[derive(Clone)]
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
    /// Seed the region from a cluster of adjacent crossing tiles. The
    /// initial bbox is the min-rect containing every seed (filled into
    /// `tiles` to match `expand_bbox`'s rectangle invariant). Each
    /// spec's frontier is seeded at the first tile in its path that
    /// falls inside the initial bbox; specs whose path doesn't enter
    /// the bbox are silently skipped.
    ///
    /// `seeds` must be non-empty. `initial_tile` is set to `seeds[0]`
    /// for trace-event and `all_crossings` deferred-exit purposes —
    /// cluster members other than the first are still recognized via
    /// `all_crossings` lookups, which don't require the seed to be the
    /// representative tile.
    pub fn from_crossings(
        seeds: &[(i32, i32)],
        initial_specs: &[&str],
        routed_paths: &FxHashMap<String, Vec<(i32, i32)>>,
        hard_obstacles: &FxHashSet<(i32, i32)>,
        strict_obstacles: &FxHashSet<(i32, i32)>,
        placed_entities: &[PlacedEntity],
    ) -> Self {
        assert!(!seeds.is_empty(), "from_crossings: seeds must be non-empty");
        let min_x = seeds.iter().map(|(x, _)| *x).min().unwrap();
        let max_x = seeds.iter().map(|(x, _)| *x).max().unwrap();
        let min_y = seeds.iter().map(|(_, y)| *y).min().unwrap();
        let max_y = seeds.iter().map(|(_, y)| *y).max().unwrap();
        let bbox = Rect {
            x: min_x,
            y: min_y,
            w: (max_x - min_x + 1) as u32,
            h: (max_y - min_y + 1) as u32,
        };

        let mut tiles = FxHashSet::default();
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                tiles.insert((x, y));
            }
        }

        let mut frontiers = FxHashMap::default();
        let mut participating = Vec::new();
        for &key in initial_specs {
            let Some(path) = routed_paths.get(key) else {
                continue;
            };
            let Some(idx) = path.iter().position(|&t| bbox.contains(t.0, t.1)) else {
                continue;
            };
            frontiers.insert(key.to_string(), (idx, idx));
            participating.push(key.to_string());
        }
        let mut region = Self {
            initial_tile: seeds[0],
            participating,
            encountered: Vec::new(),
            tiles,
            forbidden_tiles: FxHashSet::default(),
            bbox,
            frontiers,
        };
        region.refresh_forbidden(
            routed_paths,
            hard_obstacles,
            strict_obstacles,
            placed_entities,
        );
        region
    }

    /// Expand the region's bbox by the given per-side deltas, absorb
    /// every tile inside the new rectangle into the region, and
    /// recompute the frontiers of already-participating specs. A
    /// non-participating spec is *only* promoted when its path
    /// genuinely crosses another participating spec inside the new
    /// bbox (i.e. they share an interior tile). Specs that just happen
    /// to pass through the bbox without touching any participating
    /// path are left as obstacles — promoting them would over-constrain
    /// the SAT encoder with specs that aren't part of the actual
    /// crossing being resolved.
    ///
    /// This is the growth primitive used by the CEGAR loop: unlike
    /// `grow()` (which walks each spec's frontier by one step along
    /// its own axis), `expand_bbox` grows perpendicular to spec axes
    /// and can absorb perpendicular trunks the seed crossing had never
    /// heard of — the copper-cable column-4 trunk in the
    /// tier2_electronic_circuit case being the canonical example.
    ///
    /// Returns `true` if the bbox changed.
    pub fn expand_bbox(
        &mut self,
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
        routed_paths: &FxHashMap<String, Vec<(i32, i32)>>,
        hard_obstacles: &FxHashSet<(i32, i32)>,
        strict_obstacles: &FxHashSet<(i32, i32)>,
        placed_entities: &[PlacedEntity],
        protected_balancer_tiles: &FxHashSet<(i32, i32)>,
    ) -> bool {
        // Clamp each side so growth never crosses into a multi-splitter
        // balancer bbox. A side is zeroed if any tile in its new strip
        // overlaps a protected tile — the balancer becomes a hard wall
        // for the junction region. Simple single-splitter balancers are
        // not in the protected set so growth still flows through them.
        let left = clamp_side_against_protected(
            left.max(0),
            &self.bbox,
            ExpandSide::Left,
            protected_balancer_tiles,
        );
        let top = clamp_side_against_protected(
            top.max(0),
            &self.bbox,
            ExpandSide::Top,
            protected_balancer_tiles,
        );
        let right = clamp_side_against_protected(
            right.max(0),
            &self.bbox,
            ExpandSide::Right,
            protected_balancer_tiles,
        );
        let bottom = clamp_side_against_protected(
            bottom.max(0),
            &self.bbox,
            ExpandSide::Bottom,
            protected_balancer_tiles,
        );
        if left == 0 && top == 0 && right == 0 && bottom == 0 {
            return false;
        }
        let new_x = self.bbox.x - left;
        let new_y = self.bbox.y - top;
        let new_w = self.bbox.w + (left + right) as u32;
        let new_h = self.bbox.h + (top + bottom) as u32;
        self.bbox = Rect {
            x: new_x,
            y: new_y,
            w: new_w,
            h: new_h,
        };

        // Absorb every tile in the new bbox into the region's tile set.
        for dy in 0..new_h as i32 {
            for dx in 0..new_w as i32 {
                self.tiles.insert((new_x + dx, new_y + dy));
            }
        }

        let bbox = Rect {
            x: new_x,
            y: new_y,
            w: new_w,
            h: new_h,
        };
        let in_bbox = |tx: i32, ty: i32| -> bool { bbox.contains(tx, ty) };
        // Both loops below want "genuine" crossings (≥2 distinct
        // in-bbox tiles), so apply the strict `s < e` filter inline.
        let strict_range = |path: &[(i32, i32)]| -> Option<(usize, usize)> {
            path_bbox_range(path, &bbox).filter(|(s, e)| s < e)
        };

        // 1. Update frontiers for existing participating specs. Keep
        //    any spec whose in-bbox range is at least 2 tiles and has
        //    non-collapsed start < end.
        let mut new_frontiers: FxHashMap<String, (usize, usize)> = FxHashMap::default();
        let mut kept: Vec<String> = Vec::new();
        for key in &self.participating {
            if let Some(path) = routed_paths.get(key) {
                if let Some(range) = strict_range(path) {
                    new_frontiers.insert(key.clone(), range);
                    kept.push(key.clone());
                    continue;
                }
            }
            // Spec lost its in-bbox presence (shouldn't happen during
            // monotonic growth, but be defensive): drop it.
        }

        // 2. Promote a non-participating spec only if its path shares
        //    at least one in-bbox tile with an existing participating
        //    spec — that tile is the genuine crossing we want SAT to
        //    solve jointly. A spec whose path merely passes through
        //    the bbox without touching any participating path is left
        //    alone.
        let kept_tiles: FxHashSet<(i32, i32)> = kept
            .iter()
            .filter_map(|k| routed_paths.get(k))
            .flat_map(|p| {
                p.iter().copied().filter(|&(tx, ty)| in_bbox(tx, ty))
            })
            .collect();
        let kept_set: FxHashSet<&str> = kept.iter().map(|s| s.as_str()).collect();
        let mut promoted: Vec<String> = Vec::new();
        for (key, path) in routed_paths {
            if kept_set.contains(key.as_str()) {
                continue;
            }
            let Some(range) = strict_range(path) else {
                continue;
            };
            let (start, end) = range;
            let touches_participating = (start..=end)
                .any(|i| kept_tiles.contains(&path[i]));
            if !touches_participating {
                continue;
            }
            new_frontiers.insert(key.clone(), range);
            promoted.push(key.clone());
        }

        self.participating = kept;
        for key in promoted {
            self.participating.push(key);
        }
        self.frontiers = new_frontiers;
        self.encountered.retain(|k| !self.participating.contains(k));

        self.refresh_forbidden(
            routed_paths,
            hard_obstacles,
            strict_obstacles,
            placed_entities,
        );
        true
    }

    /// Advance each participating spec's frontier by one step in each
    /// direction along its path. Updates the bbox, the tile set, and
    /// the forbidden cache. Returns `true` if any new tile entered the
    /// region.
    #[allow(dead_code)]
    pub fn grow(
        &mut self,
        routed_paths: &FxHashMap<String, Vec<(i32, i32)>>,
        hard_obstacles: &FxHashSet<(i32, i32)>,
        strict_obstacles: &FxHashSet<(i32, i32)>,
        placed_entities: &[PlacedEntity],
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
            self.refresh_forbidden(
                routed_paths,
                hard_obstacles,
                strict_obstacles,
                placed_entities,
            );
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
        placed_entities: &[PlacedEntity],
    ) {
        self.forbidden_tiles.clear();
        self.encountered.clear();

        // Build a tile → entity-name index from the current layout.
        // Used to classify strict-obstacle tiles: simple surface belts
        // are *permissive* inside a SAT zone (SAT is free to lift and
        // re-stamp them). Splitters, UG entrances/exits, inserters,
        // machines, poles, pipes — all forbidden.
        let tile_kind = build_tile_kind_map(placed_entities, &self.bbox);

        // First pass: determine encountered specs (non-participating
        // specs whose path crosses the current bbox). Each contributes
        // entry + exit endpoints to the port-exemption set, so that
        // SAT can place boundary-port entities there.
        let participating: FxHashSet<&str> =
            self.participating.iter().map(|s| s.as_str()).collect();
        for (key, path) in routed_paths {
            if participating.contains(key.as_str()) {
                continue;
            }
            if path_bbox_range(path, &self.bbox).is_some()
                && !self.encountered.iter().any(|k| k == key)
            {
                self.encountered.push(key.clone());
            }
        }

        // Port tiles are exempt from forbidden. Participating frontiers
        // contribute their current entry+exit; encountered specs
        // contribute their first+last in-bbox tiles. Without this, SAT
        // rejects zones whose boundary ports land on tiles occupied by
        // Permanent feeders.
        let mut port_tiles: FxHashSet<(i32, i32)> = self
            .frontiers
            .iter()
            .filter_map(|(key, &(start, end))| {
                routed_paths.get(key).map(|p| (p, start, end))
            })
            .flat_map(|(p, start, end)| [p[start], p[end]])
            .collect();
        for key in &self.encountered {
            if let Some(path) = routed_paths.get(key) {
                if let Some((start, end)) = path_bbox_range(path, &self.bbox) {
                    port_tiles.insert(path[start]);
                    port_tiles.insert(path[end]);
                }
            }
        }

        // Walk every tile in the bbox and flag obstacles. A tile in
        // either obstacle set is forbidden iff the entity occupying it
        // is NOT a simple surface belt. Surface belts become
        // SAT-routable: SAT may re-stamp them as belts, UGs, or
        // different directions. Both `hard_obstacles` and
        // `strict_obstacles` can contain belt tiles — the ghost router
        // dumps splitter-output belts, balancer belts, and row-template
        // belts into one or the other — so we apply the same filter to
        // both. Port tiles are always exempt (a boundary port landing
        // on a Permanent would otherwise be infeasible). Tiles in an
        // obstacle set with NO matching entity in `tile_kind` default
        // forbidden (conservative).
        for y in self.bbox.y..self.bbox.y + self.bbox.h as i32 {
            for x in self.bbox.x..self.bbox.x + self.bbox.w as i32 {
                let t = (x, y);
                if port_tiles.contains(&t) {
                    continue;
                }
                let in_any_obstacle =
                    hard_obstacles.contains(&t) || strict_obstacles.contains(&t);
                if !in_any_obstacle {
                    continue;
                }
                let forbidden_kind = tile_kind
                    .get(&t)
                    .map(|n| tile_is_forbidden_kind(n))
                    .unwrap_or(true);
                if forbidden_kind {
                    self.forbidden_tiles.insert(t);
                }
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
        placed_entities: &[PlacedEntity],
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
        self.refresh_forbidden(
            routed_paths,
            hard_obstacles,
            strict_obstacles,
            placed_entities,
        );
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
            // Use arrival direction for the entry tile: items are already
            // traveling in path[start-1]→path[start] direction when they
            // reach the zone boundary. Departure (path[start]→path[start+1])
            // is wrong at corners — e.g. a splitter above the entry tile
            // outputs South but departure would say East, causing SAT to
            // place a UG-East-In that the splitter can't feed correctly.
            let entry_dir = if start > 0 {
                direction_at(path, start - 1, dir_hint)
            } else {
                direction_at(path, start, dir_hint)
            };
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
                origin: SpecOrigin::Participating,
            });
        }
        // Encountered specs: paths that cross the bbox but didn't seed
        // this cluster. Each gets a pair of SAT boundaries (IN where
        // the path first enters the bbox, OUT where it last exits) so
        // SAT can route the item through — instead of treating the
        // path tiles as forbidden obstacles. Note: if the path merely
        // touches the bbox at a single tile we skip it (no meaningful
        // crossing to route). Paths that zig-zag in and out of the
        // bbox get one IN/OUT pair spanning first→last in-bbox
        // indices; intermediate excursions are collapsed (acceptable —
        // SAT isn't obliged to mirror the ghost-router path exactly).
        for key in &self.encountered {
            let Some(path) = routed_paths.get(key) else {
                continue;
            };
            let Some((start, end)) = path_bbox_range(path, &self.bbox) else {
                continue;
            };
            if start >= end {
                continue;
            }
            let dir_hint = spec_exit_dirs.get(key).copied();
            let entry_dir = if start > 0 {
                direction_at(path, start - 1, dir_hint)
            } else {
                direction_at(path, start, dir_hint)
            };
            let exit_dir = direction_at(path, end, dir_hint);
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
                entry: PortPoint {
                    x: path[start].0,
                    y: path[start].1,
                    direction: entry_dir,
                },
                exit: PortPoint {
                    x: path[end].0,
                    y: path[end].1,
                    direction: exit_dir,
                },
                origin: SpecOrigin::Encountered,
            });
        }
        Junction {
            bbox: self.bbox,
            forbidden: self.forbidden_tiles.clone(),
            specs,
        }
    }
}

/// Build a sparse (tile → entity-name) lookup covering every tile in
/// `bbox` that a `PlacedEntity` from `placed_entities` occupies.
/// Splitters contribute both tiles; machines contribute every tile in
/// their footprint; other entities occupy a single tile. Tiles with
/// no matching entity are absent from the map — callers treat those
/// as either "no obstacle" or "unknown default-forbidden" per context.
/// Which side of a bbox an `expand_bbox` request is growing into.
#[derive(Clone, Copy)]
enum ExpandSide {
    Left,
    Top,
    Right,
    Bottom,
}

/// Reduce `amount` until the new strip of tiles on `side` of `bbox`
/// contains no tile from `protected`. The strip grows outward from the
/// current bbox edge; if the tile immediately outside the edge is
/// protected, the side is clamped to 0. Amount is clamped rather than
/// shrunk in steps because expansion is usually `1` per call.
fn clamp_side_against_protected(
    amount: i32,
    bbox: &Rect,
    side: ExpandSide,
    protected: &FxHashSet<(i32, i32)>,
) -> i32 {
    if amount <= 0 || protected.is_empty() {
        return amount.max(0);
    }
    let (x0, y0, w, h) = (bbox.x, bbox.y, bbox.w as i32, bbox.h as i32);
    for step in 1..=amount {
        let hit = match side {
            ExpandSide::Left => {
                let x = x0 - step;
                (y0..y0 + h).any(|y| protected.contains(&(x, y)))
            }
            ExpandSide::Right => {
                let x = x0 + w + (step - 1);
                (y0..y0 + h).any(|y| protected.contains(&(x, y)))
            }
            ExpandSide::Top => {
                let y = y0 - step;
                (x0..x0 + w).any(|x| protected.contains(&(x, y)))
            }
            ExpandSide::Bottom => {
                let y = y0 + h + (step - 1);
                (x0..x0 + w).any(|x| protected.contains(&(x, y)))
            }
        };
        if hit {
            return step - 1;
        }
    }
    amount
}

/// Collect every tile occupied by a multi-splitter balancer (shape
/// where n>2 or m>2 in `balancer:{item}:{n}x{m}`). These tiles are
/// treated as hard boundaries by the junction growth loop so SAT never
/// gets to re-route the balancer internals. Simple single-splitter
/// balancers (1x1 / 1x2 / 2x1 / 2x2) are omitted — growth is still
/// allowed to absorb them.
pub(crate) fn build_protected_balancer_tiles(
    placed_entities: &[PlacedEntity],
) -> FxHashSet<(i32, i32)> {
    let mut out: FxHashSet<(i32, i32)> = FxHashSet::default();
    for e in placed_entities {
        let Some(seg) = e.segment_id.as_deref() else {
            continue;
        };
        if !seg.starts_with("balancer:") {
            continue;
        }
        if balancer_seg_is_simple(seg) {
            continue;
        }
        if is_splitter(&e.name) {
            let (sx, sy) = splitter_second_tile(e);
            out.insert((e.x, e.y));
            out.insert((sx, sy));
        } else if is_machine_entity(&e.name) {
            let size = machine_size(&e.name);
            for t in machine_tiles(e.x, e.y, size) {
                out.insert(t);
            }
        } else {
            out.insert((e.x, e.y));
        }
    }
    out
}

pub(crate) fn build_tile_kind_map<'a>(
    placed_entities: &'a [PlacedEntity],
    bbox: &Rect,
) -> FxHashMap<(i32, i32), &'a str> {
    let mut tile_kind: FxHashMap<(i32, i32), &'a str> = FxHashMap::default();
    let mut insert = |t: (i32, i32), name: &'a str| {
        if bbox.contains(t.0, t.1) {
            tile_kind.insert(t, name);
        }
    };
    for e in placed_entities {
        let name = e.name.as_str();
        if is_splitter(name) {
            let (sx, sy) = splitter_second_tile(e);
            insert((e.x, e.y), name);
            insert((sx, sy), name);
        } else if is_machine_entity(name) {
            // Assembling machines / furnaces / chemical plants / etc.
            // expand to their full footprint. machine_size has a
            // defaults-to-3 fallback for unknown names, so guard with
            // is_machine_entity to avoid treating belts as 3x3.
            let size = machine_size(name);
            for t in machine_tiles(e.x, e.y, size) {
                insert(t, name);
            }
        } else {
            // Single-tile entities: belts, UG-in/out, inserters, poles,
            // pipes, beacons, etc.
            insert((e.x, e.y), name);
        }
    }
    tile_kind
}

/// First and last indices where `path` intersects `bbox`. Returns
/// `None` if the path never enters the bbox. Includes single-tile
/// crossings (start == end); callers that need strict crossings (at
/// least two distinct in-bbox tiles) should check `end > start`
/// themselves.
pub(crate) fn path_bbox_range(
    path: &[(i32, i32)],
    bbox: &Rect,
) -> Option<(usize, usize)> {
    let mut first: Option<usize> = None;
    let mut last: Option<usize> = None;
    for (i, &(tx, ty)) in path.iter().enumerate() {
        if bbox.contains(tx, ty) {
            if first.is_none() {
                first = Some(i);
            }
            last = Some(i);
        }
    }
    match (first, last) {
        (Some(s), Some(e)) => Some((s, e)),
        _ => None,
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
    /// Spec keys whose paths this strategy claims authority over. The
    /// ghost-router uses this to release every tile of every participating
    /// spec that falls inside `footprint`, so stale ghost-surface belts
    /// from the pre-solve A* don't leak into the final entity list.
    /// Empty for strategies whose footprint is narrow enough that residue
    /// can't form (e.g. the perpendicular 1×3 template).
    pub participating: Vec<String>,
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
    /// Variant label for the current attempt. Empty string for the
    /// primary attempt on the current region; non-empty for the
    /// speculative one-side expansion variants
    /// ("variant-west"/"-north"/"-east"/"-south"). Strategies forward
    /// this to their `SatInvocation` trace event so the debugger can
    /// disambiguate variant attempts that share an iter number.
    pub growth_variant: &'a str,
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

/// Outer loop. Builds a `GrowingRegion` from a cluster of one or more
/// crossing tiles and iterates: try every strategy on the current
/// region, grow, repeat. Returns the first successful solution, or
/// `None` if every strategy failed within the growth budget.
///
/// `seeds` must be non-empty. The first seed is used as the
/// representative tile for trace events and the `all_crossings`
/// deferred-exit check; other seeds are included in the initial
/// bbox but do not receive special treatment.
#[allow(clippy::too_many_arguments)]
pub fn solve_crossing(
    seeds: &[(i32, i32)],
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
    assert!(!seeds.is_empty(), "solve_crossing: seeds must be non-empty");
    let initial_tile = seeds[0];
    let mut region = GrowingRegion::from_crossings(
        seeds,
        initial_specs,
        routed_paths,
        hard_obstacles,
        strict_obstacles,
        placed_entities,
    );

    // Emit start-of-solve snapshot: seed, participating specs, and
    // stamped entities within the seed's 1-tile perimeter. This gives
    // the replay tool everything needed to understand the initial
    // conditions before any growth happens.
    {
        let participating: Vec<ParticipatingSpec> = region
            .participating
            .iter()
            .filter_map(|key| {
                let path = routed_paths.get(key)?;
                let (start, end) = *region.frontiers.get(key)?;
                let (ix, iy) = path[start];
                let item = spec_items
                    .get(key)
                    .cloned()
                    .unwrap_or_else(|| "?".to_string());
                Some(ParticipatingSpec {
                    key: key.clone(),
                    item,
                    initial_tile_x: ix,
                    initial_tile_y: iy,
                    path_len: path.len(),
                    initial_start: start,
                    initial_end: end,
                })
            })
            .collect();
        let nearby_stamped = collect_nearby_stamped(initial_tile, placed_entities);
        trace::emit(TraceEvent::JunctionGrowthStarted {
            seed_x: initial_tile.0,
            seed_y: initial_tile.1,
            participating,
            nearby_stamped,
        });
    }

    let protected_balancer_tiles = build_protected_balancer_tiles(placed_entities);
    let solve_ctx = SolveCtx {
        initial_tile,
        routed_paths,
        spec_belt_tiers,
        spec_items,
        spec_exit_dirs,
        hard_obstacles,
        strict_obstacles,
        unreleasable_obstacles,
        placed_entities,
        strategies,
        all_crossings,
        protected_balancer_tiles: &protected_balancer_tiles,
    };

    for iter in 0..MAX_GROWTH_ITERS {
        // Collect every walker-valid candidate across the primary
        // region and the four single-side expansions. The cheapest by
        // `junction_cost::solution_cost` wins — no early-return on
        // first-success, so a primary-region UG-heavy layout can be
        // beaten by a cheaper variant-east surface layout. Only if
        // every variant fails do we fall through to uniform growth.
        //
        // Variants run sequentially (SAT on these zones is low-ms) and
        // order is fixed so the trace is deterministic.
        let mut candidates: Vec<(u32, JunctionSolution, String)> = Vec::new();

        if let TryOutcome::Solved(sol) =
            try_solve_on_region(&region, iter, None, &solve_ctx)
        {
            let c = crate::bus::junction_cost::solution_cost(&sol.entities);
            candidates.push((c, sol, String::new()));
        }

        if region.tile_count() >= MAX_REGION_TILES {
            if let Some(best) = pick_cheapest_candidate(
                &mut candidates,
                initial_tile,
                iter,
                region.tile_count(),
            ) {
                return Some(best);
            }
            trace::emit(TraceEvent::JunctionGrowthCapped {
                tile_x: initial_tile.0,
                tile_y: initial_tile.1,
                iters: iter,
                region_tiles: region.tile_count(),
                reason: "tile_cap".to_string(),
            });
            return None;
        }

        for (label, (left, top, right, bottom)) in SINGLE_SIDE_VARIANTS {
            let mut variant = region.clone();
            let changed = variant.expand_bbox(
                *left,
                *top,
                *right,
                *bottom,
                routed_paths,
                hard_obstacles,
                strict_obstacles,
                placed_entities,
                solve_ctx.protected_balancer_tiles,
            );
            if !changed {
                continue;
            }
            if variant.tile_count() > MAX_REGION_TILES {
                continue;
            }
            if let TryOutcome::Solved(sol) =
                try_solve_on_region(&variant, iter, Some(label), &solve_ctx)
            {
                let c = crate::bus::junction_cost::solution_cost(&sol.entities);
                candidates.push((c, sol, (*label).to_string()));
            }
        }

        if let Some(best) = pick_cheapest_candidate(
            &mut candidates,
            initial_tile,
            iter,
            region.tile_count(),
        ) {
            return Some(best);
        }

        // Fallback: uniform +1 on all sides. Absorbs perpendicular
        // trunks/taps the one-side variants wouldn't have seen. When
        // none of the single-side variants solve, we need joint
        // decisions across the wider region.
        if !region.expand_bbox(
            1,
            1,
            1,
            1,
            routed_paths,
            hard_obstacles,
            strict_obstacles,
            placed_entities,
            solve_ctx.protected_balancer_tiles,
        ) {
            trace::emit(TraceEvent::JunctionGrowthCapped {
                tile_x: initial_tile.0,
                tile_y: initial_tile.1,
                iters: iter,
                region_tiles: region.tile_count(),
                reason: "bbox_expand_failed".to_string(),
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

/// Four single-side +1 expansions. Each tuple is `(label, (left, top,
/// right, bottom))`. Applied in a fixed order so the trace event's
/// `considered` list is deterministic — ties in cost resolve to the
/// earliest candidate to appear.
const SINGLE_SIDE_VARIANTS: &[(&str, (i32, i32, i32, i32))] = &[
    ("variant-west", (1, 0, 0, 0)),
    ("variant-north", (0, 1, 0, 0)),
    ("variant-east", (0, 0, 1, 0)),
    ("variant-south", (0, 0, 0, 1)),
];

/// Pop the cheapest `JunctionSolution` from `candidates` (if any),
/// emit the terminal trace events for the winner, and return it. On
/// ties (same cost), the earliest candidate to have been pushed wins.
/// Returns `None` when the candidate list is empty.
fn pick_cheapest_candidate(
    candidates: &mut Vec<(u32, JunctionSolution, String)>,
    initial_tile: (i32, i32),
    iter: usize,
    region_tiles: usize,
) -> Option<JunctionSolution> {
    if candidates.is_empty() {
        return None;
    }
    let considered: Vec<(String, u32)> = candidates
        .iter()
        .map(|(c, _, label)| (label.clone(), *c))
        .collect();
    // `min_by_key` is stable in std: on equal keys the first one wins.
    let winner_idx = candidates
        .iter()
        .enumerate()
        .min_by_key(|(_, (c, _, _))| *c)
        .map(|(i, _)| i)?;
    let (winner_cost, winner_sol, winner_label) = candidates.swap_remove(winner_idx);
    trace::emit(TraceEvent::JunctionVariantChosen {
        tile_x: initial_tile.0,
        tile_y: initial_tile.1,
        iter,
        variant: winner_label.clone(),
        cost: winner_cost,
        considered,
    });
    trace::emit(TraceEvent::JunctionSolved {
        tile_x: initial_tile.0,
        tile_y: initial_tile.1,
        strategy: winner_sol.strategy_name.to_string(),
        growth_iter: iter,
        region_tiles,
    });
    Some(winner_sol)
}

/// Immutable references threaded through the growth loop. Extracted so
/// `try_solve_on_region` can be called per-iter and per-variant without
/// dragging a dozen parameters through each call.
struct SolveCtx<'a> {
    initial_tile: (i32, i32),
    routed_paths: &'a FxHashMap<String, Vec<(i32, i32)>>,
    spec_belt_tiers: &'a FxHashMap<String, BeltTier>,
    spec_items: &'a FxHashMap<String, String>,
    spec_exit_dirs: &'a FxHashMap<String, EntityDirection>,
    hard_obstacles: &'a FxHashSet<(i32, i32)>,
    strict_obstacles: &'a FxHashSet<(i32, i32)>,
    unreleasable_obstacles: &'a FxHashSet<(i32, i32)>,
    placed_entities: &'a [crate::models::PlacedEntity],
    strategies: &'a [&'a dyn JunctionStrategy],
    all_crossings: &'a FxHashSet<(i32, i32)>,
    protected_balancer_tiles: &'a FxHashSet<(i32, i32)>,
}

/// Outcome of one strategy attempt on a given region state.
enum TryOutcome {
    Solved(JunctionSolution),
    Continue,
}

/// Snapshot the iteration, run every strategy on the region, and apply
/// the walker veto. Mirrors the original in-loop body so the outer
/// growth loop can call it per-iter *and* per-variant.
///
/// `variant` identifies the specific shape being tried when multiple
/// candidates share an `iter` number (for trace disambiguation). `None`
/// means "this is the primary attempt for the iter, no variant".
fn try_solve_on_region(
    region: &GrowingRegion,
    iter: usize,
    variant: Option<&str>,
    ctx: &SolveCtx,
) -> TryOutcome {
    let junction = region.to_junction(
        ctx.routed_paths,
        ctx.spec_belt_tiers,
        ctx.spec_items,
        ctx.spec_exit_dirs,
    );

    let boundaries = junction_boundaries_to_snapshots(
        &junction,
        &region.participating,
        &region.encountered,
        ctx.placed_entities,
    );
    let mut tiles: Vec<(i32, i32)> = region.tiles.iter().copied().collect();
    tiles.sort();
    let mut forbidden: Vec<(i32, i32)> = region.forbidden_tiles.iter().copied().collect();
    forbidden.sort();
    // Variant tag for trace events. Empty string = primary attempt on
    // the current region; non-empty = speculative single-side expansion.
    // The web debugger groups per-iter state keyed by (iter, variant).
    let variant_tag = variant.unwrap_or("").to_string();

    trace::emit(TraceEvent::JunctionGrowthIteration {
        seed_x: ctx.initial_tile.0,
        seed_y: ctx.initial_tile.1,
        iter,
        variant: variant_tag.clone(),
        bbox_x: region.bbox.x,
        bbox_y: region.bbox.y,
        bbox_w: region.bbox.w,
        bbox_h: region.bbox.h,
        tiles,
        forbidden_tiles: forbidden,
        boundaries,
        participating: region.participating.clone(),
        encountered: region.encountered.clone(),
    });

    let strategy_ctx = JunctionStrategyContext {
        junction: &junction,
        region,
        growth_iter: iter,
        growth_variant: &variant_tag,
        routed_paths: ctx.routed_paths,
        hard_obstacles: ctx.hard_obstacles,
        strict_obstacles: ctx.strict_obstacles,
        placed_entities: ctx.placed_entities,
        unreleasable_obstacles: ctx.unreleasable_obstacles,
    };

    for strategy in ctx.strategies {
        #[cfg(not(target_arch = "wasm32"))]
        let strategy_started = std::time::Instant::now();
        let result = strategy.try_solve(&strategy_ctx);
        #[cfg(not(target_arch = "wasm32"))]
        let elapsed_us = strategy_started.elapsed().as_micros() as u64;
        #[cfg(target_arch = "wasm32")]
        let elapsed_us = 0u64;
        let Some(sol) = result else {
            trace::emit(TraceEvent::JunctionStrategyAttempt {
                seed_x: ctx.initial_tile.0,
                seed_y: ctx.initial_tile.1,
                iter,
                variant: variant_tag.clone(),
                strategy: strategy.name().to_string(),
                outcome: "Unsatisfiable".to_string(),
                detail: String::new(),
                elapsed_us,
            });
            continue;
        };

        // Deferred-exit: a participating spec's frontier exits on
        // another unresolved crossing. Skip this attempt so the
        // growth loop can push the frontier past all consecutive
        // crossings before committing.
        let exits_at_crossing = strategy_ctx.junction.specs.iter().any(|s| {
            let exit = (s.exit.x, s.exit.y);
            exit != ctx.initial_tile && ctx.all_crossings.contains(&exit)
        });
        if exits_at_crossing {
            trace::emit(TraceEvent::JunctionStrategyAttempt {
                seed_x: ctx.initial_tile.0,
                seed_y: ctx.initial_tile.1,
                iter,
                variant: variant_tag.clone(),
                strategy: strategy.name().to_string(),
                outcome: "DeferredExit".to_string(),
                detail: "spec exits at another unresolved crossing".to_string(),
                elapsed_us,
            });
            break; // skip this iter's solution; fall through to grow
        }

        // Walker veto: shadow-view simulate the routed paths with
        // SAT's proposed placements and reject if any routed path
        // breaks. Catches locally-valid SAT solutions that break a
        // perpendicular trunk.
        let bbox = region.bbox;
        let released: FxHashSet<(i32, i32)> =
            sol.entities.iter().map(|e| (e.x, e.y)).collect();
        let affected: Vec<AffectedPath<'_>> = ctx
            .routed_paths
            .iter()
            .filter_map(|(seg, tiles)| {
                if tiles.iter().any(|&t| near_bbox(bbox, t)) {
                    let item = ctx.spec_items.get(seg).map(|s| s.as_str()).unwrap_or("");
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
        let shadow = ShadowView::build(ctx.placed_entities, &released, &sol.entities);
        if let WalkResult::Broken { breaks } = walk_affected(&affected, &shadow) {
            let detail = if let Some(first) = breaks.first() {
                trace::emit(TraceEvent::RegionWalkerVeto {
                    tile_x: ctx.initial_tile.0,
                    tile_y: ctx.initial_tile.1,
                    strategy: strategy.name().to_string(),
                    growth_iter: iter,
                    variant: variant_tag.clone(),
                    broken_segment: first.segment_id.clone(),
                    break_tile_x: first.tile.0,
                    break_tile_y: first.tile.1,
                    break_count: breaks.len(),
                });
                format!(
                    "segment={} at ({},{}) breaks={}",
                    first.segment_id, first.tile.0, first.tile.1, breaks.len()
                )
            } else {
                String::new()
            };
            trace::emit(TraceEvent::JunctionStrategyAttempt {
                seed_x: ctx.initial_tile.0,
                seed_y: ctx.initial_tile.1,
                iter,
                variant: variant_tag.clone(),
                strategy: strategy.name().to_string(),
                outcome: "Vetoed".to_string(),
                detail,
                elapsed_us,
            });
            continue;
        }

        trace::emit(TraceEvent::JunctionStrategyAttempt {
            seed_x: ctx.initial_tile.0,
            seed_y: ctx.initial_tile.1,
            iter,
            variant: variant_tag.clone(),
            strategy: strategy.name().to_string(),
            outcome: "Solved".to_string(),
            detail: format!("{} entities placed", sol.entities.len()),
            elapsed_us,
        });
        // Candidate accepted for this variant — the caller compares
        // it against the other variants' candidates by cost and emits
        // the terminal `JunctionSolved` for the winner only.
        let cost = crate::bus::junction_cost::solution_cost(&sol.entities);
        trace::emit(TraceEvent::JunctionCandidateSolved {
            tile_x: ctx.initial_tile.0,
            tile_y: ctx.initial_tile.1,
            strategy: strategy.name().to_string(),
            growth_iter: iter,
            variant: variant_tag.clone(),
            region_tiles: region.tile_count(),
            cost,
        });
        return TryOutcome::Solved(sol);
    }

    TryOutcome::Continue
}

/// Every tile inside `bbox` (inclusive on the min side, exclusive on the
/// max side, per `Rect`'s convention). Kept as a helper in case future
/// strategies want to release the whole footprint — the current walker
/// wiring releases only SAT-proposed tiles instead.
#[allow(dead_code)]
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

// ---------------------------------------------------------------------------
// Trace helpers
// ---------------------------------------------------------------------------

fn dir_label(d: EntityDirection) -> String {
    match d {
        EntityDirection::North => "North",
        EntityDirection::East => "East",
        EntityDirection::South => "South",
        EntityDirection::West => "West",
    }
    .to_string()
}

fn dir_delta(d: EntityDirection) -> (i32, i32) {
    match d {
        EntityDirection::North => (0, -1),
        EntityDirection::East => (1, 0),
        EntityDirection::South => (0, 1),
        EntityDirection::West => (-1, 0),
    }
}

/// Entities whose footprint (1-tile for belts / UG, 2-tile for
/// splitters) lies within ±2 tiles of the seed. Includes a
/// `feeds_seed_area` hint so the replay tool can highlight likely
/// perpendicular feeders.
fn collect_nearby_stamped(seed: (i32, i32), placed: &[PlacedEntity]) -> Vec<StampedNeighbor> {
    let (sx, sy) = seed;
    let mut out = Vec::new();
    for e in placed {
        // Coarse cheap filter: within ±2 of seed.
        if (e.x - sx).abs() > 2 || (e.y - sy).abs() > 2 {
            continue;
        }
        let feeds = entity_feeds_seed_area(e, seed);
        out.push(StampedNeighbor {
            x: e.x,
            y: e.y,
            name: e.name.clone(),
            direction: dir_label(e.direction),
            carries: e.carries.clone(),
            segment_id: e.segment_id.clone(),
            feeds_seed_area: feeds,
        });
    }
    out
}

/// True iff `entity`'s output lands on the seed tile or any of its 4
/// direct neighbors. Used by `collect_nearby_stamped` to hint at likely
/// sources of external feeds.
fn entity_feeds_seed_area(entity: &PlacedEntity, seed: (i32, i32)) -> bool {
    let (sx, sy) = seed;
    // UG-ins consume; they don't emit onto the surface.
    if is_ug_belt(&entity.name) && entity.io_type.as_deref() == Some("input") {
        return false;
    }
    if !(is_surface_belt(&entity.name)
        || is_splitter(&entity.name)
        || (is_ug_belt(&entity.name) && entity.io_type.as_deref() == Some("output")))
    {
        return false;
    }
    let (dx, dy) = dir_delta(entity.direction);
    let mut targets: Vec<(i32, i32)> = Vec::with_capacity(2);
    if is_splitter(&entity.name) {
        let (s2x, s2y) = splitter_second_tile(entity);
        targets.push((entity.x + dx, entity.y + dy));
        targets.push((s2x + dx, s2y + dy));
    } else {
        targets.push((entity.x + dx, entity.y + dy));
    }
    for (tx, ty) in targets {
        if tx == sx && ty == sy {
            return true;
        }
        if (tx - sx).abs() + (ty - sy).abs() == 1 {
            return true;
        }
    }
    false
}

/// Build per-boundary snapshots from a `Junction` + participating +
/// encountered spec keys. Each SpecCrossing yields two boundaries
/// (entry + exit). The external-feeder annotation is derived by
/// scanning `placed_entities` for anything that outputs onto the entry
/// tile — same logic as the SAT strategy's `physical_feeder_direction`.
/// Participating specs fill the first N slots (by their order in
/// `participating_keys`); encountered specs follow, keyed by
/// `encountered_keys`.
fn junction_boundaries_to_snapshots(
    junction: &Junction,
    participating_keys: &[String],
    encountered_keys: &[String],
    placed: &[PlacedEntity],
) -> Vec<BoundarySnapshot> {
    let mut out = Vec::with_capacity(junction.specs.len() * 2);
    let mut p_idx = 0;
    let mut e_idx = 0;
    for sc in &junction.specs {
        let key = match sc.origin {
            SpecOrigin::Participating => {
                let k = participating_keys
                    .get(p_idx)
                    .cloned()
                    .unwrap_or_else(|| String::from("?"));
                p_idx += 1;
                k
            }
            SpecOrigin::Encountered => {
                let k = encountered_keys
                    .get(e_idx)
                    .cloned()
                    .unwrap_or_else(|| String::from("?"));
                e_idx += 1;
                k
            }
        };
        let origin = match sc.origin {
            SpecOrigin::Participating => "participating".to_string(),
            SpecOrigin::Encountered => "encountered".to_string(),
        };
        let entry_feeder = find_external_feeder((sc.entry.x, sc.entry.y), placed);
        out.push(BoundarySnapshot {
            x: sc.entry.x,
            y: sc.entry.y,
            direction: dir_label(sc.entry.direction),
            item: sc.item.clone(),
            is_input: true,
            interior: false,
            spec_key: key.clone(),
            origin: origin.clone(),
            external_feeder: entry_feeder,
        });
        out.push(BoundarySnapshot {
            x: sc.exit.x,
            y: sc.exit.y,
            direction: dir_label(sc.exit.direction),
            item: sc.item.clone(),
            is_input: false,
            interior: false,
            spec_key: key,
            origin,
            external_feeder: None,
        });
    }
    out
}

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
