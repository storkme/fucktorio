//! Region walker: reachability check for the junction solver's CEGAR
//! (counter-example guided) loop.
//!
//! When a `JunctionStrategy` returns a candidate solution for a growing
//! region, we can't commit it blindly — the strategy only knows about the
//! specs it was handed, not about other routed paths that touch the
//! region's footprint. The walker is the veto: it merges the proposed
//! entities into a shadow view of the world and asks, for every
//! affected path, "can items still flow from this path's entry to its
//! exit through the shadow's belt graph?" If any path can't, the
//! solution is rejected and the outer loop grows the region and retries.
//!
//! The check is **reachability**, not tile-match. SAT may legitimately
//! reroute a path via a longer surface detour or via underground —
//! tile-by-tile match would reject those even though the flow is fine.
//! BFS through the shadow's belt graph (filtered to entities carrying
//! the path's item, jumping UG pairs and splitter siblings) lets us
//! accept any topologically valid solution.
//!
//! The walker is intentionally decoupled from `GrowingRegion` and
//! `JunctionStrategyContext`. The caller constructs an `AffectedPath`
//! list and a `ShadowView`; we return a `WalkResult`. Everything else
//! lives in the caller.

use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::VecDeque;

use crate::common::{is_splitter, is_surface_belt, is_ug_belt, splitter_second_tile};
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
    ///
    /// Retained for compatibility with telemetry consumers; the
    /// reachability-based walker no longer produces it. UG pairing
    /// problems now surface as `Unreachable` instead.
    UnpairedUgIn { direction: EntityDirection },
    /// SAT solution leaves the path's exit unreachable from its entry
    /// in the shadow's belt graph. Catches surface detours that drop
    /// items mid-route, broken UG pairings, items hopping into the
    /// wrong-item belt, etc.
    Unreachable,
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
    let Some(&path_start) = path.tiles.first() else {
        return Ok(());
    };
    let Some(&end) = path.tiles.last() else {
        return Ok(());
    };

    // Find the first tile on the path whose shadow entity carries the
    // right item. SAT may have legitimately reassigned the path's
    // leading tile(s) — e.g. replaced the trunk's first surface belt
    // with an iron-plate UG input because copper now tunnels past
    // this point via an earlier UG pair. That's fine as long as the
    // item reaches the path somewhere and flows through to `end`.
    //
    // If the item never appears anywhere on the path, the SAT solution
    // really does drop it — that's MissingEntity at the original start.
    let start = path
        .tiles
        .iter()
        .copied()
        .find(|&t| {
            shadow
                .get(t)
                .map(|e| e.carries.as_deref() == Some(path.item))
                .unwrap_or(false)
        });
    let Some(start) = start else {
        return Err(WalkBreak {
            segment_id: path.segment_id.to_string(),
            tile: path_start,
            reason: BreakReason::MissingEntity,
        });
    };

    if start == end {
        return Ok(());
    }

    let (belt_dir, ug_pairs, sibs) = build_belt_graph_for_item(shadow, path.item);
    if bfs_reach(start, end, &belt_dir, &ug_pairs, &sibs) {
        Ok(())
    } else {
        Err(WalkBreak {
            segment_id: path.segment_id.to_string(),
            tile: start,
            reason: BreakReason::Unreachable,
        })
    }
}

// ---------------------------------------------------------------------------
// Belt graph extraction + BFS
// ---------------------------------------------------------------------------

fn is_ug(e: &PlacedEntity) -> bool {
    is_ug_belt(&e.name)
}

fn is_ug_in(e: &PlacedEntity) -> bool {
    is_ug(e) && e.io_type.as_deref() == Some("input")
}

fn is_ug_out(e: &PlacedEntity) -> bool {
    is_ug(e) && e.io_type.as_deref() == Some("output")
}

fn dir_to_vec(d: EntityDirection) -> (i32, i32) {
    match d {
        EntityDirection::North => (0, -1),
        EntityDirection::East => (1, 0),
        EntityDirection::South => (0, 1),
        EntityDirection::West => (-1, 0),
    }
}

