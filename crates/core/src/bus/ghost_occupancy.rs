//! Shared occupancy map for the ghost routing pipeline.
//!
//! Tracks which tiles are claimed by which solver phase so that the ghost
//! A*, corridor template, per-tile template, and SAT crossing-zone phases
//! can all consult and mutate a single source of truth instead of each
//! maintaining its own obstacle view.
//!
//! See `docs/rfp-ghost-occupancy-refactor.md` for the design and rollout
//! plan. This module corresponds to **Step 1**: the type lands in
//! isolation with unit tests; nothing in `ghost_router.rs` consumes it
//! yet.

use rustc_hash::{FxHashMap, FxHashSet};

use crate::models::PlacedEntity;

/// Axis-aligned tile rectangle. Width and height are in tiles; the rect
/// covers `[x, x + w)` × `[y, y + h)`.
///
/// Mirrors the shape of `ghost_router::ClusterZone` so that callers can
/// pass either type once Step 6 unifies them.
#[derive(Clone, Copy, Debug)]
pub(super) struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x
            && px < self.x + self.w as i32
            && py >= self.y
            && py < self.y + self.h as i32
    }
}

/// What kind of solver-phase output owns a given tile claim.
///
/// `GhostSurface` and `RowEntity` are the two "permeable" variants —
/// every other variant is considered "permanent" (templates and SAT
/// must route around them). `GhostSurface` may be released by SAT and
/// templates; `RowEntity` is the row template's input/output belts
/// that the bus router needs to *interface* with rather than route
/// around (boundary ports may land on them).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Claim {
    /// Machine, pole, fluid lane, splitter footprint. No `PlacedEntity`
    /// associated — these are consulted only as obstacles.
    HardObstacle,
    /// Belt, inserter, pole, or pipe owned by the row template. The
    /// bus router will *interface* with these (boundary ports for SAT
    /// can land here), so they are NOT considered obstacles by either
    /// `is_permanent` or `forced_empty_in`. Distinct from `Permanent`
    /// to mirror today's `pre_existing_positions` semantics, which
    /// only includes entities present in the `entities` Vec.
    RowEntity { entity_idx: usize },
    /// Trunk belt, balancer block, splitter stamp — anything placed by
    /// the bus router's setup phases (steps 2-3). Permanent for the
    /// duration of ghost routing.
    Permanent { entity_idx: usize },
    /// Surface belt placed by ghost A* during step 5. May be replaced
    /// by a template UG-bridge or SAT solution that runs through this
    /// tile.
    GhostSurface { entity_idx: usize },
    /// Stamped by corridor or per-tile template. Permanent once placed
    /// — SAT must treat these as forced_empty.
    Template { entity_idx: usize },
    /// SAT-solved entity. Final.
    SatSolved { entity_idx: usize },
}

impl Claim {
    /// Returns the entity index for non-hard claims.
    #[allow(dead_code)] // used by test-only helpers `entity_at` and `into_entities`
    pub fn entity_idx(&self) -> Option<usize> {
        match self {
            Claim::HardObstacle => None,
            Claim::RowEntity { entity_idx }
            | Claim::Permanent { entity_idx }
            | Claim::GhostSurface { entity_idx }
            | Claim::Template { entity_idx }
            | Claim::SatSolved { entity_idx } => Some(*entity_idx),
        }
    }

    /// "Permanent" claims are those that templates and SAT must avoid
    /// when placing new entities. `GhostSurface` and `RowEntity` are
    /// excluded — the former because templates/SAT may replace it,
    /// the latter because the bus router interfaces with row-template
    /// belts via boundary ports rather than routing around them.
    pub fn is_permanent(&self) -> bool {
        !matches!(self, Claim::GhostSurface { .. } | Claim::RowEntity { .. })
    }
}

/// Caller-supplied tag for `Occupancy::place`. Mirrors `Claim` variants
/// minus the `entity_idx` field, which `place` fills in itself.
/// `RowEntity` is built by `Occupancy::new` from the row_entities
/// argument and is not a valid `place()` tag — row entities are
/// installed once at construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ClaimKindTag {
    Permanent,
    GhostSurface,
    Template,
    #[allow(dead_code)]
    SatSolved,
}

