//! Underground belt validation checks.
//!
//! Port of `src/validate.py` — `check_underground_belt_pairs`,
//! `check_underground_belt_sideloading`, and
//! `check_underground_belt_entry_sideload`.

use rustc_hash::{FxHashMap, FxHashSet};

use crate::common::{dir_to_vec, ug_max_reach};
use crate::models::{EntityDirection, LayoutResult, PlacedEntity};

use super::{Severity, ValidationIssue};

// ---------------------------------------------------------------------------
// Entity-name predicates
// ---------------------------------------------------------------------------

const SURFACE_BELT_ENTITIES: &[&str] = &[
    "transport-belt",
    "fast-transport-belt",
    "express-transport-belt",
];
const UG_BELT_ENTITIES: &[&str] = &[
    "underground-belt",
    "fast-underground-belt",
    "express-underground-belt",
];
const SPLITTER_ENTITIES: &[&str] = &["splitter", "fast-splitter", "express-splitter"];

fn is_surface_belt(name: &str) -> bool {
    SURFACE_BELT_ENTITIES.contains(&name)
}
fn is_ug_belt(name: &str) -> bool {
    UG_BELT_ENTITIES.contains(&name)
}
fn is_splitter(name: &str) -> bool {
    SPLITTER_ENTITIES.contains(&name)
}
fn is_any_belt(name: &str) -> bool {
    is_surface_belt(name) || is_ug_belt(name) || is_splitter(name)
}

/// Map underground belt entity name to its surface belt tier for reach lookup.
fn ug_belt_surface_tier(ug_name: &str) -> &'static str {
    match ug_name {
        "fast-underground-belt" => "fast-transport-belt",
        "express-underground-belt" => "express-transport-belt",
        _ => "transport-belt",
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Second tile occupied by a splitter (they span 2 tiles perpendicular to flow).
fn splitter_second_tile(e: &PlacedEntity) -> (i32, i32) {
    match e.direction {
        EntityDirection::North | EntityDirection::South => (e.x + 1, e.y),
        EntityDirection::East | EntityDirection::West => (e.x, e.y + 1),
    }
}

/// Build a map of `(x, y) → direction` for all belt-like entities,
/// expanding splitters to both tiles they occupy.
fn build_belt_dir_map(entities: &[PlacedEntity]) -> FxHashMap<(i32, i32), EntityDirection> {
    let mut map = FxHashMap::default();
    for e in entities {
        if !is_any_belt(&e.name) {
            continue;
        }
        map.insert((e.x, e.y), e.direction);
        if is_splitter(&e.name) {
            map.insert(splitter_second_tile(e), e.direction);
        }
    }
    map
}

// ---------------------------------------------------------------------------
// check_underground_belt_pairs
// ---------------------------------------------------------------------------

/// Check underground belt pairing: every input has a matching output.
///
/// Validates:
/// - Each UG input has a matching output (same direction, same axis).
/// - Distance between pairs does not exceed max reach for the tier.
/// - No intermediate UG belt of the same tier intercepts the pair.
pub fn check_underground_belt_pairs(layout_result: &LayoutResult) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    let mut ug_inputs: Vec<&PlacedEntity> = Vec::new();
    let mut ug_outputs: Vec<&PlacedEntity> = Vec::new();
    let mut all_ug: Vec<&PlacedEntity> = Vec::new();

    for e in &layout_result.entities {
        if !is_ug_belt(&e.name) {
            continue;
        }
        all_ug.push(e);
        match e.io_type.as_deref() {
            Some("input") => ug_inputs.push(e),
            Some("output") => ug_outputs.push(e),
            _ => {}
        }
    }

    let mut used_outputs: FxHashSet<(i32, i32)> = FxHashSet::default();

    for inp in &ug_inputs {
        let (dx, dy) = dir_to_vec(inp.direction);
        let max_reach = ug_max_reach(ug_belt_surface_tier(&inp.name));

        // Find the nearest matching output along the direction vector.
        let mut best_out: Option<&PlacedEntity> = None;
        let mut best_dist: i32 = i32::MAX;

        for out in &ug_outputs {
            if used_outputs.contains(&(out.x, out.y)) {
                continue;
            }
            if out.direction != inp.direction || out.name != inp.name {
                continue;
            }

            let rx = out.x - inp.x;
            let ry = out.y - inp.y;

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
                best_out = Some(out);
            }
        }

        if best_out.is_none() {
            issues.push(ValidationIssue::with_pos(
                Severity::Error,
                "underground-belt",
                format!(
                    "Unpaired underground belt input at ({},{}) facing {:?}: no matching output found",
                    inp.x, inp.y, inp.direction
                ),
                inp.x,
                inp.y,
            ));
            continue;
        }

        let best_out = best_out.unwrap();
        used_outputs.insert((best_out.x, best_out.y));

        // max_reach = gap tiles, so max entry-to-exit distance = max_reach + 1.
        if best_dist > (max_reach as i32) + 1 {
            issues.push(ValidationIssue::with_pos(
                Severity::Error,
                "underground-belt",
                format!(
                    "Underground belt pair ({},{})->({},{}) distance {} exceeds max reach {} for {}",
                    inp.x,
                    inp.y,
                    best_out.x,
                    best_out.y,
                    best_dist,
                    max_reach,
                    inp.name
                ),
                inp.x,
                inp.y,
            ));
        }

        // Warn about same-tier belts that intercept the pair.
        for ug in &all_ug {
            if std::ptr::eq(*ug, *inp) || std::ptr::eq(*ug, best_out) {
                continue;
            }
            if ug.name != inp.name || ug.direction != inp.direction {
                continue;
            }
            let rx = ug.x - inp.x;
            let ry = ug.y - inp.y;

            let udist = if dx != 0 {
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

            if udist > 0 && udist < best_dist {
                issues.push(ValidationIssue::with_pos(
                    Severity::Warning,
                    "underground-belt",
                    format!(
                        "Underground belt at ({},{}) intercepts pair ({},{})->({},{})",
                        ug.x, ug.y, inp.x, inp.y, best_out.x, best_out.y
                    ),
                    ug.x,
                    ug.y,
                ));
            }
        }
    }

    // Outputs not claimed by any input are unpaired.
    for out in &ug_outputs {
        if !used_outputs.contains(&(out.x, out.y)) {
            issues.push(ValidationIssue::with_pos(
                Severity::Error,
                "underground-belt",
                format!(
                    "Unpaired underground belt output at ({},{}) facing {:?}: no matching input found",
                    out.x, out.y, out.direction
                ),
                out.x,
                out.y,
            ));
        }
    }

    issues
}

