//! Junction-solver regression & improvement fixtures.
//!
//! Each fixture captures a real `CrossingZone` extracted from the
//! pipeline running a specific recipe URL. Tests in this file are
//! **`#[ignore]` by default** — they are *targets to beat*, not CI
//! gates. Run them explicitly:
//!
//!   cargo test --manifest-path crates/core/Cargo.toml \
//!       --test junction_fixtures -- --ignored --nocapture
//!
//! Each fixture test has two parts:
//!   1. A `BASELINE_*` constant recording the best cost the current
//!      solver can achieve. `u32::MAX` means the solver can't yet
//!      produce a valid layout (currently UNSAT — growing past the
//!      tile cap).
//!   2. An assertion `cost <= BASELINE` so any improvement ratchets
//!      the baseline down; a regression fails the test.
//!
//! **Workflow**: when you improve the solver, run `--ignored`, see
//! which baselines dropped, update the `BASELINE_*` constants to the
//! new floor, commit. Over time the baselines march toward the
//! eye-measured minimums recorded in the `TARGET_*` constants.
//!
//! Source URL (all three fixtures):
//!   /?item=electronic-circuit&rate=15&machine=assembling-machine-1\
//!    &in=iron-ore,copper-ore&belt=transport-belt
//!
//! To re-capture: run `fixture_source_ec_15s_am1_yellow_from_ore` in
//! `e2e.rs` with `FUCKTORIO_DUMP_SNAPSHOTS=1`, then extract the
//! `SatInvocation` events at the target seeds.

use fucktorio_core::bus::junction_cost::solution_cost;
use fucktorio_core::models::EntityDirection;
use fucktorio_core::sat::{solve_crossing_zone_with_cost_cap, CrossingZone, ZoneBoundary};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a south-facing input boundary + its matching output on the
/// same tile. This is the default shape for bus-trunk crossings in
/// this recipe (12 of them per zone, all carrying copper-cable).
fn bus_boundary_pair(x: i32, y: i32, item: &str) -> [ZoneBoundary; 2] {
    [
        ZoneBoundary {
            x,
            y,
            direction: EntityDirection::South,
            item: item.into(),
            is_input: true,
            interior: false,
        },
        ZoneBoundary {
            x,
            y,
            direction: EntityDirection::South,
            item: item.into(),
            is_input: false,
            interior: false,
        },
    ]
}

/// Run the solver against a fixture zone and return the layout cost,
/// or `u32::MAX` if the solver returned UNSAT.
fn solve_cost(zone: &CrossingZone, belt_tier: &str, max_ug_reach: u32) -> u32 {
    let (ents, _stats) =
        solve_crossing_zone_with_cost_cap(zone, max_ug_reach, belt_tier, None, None);
    match ents {
        Some(e) => solution_cost(&e),
        None => u32::MAX,
    }
}

// ---------------------------------------------------------------------------
// Fixture: seed_14_88 — 8-wide tapoff crossing
// ---------------------------------------------------------------------------
//
// Displayed in the web-app overlay as `Junction(14, 88)`. Internal
// crossing seed is at world tile (18, 87). The zone packs 12 bus-trunk
// copper-cable pairs across x=18..=25 plus an iron-plate tapoff that
// enters from the north at (18, 87) and exits east — so 9 items to
// keep isolated across a single row.

fn fixture_seed_14_88() -> CrossingZone {
    let mut boundaries: Vec<ZoneBoundary> = Vec::new();
    // Twelve copper-cable bus-trunks passing south through (19..=25, 87).
    for x in 19..=25 {
        boundaries.extend(bus_boundary_pair(x, 87, "copper-cable"));
    }
    // The seed tile (18, 87) carries both iron-plate (tapoff east) and
    // copper-cable (bus passthrough).
    boundaries.push(ZoneBoundary {
        x: 18, y: 87, direction: EntityDirection::South,
        item: "iron-plate".into(), is_input: true, interior: false,
    });
    boundaries.push(ZoneBoundary {
        x: 18, y: 87, direction: EntityDirection::East,
        item: "iron-plate".into(), is_input: false, interior: false,
    });
    boundaries.push(ZoneBoundary {
        x: 18, y: 87, direction: EntityDirection::South,
        item: "copper-cable".into(), is_input: true, interior: false,
    });
    boundaries.push(ZoneBoundary {
        x: 18, y: 87, direction: EntityDirection::South,
        item: "copper-cable".into(), is_input: false, interior: false,
    });
    CrossingZone {
        x: 18, y: 87, width: 8, height: 1,
        boundaries,
        forced_empty: vec![],
    }
}

const BASELINE_14_88: u32 = u32::MAX;
// const TARGET_14_88: u32 = ?; // no eye-measured target yet

#[test]
#[ignore]
fn fixture_seed_14_88_cost() {
    let zone = fixture_seed_14_88();
    let cost = solve_cost(&zone, "transport-belt", 4);
    eprintln!("seed_14_88: cost = {cost} (baseline {BASELINE_14_88})");
    #[allow(clippy::absurd_extreme_comparisons)]
    let ok = cost <= BASELINE_14_88;
    assert!(ok, "regression at seed_14_88: cost {cost} > BASELINE {BASELINE_14_88}");
}

