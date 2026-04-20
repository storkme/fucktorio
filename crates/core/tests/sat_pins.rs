//! Coverage for `solve_crossing_zone_with_pins`.
//!
//! Run with:
//!   cargo test --manifest-path crates/core/Cargo.toml --test sat_pins

use fucktorio_core::fixture::{build_zone, Fixture};
use fucktorio_core::models::{EntityDirection, PlacedEntity};
use fucktorio_core::sat::{solve_crossing_zone_with_pins, CrossingZone, ZoneBoundary};

fn load_sample_fixture() -> Fixture {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("sat_fixtures")
        .join("sample_electronic_circuit.json");
    let raw = std::fs::read_to_string(&path).expect("sample fixture present");
    serde_json::from_str(&raw).expect("sample fixture parses")
}

/// Pinning a belt that's part of the unique satisfying solution must
/// still produce a SAT result that includes that pin.
#[test]
fn pin_on_known_solution_belt() {
    let fixture = load_sample_fixture();
    let zone = build_zone(&fixture);

    // The sample is a 2×1 east-bound copper-cable lane at (7,68)→(8,68).
    // Pinning the in-belt at (7,68) must be consistent with SAT.
    let pin = PlacedEntity {
        name: "transport-belt".into(),
        x: 7,
        y: 68,
        direction: EntityDirection::East,
        carries: Some("copper-cable".into()),
        ..Default::default()
    };

    let (result, _stats) =
        solve_crossing_zone_with_pins(&zone, std::slice::from_ref(&pin), fixture.max_reach, &fixture.belt_tier, None);

    let entities = result.expect("SAT must complete the pinned belt into a valid layout");
    assert!(
        entities.iter().any(|e| e.x == pin.x && e.y == pin.y && e.name == "transport-belt"),
        "expected solution to include the pinned belt at (7,68): {entities:?}"
    );
}

/// Pinning an entity outside the zone bounds must early-return None
/// without invoking the solver.
#[test]
fn pin_outside_bbox_rejects() {
    let fixture = load_sample_fixture();
    let zone = build_zone(&fixture);

    let pin = PlacedEntity {
        name: "transport-belt".into(),
        x: 100,
        y: 100,
        direction: EntityDirection::East,
        carries: Some("copper-cable".into()),
        ..Default::default()
    };

    let (result, _stats) =
        solve_crossing_zone_with_pins(&zone, &[pin], fixture.max_reach, &fixture.belt_tier, None);
    assert!(result.is_none(), "out-of-bbox pin must reject");
}

/// Pinning a belt on a `forced_empty` tile must early-return None.
/// Build a synthetic zone (the sample fixture has no forbidden tiles)
/// with one forbidden tile in the middle and try to pin a belt there.
#[test]
fn pin_on_forbidden_tile_rejects() {
    let zone = CrossingZone {
        x: 0,
        y: 0,
        width: 3,
        height: 1,
        boundaries: vec![
            ZoneBoundary {
                x: 0,
                y: 0,
                direction: EntityDirection::East,
                item: "iron-plate".into(),
                is_input: true,
                interior: false,
            },
            ZoneBoundary {
                x: 2,
                y: 0,
                direction: EntityDirection::East,
                item: "iron-plate".into(),
                is_input: false,
                interior: false,
            },
        ],
        forced_empty: vec![(1, 0)],
    };

    let pin = PlacedEntity {
        name: "transport-belt".into(),
        x: 1,
        y: 0,
        direction: EntityDirection::East,
        carries: Some("iron-plate".into()),
        ..Default::default()
    };

    let (result, _stats) =
        solve_crossing_zone_with_pins(&zone, &[pin], 4, "transport-belt", None);
    assert!(result.is_none(), "pin on forbidden tile must reject");
}

/// Empty pin set must behave identically to `solve_crossing_zone_with_stats`
/// — i.e. the sample fixture solves to the known 2-belt layout.
#[test]
fn empty_pins_solves_like_baseline() {
    let fixture = load_sample_fixture();
    let zone = build_zone(&fixture);

    let (result, _stats) =
        solve_crossing_zone_with_pins(&zone, &[], fixture.max_reach, &fixture.belt_tier, None);
    let entities = result.expect("baseline solve");
    assert_eq!(entities.len(), 2, "sample fixture has 2 belts");
}
