//! Shared constants and utility functions for routing.
//!
//! Port of `src/routing/common.py`.

use crate::models::EntityDirection;

const DEFAULT_MACHINE_SIZE: u32 = 3;

/// Return the footprint size (in tiles) for the given entity name.
pub fn machine_size(entity: &str) -> u32 {
    match entity {
        "assembling-machine-1" | "assembling-machine-2" | "assembling-machine-3"
        | "chemical-plant" | "electric-furnace" => 3,
        "oil-refinery" => 5,
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
}