// ---------------------------------------------------------------------------
// Fixture: seed_14_96 — 7-wide tapoff crossing
// ---------------------------------------------------------------------------
//
// Displayed as `Junction(14, 96)`. Internal crossing seed (18, 95).
// Similar shape to 14_88 but one trunk narrower.

fn fixture_seed_14_96() -> CrossingZone {
    let mut boundaries: Vec<ZoneBoundary> = Vec::new();
    for x in 19..=24 {
        boundaries.extend(bus_boundary_pair(x, 95, "copper-cable"));
    }
    boundaries.push(ZoneBoundary {
        x: 18, y: 95, direction: EntityDirection::South,
        item: "copper-cable".into(), is_input: true, interior: false,
    });
    boundaries.push(ZoneBoundary {
        x: 18, y: 95, direction: EntityDirection::South,
        item: "copper-cable".into(), is_input: false, interior: false,
    });
    boundaries.push(ZoneBoundary {
        x: 18, y: 95, direction: EntityDirection::South,
        item: "iron-plate".into(), is_input: true, interior: false,
    });
    boundaries.push(ZoneBoundary {
        x: 18, y: 95, direction: EntityDirection::East,
        item: "iron-plate".into(), is_input: false, interior: false,
    });
    CrossingZone {
        x: 18, y: 95, width: 7, height: 1,
        boundaries,
        forced_empty: vec![],
    }
}

const BASELINE_14_96: u32 = u32::MAX;

#[test]
#[ignore]
fn fixture_seed_14_96_cost() {
    let zone = fixture_seed_14_96();
    let cost = solve_cost(&zone, "transport-belt", 4);
    eprintln!("seed_14_96: cost = {cost} (baseline {BASELINE_14_96})");
    #[allow(clippy::absurd_extreme_comparisons)]
    let ok = cost <= BASELINE_14_96;
    assert!(ok, "regression at seed_14_96: cost {cost} > BASELINE {BASELINE_14_96}");
}

// ---------------------------------------------------------------------------
// Fixture: seed_14_104 — 6-wide tapoff crossing
// ---------------------------------------------------------------------------
//
// Displayed as `Junction(14, 104)`. Internal crossing seed (18, 103).
// The smallest of the three: 5 copper-cable trunks + iron-plate tapoff.

fn fixture_seed_14_104() -> CrossingZone {
    let mut boundaries: Vec<ZoneBoundary> = Vec::new();
    for x in 19..=23 {
        boundaries.extend(bus_boundary_pair(x, 103, "copper-cable"));
    }
    boundaries.push(ZoneBoundary {
        x: 18, y: 103, direction: EntityDirection::South,
        item: "iron-plate".into(), is_input: true, interior: false,
    });
    boundaries.push(ZoneBoundary {
        x: 18, y: 103, direction: EntityDirection::East,
        item: "iron-plate".into(), is_input: false, interior: false,
    });
    boundaries.push(ZoneBoundary {
        x: 18, y: 103, direction: EntityDirection::South,
        item: "copper-cable".into(), is_input: true, interior: false,
    });
    boundaries.push(ZoneBoundary {
        x: 18, y: 103, direction: EntityDirection::South,
        item: "copper-cable".into(), is_input: false, interior: false,
    });
    CrossingZone {
        x: 18, y: 103, width: 6, height: 1,
        boundaries,
        forced_empty: vec![],
    }
}

const BASELINE_14_104: u32 = u32::MAX;

#[test]
#[ignore]
fn fixture_seed_14_104_cost() {
    let zone = fixture_seed_14_104();
    let cost = solve_cost(&zone, "transport-belt", 4);
    eprintln!("seed_14_104: cost = {cost} (baseline {BASELINE_14_104})");
    #[allow(clippy::absurd_extreme_comparisons)]
    let ok = cost <= BASELINE_14_104;
    assert!(ok, "regression at seed_14_104: cost {cost} > BASELINE {BASELINE_14_104}");
}

// ---------------------------------------------------------------------------
// Diagnostic: report all fixture costs in one run, formatted.
// ---------------------------------------------------------------------------

#[test]
#[ignore]
fn report_all_fixture_costs() {
    let cases = [
        ("seed_14_88", fixture_seed_14_88()),
        ("seed_14_96", fixture_seed_14_96()),
        ("seed_14_104", fixture_seed_14_104()),
    ];
    eprintln!("\n{:<14} {:>8} {:>12} {:>10}", "fixture", "w×h", "baseline", "cost");
    eprintln!("{}", "-".repeat(48));
    for (name, zone) in &cases {
        let cost = solve_cost(zone, "transport-belt", 4);
        let cost_s = if cost == u32::MAX { "UNSAT".to_string() } else { cost.to_string() };
        eprintln!(
            "{:<14} {:>4}×{:<3} {:>12} {:>10}",
            name,
            zone.width,
            zone.height,
            match *name {
                "seed_14_88" => BASELINE_14_88,
                "seed_14_96" => BASELINE_14_96,
                "seed_14_104" => BASELINE_14_104,
                _ => 0,
            },
            cost_s,
        );
    }
}
