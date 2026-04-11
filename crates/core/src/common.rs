//! Shared constants and utility functions for routing.
//!
//! Port of `src/routing/common.py`.

use rustc_hash::FxHashSet;

use crate::models::{EntityDirection, PlacedEntity, SolverResult};

const DEFAULT_MACHINE_SIZE: u32 = 3;

/// Known machine entity names (the set of all crafting machines the layout engine uses).
pub const MACHINE_ENTITY_NAMES: &[&str] = &[
    "assembling-machine-1",
    "assembling-machine-2",
    "assembling-machine-3",
    "chemical-plant",
    "electric-furnace",
    "oil-refinery",
    "electromagnetic-plant",
    "cryogenic-plant",
    "foundry",
    "biochamber",
];

/// Return `true` if `entity` is a known crafting machine.
pub fn is_machine_entity(entity: &str) -> bool {
    MACHINE_ENTITY_NAMES.contains(&entity)
}

/// Return the footprint size (in tiles) for the given entity name.
pub fn machine_size(entity: &str) -> u32 {
    match entity {
        "assembling-machine-1" | "assembling-machine-2" | "assembling-machine-3"
        | "chemical-plant" | "electric-furnace" | "biochamber" => 3,
        "oil-refinery" | "cryogenic-plant" | "foundry" => 5,
        "electromagnetic-plant" => 4,
        _ => DEFAULT_MACHINE_SIZE,
    }
}

/// All tile coordinates occupied by a machine at `(x, y)` with `size`.
pub fn machine_tiles(x: i32, y: i32, size: u32) -> Vec<(i32, i32)> {
    let s = size as i32;
    (0..s)
        .flat_map(move |dx| (0..s).map(move |dy| (x + dx, y + dy)))
        .collect()
}

/// Belt throughput tiers: (entity name, items-per-second capacity).
pub const BELT_TIERS: &[(&str, f64)] = &[
    ("transport-belt", 15.0),
    ("fast-transport-belt", 30.0),
    ("express-transport-belt", 45.0),
];

/// Underground belt max reach (tiles between entry and exit, exclusive).
pub fn ug_max_reach(belt: &str) -> u32 {
    match belt {
        "transport-belt" => 4,
        "fast-transport-belt" => 6,
        "express-transport-belt" => 8,
        _ => 4,
    }
}

/// Cost multiplier for underground belt tiles vs surface.
pub const UG_COST_MULTIPLIER: u32 = 5;

/// Pipe-to-ground max reach (tiles between entry and exit, exclusive).
pub const UG_PIPE_REACH: u32 = 10;

/// Full belt throughput (both lanes combined) for the given belt entity.
pub fn belt_throughput(belt: &str) -> f64 {
    BELT_TIERS
        .iter()
        .find(|(name, _)| *name == belt)
        .map(|(_, rate)| *rate)
        .unwrap_or(15.0)
}

/// Per-lane capacity (half of total belt throughput).
pub fn lane_capacity(belt: &str) -> f64 {
    belt_throughput(belt) / 2.0
}

// ---------------------------------------------------------------------------
// Entity classification helpers (shared across validation modules)
// ---------------------------------------------------------------------------

/// Surface (above-ground) belt entity names.
pub const SURFACE_BELT_ENTITIES: &[&str] =
    &["transport-belt", "fast-transport-belt", "express-transport-belt"];

/// Underground belt entity names.
pub const UG_BELT_ENTITIES: &[&str] = &[
    "underground-belt",
    "fast-underground-belt",
    "express-underground-belt",
];

/// Splitter entity names.
pub const SPLITTER_ENTITIES: &[&str] =
    &["splitter", "fast-splitter", "express-splitter"];

/// Inserter entity names.
pub const INSERTER_ENTITIES: &[&str] =
    &["inserter", "long-handed-inserter", "fast-inserter", "stack-inserter"];

/// Return `true` if `name` is a surface (above-ground) belt.
pub fn is_surface_belt(name: &str) -> bool {
    SURFACE_BELT_ENTITIES.contains(&name)
}

/// Return `true` if `name` is an underground belt.
pub fn is_ug_belt(name: &str) -> bool {
    UG_BELT_ENTITIES.contains(&name)
}

/// Return `true` if `name` is a splitter.
pub fn is_splitter(name: &str) -> bool {
    SPLITTER_ENTITIES.contains(&name)
}

