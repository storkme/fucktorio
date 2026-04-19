//! SAT-zone fixture regression harness.
//!
//! Reads every `*.json` under `tests/sat_fixtures/`, constructs the
//! corresponding `CrossingZone`, calls the SAT solver, and asserts the
//! result matches `expected.mode`. All fixture failures are accumulated
//! and reported together at the end.
//!
//! Run with:
//!   cargo test --manifest-path crates/core/Cargo.toml --test sat_fixtures
//!
//! See `tests/sat_fixtures/README.md` for the fixture schema and
//! workflow for adding new fixtures.

use fucktorio_core::models::EntityDirection;
use fucktorio_core::sat::{solve_crossing_zone_with_stats, CrossingZone, ZoneBoundary};
use serde::Deserialize;
use std::path::Path;

// ---------------------------------------------------------------------------
// Fixture schema types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct Fixture {
    version: u32,
    name: String,
    #[allow(dead_code)]
    notes: Option<String>,
    #[allow(dead_code)]
    source_url: Option<String>,
    #[allow(dead_code)]
    seed: [i32; 2],
    bbox: FixtureBbox,
    #[serde(default)]
    forbidden: Vec<[i32; 2]>,
    belt_tier: String,
    max_reach: u32,
    boundaries: Vec<FixtureBoundary>,
    expected: FixtureExpected,
    /// Informational only in v1 — carried along but not consumed.
    #[allow(dead_code)]
    context: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct FixtureBbox {
    x: i32,
    y: i32,
    w: u32,
    h: u32,
}

#[derive(Debug, Deserialize)]
struct FixtureBoundary {
    x: i32,
    y: i32,
    dir: String,
    item: String,
    #[serde(rename = "in")]
    is_input: bool,
    #[serde(default)]
    interior: bool,
}

#[derive(Debug, Deserialize)]
struct FixtureExpected {
    mode: String,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_direction(dir: &str) -> EntityDirection {
    match dir {
        "North" => EntityDirection::North,
        "East" => EntityDirection::East,
        "South" => EntityDirection::South,
        "West" => EntityDirection::West,
        other => panic!("unknown direction in fixture: {other:?}"),
    }
}

fn build_zone(fixture: &Fixture) -> CrossingZone {
    let boundaries: Vec<ZoneBoundary> = fixture
        .boundaries
        .iter()
        .map(|b| ZoneBoundary {
            x: b.x,
            y: b.y,
            direction: parse_direction(&b.dir),
            item: b.item.clone(),
            is_input: b.is_input,
            interior: b.interior,
        })
        .collect();

    let forced_empty: Vec<(i32, i32)> = fixture
        .forbidden
        .iter()
        .map(|&[x, y]| (x, y))
        .collect();

    CrossingZone {
        x: fixture.bbox.x,
        y: fixture.bbox.y,
        width: fixture.bbox.w,
        height: fixture.bbox.h,
        boundaries,
        forced_empty,
    }
}

// ---------------------------------------------------------------------------
// Test
// ---------------------------------------------------------------------------

#[test]
fn sat_fixtures() {
    let fixtures_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("sat_fixtures");

    // Collect all *.json files.
    let mut fixture_paths: Vec<_> = std::fs::read_dir(&fixtures_dir)
        .unwrap_or_else(|e| panic!("cannot read sat_fixtures dir {}: {e}", fixtures_dir.display()))
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                Some(path)
            } else {
                None
            }
        })
        .collect();

    // Deterministic ordering: sort by filename so the test output is stable.
    fixture_paths.sort();

    if fixture_paths.is_empty() {
        // No fixtures yet — trivially pass.
        eprintln!("sat_fixtures: no *.json files found, nothing to test");
        return;
    }

    let mut failures: Vec<String> = Vec::new();
    let mut passed = 0u32;

    for path in &fixture_paths {
        let filename = path.file_name().unwrap_or_default().to_string_lossy();
        let raw = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));

        let fixture: Fixture = serde_json::from_str(&raw).unwrap_or_else(|e| {
            panic!("cannot parse {}: {e}", path.display())
        });

        if fixture.version != 1 {
            failures.push(format!(
                "{}: unsupported fixture version {} (only version 1 supported)",
                fixture.name, fixture.version
            ));
            continue;
        }

        let zone = build_zone(&fixture);
        let belt_name: &str = &fixture.belt_tier;

        let (result, _stats) =
            solve_crossing_zone_with_stats(&zone, fixture.max_reach, belt_name, None);

        match fixture.expected.mode.as_str() {
            "solve" => {
                if result.is_none() {
                    failures.push(format!(
                        "{} ({}): expected solve, got None (UNSAT)",
                        fixture.name, filename
                    ));
                } else {
                    eprintln!(
                        "  PASS  {} ({}) — solved with {} entities",
                        fixture.name,
                        filename,
                        result.unwrap().len()
                    );
                    passed += 1;
                }
            }
            "no_solve" => {
                if let Some(entities) = result {
                    failures.push(format!(
                        "{} ({}): expected no_solve, got solution with {} entities",
                        fixture.name,
                        filename,
                        entities.len()
                    ));
                } else {
                    eprintln!(
                        "  PASS  {} ({}) — correctly UNSAT",
                        fixture.name, filename
                    );
                    passed += 1;
                }
            }
            "snapshot" => {
                // Phase F: exact entity comparison. Not implemented in v1.
                failures.push(format!(
                    "{} ({}): expected.mode=\"snapshot\" is not yet supported \
                     in the v1 harness (Phase F)",
                    fixture.name, filename
                ));
            }
            other => {
                failures.push(format!(
                    "{} ({}): unknown expected.mode {:?} — must be \"solve\", \
                     \"no_solve\", or \"snapshot\"",
                    fixture.name, filename, other
                ));
            }
        }
    }

    eprintln!(
        "\nsat_fixtures: {passed} passed, {} failed (from {} fixture files)",
        failures.len(),
        fixture_paths.len()
    );

    assert!(
        failures.is_empty(),
        "{} fixture failure(s):\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