/// Extract the per-item belt-flow graph from the shadow.
///
/// Mirrors `validate::belt_flow::build_*` helpers but localized to a
/// single item subset: we only care about tiles relevant to the path
/// we're checking.
///
/// Returns three maps:
/// - `belt_dir_map`: tile → output direction. Includes belts, UG-in,
///   UG-out, splitters (both tiles).
/// - `ug_pairs`: symmetric map of UG-in ↔ UG-out tiles. Pairing rule:
///   nearest UG-out matching direction + colinear with UG-in's facing,
///   no closer UG-in/out blocks the corridor.
/// - `splitter_siblings`: symmetric map for the two tiles of each
///   splitter.
fn build_belt_graph_for_item(
    shadow: &ShadowView,
    item: &str,
) -> (
    FxHashMap<(i32, i32), EntityDirection>,
    FxHashMap<(i32, i32), (i32, i32)>,
    FxHashMap<(i32, i32), (i32, i32)>,
) {
    let mut belt_dir_map: FxHashMap<(i32, i32), EntityDirection> = FxHashMap::default();
    let mut splitter_siblings: FxHashMap<(i32, i32), (i32, i32)> = FxHashMap::default();
    let mut ug_inputs: Vec<(i32, i32, EntityDirection)> = Vec::new();
    let mut ug_outputs: Vec<(i32, i32, EntityDirection)> = Vec::new();

    for e in shadow.by_tile.values() {
        if e.carries.as_deref() != Some(item) {
            continue;
        }
        if is_surface_belt(&e.name) {
            belt_dir_map.insert((e.x, e.y), e.direction);
        } else if is_splitter(&e.name) {
            let second = splitter_second_tile(e);
            belt_dir_map.insert((e.x, e.y), e.direction);
            belt_dir_map.insert(second, e.direction);
            splitter_siblings.insert((e.x, e.y), second);
            splitter_siblings.insert(second, (e.x, e.y));
        } else if is_ug_in(e) {
            belt_dir_map.insert((e.x, e.y), e.direction);
            ug_inputs.push((e.x, e.y, e.direction));
        } else if is_ug_out(e) {
            belt_dir_map.insert((e.x, e.y), e.direction);
            ug_outputs.push((e.x, e.y, e.direction));
        }
    }

    // UG pairing: for each UG-in pick the closest in-line UG-out facing
    // the same direction, item already filtered above.
    let mut ug_pairs: FxHashMap<(i32, i32), (i32, i32)> = FxHashMap::default();
    let mut used_outputs: FxHashSet<(i32, i32)> = FxHashSet::default();
    for &(ix, iy, idir) in &ug_inputs {
        let (dx, dy) = dir_to_vec(idir);
        let mut best: Option<(i32, i32)> = None;
        let mut best_dist = i32::MAX;
        for &(ox, oy, odir) in &ug_outputs {
            if odir != idir || used_outputs.contains(&(ox, oy)) {
                continue;
            }
            let rx = ox - ix;
            let ry = oy - iy;
            // Must be colinear with the UG-in's facing direction.
            let dist = if dx != 0 {
                if ry != 0 || (rx > 0) != (dx > 0) {
                    continue;
                }
                rx.abs()
            } else {
                if rx != 0 || (ry > 0) != (dy > 0) {
                    continue;
                }
                ry.abs()
            };
            if dist > 1 && dist < best_dist {
                best_dist = dist;
                best = Some((ox, oy));
            }
        }
        if let Some(out) = best {
            ug_pairs.insert((ix, iy), out);
            ug_pairs.insert(out, (ix, iy));
            used_outputs.insert(out);
        }
    }

    (belt_dir_map, ug_pairs, splitter_siblings)
}

