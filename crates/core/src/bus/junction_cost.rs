//! Cost model for junction solutions.
//!
//! A single place that every pick-cheapest site can call. Used by the
//! variant-selection loop in `solve_crossing` and by the SAT cost-descent
//! loop in `junction_sat_strategy`. Weights are coarse: belt tiles are
//! free, underground-belt endpoints expensive. Turn cost is intentionally
//! NOT separate — a zig-zag detour already costs more belts than the
//! straight route, so belt count alone captures it.
//!
//! Keep these constants in sync with `encode_cost_cap` in `sat.rs`: the
//! SAT encoder computes the same sum over `is_belt`, `is_ug_in`,
//! `is_ug_out` vars, so a mismatch would let descent claim a cap the
//! Rust-side cost function disagrees with.

use crate::models::PlacedEntity;

pub const COST_BELT: u32 = 1;
pub const COST_UG_IN: u32 = 5;
pub const COST_UG_OUT: u32 = 5;

fn is_transport_belt(name: &str) -> bool {
    matches!(
        name,
        "transport-belt" | "fast-transport-belt" | "express-transport-belt"
    )
}

fn is_underground_belt(name: &str) -> bool {
    matches!(
        name,
        "underground-belt" | "fast-underground-belt" | "express-underground-belt"
    )
}

/// Sum of belt/UG costs over the entity list. Entities that aren't
/// belts or UGs contribute 0 (assemblers, poles, etc. aren't stamped
/// by the junction solver so this is mostly moot).
pub fn solution_cost(entities: &[PlacedEntity]) -> u32 {
    let mut cost = 0u32;
    for e in entities {
        if is_transport_belt(&e.name) {
            cost += COST_BELT;
        } else if is_underground_belt(&e.name) {
            match e.io_type.as_deref() {
                Some("input") => cost += COST_UG_IN,
                Some("output") => cost += COST_UG_OUT,
                _ => {}
            }
        }
    }
    cost
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{EntityDirection, PlacedEntity};

    fn belt(x: i32, y: i32, dir: EntityDirection) -> PlacedEntity {
        PlacedEntity {
            name: "transport-belt".to_string(),
            x,
            y,
            direction: dir,
            ..Default::default()
        }
    }

    fn ug(x: i32, y: i32, dir: EntityDirection, io: &str) -> PlacedEntity {
        PlacedEntity {
            name: "underground-belt".to_string(),
            x,
            y,
            direction: dir,
            io_type: Some(io.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn belt_run_costs_one_per_tile() {
        let ents = vec![
            belt(0, 0, EntityDirection::East),
            belt(1, 0, EntityDirection::East),
            belt(2, 0, EntityDirection::East),
        ];
        assert_eq!(solution_cost(&ents), 3);
    }

    #[test]
    fn ug_pair_costs_ten() {
        let ents = vec![
            ug(0, 0, EntityDirection::East, "input"),
            ug(3, 0, EntityDirection::East, "output"),
        ];
        assert_eq!(solution_cost(&ents), COST_UG_IN + COST_UG_OUT);
    }

    #[test]
    fn straight_tapoff_cheaper_than_wiggle() {
        // Straight: 4 belts going east at y=0.  Cost = 4.
        let straight = vec![
            belt(0, 0, EntityDirection::East),
            belt(1, 0, EntityDirection::East),
            belt(2, 0, EntityDirection::East),
            belt(3, 0, EntityDirection::East),
        ];
        // Wiggle: detour up one then back down. Same east displacement
        // but 2 extra belts. Cost = 6.
        let wiggle = vec![
            belt(0, 0, EntityDirection::East),
            belt(1, 0, EntityDirection::North),
            belt(1, -1, EntityDirection::East),
            belt(2, -1, EntityDirection::East),
            belt(2, 0, EntityDirection::South),
            belt(3, 0, EntityDirection::East),
        ];
        assert!(solution_cost(&straight) < solution_cost(&wiggle));
    }

    #[test]
    fn four_belts_cheaper_than_ug_pair() {
        // 4-tile belt run: 4. UG pair: 10. Belts win.
        let four_belts = vec![
            belt(0, 0, EntityDirection::East),
            belt(1, 0, EntityDirection::East),
            belt(2, 0, EntityDirection::East),
            belt(3, 0, EntityDirection::East),
        ];
        let ug_pair = vec![
            ug(0, 0, EntityDirection::East, "input"),
            ug(3, 0, EntityDirection::East, "output"),
        ];
        assert!(solution_cost(&four_belts) < solution_cost(&ug_pair));
    }
}