/// Return `true` if `name` is any belt-type entity (surface, underground, or splitter).
pub fn is_belt_entity(name: &str) -> bool {
    is_surface_belt(name) || is_ug_belt(name) || is_splitter(name)
}

/// Return `true` if `name` is an inserter.
pub fn is_inserter(name: &str) -> bool {
    INSERTER_ENTITIES.contains(&name)
}

/// Inserter reach: how many tiles away the pick-up / drop position is.
pub fn inserter_reach(name: &str) -> i32 {
    if name == "long-handed-inserter" {
        2
    } else {
        1
    }
}

/// Map underground-belt entity name to its corresponding surface belt tier.
pub fn ug_to_surface_tier(ug_name: &str) -> &'static str {
    match ug_name {
        "underground-belt" => "transport-belt",
        "fast-underground-belt" => "fast-transport-belt",
        "express-underground-belt" => "express-transport-belt",
        _ => "transport-belt",
    }
}

/// Map splitter entity name to its corresponding surface belt tier.
pub fn splitter_to_surface_tier(splitter: &str) -> &'static str {
    match splitter {
        "splitter" => "transport-belt",
        "fast-splitter" => "fast-transport-belt",
        "express-splitter" => "express-transport-belt",
        _ => "transport-belt",
    }
}

// ---------------------------------------------------------------------------
// Tile helpers for entities that span multiple tiles
// ---------------------------------------------------------------------------

/// Second tile occupied by a splitter (perpendicular to flow direction).
pub fn splitter_second_tile(e: &PlacedEntity) -> (i32, i32) {
    match e.direction {
        EntityDirection::North | EntityDirection::South => (e.x + 1, e.y),
        _ => (e.x, e.y + 1),
    }
}

/// Collect the set of recipes whose inputs and outputs are all fluids (no solid items).
pub fn fluid_only_recipes(solver: Option<&SolverResult>) -> FxHashSet<String> {
    let mut out = rustc_hash::FxHashSet::default();
    if let Some(sr) = solver {
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

/// Pick the cheapest belt tier whose throughput is `>= rate`.
///
/// If `max_tier` is `Some(name)`, never select a higher tier than that.
pub fn belt_entity_for_rate(rate: f64, max_tier: Option<&str>) -> &'static str {
    let max_idx = if let Some(max) = max_tier {
        BELT_TIERS
            .iter()
            .position(|(name, _)| *name == max)
            .unwrap_or(BELT_TIERS.len() - 1)
    } else {
        BELT_TIERS.len() - 1
    };

    for (i, &(name, throughput)) in BELT_TIERS.iter().enumerate() {
        if i > max_idx {
            break;
        }
        if rate <= throughput {
            return name;
        }
    }
    BELT_TIERS[max_idx].0
}

/// Cardinal direction vectors `(dx, dy)` in order N, E, S, W.
pub const DIRECTIONS: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];

/// Convert a `(dx, dy)` unit vector to an `EntityDirection`, or `None` for non-cardinal inputs.
pub fn dir_from_vec(dx: i32, dy: i32) -> Option<EntityDirection> {
    match (dx, dy) {
        (0, -1) => Some(EntityDirection::North),
        (1, 0) => Some(EntityDirection::East),
        (0, 1) => Some(EntityDirection::South),
        (-1, 0) => Some(EntityDirection::West),
        _ => None,
    }
}

/// Convert an `EntityDirection` to its `(dx, dy)` vector.
pub fn dir_to_vec(dir: EntityDirection) -> (i32, i32) {
    match dir {
        EntityDirection::North => (0, -1),
        EntityDirection::East => (1, 0),
        EntityDirection::South => (0, 1),
        EntityDirection::West => (-1, 0),
    }
}

/// Belt lane: left relative to belt travel direction.
pub const LANE_LEFT: &str = "left";

/// Belt lane: right relative to belt travel direction.
pub const LANE_RIGHT: &str = "right";