/// Reasons a `place()` call may fail.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum OccupancyError {
    /// Tile is a `HardObstacle` — never placeable on.
    HardObstacle,
    /// Tile is already claimed by a permanent (non-`GhostSurface`) entity.
    /// The caller must explicitly release it (or the prior phase did
    /// something wrong).
    AlreadyClaimed { existing: Claim },
}

/// Shared occupancy map. Replaces the parallel `hard` / `existing_belts`
/// / `pre_existing_set` / `entities` data flows in `ghost_router.rs`.
pub(super) struct Occupancy {
    /// All entities placed so far, in placement order. Indices are
    /// stable: `Claim::*::entity_idx` always points at the same entity
    /// for the lifetime of an `Occupancy`.
    entities: Vec<PlacedEntity>,
    /// Per-tile claim record. Tiles with no claim are absent from the
    /// map.
    claims: FxHashMap<(i32, i32), Claim>,
}

impl Occupancy {
    /// Build an `Occupancy` from a hard-obstacle set and two classes
    /// of initial entities:
    ///
    /// - `row_belts`: belt/inserter/pole/pipe entities from the row
    ///   template that the bus router *interfaces* with. These get
    ///   `RowEntity` claims — boundary ports may land on them and
    ///   `forced_empty_in` skips them, mirroring today's behaviour
    ///   where row-template belts are absent from
    ///   `pre_existing_positions`. **Only belt-like row entities
    ///   belong here.** Machines, inserters, and poles from the row
    ///   template are not permeable and should go into
    ///   `permanent_entities` instead (or be captured by the `hard`
    ///   set).
    /// - `permanent_entities`: splitter stamps, balancer blocks, row
    ///   template machines/inserters/poles, or anything else that the
    ///   bus router must treat as a permanent obstacle. These get
    ///   `Permanent` claims, which `is_permanent` and `forced_empty_in`
    ///   both treat as obstacles.
    ///
    /// Hard tiles that also have an entity get the entity claim — the
    /// hard set is treated as "this tile is unconditionally unavailable
    /// even if no entity is sitting on it" (e.g. fluid-lane reservations
    /// reserve tiles that may not yet hold a pipe).
    pub fn new(
        hard: FxHashSet<(i32, i32)>,
        row_belts: Vec<PlacedEntity>,
        permanent_entities: Vec<PlacedEntity>,
    ) -> Self {
        let mut claims: FxHashMap<(i32, i32), Claim> = FxHashMap::default();
        for tile in hard {
            claims.insert(tile, Claim::HardObstacle);
        }
        let mut entities: Vec<PlacedEntity> =
            Vec::with_capacity(row_belts.len() + permanent_entities.len());
        for e in row_belts {
            let pos = (e.x, e.y);
            let idx = entities.len();
            entities.push(e);
            // RowEntity claim for permeable belts.
            claims.insert(pos, Claim::RowEntity { entity_idx: idx });
        }
        for e in permanent_entities {
            let pos = (e.x, e.y);
            let idx = entities.len();
            entities.push(e);
            // Permanent claim overwrites HardObstacle for the entity's
            // anchor tile.
            claims.insert(pos, Claim::Permanent { entity_idx: idx });
        }
        Occupancy { entities, claims }
    }

    // -------------------------------------------------------------------
    // Queries
    // -------------------------------------------------------------------

    /// Number of entities currently placed.
    #[allow(dead_code)] // API surface for tests + future callers
    pub fn entity_count(&self) -> usize {
        self.entities.len()
    }

    /// True if the tile has any claim at all.
    pub fn is_claimed(&self, tile: (i32, i32)) -> bool {
        self.claims.contains_key(&tile)
    }

    /// True if the tile is a hard obstacle.
    pub fn is_hard_obstacle(&self, tile: (i32, i32)) -> bool {
        matches!(self.claims.get(&tile), Some(Claim::HardObstacle))
    }