// ---------------------------------------------------------------------------
// check_underground_belt_sideloading
// ---------------------------------------------------------------------------

/// Check underground belt exit sideloading geometry.
///
/// For each UG belt output, checks what is on the tile it exits onto:
/// - Perpendicular sideload: valid (feeds near lane).
/// - Head-on collision (opposite direction, same axis): error.
pub fn check_underground_belt_sideloading(layout_result: &LayoutResult) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    let belt_dir = build_belt_dir_map(&layout_result.entities);

    for e in &layout_result.entities {
        if !is_ug_belt(&e.name) || e.io_type.as_deref() != Some("output") {
            continue;
        }

        let (dx, dy) = dir_to_vec(e.direction);
        let exit_tile = (e.x + dx, e.y + dy);

        let Some(&target_dir) = belt_dir.get(&exit_tile) else {
            continue;
        };

        let (tdx, tdy) = dir_to_vec(target_dir);
        if dx * tdx + dy * tdy < 0 {
            issues.push(ValidationIssue::with_pos(
                Severity::Error,
                "underground-belt",
                format!(
                    "Underground belt exit at ({},{}) facing {:?} collides head-on with belt at ({},{}) facing {:?}",
                    e.x, e.y, e.direction,
                    exit_tile.0, exit_tile.1, target_dir
                ),
                e.x,
                e.y,
            ));
        }
    }

    issues
}

// ---------------------------------------------------------------------------
// check_underground_belt_entry_sideload
// ---------------------------------------------------------------------------