/// Return which lane an inserter places items on (the far lane).
///
/// The inserter sits on one side of the belt (left or right relative to belt
/// direction); items land on the opposite (far) lane.
pub fn inserter_target_lane(
    ins_x: i32,
    ins_y: i32,
    belt_x: i32,
    belt_y: i32,
    belt_dir: EntityDirection,
) -> &'static str {
    let (dx, dy) = dir_to_vec(belt_dir);
    // Left perpendicular (CCW 90° of belt direction vector)
    let (left_dx, left_dy) = (-dy, dx);
    let dot = (ins_x - belt_x) * left_dx + (ins_y - belt_y) * left_dy;
    // Inserter on left side → items land on right (far) lane, and vice versa.
    // dot == 0 means directly in-line; default to left.
    if dot > 0 { LANE_RIGHT } else { LANE_LEFT }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_size_assembling_3() {
        assert_eq!(machine_size("assembling-machine-3"), 3);
    }

    #[test]
    fn machine_size_oil_refinery() {
        assert_eq!(machine_size("oil-refinery"), 5);
    }

    #[test]
    fn machine_size_default() {
        assert_eq!(machine_size("unknown-machine"), DEFAULT_MACHINE_SIZE);
    }

    #[test]
    fn machine_size_space_age() {
        assert_eq!(machine_size("electromagnetic-plant"), 4);
        assert_eq!(machine_size("cryogenic-plant"), 5);
        assert_eq!(machine_size("foundry"), 5);
        assert_eq!(machine_size("biochamber"), 3);
    }

    #[test]
    fn space_age_machines_in_entity_list() {
        for name in ["electromagnetic-plant", "cryogenic-plant", "foundry", "biochamber"] {
            assert!(MACHINE_ENTITY_NAMES.contains(&name), "{name} missing from MACHINE_ENTITY_NAMES");
            assert!(is_machine_entity(name), "{name} not recognized by is_machine_entity");
        }
    }

    #[test]
    fn machine_tiles_3x3() {
        let tiles = machine_tiles(0, 0, 3);
        assert_eq!(tiles.len(), 9);
        assert!(tiles.contains(&(0, 0)));
        assert!(tiles.contains(&(2, 2)));
    }

    #[test]
    fn belt_entity_for_rate_low() {
        assert_eq!(belt_entity_for_rate(10.0, None), "transport-belt");
    }

    #[test]
    fn belt_entity_for_rate_exact_15() {
        assert_eq!(belt_entity_for_rate(15.0, None), "transport-belt");
    }

    #[test]
    fn belt_entity_for_rate_mid() {
        assert_eq!(belt_entity_for_rate(20.0, None), "fast-transport-belt");
    }

    #[test]
    fn belt_entity_for_rate_high() {
        assert_eq!(belt_entity_for_rate(40.0, None), "express-transport-belt");
    }

    #[test]
    fn belt_entity_for_rate_capped_by_max_tier() {
        assert_eq!(
            belt_entity_for_rate(40.0, Some("transport-belt")),
            "transport-belt"
        );
    }

    #[test]
    fn ug_max_reach_values() {
        assert_eq!(ug_max_reach("transport-belt"), 4);
        assert_eq!(ug_max_reach("fast-transport-belt"), 6);
        assert_eq!(ug_max_reach("express-transport-belt"), 8);
    }

    #[test]
    fn lane_capacity_values() {
        assert_eq!(lane_capacity("transport-belt"), 7.5);
        assert_eq!(lane_capacity("fast-transport-belt"), 15.0);
        assert_eq!(lane_capacity("express-transport-belt"), 22.5);
    }

    #[test]
    fn dir_roundtrip() {
        for dir in [
            EntityDirection::North,
            EntityDirection::East,
            EntityDirection::South,
            EntityDirection::West,
        ] {
            let (dx, dy) = dir_to_vec(dir);
            assert_eq!(dir_from_vec(dx, dy), Some(dir));
        }
    }

    #[test]
    fn inserter_target_lane_north_belt_inserter_left() {
        // North belt, inserter to the east (left side) → far lane is right.
        assert_eq!(inserter_target_lane(1, 0, 0, 0, EntityDirection::North), LANE_RIGHT);
    }

    #[test]
    fn inserter_target_lane_north_belt_inserter_right() {
        // North belt, inserter to the west (right side) → far lane is left.
        assert_eq!(inserter_target_lane(-1, 0, 0, 0, EntityDirection::North), LANE_LEFT);
    }

    #[test]
    fn inserter_target_lane_east_belt() {
        // East belt, inserter to the south (left side) → far lane is right.
        assert_eq!(inserter_target_lane(0, 1, 0, 0, EntityDirection::East), LANE_RIGHT);
    }

    #[test]
    fn inserter_target_lane_default_inline() {
        // Inserter directly in front of belt → defaults to left lane.
        assert_eq!(inserter_target_lane(0, -1, 0, 0, EntityDirection::North), LANE_LEFT);
    }

    #[test]
    fn inserter_target_lane_south_belt() {
        // South belt, inserter to the west (left side of southward travel) → far lane is right.
        assert_eq!(inserter_target_lane(-1, 0, 0, 0, EntityDirection::South), LANE_RIGHT);
        // South belt, inserter to the east (right side) → far lane is left.
        assert_eq!(inserter_target_lane(1, 0, 0, 0, EntityDirection::South), LANE_LEFT);
    }

    #[test]
    fn inserter_target_lane_west_belt() {
        // West belt, inserter to the north (left side of westward travel) → far lane is right.
        assert_eq!(inserter_target_lane(0, -1, 0, 0, EntityDirection::West), LANE_RIGHT);
        // West belt, inserter to the south (right side) → far lane is left.
        assert_eq!(inserter_target_lane(0, 1, 0, 0, EntityDirection::West), LANE_LEFT);
    }

    #[test]
    fn machine_size_chemical_plant_and_furnace() {
        assert_eq!(machine_size("chemical-plant"), 3);
        assert_eq!(machine_size("electric-furnace"), 3);
    }

    #[test]
    fn machine_tiles_offset_origin() {
        // Tiles should be offset by (x, y), not always start at (0, 0).
        let tiles = machine_tiles(5, 3, 3);
        assert_eq!(tiles.len(), 9);
        assert!(tiles.contains(&(5, 3)));
        assert!(tiles.contains(&(7, 5)));
        assert!(!tiles.contains(&(0, 0)));
    }

    #[test]
    fn belt_throughput_values() {
        assert_eq!(belt_throughput("transport-belt"), 15.0);
        assert_eq!(belt_throughput("fast-transport-belt"), 30.0);
        assert_eq!(belt_throughput("express-transport-belt"), 45.0);
        // Unknown belt falls back to 15.0
        assert_eq!(belt_throughput("unknown-belt"), 15.0);
    }

    #[test]
    fn dir_from_vec_diagonal_returns_none() {
        assert_eq!(dir_from_vec(1, 1), None);
        assert_eq!(dir_from_vec(0, 0), None);
    }
}

