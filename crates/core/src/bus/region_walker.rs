//! Region walker: structural reachability check for the junction solver's
//! CEGAR (counter-example guided) loop.
//!
//! When a `JunctionStrategy` returns a candidate solution for a growing
//! region, we can't commit it blindly — the strategy only knows about the
//! specs it was handed, not about other routed paths that touch the
//! region's footprint. The walker is the veto: it merges the proposed
//! entities into a shadow view of the world and walks every affected
//! path end-to-end. If any walk fails, the solution is rejected and the
//! outer loop grows the region and retries.
//!
//! Scope, deliberately small for the MVP:
//! - Checks presence + item match at each tile on each affected path.
//! - Accepts UG passthroughs (same-path UG in → paired UG out) and
//!   hidden-middle tiles between them.
//! - Does *not* yet verify flow direction at each step; a belt carrying
//!   the right item is treated as "probably fine." The assumption is
//!   that the downstream validator still runs on the committed layout
//!   and catches direction bugs we miss here. This keeps the walker
//!   simple enough to trust and cheap enough to run on every iteration.
//!
//! The walker is intentionally decoupled from `GrowingRegion` and
//! `JunctionStrategyContext`. The caller constructs an `AffectedPath`
//! list and a `ShadowView`; we return a `WalkResult`. Everything else
//! lives in the caller.

use rustc_hash::{FxHashMap, FxHashSet};

use crate::models::{EntityDirection, PlacedEntity};

/// A routed path the caller wants us to verify against the shadow view.
/// Held by reference so the caller doesn't need to clone path data.
#[derive(Debug, Clone, Copy)]
pub struct AffectedPath<'a> {
    /// Segment id used in traces and error reports (e.g. `trunk:copper-cable#0`).
    pub segment_id: &'a str,
    /// Ordered tile sequence from source to sink, inclusive.
    pub tiles: &'a [(i32, i32)],
    /// Item this path carries. Used for item-match checks.
    pub item: &'a str,
}

/// Outcome of walking all affected paths.
#[derive(Debug, Clone)]
pub enum WalkResult {
    /// Every affected path walks cleanly from source to sink. Commit the
    /// proposed solution.
    Passed,
    /// At least one path failed. Caller should reject the solution and
    /// grow the region.
    Broken { breaks: Vec<WalkBreak> },
}

impl WalkResult {
    #[allow(dead_code)]
    pub fn is_passed(&self) -> bool {
        matches!(self, WalkResult::Passed)
    }
}

/// A single point of failure on a path. One walk can produce at most one
/// break — we stop at the first bad tile.
///
/// The `reason` field is only observed via `Debug` today (trace output,
/// test panics); `#[allow(dead_code)]` is there so rustc doesn't flag
/// the variant fields while we iterate on the walker's surface.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct WalkBreak {
    pub segment_id: String,
    pub tile: (i32, i32),
    pub reason: BreakReason,
}

/// Why a walk step failed. Fields carry enough detail for a
/// human-readable trace line; not yet read programmatically.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum BreakReason {
    /// Nothing at this tile in the shadow, and the tile is not a
    /// legitimate UG hidden-middle on this path.
    MissingEntity,
    /// Shadow has an entity but it's carrying the wrong item.
    ItemMismatch {
        expected: String,
        actual: Option<String>,
    },
    /// Found a UG input on this path but couldn't find a paired UG
    /// output further along the path carrying the same item and
    /// facing the same direction.
    UnpairedUgIn { direction: EntityDirection },
}

/// A view of the world after a region's proposed entities are committed:
/// existing entities outside the region, minus any tiles the region
/// releases, plus the proposed entities inside the region.
///
/// This is a plain tile map; it's cheap to build and throw away because
/// affected regions are small.
///
/// Perf note: `build` clones every existing entity into the map. For a
/// ~1000-entity layout with a ~50-tile region that's fine (runs per
/// veto attempt, measured in microseconds), but if profiling ever shows
/// it as hot we can switch to a borrowed overlay (`FxHashMap<(i32,i32),
/// &PlacedEntity>`) with a lifetime tied to `existing`.
pub struct ShadowView {
    by_tile: FxHashMap<(i32, i32), PlacedEntity>,
}

