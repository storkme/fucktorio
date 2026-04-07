//! Inserter chain validity and direction checks.
//!
//! Port of `check_inserter_chains` and `check_inserter_direction` from
//! `src/validate.py`.

use rustc_hash::FxHashSet;

use crate::common::{dir_to_vec, is_machine_entity, machine_size, machine_tiles};
use crate::models::{LayoutResult, SolverResult};

use super::{Severity, ValidationIssue};

// ── Entity classification ────────────────────────────────────────────────────

const INSERTER_ENTITIES: &[&str] =
    &["inserter", "long-handed-inserter", "fast-inserter", "stack-inserter"];

fn is_inserter(name: &str) -> bool {
    INSERTER_ENTITIES.contains(&name)
}

/// Inserter reach: how many tiles away the pick-up / drop position is.
fn inserter_reach(name: &str) -> i32 {
    match name {
        "long-handed-inserter" => 2,
        _ => 1,
    }
}

// ── Helper: build machine tile set ───────────────────────────────────────────

fn build_machine_tile_set(layout: &LayoutResult) -> FxHashSet<(i32, i32)> {
    let mut tiles = FxHashSet::default();
    for e in &layout.entities {
        if is_machine_entity(&e.name) {
            tiles.extend(machine_tiles(e.x, e.y, machine_size(&e.name)));
        }
    }
    tiles
}

// ── Helper: fluid-only recipes ───────────────────────────────────────────────

fn fluid_only_recipes(solver_result: Option<&SolverResult>) -> FxHashSet<String> {
    let mut out = FxHashSet::default();
    if let Some(sr) = solver_result {
        for spec in &sr.machines {
            let has_solid = spec
                .inputs
                .iter()
                .chain(spec.outputs.iter())
                .any(|f| !f.is_fluid);
            if !has_solid {
                out.insert(spec.recipe.clone());
            }
        }
    }
    out
}

// ── check_inserter_chains ─────────────────────────────────────────────────────

/// Check that every machine with solid I/O has at least one adjacent inserter.
///
/// Machines whose recipe only has fluid inputs/outputs are skipped.
pub fn check_inserter_chains(
    layout: &LayoutResult,
    solver_result: Option<&SolverResult>,
) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    let fluid_only = fluid_only_recipes(solver_result);

    // Separate inserter position sets by reach
    let mut short_inserter_positions: FxHashSet<(i32, i32)> = FxHashSet::default();
    let mut long_inserter_positions: FxHashSet<(i32, i32)> = FxHashSet::default();
    for e in &layout.entities {
        if is_inserter(&e.name) {
            if e.name == "long-handed-inserter" {
                long_inserter_positions.insert((e.x, e.y));
            } else {
                short_inserter_positions.insert((e.x, e.y));
            }
        }
    }

    // Check each machine exactly once
    let mut checked_machines: FxHashSet<(i32, i32)> = FxHashSet::default();
    for e in &layout.entities {
        if !is_machine_entity(&e.name) {
            continue;
        }
        if !checked_machines.insert((e.x, e.y)) {
            continue;
        }

        // Skip fluid-only machines
        if let Some(recipe) = &e.recipe {
            if fluid_only.contains(recipe.as_str()) {
                continue;
            }
        }

        let size = machine_size(&e.name) as i32;
        let mut has_inserter = false;

        // Short inserters: 1 tile from border → dx/dy in [-1, size]
        'outer_short: for dx in -1..=(size) {
            for dy in -1..=(size) {
                if short_inserter_positions.contains(&(e.x + dx, e.y + dy)) {
                    has_inserter = true;
                    break 'outer_short;
                }
            }
        }

        // Long-handed inserters: 2 tiles from border → dx/dy in [-2, size+1]
        if !has_inserter {
            'outer_long: for dx in -2..=(size + 1) {
                for dy in -2..=(size + 1) {
                    if long_inserter_positions.contains(&(e.x + dx, e.y + dy)) {
                        has_inserter = true;
                        break 'outer_long;
                    }
                }
            }
        }

        if !has_inserter {
            issues.push(ValidationIssue::with_pos(
                Severity::Error,
                "inserter",
                format!("{} at ({},{}): no inserter adjacent", e.name, e.x, e.y),
                e.x,
                e.y,
            ));
        }
    }

    issues
}

// ── check_inserter_direction ──────────────────────────────────────────────────