/// Direction-aware BFS through the belt graph from `start`. Returns
/// true iff `target` is visited. Mirrors
/// `validate::belt_flow::bfs_belt_downstream` but short-circuits.
fn bfs_reach(
    start: (i32, i32),
    target: (i32, i32),
    belt_dir_map: &FxHashMap<(i32, i32), EntityDirection>,
    ug_pairs: &FxHashMap<(i32, i32), (i32, i32)>,
    splitter_siblings: &FxHashMap<(i32, i32), (i32, i32)>,
) -> bool {
    if !belt_dir_map.contains_key(&start) {
        return false;
    }
    if start == target {
        return true;
    }
    let mut visited: FxHashSet<(i32, i32)> = FxHashSet::default();
    let mut queue: VecDeque<(i32, i32)> = VecDeque::new();
    visited.insert(start);
    queue.push_back(start);

    while let Some((x, y)) = queue.pop_front() {
        // Step in the entity's output direction.
        if let Some(&d) = belt_dir_map.get(&(x, y)) {
            let (dx, dy) = dir_to_vec(d);
            let nb = (x + dx, y + dy);
            if belt_dir_map.contains_key(&nb) && visited.insert(nb) {
                if nb == target {
                    return true;
                }
                queue.push_back(nb);
            }
        }
        // Underground tunnel jump.
        if let Some(&paired) = ug_pairs.get(&(x, y)) {
            if visited.insert(paired) {
                if paired == target {
                    return true;
                }
                queue.push_back(paired);
            }
        }
        // Splitter sibling — items on one tile are also on the other.
        if let Some(&sib) = splitter_siblings.get(&(x, y)) {
            if visited.insert(sib) {
                if sib == target {
                    return true;
                }
                queue.push_back(sib);
            }
        }
    }
    false
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
        // that SAT placed there. The walker must reject this — copper
        // can't reach (3,11) from (3,8) because (3,10) carries iron.
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
                // Reachability reports the break at the entry tile;
                // the iron-plate gap at (3,10) makes (3,11) unreachable.
                assert_eq!(breaks[0].tile, (3, 8));
                assert!(matches!(breaks[0].reason, BreakReason::Unreachable));
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
        // shadow and there's no UG pair to justify it. Reachability
        // reports the break at the path entry — the gap leaves (2,4)
        // unreachable from (2,2).
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
                assert_eq!(breaks[0].tile, (2, 2));
                assert!(matches!(breaks[0].reason, BreakReason::Unreachable));
            }
            r => panic!("expected Broken(Unreachable), got {r:?}"),
        }
    }

    #[test]
    fn fails_on_unpaired_ug_in() {
        // Path has a UG input at (1,1) but no matching UG output later.
        // Without a paired UG-out, the BFS can't tunnel anywhere from
        // (1,1) — it tries to step East to (2,1) but no entity there,
        // so (3,1) is unreachable. Reported as Unreachable (not the
        // legacy UnpairedUgIn variant).
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
                assert!(matches!(breaks[0].reason, BreakReason::Unreachable));
            }
            r => panic!("expected Broken(Unreachable), got {r:?}"),
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

    #[test]
    fn passes_when_sat_detours_around_obstacle() {
        // Original ghost path: iron-plate East at (2,10)→(3,10)→(4,10)
        // →(5,10). SAT can't use (3,10)/(4,10) (foreign Permanent
        // belts there), so it routes south then back up:
        //   (2,10) S → (2,11) E → (3,11) E → (4,11) E → (5,11) N → (5,10).
        // Reachability must accept this even though the original tile
        // sequence is no longer item-matched mid-path.
        let existing = vec![
            // The blocking foreign belts (copper) — appear in shadow,
            // not on the path's released set.
            belt(3, 10, EntityDirection::South, "copper-cable"),
            belt(4, 10, EntityDirection::South, "copper-cable"),
        ];
        let released: FxHashSet<(i32, i32)> = [(2, 10), (5, 10)].into_iter().collect();
        let proposed = vec![
            belt(2, 10, EntityDirection::South, "iron-plate"),
            belt(2, 11, EntityDirection::East, "iron-plate"),
            belt(3, 11, EntityDirection::East, "iron-plate"),
            belt(4, 11, EntityDirection::East, "iron-plate"),
            belt(5, 11, EntityDirection::North, "iron-plate"),
            belt(5, 10, EntityDirection::East, "iron-plate"),
        ];
        let shadow = ShadowView::build(&existing, &released, &proposed);

        let tiles = vec![(2, 10), (3, 10), (4, 10), (5, 10)];
        let iron = AffectedPath {
            segment_id: "tap:iron-plate#0",
            tiles: &tiles,
            item: "iron-plate",
        };
        let result = walk_affected(&[iron], &shadow);
        assert!(
            result.is_passed(),
            "SAT detour around obstacle should pass reachability, got {result:?}"
        );
    }

    #[test]
    fn passes_when_path_start_is_sat_reassigned() {
        // The junction (3,18) iter-1 variant-east case: trunk:copper-cable
        // starts at (3,18), but SAT put an iron-plate UG-in there because
        // copper now tunnels from (3,17) UG-in → (3,19) UG-out, bridging
        // past (3,18). The walker used to reject at (3,18) because the
        // entry-tile item-match ran before BFS. With the fix, it walks
        // forward to the first copper tile on the path and BFSes from
        // there.
        let existing = vec![
            // Downstream of the zone — still copper surface belts.
            belt(3, 20, EntityDirection::South, "copper-cable"),
            belt(3, 21, EntityDirection::South, "copper-cable"),
        ];
        let released: FxHashSet<(i32, i32)> = [(3, 17), (3, 18), (3, 19)].into_iter().collect();
        let proposed = vec![
            ug_in(3, 17, EntityDirection::South, "copper-cable"),
            ug_in(3, 18, EntityDirection::East, "iron-plate"),
            ug_out(3, 19, EntityDirection::South, "copper-cable"),
        ];
        let shadow = ShadowView::build(&existing, &released, &proposed);

        // trunk:copper-cable:3 path as recorded by the ghost router.
        // First tile is (3,18) — now iron-plate — but copper is still
        // reachable via the UG corridor at (3,17) → (3,19).
        let tiles = vec![(3, 18), (3, 19), (3, 20), (3, 21)];
        let copper = AffectedPath {
            segment_id: "trunk:copper-cable:3",
            tiles: &tiles,
            item: "copper-cable",
        };
        let result = walk_affected(&[copper], &shadow);
        assert!(
            result.is_passed(),
            "walker must tolerate SAT reassigning the path's leading tile(s) when the item still flows; got {result:?}"
        );
    }

    #[test]
    fn fails_when_item_missing_from_entire_path() {
        // If SAT's proposal genuinely drops the item — no surface belt,
        // no UG pair — every tile on the path is missing or mis-itemed,
        // and the walker must reject with MissingEntity at the path's
        // original start tile.
        let released: FxHashSet<(i32, i32)> = [(3, 17), (3, 18), (3, 19)].into_iter().collect();
        let proposed = vec![
            // SAT took over these tiles but dropped copper entirely —
            // only iron belts, no copper anywhere on the path.
            belt(3, 17, EntityDirection::South, "iron-plate"),
            belt(3, 18, EntityDirection::South, "iron-plate"),
            belt(3, 19, EntityDirection::South, "iron-plate"),
        ];
        let shadow = ShadowView::build(&[], &released, &proposed);
        let tiles = vec![(3, 17), (3, 18), (3, 19)];
        let copper = AffectedPath {
            segment_id: "trunk:copper-cable:3",
            tiles: &tiles,
            item: "copper-cable",
        };
        match walk_affected(&[copper], &shadow) {
            WalkResult::Broken { breaks } => {
                assert_eq!(breaks.len(), 1);
                assert_eq!(breaks[0].tile, (3, 17));
                assert!(
                    matches!(breaks[0].reason, BreakReason::MissingEntity),
                    "got {:?}",
                    breaks[0].reason
                );
            }
            r => panic!("expected Broken(MissingEntity), got {r:?}"),
        }
    }

    #[test]
    fn passes_when_ug_corridor_replaces_surface() {
        // Original ghost path: iron-plate East at (2,10)→(3,10)→(4,10)
        // →(5,10) all surface. SAT collapses the middle into a UG
        // corridor: UG-in East at (2,10), UG-out East at (5,10), with
        // foreign Permanent copper-cable belts at (3,10)/(4,10) in the
        // shadow.
        let existing = vec![
            belt(3, 10, EntityDirection::South, "copper-cable"),
            belt(4, 10, EntityDirection::South, "copper-cable"),
        ];
        let released: FxHashSet<(i32, i32)> = [(2, 10), (5, 10)].into_iter().collect();
        let proposed = vec![
            ug_in(2, 10, EntityDirection::East, "iron-plate"),
            ug_out(5, 10, EntityDirection::East, "iron-plate"),
        ];
        let shadow = ShadowView::build(&existing, &released, &proposed);

        let tiles = vec![(2, 10), (3, 10), (4, 10), (5, 10)];
        let iron = AffectedPath {
            segment_id: "tap:iron-plate#0",
            tiles: &tiles,
            item: "iron-plate",
        };
        let result = walk_affected(&[iron], &shadow);
        assert!(
            result.is_passed(),
            "UG corridor through foreign tiles should pass via ug_pairs, got {result:?}"
        );
    }
}