/// Check that underground belt inputs are fed straight, not from the side.
///
/// Sideloading onto a UG input only fills the far lane — items on the near
/// lane are lost.  In bus routing this is almost always a bug.
pub fn check_underground_belt_entry_sideload(layout_result: &LayoutResult) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    // Surface belts, splitters, and UG outputs act as feeders.
    let mut belt_dir: FxHashMap<(i32, i32), EntityDirection> = FxHashMap::default();
    let mut ug_inputs: Vec<&PlacedEntity> = Vec::new();

    for e in &layout_result.entities {
        if is_surface_belt(&e.name) || is_splitter(&e.name) {
            belt_dir.insert((e.x, e.y), e.direction);
            if is_splitter(&e.name) {
                belt_dir.insert(splitter_second_tile(e), e.direction);
            }
        } else if is_ug_belt(&e.name) {
            match e.io_type.as_deref() {
                Some("output") => {
                    belt_dir.insert((e.x, e.y), e.direction);
                }
                Some("input") => {
                    ug_inputs.push(e);
                }
                _ => {}
            }
        }
    }

    for ug in &ug_inputs {
        let (ug_dx, ug_dy) = dir_to_vec(ug.direction);

        for (ndx, ndy) in [(0i32, -1i32), (0, 1), (-1, 0), (1, 0)] {
            let (nx, ny) = (ug.x + ndx, ug.y + ndy);
            let Some(&n_dir) = belt_dir.get(&(nx, ny)) else {
                continue;
            };
            let (n_dx, n_dy) = dir_to_vec(n_dir);

            // Skip neighbours that don't flow into this UG input tile.
            if (nx + n_dx, ny + n_dy) != (ug.x, ug.y) {
                continue;
            }

            // dot == 0 → perpendicular sideload (only one lane loaded).
            if n_dx * ug_dx + n_dy * ug_dy == 0 {
                issues.push(ValidationIssue::with_pos(
                    Severity::Warning,
                    "underground-belt",
                    format!(
                        "Belt at ({},{}) facing {:?} sideloads into underground input at ({},{}) facing {:?} \
                         — only one lane loaded, must feed UG inputs straight",
                        nx, ny, n_dir, ug.x, ug.y, ug.direction
                    ),
                    ug.x,
                    ug.y,
                ));
            }
        }
    }

    issues
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{EntityDirection, LayoutResult, PlacedEntity};

    fn ug(x: i32, y: i32, dir: EntityDirection, io_type: &str) -> PlacedEntity {
        PlacedEntity {
            name: "underground-belt".to_string(),
            x,
            y,
            direction: dir,
            io_type: Some(io_type.to_string()),
            ..Default::default()
        }
    }

    fn belt(x: i32, y: i32, dir: EntityDirection) -> PlacedEntity {
        PlacedEntity {
            name: "transport-belt".to_string(),
            x,
            y,
            direction: dir,
            ..Default::default()
        }
    }

    fn layout(entities: Vec<PlacedEntity>) -> LayoutResult {
        LayoutResult { entities, width: 20, height: 20, ..Default::default() }
    }

    // --- check_underground_belt_pairs ---

    #[test]
    fn pairs_valid_horizontal() {
        let lr = layout(vec![
            ug(0, 0, EntityDirection::East, "input"),
            ug(3, 0, EntityDirection::East, "output"),
        ]);
        assert!(check_underground_belt_pairs(&lr).is_empty());
    }

    #[test]
    fn pairs_valid_vertical() {
        let lr = layout(vec![
            ug(0, 0, EntityDirection::South, "input"),
            ug(0, 3, EntityDirection::South, "output"),
        ]);
        assert!(check_underground_belt_pairs(&lr).is_empty());
    }

    #[test]
    fn pairs_unpaired_input() {
        let lr = layout(vec![ug(0, 0, EntityDirection::East, "input")]);
        let issues = check_underground_belt_pairs(&lr);
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("Unpaired"));
        assert!(errors[0].message.contains("input"));
    }

    #[test]
    fn pairs_unpaired_output() {
        let lr = layout(vec![ug(5, 0, EntityDirection::East, "output")]);
        let issues = check_underground_belt_pairs(&lr);
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("Unpaired"));
        assert!(errors[0].message.contains("output"));
    }

    #[test]
    fn pairs_over_range() {
        // transport-belt max reach = 4; distance 6 exceeds max_reach+1 = 5.
        let lr = layout(vec![
            ug(0, 0, EntityDirection::East, "input"),
            ug(6, 0, EntityDirection::East, "output"),
        ]);
        let issues = check_underground_belt_pairs(&lr);
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        assert!(errors.iter().any(|e| e.message.contains("exceeds max reach")));
    }

    #[test]
    fn pairs_at_max_range() {
        // transport-belt max_reach = 4; distance 4 is within max_reach+1 = 5.
        let lr = layout(vec![
            ug(0, 0, EntityDirection::East, "input"),
            ug(4, 0, EntityDirection::East, "output"),
        ]);
        let issues = check_underground_belt_pairs(&lr);
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        assert!(errors.is_empty());
    }

    #[test]
    fn pairs_wrong_direction_not_paired() {
        let lr = layout(vec![
            ug(0, 0, EntityDirection::East, "input"),
            ug(3, 0, EntityDirection::West, "output"),
        ]);
        let issues = check_underground_belt_pairs(&lr);
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn pairs_intercepting_ug_warning() {
        let lr = layout(vec![
            ug(0, 0, EntityDirection::East, "input"),
            ug(2, 0, EntityDirection::East, "input"),
            ug(3, 0, EntityDirection::East, "output"),
        ]);
        let issues = check_underground_belt_pairs(&lr);
        let warnings: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Warning).collect();
        assert!(warnings.iter().any(|w| w.message.contains("intercepts")));
    }

    // --- check_underground_belt_sideloading ---

    #[test]
    fn sideload_no_issue_same_direction() {
        let lr = layout(vec![
            ug(0, 0, EntityDirection::East, "output"),
            belt(1, 0, EntityDirection::East),
        ]);
        assert!(check_underground_belt_sideloading(&lr).is_empty());
    }

    #[test]
    fn sideload_head_on_collision_error() {
        let lr = layout(vec![
            ug(0, 0, EntityDirection::East, "output"),
            belt(1, 0, EntityDirection::West),
        ]);
        let issues = check_underground_belt_sideloading(&lr);
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("head-on"));
    }

    #[test]
    fn sideload_perpendicular_ok() {
        let lr = layout(vec![
            ug(0, 0, EntityDirection::East, "output"),
            belt(1, 0, EntityDirection::North),
        ]);
        let issues = check_underground_belt_sideloading(&lr);
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        assert!(errors.is_empty());
    }

    #[test]
    fn sideload_input_ug_ignored() {
        let lr = layout(vec![
            ug(0, 0, EntityDirection::East, "input"),
            belt(1, 0, EntityDirection::West),
        ]);
        assert!(check_underground_belt_sideloading(&lr).is_empty());
    }

    // --- check_underground_belt_entry_sideload ---

    #[test]
    fn entry_sideload_straight_feed_ok() {
        let lr = layout(vec![
            belt(0, 0, EntityDirection::East),
            ug(1, 0, EntityDirection::East, "input"),
        ]);
        assert!(check_underground_belt_entry_sideload(&lr).is_empty());
    }

    #[test]
    fn entry_sideload_perpendicular_warns() {
        // Belt at (0,0) faces South → points to (0,1); UG input at (0,1) faces East.
        // Perpendicular feed → only one lane loaded.
        let lr = layout(vec![
            belt(0, 0, EntityDirection::South),
            ug(0, 1, EntityDirection::East, "input"),
        ]);
        let issues = check_underground_belt_entry_sideload(&lr);
        let warnings: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Warning).collect();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].message.contains("sideloads into underground input"));
    }

    #[test]
    fn entry_sideload_no_feeder_no_issue() {
        let lr = layout(vec![ug(5, 5, EntityDirection::East, "input")]);
        assert!(check_underground_belt_entry_sideload(&lr).is_empty());
    }

    #[test]
    fn entry_sideload_ug_output_as_feeder_straight_ok() {
        let lr = layout(vec![
            ug(0, 0, EntityDirection::East, "output"),
            ug(1, 0, EntityDirection::East, "input"),
        ]);
        assert!(check_underground_belt_entry_sideload(&lr).is_empty());
    }
}