/// Check that each inserter has its drop or pickup side pointing at a machine.
///
/// An inserter facing parallel to the nearest machine border won't transfer
/// items; that is reported as an error.
pub fn check_inserter_direction(layout: &LayoutResult) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    let machine_tiles_set = build_machine_tile_set(layout);

    for e in &layout.entities {
        if !is_inserter(&e.name) {
            continue;
        }

        let (dx, dy) = dir_to_vec(e.direction);
        let (odx, ody) = (-dx, -dy);

        let reach = inserter_reach(&e.name);

        let drop_pos = (e.x + dx * reach, e.y + dy * reach);
        let pickup_pos = (e.x + odx * reach, e.y + ody * reach);

        let drop_touches = machine_tiles_set.contains(&drop_pos);
        let pickup_touches = machine_tiles_set.contains(&pickup_pos);

        if !drop_touches && !pickup_touches {
            issues.push(ValidationIssue::with_pos(
                Severity::Error,
                "inserter-direction",
                format!(
                    "inserter at ({},{}) facing {:?}: neither drop nor pickup side touches a machine",
                    e.x, e.y, e.direction
                ),
                e.x,
                e.y,
            ));
        }
    }

    issues
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{EntityDirection, ItemFlow, MachineSpec, PlacedEntity, SolverResult};

    fn solid_flow(item: &str) -> ItemFlow {
        ItemFlow { item: item.to_string(), rate: 1.0, is_fluid: false }
    }

    fn fluid_flow(item: &str) -> ItemFlow {
        ItemFlow { item: item.to_string(), rate: 1.0, is_fluid: true }
    }

    // ── check_inserter_direction ─────────────────────────────────────────────

    #[test]
    fn inserter_facing_machine_ok() {
        // 3x3 machine at (0,0); inserter at (1,-1) facing SOUTH → drops into (1,0)
        let lr = LayoutResult {
            entities: vec![
                PlacedEntity {
                    name: "assembling-machine-1".into(),
                    x: 0,
                    y: 0,
                    recipe: Some("iron-gear-wheel".into()),
                    direction: EntityDirection::North,
                    ..Default::default()
                },
                PlacedEntity {
                    name: "inserter".into(),
                    x: 1,
                    y: -1,
                    direction: EntityDirection::South,
                    ..Default::default()
                },
            ],
            width: 10,
            height: 10,
            ..Default::default()
        };
        let issues = check_inserter_direction(&lr);
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        assert_eq!(errors.len(), 0);
    }

    #[test]
    fn inserter_facing_away_from_machine_ok() {
        // Inserter at (1,-1) facing NORTH → picks from machine at (1,0)
        let lr = LayoutResult {
            entities: vec![
                PlacedEntity {
                    name: "assembling-machine-1".into(),
                    x: 0,
                    y: 0,
                    recipe: Some("iron-gear-wheel".into()),
                    direction: EntityDirection::North,
                    ..Default::default()
                },
                PlacedEntity {
                    name: "inserter".into(),
                    x: 1,
                    y: -1,
                    direction: EntityDirection::North,
                    ..Default::default()
                },
            ],
            width: 10,
            height: 10,
            ..Default::default()
        };
        let issues = check_inserter_direction(&lr);
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        assert_eq!(errors.len(), 0);
    }

    #[test]
    fn inserter_facing_parallel_error() {
        // Inserter at (1,-1) facing EAST → parallel to top border, neither side hits machine
        let lr = LayoutResult {
            entities: vec![
                PlacedEntity {
                    name: "assembling-machine-1".into(),
                    x: 0,
                    y: 0,
                    recipe: Some("iron-gear-wheel".into()),
                    direction: EntityDirection::North,
                    ..Default::default()
                },
                PlacedEntity {
                    name: "inserter".into(),
                    x: 1,
                    y: -1,
                    direction: EntityDirection::East,
                    ..Default::default()
                },
            ],
            width: 10,
            height: 10,
            ..Default::default()
        };
        let issues = check_inserter_direction(&lr);
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].category, "inserter-direction");
    }

    #[test]
    fn inserter_not_near_machine_error() {
        let lr = LayoutResult {
            entities: vec![
                PlacedEntity {
                    name: "assembling-machine-1".into(),
                    x: 0,
                    y: 0,
                    recipe: Some("iron-gear-wheel".into()),
                    direction: EntityDirection::North,
                    ..Default::default()
                },
                PlacedEntity {
                    name: "inserter".into(),
                    x: 10,
                    y: 10,
                    direction: EntityDirection::South,
                    ..Default::default()
                },
            ],
            width: 20,
            height: 20,
            ..Default::default()
        };
        let issues = check_inserter_direction(&lr);
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        assert_eq!(errors.len(), 1);
    }

    #[test]
    fn inserter_facing_electric_furnace_ok() {
        // 3x3 electric-furnace at (0,0); inserter at (1,-1) facing SOUTH
        let lr = LayoutResult {
            entities: vec![
                PlacedEntity {
                    name: "electric-furnace".into(),
                    x: 0,
                    y: 0,
                    recipe: Some("iron-plate".into()),
                    direction: EntityDirection::North,
                    ..Default::default()
                },
                PlacedEntity {
                    name: "inserter".into(),
                    x: 1,
                    y: -1,
                    direction: EntityDirection::South,
                    ..Default::default()
                },
            ],
            width: 10,
            height: 10,
            ..Default::default()
        };
        let issues = check_inserter_direction(&lr);
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        assert_eq!(errors.len(), 0);
    }

    #[test]
    fn long_handed_inserter_direction_ok() {
        // long-handed-inserter at (1,-2) facing SOUTH reaches (1,0) which is inside machine
        let lr = LayoutResult {
            entities: vec![
                PlacedEntity {
                    name: "assembling-machine-1".into(),
                    x: 0,
                    y: 0,
                    recipe: Some("iron-gear-wheel".into()),
                    direction: EntityDirection::North,
                    ..Default::default()
                },
                PlacedEntity {
                    name: "long-handed-inserter".into(),
                    x: 1,
                    y: -2,
                    direction: EntityDirection::South,
                    ..Default::default()
                },
            ],
            width: 10,
            height: 10,
            ..Default::default()
        };
        let issues = check_inserter_direction(&lr);
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        assert_eq!(errors.len(), 0, "long-handed should reach 2 tiles");
    }

    // ── check_inserter_chains ────────────────────────────────────────────────

    #[test]
    fn machine_without_inserter_error() {
        // Machine with no inserters nearby
        let lr = LayoutResult {
            entities: vec![PlacedEntity {
                name: "assembling-machine-1".into(),
                x: 0,
                y: 0,
                recipe: Some("iron-gear-wheel".into()),
                direction: EntityDirection::North,
                ..Default::default()
            }],
            width: 10,
            height: 10,
            ..Default::default()
        };
        let issues = check_inserter_chains(&lr, None);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].category, "inserter");
        assert_eq!(issues[0].severity, Severity::Error);
    }

    #[test]
    fn machine_with_adjacent_inserter_ok() {
        let lr = LayoutResult {
            entities: vec![
                PlacedEntity {
                    name: "assembling-machine-1".into(),
                    x: 0,
                    y: 0,
                    recipe: Some("iron-gear-wheel".into()),
                    direction: EntityDirection::North,
                    ..Default::default()
                },
                PlacedEntity {
                    name: "inserter".into(),
                    x: 1,
                    y: -1,
                    direction: EntityDirection::South,
                    ..Default::default()
                },
            ],
            width: 10,
            height: 10,
            ..Default::default()
        };
        let issues = check_inserter_chains(&lr, None);
        assert_eq!(issues.len(), 0);
    }

    #[test]
    fn fluid_only_machine_skipped() {
        // Machine whose recipe only has fluid I/O should not require an inserter
        let sr = SolverResult {
            machines: vec![MachineSpec {
                entity: "oil-refinery".into(),
                recipe: "basic-oil-processing".into(),
                count: 1.0,
                inputs: vec![fluid_flow("crude-oil")],
                outputs: vec![fluid_flow("petroleum-gas")],
            }],
            external_inputs: vec![],
            external_outputs: vec![],
            dependency_order: vec![],
        };
        let lr = LayoutResult {
            entities: vec![PlacedEntity {
                name: "oil-refinery".into(),
                x: 0,
                y: 0,
                recipe: Some("basic-oil-processing".into()),
                direction: EntityDirection::North,
                ..Default::default()
            }],
            width: 10,
            height: 10,
            ..Default::default()
        };
        let issues = check_inserter_chains(&lr, Some(&sr));
        assert_eq!(issues.len(), 0, "fluid-only machine should be skipped");
    }

    #[test]
    fn mixed_solid_fluid_machine_needs_inserter() {
        // Machine with one solid input → still needs an inserter
        let sr = SolverResult {
            machines: vec![MachineSpec {
                entity: "chemical-plant".into(),
                recipe: "plastic-bar".into(),
                count: 1.0,
                inputs: vec![solid_flow("coal"), fluid_flow("petroleum-gas")],
                outputs: vec![solid_flow("plastic-bar")],
            }],
            external_inputs: vec![],
            external_outputs: vec![],
            dependency_order: vec![],
        };
        let lr = LayoutResult {
            entities: vec![PlacedEntity {
                name: "chemical-plant".into(),
                x: 0,
                y: 0,
                recipe: Some("plastic-bar".into()),
                direction: EntityDirection::North,
                ..Default::default()
            }],
            width: 10,
            height: 10,
            ..Default::default()
        };
        let issues = check_inserter_chains(&lr, Some(&sr));
        assert_eq!(issues.len(), 1, "mixed recipe still needs an inserter");
    }

    #[test]
    fn long_handed_inserter_satisfies_chain_check() {
        // long-handed inserter 2 tiles from border counts
        let lr = LayoutResult {
            entities: vec![
                PlacedEntity {
                    name: "assembling-machine-1".into(),
                    x: 0,
                    y: 0,
                    recipe: Some("iron-gear-wheel".into()),
                    direction: EntityDirection::North,
                    ..Default::default()
                },
                // 2 tiles above the top border (y = -2)
                PlacedEntity {
                    name: "long-handed-inserter".into(),
                    x: 1,
                    y: -2,
                    direction: EntityDirection::South,
                    ..Default::default()
                },
            ],
            width: 10,
            height: 10,
            ..Default::default()
        };
        let issues = check_inserter_chains(&lr, None);
        assert_eq!(issues.len(), 0, "long-handed inserter at -2 should satisfy chain");
    }

    #[test]
    fn reversed_inserter_direction_produces_issue() {
        // Done-when criterion: reversed inserter direction → expected issue
        // Inserter at (1,-1) facing WEST → drop at (0,-1), pickup at (2,-1), neither in machine
        let lr = LayoutResult {
            entities: vec![
                PlacedEntity {
                    name: "assembling-machine-1".into(),
                    x: 0,
                    y: 0,
                    recipe: Some("iron-gear-wheel".into()),
                    direction: EntityDirection::North,
                    ..Default::default()
                },
                PlacedEntity {
                    name: "inserter".into(),
                    x: 1,
                    y: -1,
                    direction: EntityDirection::West,
                    ..Default::default()
                },
            ],
            width: 10,
            height: 10,
            ..Default::default()
        };
        let issues = check_inserter_direction(&lr);
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        assert!(!errors.is_empty(), "reversed inserter should produce at least one error");
        assert_eq!(errors[0].category, "inserter-direction");
        assert_eq!(errors[0].x, Some(1));
        assert_eq!(errors[0].y, Some(-1));
    }

    #[test]
    fn valid_layout_no_direction_issues() {
        // Done-when criterion: valid layout produces no issues
        // Machine at (0,0), inserter at (1,-1) facing South → drop at (1,0) which is inside machine
        let lr = LayoutResult {
            entities: vec![
                PlacedEntity {
                    name: "assembling-machine-1".into(),
                    x: 0,
                    y: 0,
                    recipe: Some("iron-gear-wheel".into()),
                    direction: EntityDirection::North,
                    ..Default::default()
                },
                PlacedEntity {
                    name: "inserter".into(),
                    x: 1,
                    y: -1,
                    direction: EntityDirection::South,
                    ..Default::default()
                },
            ],
            width: 10,
            height: 10,
            ..Default::default()
        };
        let dir_issues = check_inserter_direction(&lr);
        let chain_issues = check_inserter_chains(&lr, None);
        assert_eq!(dir_issues.len(), 0, "valid layout should produce no direction issues");
        assert_eq!(chain_issues.len(), 0, "valid layout should produce no chain issues");
    }

    #[test]
    fn no_solver_result_treats_all_machines_as_needing_inserters() {
        // Without solver_result, no recipes are skipped → machine needs inserter
        let lr = LayoutResult {
            entities: vec![PlacedEntity {
                name: "oil-refinery".into(),
                x: 0,
                y: 0,
                recipe: Some("basic-oil-processing".into()),
                direction: EntityDirection::North,
                ..Default::default()
            }],
            width: 10,
            height: 10,
            ..Default::default()
        };
        let issues = check_inserter_chains(&lr, None);
        // Without solver context, we can't know it's fluid-only → should flag
        assert_eq!(issues.len(), 1);
    }
}