// ---- Module and beacon data ----

/// Speed and productivity bonuses for a module type.
#[derive(Debug, Clone, Copy)]
pub struct ModuleEffect {
    pub speed: f64,
    pub productivity: f64,
}

/// Look up module effects by entity name. Returns (0,0) for unknown modules.
pub fn module_effect(name: &str) -> ModuleEffect {
    match name {
        "speed-module" => ModuleEffect { speed: 0.2, productivity: 0.0 },
        "speed-module-2" => ModuleEffect { speed: 0.3, productivity: 0.0 },
        "speed-module-3" => ModuleEffect { speed: 0.5, productivity: 0.0 },
        "productivity-module" => ModuleEffect { speed: -0.05, productivity: 0.04 },
        "productivity-module-2" => ModuleEffect { speed: -0.10, productivity: 0.06 },
        "productivity-module-3" => ModuleEffect { speed: -0.15, productivity: 0.10 },
        "quality-module" => ModuleEffect { speed: -0.05, productivity: 0.0 },
        "quality-module-2" => ModuleEffect { speed: -0.05, productivity: 0.0 },
        "quality-module-3" => ModuleEffect { speed: -0.05, productivity: 0.0 },
        // Efficiency modules have no speed/productivity effect
        _ => ModuleEffect { speed: 0.0, productivity: 0.0 },
    }
}

/// Number of module slots for a machine entity.
pub fn module_slots(entity: &str) -> u32 {
    match entity {
        "assembling-machine-1" | "stone-furnace" | "steel-furnace" => 0,
        "assembling-machine-2" | "electric-furnace" | "centrifuge" | "crusher" | "lab" => 2,
        "chemical-plant" | "oil-refinery" => 3,
        "assembling-machine-3" | "rocket-silo" | "foundry" | "biochamber" | "biolab"
        | "recycler" => 4,
        "electromagnetic-plant" => 5,
        "cryogenic-plant" => 8,
        "beacon" => 2,
        _ => 0,
    }
}

/// Beacon supply area distance (tiles from edge of 3×3 beacon).
pub const BEACON_SUPPLY_DISTANCE: i32 = 3;

/// Beacon distribution effectivity in Factorio 2.0.
/// In 2.0 this is distance-based, but for tile-distance=0 it's 1.5.
/// For a simple model we use 0.5 (the Factorio 1.x value and a reasonable
/// average across distances in 2.0 with the profile falloff).
pub const BEACON_DISTRIBUTION_EFFECTIVITY: f64 = 0.5;