    /// True if the tile is unclaimed.
    #[allow(dead_code)]
    pub fn is_free(&self, tile: (i32, i32)) -> bool {
        !self.claims.contains_key(&tile)
    }

    /// True if the tile holds a "permanent" claim (anything other than a
    /// `GhostSurface`). Templates and SAT must not stamp over these.
    pub fn is_permanent(&self, tile: (i32, i32)) -> bool {
        self.claims.get(&tile).is_some_and(|c| c.is_permanent())
    }

    /// Inspect the claim at a tile.
    pub fn claim_at(&self, tile: (i32, i32)) -> Option<&Claim> {
        self.claims.get(&tile)
    }

    /// Fetch the entity at a tile, if any. Returns `None` for
    /// `HardObstacle` and unclaimed tiles.
    #[allow(dead_code)] // API surface for tests + future callers
    pub fn entity_at(&self, tile: (i32, i32)) -> Option<&PlacedEntity> {
        self.claims
            .get(&tile)
            .and_then(|c| c.entity_idx())
            .and_then(|i| self.entities.get(i))
    }

    /// Snapshot the set of tiles that currently hold a "permanent"
    /// claim — `HardObstacle | Permanent | Template | SatSolved` (i.e.
    /// everything `is_permanent` returns true for). The result is a
    /// frozen `FxHashSet` that does not update as the Occupancy is
    /// further mutated.
    ///
    /// Used by `resolve_clusters` to build a static obstacle set at
    /// the top of the SAT cluster loop, mirroring today's behaviour
    /// where `pre_existing_positions` is computed once and not
    /// updated as later SAT clusters claim tiles.
    #[allow(dead_code)]
    pub fn snapshot_permanent_tiles(&self) -> FxHashSet<(i32, i32)> {
        self.claims
            .iter()
            .filter(|(_, c)| c.is_permanent())
            .map(|(t, _)| *t)
            .collect()
    }

    /// Snapshot every tile a junction strategy must avoid stamping on:
    /// hard obstacles, permanent / template / SAT-solved entities, AND
    /// row-template belts (`RowEntity`). `GhostSurface` claims are
    /// excluded — strategies may legitimately replace ghost-routed
    /// belts when they re-stamp a region. Used by `ghost_router` step
    /// 6a to build the obstacle set passed into `solve_crossing` so
    /// SAT (and any future strategy) sees the full picture instead of
    /// just the narrow `hard` set.
    pub fn snapshot_junction_obstacles(&self) -> FxHashSet<(i32, i32)> {
        self.claims
            .iter()
            .filter(|(_, c)| {
                !matches!(c, Claim::GhostSurface { .. })
            })
            .map(|(t, _)| *t)
            .collect()
    }

    /// Tiles inside `zone` that the SAT solver must treat as
    /// `forced_empty`: every claimed tile inside the zone, except
    /// boundary ports, ghost-surface belts (which SAT replaces), and
    /// row-entity belts (which the bus router interfaces with via
    /// boundary ports rather than routes around).
    ///
    /// `boundaries` is the set of `(x, y)` positions that the SAT zone
    /// has already designated as boundary ports — those must not appear
    /// in `forced_empty` because they're entry/exit ports for the
    /// solver.
    #[allow(dead_code)]
    pub fn forced_empty_in(
        &self,
        zone: &Rect,
        boundaries: &FxHashSet<(i32, i32)>,
    ) -> Vec<(i32, i32)> {
        let mut out = Vec::new();
        for (&tile, claim) in &self.claims {
            if !zone.contains(tile.0, tile.1) {
                continue;
            }
            if boundaries.contains(&tile) {
                continue;
            }
            if matches!(
                claim,
                Claim::GhostSurface { .. } | Claim::RowEntity { .. }
            ) {
                continue;
            }
            out.push(tile);
        }
        out
    }

    // -------------------------------------------------------------------
    // Mutations
    // -------------------------------------------------------------------

