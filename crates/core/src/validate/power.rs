//! Power coverage validation.
//!
//! Port of `check_power_coverage` from `src/validate.py`.
//!
//! Checks that every machine is within range of a medium-electric-pole.
//! A medium electric pole has a 7×7 supply area (3 tiles in each direction
//! from the pole center).

use crate::common::machine_size;
use crate::models::LayoutResult;
use crate::validate::{Severity, ValidationIssue};

/// Machine entities that must be covered by power.
///
/// Mirrors `_MACHINE_ENTITIES` from `src/validate.py` (derived from `_MACHINE_SIZE`).
const MACHINE_ENTITIES: &[&str] = &[
    "assembling-machine-1",
    "assembling-machine-2",
    "assembling-machine-3",
    "chemical-plant",
    "electric-furnace",
    "oil-refinery",
];

/// Radius (in tiles) of a medium-electric-pole supply area.
const POLE_RANGE: i32 = 3;

/// Check that every machine is within range of a medium-electric-pole.
///
/// Returns a list of [`ValidationIssue`]s (all with severity `Warning`).
pub fn check_power_coverage(layout_result: &LayoutResult) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    let pole_positions: Vec<(i32, i32)> = layout_result
        .entities
        .iter()
        .filter(|e| e.name == "medium-electric-pole")
        .map(|e| (e.x, e.y))
        .collect();

    if pole_positions.is_empty() {
        issues.push(ValidationIssue::new(
            Severity::Warning,
            "power",
            "No power poles in layout",
        ));
        return issues;
    }

    for e in &layout_result.entities {
        if !MACHINE_ENTITIES.contains(&e.name.as_str()) {
            continue;
        }

        let size = machine_size(&e.name) as i32;
        // Machine center tile (integer division, same as Python `size // 2`)
        let cx = e.x + size / 2;
        let cy = e.y + size / 2;

        let powered = pole_positions
            .iter()
            .any(|(px, py)| (cx - px).abs() <= POLE_RANGE && (cy - py).abs() <= POLE_RANGE);

        if !powered {
            issues.push(ValidationIssue::with_pos(
                Severity::Warning,
                "power",
                format!("{} at ({},{}): not in range of any power pole", e.name, e.x, e.y),
                e.x,
                e.y,
            ));
        }
    }

    issues
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{EntityDirection, PlacedEntity};

    fn machine(name: &str, x: i32, y: i32) -> PlacedEntity {
        PlacedEntity {
            name: name.to_string(),
            x,
            y,
            direction: EntityDirection::North,
            recipe: Some("iron-gear-wheel".to_string()),
            io_type: None,
            carries: None,
            mirror: false,
            segment_id: None,
        }
    }

    fn pole(x: i32, y: i32) -> PlacedEntity {
        PlacedEntity {
            name: "medium-electric-pole".to_string(),
            x,
            y,
            direction: EntityDirection::North,
            recipe: None,
            io_type: None,
            carries: None,
            mirror: false,
            segment_id: None,
        }
    }

    fn layout(entities: Vec<PlacedEntity>) -> LayoutResult {
        LayoutResult {
            entities,
            width: 20,
            height: 20,
        }
    }

    // --- No poles at all ---

    #[test]
    fn no_poles_returns_single_warning() {
        let lr = layout(vec![machine("assembling-machine-1", 0, 0)]);
        let issues = check_power_coverage(&lr);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, Severity::Warning);
        assert_eq!(issues[0].category, "power");
        assert!(issues[0].message.contains("No power poles"));
    }

    #[test]
    fn no_poles_empty_layout_returns_single_warning() {
        let lr = layout(vec![]);
        let issues = check_power_coverage(&lr);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].message, "No power poles in layout");
    }

    // --- Machine within range ---

    #[test]
    fn machine_within_range_no_issues() {
        // 3x3 machine at (0,0): center = (1,1); pole at (4,4): distance = (3,3) — exactly at edge
        let lr = layout(vec![machine("assembling-machine-1", 0, 0), pole(4, 4)]);
        let issues = check_power_coverage(&lr);
        assert_eq!(issues.len(), 0);
    }

    #[test]
    fn machine_directly_under_pole_no_issues() {
        let lr = layout(vec![machine("assembling-machine-2", 0, 0), pole(1, 1)]);
        let issues = check_power_coverage(&lr);
        assert_eq!(issues.len(), 0);
    }

    // --- Machine out of range ---

    #[test]
    fn machine_out_of_range_returns_warning() {
        // 3x3 machine at (0,0): center = (1,1); pole at (10,10): clearly out of range
        let lr = layout(vec![machine("assembling-machine-1", 0, 0), pole(10, 10)]);
        let issues = check_power_coverage(&lr);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, Severity::Warning);
        assert_eq!(issues[0].category, "power");
        assert!(issues[0].message.contains("assembling-machine-1"));
        assert_eq!(issues[0].x, Some(0));
        assert_eq!(issues[0].y, Some(0));
    }

    #[test]
    fn machine_just_outside_range_returns_warning() {
        // 3x3 machine at (0,0): center = (1,1); pole at (5,5): distance = (4,4) > 3
        let lr = layout(vec![machine("assembling-machine-3", 0, 0), pole(5, 5)]);
        let issues = check_power_coverage(&lr);
        assert_eq!(issues.len(), 1);
    }

    // --- Oil refinery (5x5) ---

    #[test]
    fn oil_refinery_center_computed_correctly() {
        // 5x5 oil-refinery at (0,0): center = (2,2); pole at (5,5): distance = (3,3) — at edge
        let lr = layout(vec![
            PlacedEntity {
                name: "oil-refinery".to_string(),
                x: 0,
                y: 0,
                direction: EntityDirection::North,
                recipe: Some("basic-oil-processing".to_string()),
                io_type: None,
                carries: None,
                mirror: false,
                segment_id: None,
            },
            pole(5, 5),
        ]);
        let issues = check_power_coverage(&lr);
        assert_eq!(issues.len(), 0, "oil-refinery center (2,2) should be within range of pole at (5,5)");
    }

    // --- Multiple machines, mixed coverage ---

    #[test]
    fn multiple_machines_only_uncovered_reported() {
        let lr = layout(vec![
            machine("assembling-machine-1", 0, 0),  // center (1,1), pole at (2,2) → in range
            machine("assembling-machine-2", 15, 15), // center (16,16), out of range
            pole(2, 2),
        ]);
        let issues = check_power_coverage(&lr);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].x, Some(15));
    }

    #[test]
    fn multiple_poles_any_covers_machine() {
        // Machine center (1,1); no single pole within range, but two poles together cover it
        let lr = layout(vec![
            machine("assembling-machine-1", 0, 0),
            pole(1, 10), // out of range
            pole(1, 1),  // in range
        ]);
        let issues = check_power_coverage(&lr);
        assert_eq!(issues.len(), 0);
    }

    // --- Non-machine entities are ignored ---

    #[test]
    fn non_machine_entities_ignored() {
        let belt = PlacedEntity {
            name: "transport-belt".to_string(),
            x: 0,
            y: 0,
            direction: EntityDirection::North,
            recipe: None,
            io_type: None,
            carries: None,
            mirror: false,
            segment_id: None,
        };
        // No poles, but only a belt → the "No power poles" warning fires (not a per-entity warning)
        let lr = layout(vec![belt]);
        let issues = check_power_coverage(&lr);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("No power poles"));
    }

    // --- All machine types covered ---

    #[test]
    fn all_machine_types_checked() {
        let machine_names = [
            "assembling-machine-1",
            "assembling-machine-2",
            "assembling-machine-3",
            "chemical-plant",
            "electric-furnace",
            "oil-refinery",
        ];
        for name in &machine_names {
            let lr = layout(vec![machine(name, 0, 0)]);
            // No poles → warning
            let issues = check_power_coverage(&lr);
            assert_eq!(issues.len(), 1, "{} should trigger 'No power poles' warning", name);
        }
    }

    // --- Done-when criterion: layout missing power reports uncovered machines ---

    #[test]
    fn layout_missing_power_reports_uncovered_machines() {
        // 3 machines, no poles → "No power poles" single warning
        let lr = layout(vec![
            machine("assembling-machine-1", 0, 0),
            machine("assembling-machine-2", 5, 0),
            machine("assembling-machine-3", 10, 0),
        ]);
        let issues = check_power_coverage(&lr);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("No power poles"));
    }

    #[test]
    fn layout_with_full_coverage_reports_zero_issues() {
        // Pole at (1,1) covers machine at (0,0) with center (1,1) → distance 0
        let lr = layout(vec![machine("assembling-machine-1", 0, 0), pole(1, 1)]);
        let issues = check_power_coverage(&lr);
        assert_eq!(issues.len(), 0);
    }

    #[test]
    fn pole_range_boundary_exact_3_tiles() {
        // Machine center at (0,0) (1x1 for simplicity — but our smallest is 3x3)
        // Use 3x3 at (-1,-1) so center = (0,0); pole at (3,0) → distance = exactly 3
        let lr = layout(vec![machine("assembling-machine-1", -1, -1), pole(3, 0)]);
        let issues = check_power_coverage(&lr);
        assert_eq!(issues.len(), 0, "distance of exactly 3 should be within range");
    }

    #[test]
    fn pole_range_boundary_4_tiles_out_of_range() {
        // 3x3 at (-1,-1) center (0,0); pole at (4,0) → distance = 4 > POLE_RANGE
        let lr = layout(vec![machine("assembling-machine-1", -1, -1), pole(4, 0)]);
        let issues = check_power_coverage(&lr);
        assert_eq!(issues.len(), 1, "distance of 4 should be out of range");
    }
}