impl ShadowView {
    /// Build a shadow view from the current world state plus a region's
    /// proposed solution.
    pub fn build(
        existing: &[PlacedEntity],
        released: &FxHashSet<(i32, i32)>,
        proposed: &[PlacedEntity],
    ) -> Self {
        let mut by_tile: FxHashMap<(i32, i32), PlacedEntity> = FxHashMap::default();
        for e in existing {
            if released.contains(&(e.x, e.y)) {
                continue;
            }
            by_tile.insert((e.x, e.y), e.clone());
        }
        // Proposed entities always win over existing — the region is
        // authoritative inside its footprint.
        for e in proposed {
            by_tile.insert((e.x, e.y), e.clone());
        }
        Self { by_tile }
    }

    pub fn get(&self, tile: (i32, i32)) -> Option<&PlacedEntity> {
        self.by_tile.get(&tile)
    }
}

/// Walk every affected path in the shadow view and return a combined
/// result.
pub fn walk_affected(paths: &[AffectedPath<'_>], shadow: &ShadowView) -> WalkResult {
    let mut breaks = Vec::new();
    for p in paths {
        if let Err(br) = walk_single(p, shadow) {
            breaks.push(br);
        }
    }
    if breaks.is_empty() {
        WalkResult::Passed
    } else {
        WalkResult::Broken { breaks }
    }
}

fn walk_single(path: &AffectedPath<'_>, shadow: &ShadowView) -> Result<(), WalkBreak> {
    let tiles = path.tiles;
    if tiles.is_empty() {
        return Ok(());
    }
    let mut i = 0usize;
    while i < tiles.len() {
        let t = tiles[i];
        match shadow.get(t) {
            Some(e) => {
                // Item must match. Wrong carries means SAT replaced our
                // belt with something else (the tier2_ec sideload bug).
                if e.carries.as_deref() != Some(path.item) {
                    return Err(WalkBreak {
                        segment_id: path.segment_id.to_string(),
                        tile: t,
                        reason: BreakReason::ItemMismatch {
                            expected: path.item.into(),
                            actual: e.carries.clone(),
                        },
                    });
                }
                // A UG input on this path jumps to its paired output
                // further along the sequence; tiles between them are
                // hidden middles we don't need to verify.
                if is_ug_in(e) {
                    let dir = e.direction;
                    let paired = (i + 1..tiles.len()).find(|&j| {
                        shadow
                            .get(tiles[j])
                            .map(|e2| {
                                is_ug_out(e2)
                                    && e2.direction == dir
                                    && e2.carries.as_deref() == Some(path.item)
                            })
                            .unwrap_or(false)
                    });
                    match paired {
                        Some(j) => {
                            i = j + 1;
                            continue;
                        }
                        None => {
                            return Err(WalkBreak {
                                segment_id: path.segment_id.to_string(),
                                tile: t,
                                reason: BreakReason::UnpairedUgIn { direction: dir },
                            });
                        }
                    }
                }
                i += 1;
            }
            None => {
                // No entity at this tile. Only legal if it's the hidden
                // middle of a UG pair belonging to this same path (in →
                // middle → out all in `tiles` with `i` strictly between
                // the in and out indices).
                if is_hidden_middle_of_own_ug(tiles, i, path.item, shadow) {
                    i += 1;
                    continue;
                }
                return Err(WalkBreak {
                    segment_id: path.segment_id.to_string(),
                    tile: t,
                    reason: BreakReason::MissingEntity,
                });
            }
        }
    }
    Ok(())
}

fn is_ug(e: &PlacedEntity) -> bool {
    e.name.contains("underground-belt")
}

fn is_ug_in(e: &PlacedEntity) -> bool {
    is_ug(e) && e.io_type.as_deref() == Some("input")
}

fn is_ug_out(e: &PlacedEntity) -> bool {
    is_ug(e) && e.io_type.as_deref() == Some("output")
}

/// True iff `tiles[idx]` sits strictly between a UG-input and matching
/// UG-output on this same path. A matching pair carries the same item.
fn is_hidden_middle_of_own_ug(
    tiles: &[(i32, i32)],
    idx: usize,
    item: &str,
    shadow: &ShadowView,
) -> bool {
    // Find the nearest UG input on this path at some j < idx.
    let in_idx = (0..idx).rev().find(|&j| {
        shadow
            .get(tiles[j])
            .map(|e| is_ug_in(e) && e.carries.as_deref() == Some(item))
            .unwrap_or(false)
    });
    let Some(in_idx) = in_idx else { return false; };
    // Find a matching UG output strictly after idx.
    let out_idx = (idx + 1..tiles.len()).find(|&k| {
        shadow
            .get(tiles[k])
            .map(|e| is_ug_out(e) && e.carries.as_deref() == Some(item))
            .unwrap_or(false)
    });
    match out_idx {
        Some(k) => in_idx < idx && idx < k,
        None => false,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::EntityDirection;

    fn belt(x: i32, y: i32, dir: EntityDirection, item: &str) -> PlacedEntity {
        PlacedEntity {
            name: "fast-transport-belt".into(),
            x,
            y,
            direction: dir,
            carries: Some(item.into()),
            ..Default::default()
        }
    }

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

    #[test]
    fn passes_unchanged_surface_path() {
        // Straight copper-cable south belt run: (3,8)..(3,12).
        let existing = vec![
            belt(3, 8, EntityDirection::South, "copper-cable"),
            belt(3, 9, EntityDirection::South, "copper-cable"),
            belt(3, 10, EntityDirection::South, "copper-cable"),
            belt(3, 11, EntityDirection::South, "copper-cable"),
            belt(3, 12, EntityDirection::South, "copper-cable"),
        ];
        let shadow = ShadowView::build(&existing, &FxHashSet::default(), &[]);
        let tiles = vec![(3, 8), (3, 9), (3, 10), (3, 11), (3, 12)];
        let path = AffectedPath {
            segment_id: "trunk:copper-cable#0",
            tiles: &tiles,
            item: "copper-cable",
        };
        let result = walk_affected(&[path], &shadow);
        assert!(result.is_passed(), "expected Passed, got {result:?}");
    }

    #[test]
    fn fails_on_item_mismatch_tier2_ec_bug() {
        // Simulates the buggy tier2_electronic_circuit layout: column-3
        // copper-cable is broken at (3,10) by an iron-plate UG input
        // that SAT placed there. The walker must reject this.
        let existing = vec![
            belt(3, 8, EntityDirection::South, "copper-cable"),
            belt(3, 9, EntityDirection::South, "copper-cable"),
            // (3,10) was copper-cable, now the region released it.
            belt(3, 11, EntityDirection::South, "copper-cable"),
        ];
        let released: FxHashSet<(i32, i32)> = [(3, 10)].into_iter().collect();
        let proposed = vec![
            // SAT put an iron-plate UG input East at (3,10) — the bug.
            ug_in(3, 10, EntityDirection::East, "iron-plate"),
            ug_out(5, 10, EntityDirection::East, "iron-plate"),
        ];
        let shadow = ShadowView::build(&existing, &released, &proposed);

        let tiles = vec![(3, 8), (3, 9), (3, 10), (3, 11)];
        let copper = AffectedPath {
            segment_id: "trunk:copper-cable#0",
            tiles: &tiles,
            item: "copper-cable",
        };
        let result = walk_affected(&[copper], &shadow);
        match result {
            WalkResult::Broken { breaks } => {
                assert_eq!(breaks.len(), 1);
                assert_eq!(breaks[0].tile, (3, 10));
                matches!(breaks[0].reason, BreakReason::ItemMismatch { .. });
            }
            _ => panic!("expected Broken, got {result:?}"),
        }
    }

    #[test]
    fn passes_when_ug_pair_bridges_the_conflict() {
        // Simulates the 3×4 SAT-solved layout: column-3 copper-cable is
        // undergrounded at (3,9) → (3,11), letting iron-plate take
        // (3,10) as its own UG input without breaking copper-cable.
        // The walker must accept this.
        let existing = vec![
            belt(3, 8, EntityDirection::South, "copper-cable"),
            belt(3, 12, EntityDirection::South, "copper-cable"),
        ];
        let released: FxHashSet<(i32, i32)> = [(3, 9), (3, 10), (3, 11)].into_iter().collect();
        let proposed = vec![
            ug_in(3, 9, EntityDirection::South, "copper-cable"),
            ug_in(3, 10, EntityDirection::East, "iron-plate"),
            ug_out(3, 11, EntityDirection::South, "copper-cable"),
        ];
        let shadow = ShadowView::build(&existing, &released, &proposed);

        let tiles = vec![(3, 8), (3, 9), (3, 10), (3, 11), (3, 12)];
        let copper = AffectedPath {
            segment_id: "trunk:copper-cable#0",
            tiles: &tiles,
            item: "copper-cable",
        };
        let result = walk_affected(&[copper], &shadow);
        assert!(
            result.is_passed(),
            "expected Passed (legitimate UG bridge), got {result:?}"
        );
    }

    #[test]
    fn passes_when_hidden_middle_between_ug_pair_on_own_path() {
        // Path is entirely undergrounded from (5,5) to (5,8) with
        // hidden middles at (5,6) and (5,7). Shadow has nothing at the
        // middles — the walker's is_hidden_middle_of_own_ug check must
        // recognise them as legal.
        let proposed = vec![
            ug_in(5, 5, EntityDirection::South, "iron-plate"),
            ug_out(5, 8, EntityDirection::South, "iron-plate"),
        ];
        let shadow = ShadowView::build(&[], &FxHashSet::default(), &proposed);
        let tiles = vec![(5, 5), (5, 6), (5, 7), (5, 8)];
        let path = AffectedPath {
            segment_id: "tapoff:iron-plate#2",
            tiles: &tiles,
            item: "iron-plate",
        };
        let result = walk_affected(&[path], &shadow);
        assert!(result.is_passed(), "expected Passed, got {result:?}");
    }

    #[test]
    fn fails_on_missing_entity_outside_ug_pair() {
        // Path expects belts at (2,2)..(2,4) but (2,3) is empty in the
        // shadow and there's no UG pair to justify it.
        let existing = vec![
            belt(2, 2, EntityDirection::South, "iron-plate"),
            belt(2, 4, EntityDirection::South, "iron-plate"),
        ];
        let shadow = ShadowView::build(&existing, &FxHashSet::default(), &[]);
        let tiles = vec![(2, 2), (2, 3), (2, 4)];
        let path = AffectedPath {
            segment_id: "tapoff:iron-plate#0",
            tiles: &tiles,
            item: "iron-plate",
        };
        match walk_affected(&[path], &shadow) {
            WalkResult::Broken { breaks } => {
                assert_eq!(breaks.len(), 1);
                assert_eq!(breaks[0].tile, (2, 3));
                assert!(matches!(breaks[0].reason, BreakReason::MissingEntity));
            }
            r => panic!("expected Broken(MissingEntity), got {r:?}"),
        }
    }

    #[test]
    fn fails_on_unpaired_ug_in() {
        // Path has a UG input at (1,1) but no matching UG output later.
        let proposed = vec![ug_in(1, 1, EntityDirection::East, "iron-plate")];
        let shadow = ShadowView::build(&[], &FxHashSet::default(), &proposed);
        let tiles = vec![(1, 1), (2, 1), (3, 1)];
        let path = AffectedPath {
            segment_id: "tapoff:iron-plate#1",
            tiles: &tiles,
            item: "iron-plate",
        };
        match walk_affected(&[path], &shadow) {
            WalkResult::Broken { breaks } => {
                assert_eq!(breaks[0].tile, (1, 1));
                assert!(matches!(breaks[0].reason, BreakReason::UnpairedUgIn { .. }));
            }
            r => panic!("expected Broken(UnpairedUgIn), got {r:?}"),
        }
    }

    #[test]
    fn proposed_entity_wins_over_existing_at_same_tile() {
        // If the same tile appears in both existing and proposed, the
        // proposal must take priority — it's the region's authoritative
        // solution.
        let existing = vec![belt(4, 4, EntityDirection::South, "copper-cable")];
        let proposed = vec![belt(4, 4, EntityDirection::East, "iron-plate")];
        let shadow = ShadowView::build(&existing, &FxHashSet::default(), &proposed);
        let e = shadow.get((4, 4)).unwrap();
        assert_eq!(e.carries.as_deref(), Some("iron-plate"));
        assert_eq!(e.direction, EntityDirection::East);
    }
}