    /// Place an entity with the given claim kind.
    ///
    /// Returns `Err` if the target tile is a `HardObstacle` or already
    /// holds any non-`GhostSurface` claim. A `GhostSurface` claim is
    /// released and the new entity replaces it.
    ///
    /// On success, the entity is appended to the entity list and the
    /// returned `usize` is its index.
    pub fn place(
        &mut self,
        entity: PlacedEntity,
        kind: ClaimKindTag,
    ) -> Result<usize, OccupancyError> {
        let tile = (entity.x, entity.y);
        if let Some(existing) = self.claims.get(&tile) {
            match existing {
                Claim::HardObstacle => return Err(OccupancyError::HardObstacle),
                Claim::GhostSurface { .. } => {
                    // Allowed: replace the ghost-surface claim. The
                    // backing entity stays in `self.entities` for index
                    // stability but becomes orphaned (no claim points to
                    // it). Step 6 may add a `garbage_collect` pass; for
                    // now the orphan is harmless because `into_entities`
                    // consumers only look up entities via current claims.
                }
                other => {
                    return Err(OccupancyError::AlreadyClaimed {
                        existing: other.clone(),
                    });
                }
            }
        }
        let idx = self.entities.len();
        self.entities.push(entity);
        let claim = match kind {
            ClaimKindTag::Permanent => Claim::Permanent { entity_idx: idx },
            ClaimKindTag::GhostSurface => Claim::GhostSurface { entity_idx: idx },
            ClaimKindTag::Template => Claim::Template { entity_idx: idx },
            ClaimKindTag::SatSolved => Claim::SatSolved { entity_idx: idx },
        };
        self.claims.insert(tile, claim);
        Ok(idx)
    }

    /// Drop every `GhostSurface` claim whose tile lies inside `zone`.
    /// The corresponding entities become orphaned (see `place`'s note).
    /// Used by SAT before claiming tiles for its own solution.
    ///
    /// Returns the number of claims released.
    pub fn release_ghost_surface_in(&mut self, zone: &Rect) -> usize {
        let to_remove: Vec<(i32, i32)> = self
            .claims
            .iter()
            .filter(|(tile, claim)| {
                zone.contains(tile.0, tile.1)
                    && matches!(claim, Claim::GhostSurface { .. })
            })
            .map(|(tile, _)| *tile)
            .collect();
        let n = to_remove.len();
        for tile in to_remove {
            self.claims.remove(&tile);
        }
        n
    }

    /// Drop every `GhostSurface` claim, plus every `Permanent` claim
    /// whose entity is a trunk or tap-off belt (segment id starting with
    /// `"trunk:"` or `"tapoff:"`), inside `zone`. Used by the per-tile
    /// template phase to clear the 1×3 footprint before stamping a
    /// crossing template.
    ///
    /// `HardObstacle` claims are left alone — those represent machines
    /// or fluid lanes that the template can't displace.
    ///
    /// Returns the number of claims released.
    pub fn release_for_pertile_template(&mut self, zone: &Rect) -> usize {
        let to_remove: Vec<(i32, i32)> = self
            .claims
            .iter()
            .filter(|(tile, claim)| {
                if !zone.contains(tile.0, tile.1) {
                    return false;
                }
                match claim {
                    Claim::GhostSurface { .. } => true,
                    Claim::Permanent { entity_idx } => self
                        .entities
                        .get(*entity_idx)
                        .and_then(|e| e.segment_id.as_deref())
                        .is_some_and(|s| s.starts_with("trunk:") || s.starts_with("tapoff:")),
                    _ => false,
                }
            })
            .map(|(tile, _)| *tile)
            .collect();
        let n = to_remove.len();
        for tile in to_remove {
            self.claims.remove(&tile);
        }
        n
    }

