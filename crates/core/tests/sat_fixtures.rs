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

use fucktorio_core::bus::junction_cost::solution_cost;
use fucktorio_core::fixture::{build_zone, Fixture};
use fucktorio_core::sat::solve_crossing_zone_with_stats;
use std::path::Path;

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
            "solve" => match result {
                None => {
                    failures.push(format!(
                        "{} ({}): expected solve, got None (UNSAT)",
                        fixture.name, filename
                    ));
                }
                Some(entities) => {
                    let actual_cost = solution_cost(&entities);
                    if let Some(max_cost) = fixture.expected.max_cost {
                        if actual_cost > max_cost {
                            failures.push(format!(
                                "{} ({}): solver cost {actual_cost} exceeds fixture max_cost {max_cost}",
                                fixture.name, filename
                            ));
                            continue;
                        }
                    }
                    eprintln!(
                        "  PASS  {} ({}) — solved with {} entities, cost {}",
                        fixture.name,
                        filename,
                        entities.len(),
                        actual_cost
                    );
                    passed += 1;
                }
            },
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