    /// Consume the occupancy map and return all entities that still
    /// have a live claim, in placement order. Orphaned entities (those
    /// whose tile was reclaimed by a later `place` call) are dropped.
    #[allow(dead_code)] // API surface for tests + future callers
    pub fn into_entities(self) -> Vec<PlacedEntity> {
        let live_idxs: FxHashSet<usize> = self
            .claims
            .values()
            .filter_map(|c| c.entity_idx())
            .collect();
        self.entities
            .into_iter()
            .enumerate()
            .filter_map(|(i, e)| if live_idxs.contains(&i) { Some(e) } else { None })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::EntityDirection;

    fn belt(x: i32, y: i32, seg: &str) -> PlacedEntity {
        PlacedEntity {
            name: "transport-belt".to_string(),
            x,
            y,
            direction: EntityDirection::East,
            segment_id: Some(seg.to_string()),
            ..Default::default()
        }
    }

    fn empty_boundaries() -> FxHashSet<(i32, i32)> {
        FxHashSet::default()
    }

    #[test]
    fn new_treats_hard_set_and_setup_entities_correctly() {
        let mut hard: FxHashSet<(i32, i32)> = FxHashSet::default();
        hard.insert((5, 5));
        hard.insert((6, 6));
        let setup = vec![belt(10, 10, "trunk:iron-plate")];

        let occ = Occupancy::new(hard, Vec::new(), setup);

        assert!(occ.is_hard_obstacle((5, 5)));
        assert!(occ.is_hard_obstacle((6, 6)));
        assert!(!occ.is_hard_obstacle((10, 10)));

        assert!(occ.is_permanent((10, 10)));
        assert!(occ.is_permanent((5, 5)));

        assert!(occ.is_claimed((5, 5)));
        assert!(occ.is_claimed((10, 10)));
        assert!(occ.is_free((0, 0)));

        assert_eq!(occ.entity_count(), 1);
        assert_eq!(occ.entity_at((10, 10)).map(|e| e.name.as_str()), Some("transport-belt"));
        assert!(occ.entity_at((5, 5)).is_none(), "hard tile has no entity");
    }

    #[test]
    fn new_treats_row_entities_as_permeable() {
        // Row entities are claimed but `is_permanent` returns false for
        // them (they're permeable for boundary ports and forced_empty).
        let row = vec![belt(7, 7, "row:iron-plate")];
        let occ = Occupancy::new(FxHashSet::default(), row, Vec::new());

        assert!(occ.is_claimed((7, 7)));
        assert!(!occ.is_permanent((7, 7)), "row entity belts must not be 'permanent'");
        assert!(matches!(occ.claim_at((7, 7)), Some(Claim::RowEntity { .. })));
        assert_eq!(occ.entity_at((7, 7)).map(|e| e.name.as_str()), Some("transport-belt"));
    }

    #[test]
    fn forced_empty_in_excludes_row_entities() {
        // Row entity belts must not be in forced_empty — SAT can route
        // through them via boundary ports.
        let row = vec![belt(3, 0, "row:a")];
        let setup = vec![belt(2, 0, "trunk:b")];
        let occ = Occupancy::new(FxHashSet::default(), row, setup);
        let zone = Rect { x: 0, y: 0, w: 5, h: 1 };
        let mut got = occ.forced_empty_in(&zone, &empty_boundaries());
        got.sort();
        // Only the trunk (Permanent) is in forced_empty; the row entity
        // belt is not.
        assert_eq!(got, vec![(2, 0)]);
    }

    #[test]
    fn place_succeeds_on_free_tile_for_each_kind() {
        let mut occ = Occupancy::new(FxHashSet::default(), Vec::new(), Vec::new());

        assert!(occ.place(belt(0, 0, "ghost:a"), ClaimKindTag::GhostSurface).is_ok());
        assert!(occ.place(belt(1, 0, "junction:a"), ClaimKindTag::Template).is_ok());
        assert!(occ.place(belt(2, 0, "sat:a"), ClaimKindTag::SatSolved).is_ok());
        assert!(occ.place(belt(3, 0, "trunk:a"), ClaimKindTag::Permanent).is_ok());

        assert!(matches!(occ.claim_at((0, 0)), Some(Claim::GhostSurface { .. })));
        assert!(matches!(occ.claim_at((1, 0)), Some(Claim::Template { .. })));
        assert!(matches!(occ.claim_at((2, 0)), Some(Claim::SatSolved { .. })));
        assert!(matches!(occ.claim_at((3, 0)), Some(Claim::Permanent { .. })));
    }

    #[test]
    fn place_replaces_ghost_surface_claim() {
        let mut occ = Occupancy::new(FxHashSet::default(), Vec::new(), Vec::new());
        occ.place(belt(0, 0, "ghost:a"), ClaimKindTag::GhostSurface).unwrap();
        let result = occ.place(belt(0, 0, "junction:a"), ClaimKindTag::Template);
        assert!(result.is_ok(), "template should replace ghost surface");
        assert!(matches!(occ.claim_at((0, 0)), Some(Claim::Template { .. })));
    }

    #[test]
    fn place_rejects_hard_obstacle() {
        let mut hard = FxHashSet::default();
        hard.insert((0, 0));
        let mut occ = Occupancy::new(hard, Vec::new(), Vec::new());
        let result = occ.place(belt(0, 0, "ghost:a"), ClaimKindTag::GhostSurface);
        assert_eq!(result, Err(OccupancyError::HardObstacle));
    }

    #[test]
    fn place_rejects_permanent_template_satsolved_claims() {
        let mut occ = Occupancy::new(FxHashSet::default(), Vec::new(), Vec::new());
        occ.place(belt(0, 0, "trunk:a"), ClaimKindTag::Permanent).unwrap();
        occ.place(belt(1, 0, "junction:a"), ClaimKindTag::Template).unwrap();
        occ.place(belt(2, 0, "sat:a"), ClaimKindTag::SatSolved).unwrap();

        for (tile, _label) in [((0, 0), "Permanent"), ((1, 0), "Template"), ((2, 0), "SatSolved")] {
            let result = occ.place(belt(tile.0, tile.1, "ghost:b"), ClaimKindTag::GhostSurface);
            assert!(matches!(result, Err(OccupancyError::AlreadyClaimed { .. })));
        }
    }

    #[test]
    fn release_ghost_surface_in_only_drops_ghost_surface() {
        let mut occ = Occupancy::new(FxHashSet::default(), Vec::new(), Vec::new());
        occ.place(belt(0, 0, "ghost:a"), ClaimKindTag::GhostSurface).unwrap();
        occ.place(belt(1, 0, "ghost:b"), ClaimKindTag::GhostSurface).unwrap();
        occ.place(belt(2, 0, "trunk:c"), ClaimKindTag::Permanent).unwrap();
        occ.place(belt(3, 0, "junction:d"), ClaimKindTag::Template).unwrap();
        // (10, 10) lies outside the zone.
        occ.place(belt(10, 10, "ghost:e"), ClaimKindTag::GhostSurface).unwrap();

        let zone = Rect { x: 0, y: 0, w: 4, h: 1 };
        let n = occ.release_ghost_surface_in(&zone);
        assert_eq!(n, 2, "two ghost surface claims inside the zone");

        assert!(occ.is_free((0, 0)));
        assert!(occ.is_free((1, 0)));
        assert!(occ.is_permanent((2, 0)), "trunk untouched");
        assert!(occ.is_permanent((3, 0)), "template untouched");
        assert!(occ.is_claimed((10, 10)), "ghost outside zone untouched");
    }

    #[test]
    fn release_for_pertile_template_drops_ghost_and_trunk_and_tapoff() {
        let mut occ = Occupancy::new(FxHashSet::default(), Vec::new(), Vec::new());
        occ.place(belt(0, 0, "ghost:a"), ClaimKindTag::GhostSurface).unwrap();
        occ.place(belt(1, 0, "trunk:b"), ClaimKindTag::Permanent).unwrap();
        occ.place(belt(2, 0, "tapoff:c"), ClaimKindTag::Permanent).unwrap();
        // Non-trunk/non-tapoff permanent — must be left alone.
        occ.place(belt(3, 0, "row:d"), ClaimKindTag::Permanent).unwrap();
        // Template inside the zone — must be left alone (templates don't
        // displace each other).
        occ.place(belt(4, 0, "junction:e"), ClaimKindTag::Template).unwrap();

        let zone = Rect { x: 0, y: 0, w: 5, h: 1 };
        let n = occ.release_for_pertile_template(&zone);
        assert_eq!(n, 3, "ghost + trunk + tapoff released");

        assert!(occ.is_free((0, 0)));
        assert!(occ.is_free((1, 0)));
        assert!(occ.is_free((2, 0)));
        assert!(occ.is_permanent((3, 0)), "row template kept");
        assert!(occ.is_permanent((4, 0)), "template kept");
    }

    #[test]
    fn release_for_pertile_template_leaves_hard_obstacles() {
        let mut hard = FxHashSet::default();
        hard.insert((0, 0));
        let mut occ = Occupancy::new(hard, Vec::new(), Vec::new());
        let zone = Rect { x: 0, y: 0, w: 1, h: 1 };
        let n = occ.release_for_pertile_template(&zone);
        assert_eq!(n, 0);
        assert!(occ.is_hard_obstacle((0, 0)));
    }

    #[test]
    fn forced_empty_in_excludes_ghost_surface_and_boundaries() {
        let mut hard = FxHashSet::default();
        hard.insert((0, 0));
        let mut occ = Occupancy::new(hard, Vec::new(), Vec::new());
        occ.place(belt(1, 0, "trunk:a"), ClaimKindTag::Permanent).unwrap();
        occ.place(belt(2, 0, "ghost:b"), ClaimKindTag::GhostSurface).unwrap();
        occ.place(belt(3, 0, "junction:c"), ClaimKindTag::Template).unwrap();
        // Outside the zone — should never appear.
        occ.place(belt(99, 99, "trunk:far"), ClaimKindTag::Permanent).unwrap();

        let zone = Rect { x: 0, y: 0, w: 5, h: 1 };

        // No boundaries: ghost-surface (2,0) is excluded; everything else
        // claimed inside the zone is forced_empty.
        let mut got = occ.forced_empty_in(&zone, &empty_boundaries());
        got.sort();
        assert_eq!(got, vec![(0, 0), (1, 0), (3, 0)]);

        // With (1,0) declared as a boundary port, it drops out.
        let mut boundaries = FxHashSet::default();
        boundaries.insert((1, 0));
        let mut got = occ.forced_empty_in(&zone, &boundaries);
        got.sort();
        assert_eq!(got, vec![(0, 0), (3, 0)]);
    }

    #[test]
    fn into_entities_drops_orphans_from_replaced_ghost_surfaces() {
        let mut occ = Occupancy::new(FxHashSet::default(), Vec::new(), Vec::new());
        // First place a ghost-surface belt, then immediately replace it
        // with a template belt. The original entity becomes orphaned.
        occ.place(belt(0, 0, "ghost:a"), ClaimKindTag::GhostSurface).unwrap();
        occ.place(belt(0, 0, "junction:a"), ClaimKindTag::Template).unwrap();
        // And another live ghost belt for good measure.
        occ.place(belt(1, 0, "ghost:b"), ClaimKindTag::GhostSurface).unwrap();

        let entities = occ.into_entities();
        assert_eq!(entities.len(), 2);
        let segs: Vec<&str> = entities
            .iter()
            .filter_map(|e| e.segment_id.as_deref())
            .collect();
        assert!(segs.contains(&"junction:a"));
        assert!(segs.contains(&"ghost:b"));
        assert!(!segs.contains(&"ghost:a"), "orphaned ghost belt was dropped");
    }

    #[test]
    fn into_entities_preserves_placement_order_for_live_claims() {
        let mut occ = Occupancy::new(FxHashSet::default(), Vec::new(), Vec::new());
        for i in 0..5 {
            occ.place(belt(i, 0, &format!("trunk:{i}")), ClaimKindTag::Permanent).unwrap();
        }
        let entities = occ.into_entities();
        let xs: Vec<i32> = entities.iter().map(|e| e.x).collect();
        assert_eq!(xs, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn rect_contains_is_half_open() {
        let r = Rect { x: 5, y: 10, w: 3, h: 2 };
        assert!(r.contains(5, 10));
        assert!(r.contains(7, 11));
        assert!(!r.contains(8, 10), "right edge is exclusive");
        assert!(!r.contains(5, 12), "bottom edge is exclusive");
        assert!(!r.contains(4, 10));
    }
}
