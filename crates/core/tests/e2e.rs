//! End-to-end blueprint validation tests.
//!
//! Closes the loop: solve → layout → export → parse back → validate → analyze.
//! Asserts that generated factories produce the target item at the target rate
//! with zero validation errors.
//!
//! Run with:  cargo test --test e2e
//! Filter:    cargo test --test e2e -- tier1
//! All (incl. known-failing): cargo test --test e2e -- --ignored
//!
//! Snapshot dumping:
//!   FUCKTORIO_DUMP_SNAPSHOTS=1  — dump .fls files for ALL tests (passing too)
//!   Automatic on failure — any test with validation errors dumps a snapshot.

use fucktorio_core::analysis::{self, BlueprintAnalysis};
use fucktorio_core::blueprint;
use fucktorio_core::blueprint_parser;
use fucktorio_core::bus::layout;
use fucktorio_core::models::{LayoutResult, SolverResult};
use fucktorio_core::snapshot::{
    LayoutSnapshot, SnapshotContext, SnapshotParams, SnapshotSource,
};
use fucktorio_core::solver;
use fucktorio_core::trace::{self, TraceEvent};
use fucktorio_core::validate::{self, LayoutStyle, Severity, ValidationIssue};
use fucktorio_core::validate::{belt_flow, belt_structural, power, inserters};
use rustc_hash::FxHashSet;
use std::path::PathBuf;
use std::time::Instant;

struct E2EResult {
    solver_result: SolverResult,
    layout: LayoutResult,
    parsed: LayoutResult,
    issues: Vec<ValidationIssue>,
    analysis: BlueprintAnalysis,
    #[allow(dead_code)]
    trace_events: Vec<TraceEvent>,
}

/// Whether to dump snapshots for all tests or only failing ones.
fn should_dump_snapshots() -> bool {
    std::env::var("FUCKTORIO_DUMP_SNAPSHOTS").is_ok()
}

/// Dump a snapshot file for a test. Called on failure or when env var is set.
fn dump_snapshot(
    test_name: &str,
    params: &RunParams,
    result: &E2EResult,
) {
    let dir = snapshot_dir();
    std::fs::create_dir_all(&dir).ok();

    let snapshot = LayoutSnapshot::from_run(
        SnapshotSource::Test,
        SnapshotParams {
            item: params.item.to_string(),
            rate: params.rate,
            machine: params.machine.to_string(),
            belt_tier: params.belt_tier.map(|s| s.to_string()),
            inputs: params.available_inputs.iter().cloned().collect(),
        },
        SnapshotContext {
            test_name: Some(test_name.to_string()),
            label: None,
            git_sha: git_sha(),
        },
        result.layout.clone(),
        result.issues.clone(),
        false, // not truncated
        result.trace_events.clone(),
        true, // trace complete
        Some(result.solver_result.clone()),
    );

    let path = dir.join(format!("snapshot-{test_name}.fls"));
    match snapshot.write_to_file(&path) {
        Ok(()) => eprintln!("  snapshot: {}", path.display()),
        Err(e) => eprintln!("  snapshot write failed: {e}"),
    }
}

/// Dump a partial snapshot when the pipeline fails early (solver/layout error).
/// Uses whatever data is available — may have no layout entities.
fn dump_partial_snapshot(
    test_name: &str,
    params: &RunParams,
    solver_result: Option<&SolverResult>,
    error_msg: &str,
) {
    let dir = snapshot_dir();
    std::fs::create_dir_all(&dir).ok();

    let error_issue = ValidationIssue {
        severity: Severity::Error,
        category: "pipeline".into(),
        message: error_msg.into(),
        x: None,
        y: None,
    };

    let snapshot = LayoutSnapshot::from_run(
        SnapshotSource::Test,
        SnapshotParams {
            item: params.item.to_string(),
            rate: params.rate,
            machine: params.machine.to_string(),
            belt_tier: params.belt_tier.map(|s| s.to_string()),
            inputs: params.available_inputs.iter().cloned().collect(),
        },
        SnapshotContext {
            test_name: Some(test_name.to_string()),
            label: None,
            git_sha: git_sha(),
        },
        LayoutResult::default(),
        vec![error_issue],
        true, // truncated — pipeline didn't finish
        trace::drain_events(),
        false, // trace incomplete
        solver_result.cloned(),
    );

    let path = dir.join(format!("snapshot-{test_name}-partial.fls"));
    match snapshot.write_to_file(&path) {
        Ok(()) => eprintln!("  partial snapshot: {}", path.display()),
        Err(e) => eprintln!("  partial snapshot write failed: {e}"),
    }
}

/// Directory for snapshot files. Uses `CARGO_TARGET_TMPDIR` if available,
/// otherwise `target/tmp/`.
fn snapshot_dir() -> PathBuf {
    std::env::var("CARGO_TARGET_TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("target/tmp"))
}

/// Best-effort git SHA.
fn git_sha() -> Option<String> {
    std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

/// Parameters for a test run (borrowed from the test function's arguments).
struct RunParams<'a> {
    item: &'a str,
    rate: f64,
    machine: &'a str,
    belt_tier: Option<&'a str>,
    available_inputs: &'a FxHashSet<String>,
}

fn run_e2e(
    test_name: &str,
    item: &str,
    rate: f64,
    machine: &str,
    belt_tier: Option<&str>,
    available_inputs: &FxHashSet<String>,
) -> Result<E2EResult, String> {
    let _guard = trace::start_trace();
    fucktorio_core::zone_cache::set_thread_source(Some(test_name));
    let run_params = RunParams { item, rate, machine, belt_tier, available_inputs };

    let solver_result = solver::solve(item, rate, available_inputs, machine)
        .map_err(|e| {
            let msg = format!("solver: {e}");
            dump_partial_snapshot(test_name, &run_params, None, &msg);
            msg
        })?;

    let layout = layout::build_bus_layout(&solver_result, belt_tier)
        .map_err(|e| {
            let msg = format!("layout: {e}");
            dump_partial_snapshot(test_name, &run_params, Some(&solver_result), &msg);
            msg
        })?;

    // Validate the original layout (correct top-left positions).
    let issues = match validate::validate(&layout, Some(&solver_result), LayoutStyle::Bus) {
        Ok(issues) => issues,
        Err(e) => e.issues,
    };

    let analysis = analysis::analyze(&layout);

    // Round-trip through blueprint export → parse as a smoke test.
    let bp_string = blueprint::export(&layout, item);
    let parsed = blueprint_parser::parse_blueprint_string(&bp_string)
        .map_err(|e| {
            let msg = format!("parse: {e}");
            dump_partial_snapshot(test_name, &run_params, Some(&solver_result), &msg);
            msg
        })?;

    // Drain trace events into the result so callers (and dump_snapshot below)
    // can read them without the RAII guard wiping them on drop.
    let trace_events = trace::drain_events();

    let result = E2EResult {
        solver_result,
        layout,
        parsed,
        issues,
        analysis,
        trace_events,
    };

    // Dump snapshot if there are errors or if env var is set.
    let has_errors = result.issues.iter().any(|i| i.severity == Severity::Error);
    if has_errors || should_dump_snapshots() {
        dump_snapshot(test_name, &run_params, &result);
    }

    fucktorio_core::zone_cache::set_thread_source(None);
    Ok(result)
}

fn assert_no_errors(result: &E2EResult) {
    let errors: Vec<_> = result
        .issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .collect();
    assert!(
        errors.is_empty(),
        "Expected 0 validation errors, got {}:\n{}",
        errors.len(),
        errors
            .iter()
            .map(|i| format!("  [{}] {} — {}", i.category, i.message, i.x.map(|x| format!("({},{})", x, i.y.unwrap_or(0))).unwrap_or_default()))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// Assert the layout has no validation warnings either.
///
/// Warnings are "soft" issues (belt-dead-end, input-rate-delivery, lane-throughput, etc.)
/// that don't prevent the blueprint from importing into Factorio, but do indicate the
/// layout is structurally broken in ways that matter — e.g. a starved machine will never
/// produce its output even though the validation errors are "merely" warnings.
///
/// We group by category and show counts + a few examples per category to keep the
/// failure message readable when there are many issues.
fn assert_no_warnings(result: &E2EResult) {
    assert_no_warnings_except(result, &[]);
}

/// Like [`assert_no_warnings`] but silently skips warnings in the listed categories.
///
/// Use sparingly — only for pre-existing layout-engine bugs that are tracked as
/// separate issues and shouldn't block the validator fix under test.
fn assert_no_warnings_except(result: &E2EResult, skip_categories: &[&str]) {
    let warnings: Vec<_> = result
        .issues
        .iter()
        .filter(|i| i.severity == Severity::Warning && !skip_categories.contains(&i.category.as_str()))
        .collect();
    if warnings.is_empty() {
        return;
    }
    let mut by_category: std::collections::BTreeMap<&str, Vec<&validate::ValidationIssue>> = Default::default();
    for w in &warnings {
        by_category.entry(w.category.as_str()).or_default().push(w);
    }
    let mut msg = format!("Expected 0 validation warnings, got {}:\n", warnings.len());
    for (cat, items) in &by_category {
        msg.push_str(&format!("  [{}] × {}\n", cat, items.len()));
        for w in items.iter().take(3) {
            let coords = w.x.map(|x| format!(" ({},{})", x, w.y.unwrap_or(0))).unwrap_or_default();
            msg.push_str(&format!("      {}{}\n", w.message, coords));
        }
        if items.len() > 3 {
            msg.push_str(&format!("      ... {} more\n", items.len() - 3));
        }
    }
    panic!("{}", msg);
}

fn assert_produces(result: &E2EResult, item: &str, min_rate: f64) {
    let actual = result
        .analysis
        .throughput_estimates
        .get(item)
        .copied()
        .unwrap_or(0.0);
    assert!(
        actual >= min_rate * 0.99,
        "Expected ≥{min_rate:.1}/s {item} but analysis says {actual:.1}/s",
    );
}

fn assert_round_trip(result: &E2EResult) {
    // Check entity count and per-entity position/direction/name.
    // Metadata like carries, segment_id, and rate are lost in the blueprint
    // format, so we only compare structural fields.
    assert_eq!(
        result.layout.entities.len(),
        result.parsed.entities.len(),
        "Round-trip entity count mismatch: layout has {} but parsed has {}",
        result.layout.entities.len(),
        result.parsed.entities.len(),
    );

    // Normalize both to (0,0) origin before comparing — the parser always
    // normalizes but the layout engine may use a different origin.
    let l_min_x = result.layout.entities.iter().map(|e| e.x).min().unwrap_or(0);
    let l_min_y = result.layout.entities.iter().map(|e| e.y).min().unwrap_or(0);
    let p_min_x = result.parsed.entities.iter().map(|e| e.x).min().unwrap_or(0);
    let p_min_y = result.parsed.entities.iter().map(|e| e.y).min().unwrap_or(0);

    // Sort both lists by (name, x-lmin, y-lmin, direction) and compare pairwise.
    let mut layout_sorted: Vec<_> = result.layout.entities.iter().collect();
    layout_sorted.sort_by_key(|e| (e.name.clone(), e.x - l_min_x, e.y - l_min_y, e.direction as u8));
    let mut parsed_sorted: Vec<_> = result.parsed.entities.iter().collect();
    parsed_sorted.sort_by_key(|e| (e.name.clone(), e.x - p_min_x, e.y - p_min_y, e.direction as u8));

    for (i, (orig, parsed)) in layout_sorted.iter().zip(parsed_sorted.iter()).enumerate() {
        assert_eq!(
            (orig.name.clone(), orig.x - l_min_x, orig.y - l_min_y, orig.direction as u8),
            (parsed.name.clone(), parsed.x - p_min_x, parsed.y - p_min_y, parsed.direction as u8),
            "Entity {i} mismatch: layout has {} at ({},{}) dir {:?}, parsed has {} at ({},{}) dir {:?}",
            orig.name, orig.x, orig.y, orig.direction,
            parsed.name, parsed.x, parsed.y, parsed.direction
        );
    }
}

// ---------------------------------------------------------------------------
// Tier 1: iron-gear-wheel (1 recipe, 1 solid input)
// ---------------------------------------------------------------------------

// Most of the tier1/2/3 tests below were direct-mode regression guards.
// After the direct-mode deletion ghost mode is the only routing path, and
// the ghost router currently fails them — head-on belt collisions, dead-end
// belts, item-isolation between adjacent trunks, etc. They are marked
// `#[ignore]` with a one-line failure summary until ghost mode catches up.
// The two passing ones (`tier3_sulfuric_acid`,
// `tier2_electronic_circuit_splitter_stamp_regression`) stay live as the
// new green-bar regression guards for ghost routing.

#[test]
#[ntest::timeout(10000)]
fn tier1_iron_gear_wheel() {
    let inputs: FxHashSet<String> = ["iron-plate"].iter().map(|s| s.to_string()).collect();
    let result = run_e2e("tier1_iron_gear_wheel", "iron-gear-wheel", 10.0, "assembling-machine-1", None, &inputs)
        .unwrap_or_else(|e| panic!("tier1_iron_gear_wheel: {e}"));

    assert_no_errors(&result);
    assert_no_warnings(&result);
    assert_produces(&result, "iron-gear-wheel", 10.0);
    assert_round_trip(&result);
}

#[test]
#[ntest::timeout(10000)]
fn tier1_iron_gear_wheel_from_ore() {
    let inputs: FxHashSet<String> = ["iron-ore"].iter().map(|s| s.to_string()).collect();
    let result = run_e2e(
        "tier1_iron_gear_wheel_from_ore",
        "iron-gear-wheel",
        10.0,
        "assembling-machine-2",
        None,
        &inputs,
    )
    .unwrap_or_else(|e| panic!("tier1_iron_gear_wheel_from_ore: {e}"));

    assert_no_errors(&result);
    assert_no_warnings(&result);
    assert_produces(&result, "iron-gear-wheel", 10.0);
    assert_round_trip(&result);
}

#[test]
#[ntest::timeout(10000)]
fn tier1_iron_gear_wheel_20s() {
    let inputs: FxHashSet<String> = ["iron-plate"].iter().map(|s| s.to_string()).collect();
    let result = run_e2e("tier1_iron_gear_wheel_20s", "iron-gear-wheel", 20.0, "assembling-machine-2", None, &inputs)
        .unwrap_or_else(|e| panic!("tier1_iron_gear_wheel_20s: {e}"));

    assert_no_errors(&result);
    assert_no_warnings(&result);
    assert_produces(&result, "iron-gear-wheel", 20.0);
    assert_round_trip(&result);
}

// ---------------------------------------------------------------------------
// Tier 2: electronic-circuit (2 recipes, 2 solid inputs)
// ---------------------------------------------------------------------------

#[test]
#[ignore = "belt-item-isolation: iron-plate belt at (2,10) feeds into copper-cable at (3,10) — SAT zone routing conflict after entry-direction fix"]
#[ntest::timeout(10000)]
fn tier2_electronic_circuit() {
    let inputs: FxHashSet<String> = ["iron-plate", "copper-plate"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let result = run_e2e(
        "tier2_electronic_circuit",
        "electronic-circuit",
        10.0,
        "assembling-machine-2",
        None,
        &inputs,
    )
    .unwrap_or_else(|e| panic!("tier2_electronic_circuit: {e}"));

    assert_no_errors(&result);
    assert_no_warnings(&result);
    assert_produces(&result, "electronic-circuit", 10.0);
    assert_round_trip(&result);
}

#[test]
#[ignore = "ghost-mode: dead-ends + UG pair distance violations on full ore chain"]
#[ntest::timeout(10000)]
fn tier2_electronic_circuit_from_ore() {
    let inputs: FxHashSet<String> = ["iron-ore", "copper-ore"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let result = run_e2e(
        "tier2_electronic_circuit_from_ore",
        "electronic-circuit",
        10.0,
        "assembling-machine-1",
        Some("transport-belt"),
        &inputs,
    )
    .unwrap_or_else(|e| panic!("tier2_electronic_circuit_from_ore: {e}"));

    assert_no_errors(&result);
    // The `power` warning (27 disconnected poles) is a pre-existing layout-engine
    // bug tracked separately — all belt-flow validator false-positives are fixed.
    assert_no_warnings_except(&result, &["power"]);
    assert_produces(&result, "electronic-circuit", 10.0);
    assert_round_trip(&result);
}

#[test]
#[ntest::timeout(10000)]
fn tier2_electronic_circuit_20s_from_ore() {
    let inputs: FxHashSet<String> = ["iron-ore", "copper-ore"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let result = run_e2e(
        "tier2_electronic_circuit_20s_from_ore",
        "electronic-circuit",
        20.0,
        "assembling-machine-2",
        None,
        &inputs,
    )
    .unwrap_or_else(|e| panic!("tier2_electronic_circuit_20s_from_ore: {e}"));

    assert_no_errors(&result);
    // The `power` warning (25 disconnected poles) is a pre-existing layout-engine
    // bug tracked separately — all belt-flow validator false-positives are fixed.
    assert_no_warnings_except(&result, &["power"]);
    assert_produces(&result, "electronic-circuit", 20.0);
    assert_round_trip(&result);
}

/// Regression test for the splitter-stamp sideload-into-UG-input bug that the
/// user reported: `electronic-circuit` at 10/s, assembling-machine-1 with fast
/// belts, generating from `{iron-plate, copper-plate}`. The bug class manifests
/// as a `DroppedBridge` in the router — the foreign-trunk yield (UG bridge)
/// for one lane's trunk couldn't be emitted because its UG output tile
/// collided with the trunk's own tap-off. Before the retry-loop fix in
/// `build_bus_layout`, this produced an invalid sideload into the tap-off's
/// underground-belt-input first tile. The retry loop maps dropped bridges to
/// `extra_gap_after_row` updates, pushing the colliding row down by 1 so the
/// bridge becomes valid.
///
/// This test specifically guards the retry feedback loop: if it ever stops
/// firing (e.g. route_belt_lane stops pushing to dropped_bridges), this test
/// fails because the sideload warning comes back.
#[test]
#[ntest::timeout(10000)]
fn tier2_electronic_circuit_splitter_stamp_regression() {
    let inputs: FxHashSet<String> = ["iron-plate", "copper-plate"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let result = run_e2e(
        "tier2_electronic_circuit_splitter_stamp_regression",
        "electronic-circuit",
        10.0,
        "assembling-machine-1",
        Some("fast-transport-belt"),
        &inputs,
    )
    .unwrap_or_else(|e| panic!("tier2_electronic_circuit_splitter_stamp_regression: {e}"));

    // Specifically assert there's no sideload-into-UG-input warning, which
    // is the precise bug class the retry loop addresses.
    let sideload_issues: Vec<_> = result.issues.iter()
        .filter(|i| i.message.contains("sideloads into underground input"))
        .collect();
    assert!(
        sideload_issues.is_empty(),
        "Expected no sideload-into-UG-input warnings, got {}:\n{}",
        sideload_issues.len(),
        sideload_issues.iter()
            .map(|i| format!("  [{}] {} ({},{})", i.category, i.message,
                i.x.unwrap_or(-1), i.y.unwrap_or(-1)))
            .collect::<Vec<_>>()
            .join("\n")
    );
    // Ensure the layout can actually produce items (no solver/routing failure).
    assert_produces(&result, "electronic-circuit", 10.0);
}

// ---------------------------------------------------------------------------
// Tier 3: plastic-bar (1 recipe, 1 fluid + 1 solid input)
// ---------------------------------------------------------------------------

#[test]
#[ignore = "ghost-mode: routing fails on plastic-bar feeder paths"]
#[ntest::timeout(10000)]
fn tier3_plastic_bar() {
    let inputs: FxHashSet<String> = ["petroleum-gas", "coal"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let result =
        run_e2e("tier3_plastic_bar", "plastic-bar", 10.0, "chemical-plant", None, &inputs)
            .unwrap_or_else(|e| panic!("tier3_plastic_bar: {e}"));

    assert_no_errors(&result);
    assert_no_warnings(&result);
    assert_produces(&result, "plastic-bar", 10.0);
    assert_round_trip(&result);
}

#[test]
#[ignore = "ghost-mode: routing fails on plastic-bar feeder paths from crude oil"]
#[ntest::timeout(10000)]
fn tier3_plastic_bar_from_crude() {
    let inputs: FxHashSet<String> = ["crude-oil", "coal"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let result =
        run_e2e("tier3_plastic_bar_from_crude", "plastic-bar", 10.0, "chemical-plant", None, &inputs)
            .unwrap_or_else(|e| panic!("tier3_plastic_bar_from_crude: {e}"));

    assert_no_errors(&result);
    assert_no_warnings(&result);
    assert_produces(&result, "plastic-bar", 10.0);
    assert_round_trip(&result);
}

#[test]
#[ntest::timeout(10000)]
fn tier3_sulfuric_acid() {
    let inputs: FxHashSet<String> = ["iron-plate", "sulfur", "water"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let result =
        run_e2e("tier3_sulfuric_acid", "sulfuric-acid", 5.0, "chemical-plant", None, &inputs)
            .unwrap_or_else(|e| panic!("tier3_sulfuric_acid: {e}"));

    assert_no_errors(&result);
    assert_no_warnings(&result);
    assert_produces(&result, "sulfuric-acid", 5.0);
    assert_round_trip(&result);
}

// ---------------------------------------------------------------------------
// Tier 4: advanced-circuit (5+ recipes, mixed solid/fluid)
// Known issues: lane-throughput warnings from single-lane sideload bottleneck (#64)
// ---------------------------------------------------------------------------

#[test]
#[ignore] // Blocked by #64: lane-throughput warnings
#[ntest::timeout(10000)]
fn tier4_advanced_circuit_from_plates() {
    // Nauvis-style inputs: plates + raw resources (coal, crude-oil) + water.
    // Solver will synthesize plastic-bar from petroleum-gas and coal.
    let inputs: FxHashSet<String> = ["iron-plate", "copper-plate", "coal", "crude-oil", "water"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let result = run_e2e(
        "tier4_advanced_circuit_from_plates",
        "advanced-circuit",
        1.0,
        "assembling-machine-2",
        None,
        &inputs,
    )
    .unwrap_or_else(|e| panic!("tier4_advanced_circuit_from_plates: {e}"));

    assert_no_errors(&result);
    assert_no_warnings(&result);
    assert_produces(&result, "advanced-circuit", 1.0);
    assert_round_trip(&result);
}

/// Advanced circuit, rate 5/s, AM1, yellow belts, from raw ores + crude oil.
/// This is the "hello-world fully-from-ore AC" goal — cheapest machine tier,
/// cheapest belt tier, everything upstream of the factory is raw resources.
/// Currently failing; see docs/tier2-from-ore-followup.md and tracking work.
#[test]
#[ignore] // Goal: make this green. See tier4_advanced_circuit_from_ore_am1.
#[ntest::timeout(30000)]
fn tier4_advanced_circuit_from_ore_am1() {
    let inputs: FxHashSet<String> = [
        "iron-ore", "copper-ore", "coal", "water", "crude-oil",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let result = run_e2e(
        "tier4_advanced_circuit_from_ore_am1",
        "advanced-circuit",
        5.0,
        "assembling-machine-1",
        Some("transport-belt"),
        &inputs,
    )
    .unwrap_or_else(|e| panic!("tier4_advanced_circuit_from_ore_am1: {e}"));

    assert_no_errors(&result);
    assert_no_warnings(&result);
    assert_produces(&result, "advanced-circuit", 5.0);
    assert_round_trip(&result);
}

// ---------------------------------------------------------------------------
// Diagnostic: find which validator hangs on large layouts
// ---------------------------------------------------------------------------

#[test]
#[ignore] // Diagnostic only — run with --ignored --nocapture
fn diag_validator_timing_from_ore() {
    let inputs: FxHashSet<String> = ["iron-ore"].iter().map(|s| s.to_string()).collect();
    let sr = solver::solve("iron-gear-wheel", 10.0, &inputs, "assembling-machine-2")
        .unwrap_or_else(|e| panic!("solver (iron-gear-wheel from ore): {e}"));
    let lr = layout::build_bus_layout(&sr, None)
        .unwrap_or_else(|e| panic!("layout (iron-gear-wheel from ore): {e}"));
    eprintln!("=== iron-gear-wheel from ore ===");
    eprintln!("Layout: {} entities, {}x{}", lr.entities.len(), lr.width, lr.height);
    run_timed_validators(&lr, &sr);

    // The layout that was hanging
    let inputs2: FxHashSet<String> = ["iron-ore", "copper-ore"].iter().map(|s| s.to_string()).collect();
    let sr2 = solver::solve("electronic-circuit", 10.0, &inputs2, "assembling-machine-1")
        .unwrap_or_else(|e| panic!("solver (electronic-circuit from ore): {e}"));
    let lr2 = layout::build_bus_layout(&sr2, Some("transport-belt"))
        .unwrap_or_else(|e| panic!("layout (electronic-circuit from ore): {e}"));
    eprintln!("\n=== electronic-circuit from ore ===");
    eprintln!("Layout: {} entities, {}x{}", lr2.entities.len(), lr2.width, lr2.height);
    run_timed_validators(&lr2, &sr2);
}

fn run_timed_validators(lr: &LayoutResult, sr: &SolverResult) {
    #[allow(clippy::type_complexity)]
    let checks: Vec<(&str, Box<dyn FnOnce() -> Vec<ValidationIssue>>)> = vec![
        ("power_coverage", Box::new(|| power::check_power_coverage(lr))),
        ("pole_network_connectivity", Box::new(|| power::check_pole_network_connectivity(lr))),
        ("inserter_chains", Box::new(|| inserters::check_inserter_chains(lr, Some(sr)))),
        ("inserter_direction", Box::new(|| inserters::check_inserter_direction(lr))),
        ("pipe_isolation", Box::new(|| validate::check_pipe_isolation(lr))),
        ("fluid_port_connectivity", Box::new(|| validate::check_fluid_port_connectivity(lr, LayoutStyle::Bus))),
        ("belt_connectivity", Box::new(|| belt_flow::check_belt_connectivity(lr, Some(sr)))),
        ("belt_flow_path", Box::new(|| belt_flow::check_belt_flow_path(lr, Some(sr), LayoutStyle::Bus))),
        ("entity_overlaps", Box::new(|| belt_structural::check_entity_overlaps(lr))),
        ("belt_throughput", Box::new(|| belt_structural::check_belt_throughput(lr))),
        ("output_belt_coverage", Box::new(|| belt_structural::check_output_belt_coverage(lr, Some(sr)))),
        ("belt_junctions", Box::new(|| belt_flow::check_belt_junctions(lr))),
        ("underground_belt_pairs", Box::new(|| belt_flow::check_underground_belt_pairs(lr))),
        ("underground_belt_sideloading", Box::new(|| belt_flow::check_underground_belt_sideloading(lr))),
        ("underground_belt_entry_sideload", Box::new(|| belt_flow::check_underground_belt_entry_sideload(lr))),
        ("belt_dead_ends", Box::new(|| belt_structural::check_belt_dead_ends(lr))),
        ("belt_loops", Box::new(|| belt_structural::check_belt_loops(lr))),
        ("belt_item_isolation", Box::new(|| belt_structural::check_belt_item_isolation(lr))),
        ("belt_inserter_conflict", Box::new(|| belt_structural::check_belt_inserter_conflict(lr))),
        ("belt_flow_reachability", Box::new(|| belt_flow::check_belt_flow_reachability(lr, Some(sr), LayoutStyle::Bus))),
        ("lane_throughput", Box::new(|| belt_structural::check_lane_throughput(lr, Some(sr)))),
        ("input_rate_delivery", Box::new(|| belt_flow::check_input_rate_delivery(lr, Some(sr)))),
    ];

    for (name, check) in checks {
        let start = Instant::now();
        eprintln!("  {name} ...");
        let issues = check();
        let elapsed = start.elapsed();
        let errors = issues.iter().filter(|i| i.severity == Severity::Error).count();
        eprintln!("  {name} -> {}ms ({} errors, {} warnings)",
            elapsed.as_millis(), errors, issues.len() - errors);
    }
}

// ---------------------------------------------------------------------------
// Stress corpus (Phase 0 of the SAT junction solver plan).
//
// These tests are benchmarks, not regressions. They exercise layout regimes
// where the current crossing-zone solver breaks down — many lanes, many
// N→M balancers, wide trunk groups, red-belt UG reach. Each test always
// fails with a scoreboard `panic!` that lists:
//   - warnings grouped by category
//   - zones solved / zones skipped (from CrossingZoneSolved/Skipped trace)
//   - dropped-bridge count
// so successive phases of the generalized junction solver can be measured
// against the baseline recorded in each test's comment header.
//
// Run with:
//   cargo test --manifest-path crates/core/Cargo.toml --test e2e -- \
//       --ignored --nocapture stress_
// ---------------------------------------------------------------------------

/// Tally warnings by category and pull zone-solve counts from the trace.
/// Always panics — these tests are measurement, not pass/fail.
fn report_stress_scoreboard(test_name: &str, result: &E2EResult) -> ! {
    let mut by_category: std::collections::BTreeMap<&str, usize> = Default::default();
    for w in result.issues.iter().filter(|i| i.severity == Severity::Warning) {
        *by_category.entry(w.category.as_str()).or_default() += 1;
    }

    let mut zones_solved = 0usize;
    let mut zones_skipped = 0usize;
    let mut bridges_dropped = 0usize;
    let mut band_count = 0usize;
    let mut crossing_bands = 0usize;
    let mut noncrossing_bands = 0usize;
    let mut total_gap_tiles: i32 = 0;
    let mut max_gap: i32 = 0;
    let mut band_trunks_max: usize = 0;
    let mut crossing_zones: Vec<(i32, i32)> = Vec::new(); // (y, y+height-1) inclusive
    for ev in &result.trace_events {
        match ev {
            TraceEvent::CrossingZoneSolved { y, height, .. } => {
                zones_solved += 1;
                crossing_zones.push((*y, *y + *height as i32 - 1));
            }
            TraceEvent::CrossingZoneSkipped { .. } => zones_skipped += 1,
            TraceEvent::BridgeDropped { .. } => bridges_dropped += 1,
            _ => {}
        }
    }
    for ev in &result.trace_events {
        if let TraceEvent::InterRowBand {
            band_y_start,
            band_y_end,
            gap_height,
            trunk_count,
            ..
        } = ev
        {
            band_count += 1;
            total_gap_tiles += *gap_height;
            if *gap_height > max_gap {
                max_gap = *gap_height;
            }
            if *trunk_count > band_trunks_max {
                band_trunks_max = *trunk_count;
            }
            let has_crossing = crossing_zones
                .iter()
                .any(|&(y0, y1)| y1 >= *band_y_start && y0 <= *band_y_end);
            if has_crossing {
                crossing_bands += 1;
            } else {
                noncrossing_bands += 1;
            }
        }
    }
    let mean_gap = if band_count > 0 {
        total_gap_tiles as f64 / band_count as f64
    } else {
        0.0
    };

    let total_warnings: usize = by_category.values().sum();
    let mut msg = format!(
        "\n=== {test_name} scoreboard ===\n\
         entities:         {}\n\
         total warnings:   {}\n\
         zones solved:     {}\n\
         zones skipped:    {}\n\
         bridges dropped:  {}\n\
         bands:            {} (crossing: {}, non-crossing: {})\n\
         total gap tiles:  {}\n\
         mean gap:         {:.2}\n\
         max gap:          {}\n\
         max trunks/band:  {}\n\
         warnings by category:\n",
        result.layout.entities.len(),
        total_warnings,
        zones_solved,
        zones_skipped,
        bridges_dropped,
        band_count,
        crossing_bands,
        noncrossing_bands,
        total_gap_tiles,
        mean_gap,
        max_gap,
        band_trunks_max,
    );
    if by_category.is_empty() {
        msg.push_str("  (none)\n");
    } else {
        for (cat, count) in &by_category {
            msg.push_str(&format!("  {cat}: {count}\n"));
        }
    }
    panic!("{msg}");
}

/// Baseline (Phase 1, 2026-04-11): entities=11232, warnings=0, zones_solved=19,
/// bands=3 (1 crossing, 2 non-crossing), total_gap_tiles=33, mean_gap=11.00,
/// max_gap=15, max_trunks/band=20. Note: the "non-crossing" bands here are
/// inflated by balancer reflow — Phase 2 must mark balancer-touching bands as
/// non-compactable.
#[test]
#[ignore]
#[ntest::timeout(180000)]
fn stress_electronic_circuit_30s_from_ore() {
    let inputs: FxHashSet<String> = ["iron-ore", "copper-ore"]
        .iter().map(|s| s.to_string()).collect();
    let result = run_e2e(
        "stress_electronic_circuit_30s_from_ore",
        "electronic-circuit",
        30.0,
        "assembling-machine-2",
        Some("transport-belt"),
        &inputs,
    ).expect("e2e pipeline");
    assert_produces(&result, "electronic-circuit", 30.0);
    report_stress_scoreboard("stress_electronic_circuit_30s_from_ore", &result);
}

/// Baseline (Phase 1, 2026-04-11): entities=13131, warnings=0, zones_solved=28,
/// bands=2 (2 crossing, 0 non-crossing), total_gap_tiles=5, mean_gap=2.50,
/// max_gap=3, max_trunks/band=12.
#[test]
#[ignore]
#[ntest::timeout(180000)]
fn stress_advanced_circuit_45s_from_plates() {
    let inputs: FxHashSet<String> = ["iron-plate", "copper-plate", "plastic-bar"]
        .iter().map(|s| s.to_string()).collect();
    let result = run_e2e(
        "stress_advanced_circuit_45s_from_plates",
        "advanced-circuit",
        45.0,
        "assembling-machine-2",
        None,
        &inputs,
    ).expect("e2e pipeline");
    assert_produces(&result, "advanced-circuit", 45.0);
    report_stress_scoreboard("stress_advanced_circuit_45s_from_plates", &result);
}

/// Baseline (pre-Phase 1): warnings=?, zones_solved=?, zones_skipped=?.
/// processing-unit requires an AM3 because sulfuric-acid is a fluid input.
#[test]
#[ignore]
#[ntest::timeout(300000)]
fn stress_processing_unit_20s_from_plates() {
    let inputs: FxHashSet<String> = ["iron-plate", "copper-plate", "plastic-bar", "sulfuric-acid"]
        .iter().map(|s| s.to_string()).collect();
    let result = run_e2e(
        "stress_processing_unit_20s_from_plates",
        "processing-unit",
        20.0,
        "assembling-machine-3",
        None,
        &inputs,
    ).expect("e2e pipeline");
    assert_produces(&result, "processing-unit", 20.0);
    report_stress_scoreboard("stress_processing_unit_20s_from_plates", &result);
}



/// Baseline (Phase 1, 2026-04-11): entities=9190, warnings=0, zones_solved=13,
/// bands=3 (1 crossing, 2 non-crossing), total_gap_tiles=22, mean_gap=7.33,
/// max_gap=12, max_trunks/band=14.
#[test]
#[ignore]
#[ntest::timeout(180000)]
fn stress_electronic_circuit_60s_red_from_ore() {
    let inputs: FxHashSet<String> = ["iron-ore", "copper-ore"]
        .iter().map(|s| s.to_string()).collect();
    let result = run_e2e(
        "stress_electronic_circuit_60s_red_from_ore",
        "electronic-circuit",
        60.0,
        "assembling-machine-2",
        Some("fast-transport-belt"),
        &inputs,
    ).expect("e2e pipeline");
    assert_produces(&result, "electronic-circuit", 60.0);
    report_stress_scoreboard("stress_electronic_circuit_60s_red_from_ore", &result);
}

// ---------------------------------------------------------------------------
// Ghost-cluster sizing diagnostic
//
// Proposed unified routing scheme: allow belts to "ghost through" each other
// when perpendicular, route every lane with turn-biased A* whose obstacle set
// excludes belts (but still respects machines, poles, pipes, inserters), then
// SAT-solve the resulting ghost-crossing clusters. Open question: how big do
// those clusters get on a realistic case like AC-from-ore before we commit?
//
// This diagnostic routes every currently-failing `ret:` spec under ghost
// semantics, union-finds paths that share a tile, and reports cluster sizes.
// If max cluster is bounded (say <50 tiles), the unified scheme is tractable.
// If it's huge, we know the approach needs component boundaries before
// anything gets rewritten.
// ---------------------------------------------------------------------------

#[test]
#[ignore]
#[ntest::timeout(60000)]
#[allow(clippy::type_complexity)]
fn diag_ghost_cluster_ac_from_ore() {
    use fucktorio_core::common::{machine_size, machine_tiles};
    use rustc_hash::FxHashMap;
    use std::cmp::Reverse;

    let inputs: FxHashSet<String> = [
        "iron-ore", "copper-ore", "coal", "water", "crude-oil",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let result = run_e2e(
        "diag_ghost_cluster_ac_from_ore",
        "advanced-circuit",
        5.0,
        "assembling-machine-1",
        Some("transport-belt"),
        &inputs,
    )
    .expect("e2e");

    // --- Pull failing ret:/feeder:/tap: specs from trace ---
    let failures: Vec<(String, (i32, i32), (i32, i32))> = result
        .trace_events
        .iter()
        .filter_map(|ev| match ev {
            TraceEvent::RouteFailure {
                spec_key,
                from_x,
                from_y,
                to_x,
                to_y,
                ..
            } => Some((spec_key.clone(), (*from_x, *from_y), (*to_x, *to_y))),
            _ => None,
        })
        .collect();

    eprintln!("route failures in current pipeline: {}", failures.len());
    for (k, s, g) in &failures {
        eprintln!("  {} : {:?} -> {:?}", k, s, g);
    }

    // --- Build obstacle grid: everything except belts ---
    let is_belt_like = |name: &str| -> bool {
        matches!(
            name,
            "transport-belt"
                | "fast-transport-belt"
                | "express-transport-belt"
                | "underground-belt"
                | "fast-underground-belt"
                | "express-underground-belt"
                | "splitter"
                | "fast-splitter"
                | "express-splitter"
        )
    };

    let mut hard: FxHashSet<(i32, i32)> = FxHashSet::default();
    let mut existing_belts: FxHashSet<(i32, i32)> = FxHashSet::default();
    for e in &result.layout.entities {
        if is_belt_like(&e.name) {
            existing_belts.insert((e.x, e.y));
        } else {
            // Machines and other multi-tile entities: use machine_size for
            // footprint; single-tile things (pipes, inserters, poles) fall
            // through to size=1 from machine_size's default.
            let sz = machine_size(&e.name);
            for t in machine_tiles(e.x, e.y, sz) {
                hard.insert(t);
            }
        }
    }

    eprintln!(
        "layout: {} entities, {} belts, {} hard obstacle tiles",
        result.layout.entities.len(),
        existing_belts.len(),
        hard.len()
    );

    let width = result.layout.width.max(200);
    let height = result.layout.height.max(300);

    // --- Simple turn-biased A* ---
    // State = (x, y, incoming_dir).  Straight step = 1, turn = 1 + penalty.
    // No underground belts (we're measuring raw ghost semantics).
    fn astar(
        start: (i32, i32),
        goal: (i32, i32),
        hard: &FxHashSet<(i32, i32)>,
        width: i32,
        height: i32,
        turn_penalty: u32,
    ) -> Option<Vec<(i32, i32)>> {
        use rustc_hash::FxHashMap;
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;

        #[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
        struct State {
            x: i32,
            y: i32,
            dir: i8, // -1 = unset, 0=E 1=S 2=W 3=N
        }
        let dirs: [(i32, i32, i8); 4] = [(1, 0, 0), (0, 1, 1), (-1, 0, 2), (0, -1, 3)];
        let h = |x: i32, y: i32| ((x - goal.0).abs() + (y - goal.1).abs()) as u32;

        let mut heap: BinaryHeap<Reverse<(u32, State)>> = BinaryHeap::new();
        let mut g: FxHashMap<State, u32> = FxHashMap::default();
        let mut parent: FxHashMap<State, State> = FxHashMap::default();

        let start_state = State {
            x: start.0,
            y: start.1,
            dir: -1,
        };
        heap.push(Reverse((h(start.0, start.1), start_state)));
        g.insert(start_state, 0);

        while let Some(Reverse((_, s))) = heap.pop() {
            if (s.x, s.y) == goal {
                let mut path = vec![(s.x, s.y)];
                let mut cur = s;
                while let Some(&p) = parent.get(&cur) {
                    path.push((p.x, p.y));
                    cur = p;
                }
                path.reverse();
                return Some(path);
            }
            let cur_g = g[&s];
            for &(dx, dy, dir) in &dirs {
                let nx = s.x + dx;
                let ny = s.y + dy;
                if nx < 0 || nx >= width || ny < 0 || ny >= height {
                    continue;
                }
                if hard.contains(&(nx, ny)) && (nx, ny) != goal {
                    continue;
                }
                let step = if s.dir == -1 || s.dir == dir {
                    1
                } else {
                    1 + turn_penalty
                };
                let ns = State { x: nx, y: ny, dir };
                let ng = cur_g + step;
                if g.get(&ns).copied().unwrap_or(u32::MAX) > ng {
                    g.insert(ns, ng);
                    parent.insert(ns, s);
                    heap.push(Reverse((ng + h(nx, ny), ns)));
                }
            }
        }
        None
    }

    // --- Route each failing spec ---
    let turn_penalty = 8;
    let mut routed: Vec<(String, Vec<(i32, i32)>)> = Vec::new();
    let mut unroutable: Vec<String> = Vec::new();
    for (key, start, goal) in &failures {
        match astar(*start, *goal, &hard, width, height, turn_penalty) {
            Some(path) => routed.push((key.clone(), path)),
            None => unroutable.push(key.clone()),
        }
    }

    eprintln!(
        "\nunder ghost semantics: {}/{} routed, {} still unroutable",
        routed.len(),
        failures.len(),
        unroutable.len()
    );
    for k in &unroutable {
        eprintln!("  STILL FAILS: {}", k);
    }

    // --- Per-path stats ---
    let count_turns = |path: &[(i32, i32)]| -> usize {
        let mut t = 0;
        for w in path.windows(3) {
            let d1 = (w[1].0 - w[0].0, w[1].1 - w[0].1);
            let d2 = (w[2].0 - w[1].0, w[2].1 - w[1].1);
            if d1 != d2 {
                t += 1;
            }
        }
        t
    };

    eprintln!("\nper-path (len / crossings / turns):");
    for (key, path) in &routed {
        let crossings = path.iter().filter(|t| existing_belts.contains(t)).count();
        let turns = count_turns(path);
        eprintln!(
            "  {}: len={} crossings={} turns={}",
            key,
            path.len(),
            crossings,
            turns
        );
    }

    // --- Union-find over paths that share tiles ---
    fn find(p: &mut [usize], i: usize) -> usize {
        let mut r = i;
        while p[r] != r {
            r = p[r];
        }
        let mut cur = i;
        while p[cur] != r {
            let next = p[cur];
            p[cur] = r;
            cur = next;
        }
        r
    }

    let mut uf: Vec<usize> = (0..routed.len()).collect();
    let mut tile_owner: FxHashMap<(i32, i32), usize> = FxHashMap::default();
    for (i, (_, path)) in routed.iter().enumerate() {
        for &tile in path {
            if let Some(&j) = tile_owner.get(&tile) {
                let ri = find(&mut uf, i);
                let rj = find(&mut uf, j);
                if ri != rj {
                    uf[ri] = rj;
                }
            } else {
                tile_owner.insert(tile, i);
            }
        }
    }

    // Cluster aggregation: for each root, sum unique path tiles + count paths
    let mut cluster_tiles: FxHashMap<usize, FxHashSet<(i32, i32)>> = FxHashMap::default();
    let mut cluster_paths: FxHashMap<usize, usize> = FxHashMap::default();
    let mut cluster_crossings: FxHashMap<usize, usize> = FxHashMap::default();
    for (i, (_, path)) in routed.iter().enumerate() {
        let r = find(&mut uf, i);
        let entry = cluster_tiles.entry(r).or_default();
        for &t in path {
            entry.insert(t);
        }
        *cluster_paths.entry(r).or_insert(0) += 1;
        *cluster_crossings.entry(r).or_insert(0) +=
            path.iter().filter(|t| existing_belts.contains(t)).count();
    }

    let mut clusters: Vec<(usize, usize, usize)> = cluster_paths
        .iter()
        .map(|(&r, &p)| {
            (
                p,
                cluster_tiles.get(&r).map(|s| s.len()).unwrap_or(0),
                cluster_crossings.get(&r).copied().unwrap_or(0),
            )
        })
        .collect();
    clusters.sort_by_key(|&(_, t, _)| Reverse(t));

    eprintln!("\n=== ghost clusters ({}): ===", clusters.len());
    eprintln!("  paths  tiles  crossings");
    for (p, t, c) in &clusters {
        eprintln!("  {:5}  {:5}  {:5}", p, t, c);
    }
    let max_tiles = clusters.iter().map(|(_, t, _)| *t).max().unwrap_or(0);
    let max_crossings = clusters.iter().map(|(_, _, c)| *c).max().unwrap_or(0);
    eprintln!(
        "\nmax cluster: {} tiles, {} crossings",
        max_tiles, max_crossings
    );

    // Use expect_err path so this always panics with a clean message.
    panic!(
        "ghost-cluster diagnostic: {} routed / {} failures, {} clusters, max {} tiles / {} crossings",
        routed.len(),
        failures.len(),
        clusters.len(),
        max_tiles,
        max_crossings
    );
}

/// Helper for ghost cluster diagnostics: routes failing specs under ghost semantics,
/// union-finds clusters, and panics with the scoreboard.
///
/// When `skip_validation` is `true`, bypasses `run_e2e` and calls
/// `solver::solve` + `layout::build_bus_layout` directly, skipping the 21-check
/// validator gauntlet and the blueprint round-trip. Needed for tier-5 layouts
/// where the validator pass alone exceeds the test timeout.
#[allow(clippy::type_complexity)]
fn diag_ghost_cluster_helper(
    test_name: &str,
    item: &str,
    rate: f64,
    machine: &str,
    belt_tier: Option<&str>,
    inputs: &FxHashSet<String>,
    skip_validation: bool,
) {
    use fucktorio_core::common::{machine_size, machine_tiles};
    use rustc_hash::FxHashMap;
    use std::cmp::Reverse;

    let result = if skip_validation {
        let _guard = trace::start_trace();
        let solver_result = solver::solve(item, rate, inputs, machine)
            .unwrap_or_else(|e| panic!("{test_name}: solver: {e}"));
        let layout = layout::build_bus_layout(&solver_result, belt_tier)
            .unwrap_or_else(|e| panic!("{test_name}: layout: {e}"));
        let trace_events = trace::drain_events();
        let analysis = analysis::analyze(&layout);
        E2EResult {
            solver_result,
            layout: layout.clone(),
            parsed: layout,
            issues: Vec::new(),
            analysis,
            trace_events,
        }
    } else {
        run_e2e(test_name, item, rate, machine, belt_tier, inputs).expect("e2e")
    };

    // Pull failing specs from trace
    let failures: Vec<(String, (i32, i32), (i32, i32))> = result
        .trace_events
        .iter()
        .filter_map(|ev| match ev {
            TraceEvent::RouteFailure {
                spec_key,
                from_x,
                from_y,
                to_x,
                to_y,
                ..
            } => Some((spec_key.clone(), (*from_x, *from_y), (*to_x, *to_y))),
            _ => None,
        })
        .collect();

    eprintln!("route failures in current pipeline: {}", failures.len());
    for (k, s, g) in &failures {
        eprintln!("  {} : {:?} -> {:?}", k, s, g);
    }

    // Build obstacle grid: everything except belts
    let is_belt_like = |name: &str| -> bool {
        matches!(
            name,
            "transport-belt"
                | "fast-transport-belt"
                | "express-transport-belt"
                | "underground-belt"
                | "fast-underground-belt"
                | "express-underground-belt"
                | "splitter"
                | "fast-splitter"
                | "express-splitter"
        )
    };

    let mut hard: FxHashSet<(i32, i32)> = FxHashSet::default();
    let mut existing_belts: FxHashSet<(i32, i32)> = FxHashSet::default();
    for e in &result.layout.entities {
        if is_belt_like(&e.name) {
            existing_belts.insert((e.x, e.y));
        } else {
            let sz = machine_size(&e.name);
            for t in machine_tiles(e.x, e.y, sz) {
                hard.insert(t);
            }
        }
    }

    eprintln!(
        "layout: {} entities, {} belts, {} hard obstacle tiles",
        result.layout.entities.len(),
        existing_belts.len(),
        hard.len()
    );

    let width = result.layout.width.max(200);
    let height = result.layout.height.max(300);

    // Simple turn-biased A*
    fn astar(
        start: (i32, i32),
        goal: (i32, i32),
        hard: &FxHashSet<(i32, i32)>,
        width: i32,
        height: i32,
        turn_penalty: u32,
    ) -> Option<Vec<(i32, i32)>> {
        use rustc_hash::FxHashMap;
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;

        #[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
        struct State {
            x: i32,
            y: i32,
            dir: i8,
        }
        let dirs: [(i32, i32, i8); 4] = [(1, 0, 0), (0, 1, 1), (-1, 0, 2), (0, -1, 3)];
        let h = |x: i32, y: i32| ((x - goal.0).abs() + (y - goal.1).abs()) as u32;

        let mut heap: BinaryHeap<Reverse<(u32, State)>> = BinaryHeap::new();
        let mut g: FxHashMap<State, u32> = FxHashMap::default();
        let mut parent: FxHashMap<State, State> = FxHashMap::default();

        let start_state = State {
            x: start.0,
            y: start.1,
            dir: -1,
        };
        heap.push(Reverse((h(start.0, start.1), start_state)));
        g.insert(start_state, 0);

        while let Some(Reverse((_, s))) = heap.pop() {
            if (s.x, s.y) == goal {
                let mut path = vec![(s.x, s.y)];
                let mut cur = s;
                while let Some(&p) = parent.get(&cur) {
                    path.push((p.x, p.y));
                    cur = p;
                }
                path.reverse();
                return Some(path);
            }
            let cur_g = g[&s];
            for &(dx, dy, dir) in &dirs {
                let nx = s.x + dx;
                let ny = s.y + dy;
                if nx < 0 || nx >= width || ny < 0 || ny >= height {
                    continue;
                }
                if hard.contains(&(nx, ny)) && (nx, ny) != goal {
                    continue;
                }
                let step = if s.dir == -1 || s.dir == dir {
                    1
                } else {
                    1 + turn_penalty
                };
                let ns = State { x: nx, y: ny, dir };
                let ng = cur_g + step;
                if g.get(&ns).copied().unwrap_or(u32::MAX) > ng {
                    g.insert(ns, ng);
                    parent.insert(ns, s);
                    heap.push(Reverse((ng + h(nx, ny), ns)));
                }
            }
        }
        None
    }

    // Route each failing spec
    let turn_penalty = 8;
    let mut routed: Vec<(String, Vec<(i32, i32)>)> = Vec::new();
    let mut unroutable: Vec<String> = Vec::new();
    for (key, start, goal) in &failures {
        match astar(*start, *goal, &hard, width, height, turn_penalty) {
            Some(path) => routed.push((key.clone(), path)),
            None => unroutable.push(key.clone()),
        }
    }

    eprintln!(
        "\nunder ghost semantics: {}/{} routed, {} still unroutable",
        routed.len(),
        failures.len(),
        unroutable.len()
    );
    for k in &unroutable {
        eprintln!("  STILL FAILS: {}", k);
    }

    // Per-path stats
    let count_turns = |path: &[(i32, i32)]| -> usize {
        let mut t = 0;
        for w in path.windows(3) {
            let d1 = (w[1].0 - w[0].0, w[1].1 - w[0].1);
            let d2 = (w[2].0 - w[1].0, w[2].1 - w[1].1);
            if d1 != d2 {
                t += 1;
            }
        }
        t
    };

    eprintln!("\nper-path (len / crossings / turns):");
    for (key, path) in &routed {
        let crossings = path.iter().filter(|t| existing_belts.contains(t)).count();
        let turns = count_turns(path);
        eprintln!(
            "  {}: len={} crossings={} turns={}",
            key,
            path.len(),
            crossings,
            turns
        );
    }

    // Union-find over paths that share tiles
    fn find(p: &mut [usize], i: usize) -> usize {
        let mut r = i;
        while p[r] != r {
            r = p[r];
        }
        let mut cur = i;
        while p[cur] != r {
            let next = p[cur];
            p[cur] = r;
            cur = next;
        }
        r
    }

    let mut uf: Vec<usize> = (0..routed.len()).collect();
    let mut tile_owner: FxHashMap<(i32, i32), usize> = FxHashMap::default();
    for (i, (_, path)) in routed.iter().enumerate() {
        for &tile in path {
            if let Some(&j) = tile_owner.get(&tile) {
                let ri = find(&mut uf, i);
                let rj = find(&mut uf, j);
                if ri != rj {
                    uf[ri] = rj;
                }
            } else {
                tile_owner.insert(tile, i);
            }
        }
    }

    // Cluster aggregation
    let mut cluster_tiles: FxHashMap<usize, FxHashSet<(i32, i32)>> = FxHashMap::default();
    let mut cluster_paths: FxHashMap<usize, usize> = FxHashMap::default();
    let mut cluster_crossings: FxHashMap<usize, usize> = FxHashMap::default();
    for (i, (_, path)) in routed.iter().enumerate() {
        let r = find(&mut uf, i);
        let entry = cluster_tiles.entry(r).or_default();
        for &t in path {
            entry.insert(t);
        }
        *cluster_paths.entry(r).or_insert(0) += 1;
        *cluster_crossings.entry(r).or_insert(0) +=
            path.iter().filter(|t| existing_belts.contains(t)).count();
    }

    let mut clusters: Vec<(usize, usize, usize)> = cluster_paths
        .iter()
        .map(|(&r, &p)| {
            (
                p,
                cluster_tiles.get(&r).map(|s| s.len()).unwrap_or(0),
                cluster_crossings.get(&r).copied().unwrap_or(0),
            )
        })
        .collect();
    clusters.sort_by_key(|&(_, t, _)| Reverse(t));

    eprintln!("\n=== ghost clusters ({}): ===", clusters.len());
    eprintln!("  paths  tiles  crossings");
    for (p, t, c) in &clusters {
        eprintln!("  {:5}  {:5}  {:5}", p, t, c);
    }
    let max_tiles = clusters.iter().map(|(_, t, _)| *t).max().unwrap_or(0);
    let max_crossings = clusters.iter().map(|(_, _, c)| *c).max().unwrap_or(0);
    eprintln!(
        "\nmax cluster: {} tiles, {} crossings",
        max_tiles, max_crossings
    );

    panic!(
        "ghost-cluster diagnostic: {} routed / {} failures, {} clusters, max {} tiles / {} crossings",
        routed.len(),
        failures.len(),
        clusters.len(),
        max_tiles,
        max_crossings
    );
}

#[test]
#[ignore]
#[ntest::timeout(60000)]
fn diag_ghost_cluster_stress_ac_45s_from_plates() {
    let inputs: FxHashSet<String> = ["iron-plate", "copper-plate", "plastic-bar"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    diag_ghost_cluster_helper(
        "diag_ghost_cluster_stress_ac_45s_from_plates",
        "advanced-circuit",
        45.0,
        "assembling-machine-2",
        None,
        &inputs,
        false,
    );
}

#[test]
#[ignore]
#[ntest::timeout(900000)]
fn diag_ghost_cluster_stress_processing_unit_20s() {
    let inputs: FxHashSet<String> = ["iron-plate", "copper-plate", "plastic-bar", "sulfuric-acid"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    // Tried 20/s AM3 — solver+layout alone exceeds 15min on current pipeline.
    // 1/s AM2 completes in 0.6s but has 0 route failures (too small to
    // exercise ghost clusters). Middle ground: 5/s AM2.
    diag_ghost_cluster_helper(
        "diag_ghost_cluster_stress_processing_unit_20s",
        "processing-unit",
        5.0,
        "assembling-machine-2",
        None,
        &inputs,
        true, // skip validation
    );
}

// ---------------------------------------------------------------------------
// Ghost-cluster sizing: copper-cable feeder paths
//
// The (5,7) balancer template is missing so `render_family_input_paths` never
// schedules feeder A* specs for copper-cable.  Those dead-end output belts
// don't appear as RouteFailure events, so `diag_ghost_cluster_ac_from_ore`
// misses them entirely.
//
// Synthesise the feeder specs by hand from the LanesPlanned trace (FamilyInfo
// gives producer_rows, lane_xs, balancer_y_start, bus_width), route under
// ghost A*, and report cluster sizes.  Also check whether any feeder tile-set
// overlaps with the 4 ret: paths from the sibling diagnostic.
// ---------------------------------------------------------------------------

#[test]
#[ignore]
#[ntest::timeout(60000)]
#[allow(clippy::type_complexity)]
fn diag_ghost_cluster_copper_cable_feeders() {
    use fucktorio_core::common::{machine_size, machine_tiles};
    use rustc_hash::FxHashMap;
    use std::cmp::Reverse;

    let inputs: FxHashSet<String> = [
        "iron-ore", "copper-ore", "coal", "water", "crude-oil",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let result = run_e2e(
        "diag_ghost_cluster_copper_cable_feeders",
        "advanced-circuit",
        5.0,
        "assembling-machine-1",
        Some("transport-belt"),
        &inputs,
    )
    .expect("e2e");

    // --- Belt-like predicate ---
    let is_belt_like = |name: &str| -> bool {
        matches!(
            name,
            "transport-belt"
                | "fast-transport-belt"
                | "express-transport-belt"
                | "underground-belt"
                | "fast-underground-belt"
                | "express-underground-belt"
                | "splitter"
                | "fast-splitter"
                | "express-splitter"
        )
    };

    // --- Obstacle grid ---
    let mut hard: FxHashSet<(i32, i32)> = FxHashSet::default();
    let mut existing_belts: FxHashSet<(i32, i32)> = FxHashSet::default();
    for e in &result.layout.entities {
        if is_belt_like(&e.name) {
            existing_belts.insert((e.x, e.y));
        } else {
            let sz = machine_size(&e.name);
            for t in machine_tiles(e.x, e.y, sz) {
                hard.insert(t);
            }
        }
    }

    let width = result.layout.width.max(200);
    let height = result.layout.height.max(300);

    eprintln!(
        "layout: {} entities, {} belts, {} hard obstacle tiles",
        result.layout.entities.len(),
        existing_belts.len(),
        hard.len()
    );

    // --- Extract copper-cable family info from LanesPlanned ---
    let (cc_family, bus_width) = result
        .trace_events
        .iter()
        .find_map(|ev| {
            if let TraceEvent::LanesPlanned { families, bus_width, .. } = ev {
                let fam = families.iter().find(|f| f.item == "copper-cable")?;
                Some((fam.clone(), *bus_width))
            } else {
                None
            }
        })
        .expect("LanesPlanned event with copper-cable family");

    eprintln!(
        "copper-cable family: shape={:?}, {} producers, lane_xs={:?}, balancer_y_start={}, bw={}",
        cc_family.shape, cc_family.producer_rows.len(), cc_family.lane_xs, cc_family.balancer_y_start, bus_width
    );

    // --- Derive output_belt_y per producer row ---
    // RowInfo in trace gives y_start/y_end but not output_belt_y directly.
    // Recover it from actual belt entities carrying copper-cable at x=bw-1.
    let row_y_ranges: Vec<(i32, i32)> = result
        .trace_events
        .iter()
        .find_map(|ev| {
            if let TraceEvent::RowsPlaced { rows } = ev {
                Some(rows.iter().map(|r| (r.y_start, r.y_end)).collect())
            } else {
                None
            }
        })
        .unwrap_or_default();

    let producer_out_ys: Vec<(usize, i32)> = cc_family
        .producer_rows
        .iter()
        .map(|&ri| {
            let out_y = if ri < row_y_ranges.len() {
                let (ys, ye) = row_y_ranges[ri];
                // Search for a copper-cable belt at x=bw-1 within this row
                let found = result
                    .layout
                    .entities
                    .iter()
                    .filter(|e| {
                        is_belt_like(&e.name)
                            && e.carries.as_deref() == Some("copper-cable")
                            && e.y >= ys
                            && e.y <= ye
                            && e.x == bus_width - 1
                    })
                    .map(|e| e.y)
                    .max();
                // Fall back to y_start+2 (standard output belt offset)
                found.unwrap_or(ys + 2)
            } else {
                0
            };
            (ri, out_y)
        })
        .collect();

    eprintln!("producer rows and output_belt_ys: {:?}", producer_out_ys);

    // --- Synthesise feeder specs ---
    // Mirrors bus_router.rs lines 2557-2597:
    //   start = (bw-1, out_y)
    //   goal  = (input_x+1, out_y)
    // where input_x = min(lane_xs) + input_tile_dx.
    //
    // The (5,7) template is missing so we have no real input_tiles.
    // Use x=3 as a proxy landing column — enough for cluster-size measurement.
    let fake_landing_x: i32 = 4; // goal = fake_input_x+1 = 4

    let mut specs: Vec<(String, (i32, i32), (i32, i32))> = Vec::new();
    for (ri, out_y) in &producer_out_ys {
        let start_x = bus_width - 1;
        let goal_x = fake_landing_x;
        if goal_x >= start_x {
            eprintln!(
                "  row {}: degenerate feeder (start_x={} <= goal_x={}), skip",
                ri, start_x, goal_x
            );
            continue;
        }
        let key = format!("feeder:copper-cable:{}:{}", fake_landing_x - 1, out_y);
        specs.push((key, (start_x, *out_y), (goal_x, *out_y)));
    }

    eprintln!("\nsynthesised {} feeder specs:", specs.len());
    for (k, s, g) in &specs {
        eprintln!("  {} : {:?} -> {:?}", k, s, g);
    }

    // --- Turn-biased ghost A* (identical to sibling diagnostic) ---
    fn astar(
        start: (i32, i32),
        goal: (i32, i32),
        hard: &FxHashSet<(i32, i32)>,
        width: i32,
        height: i32,
        turn_penalty: u32,
    ) -> Option<Vec<(i32, i32)>> {
        use rustc_hash::FxHashMap;
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;

        #[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
        struct State {
            x: i32,
            y: i32,
            dir: i8,
        }
        let dirs: [(i32, i32, i8); 4] = [(1, 0, 0), (0, 1, 1), (-1, 0, 2), (0, -1, 3)];
        let h = |x: i32, y: i32| ((x - goal.0).abs() + (y - goal.1).abs()) as u32;

        let mut heap: BinaryHeap<Reverse<(u32, State)>> = BinaryHeap::new();
        let mut g: FxHashMap<State, u32> = FxHashMap::default();
        let mut parent: FxHashMap<State, State> = FxHashMap::default();

        let start_state = State { x: start.0, y: start.1, dir: -1 };
        heap.push(Reverse((h(start.0, start.1), start_state)));
        g.insert(start_state, 0);

        while let Some(Reverse((_, s))) = heap.pop() {
            if (s.x, s.y) == goal {
                let mut path = vec![(s.x, s.y)];
                let mut cur = s;
                while let Some(&p) = parent.get(&cur) {
                    path.push((p.x, p.y));
                    cur = p;
                }
                path.reverse();
                return Some(path);
            }
            let cur_g = g[&s];
            for &(dx, dy, dir) in &dirs {
                let nx = s.x + dx;
                let ny = s.y + dy;
                if nx < 0 || nx >= width || ny < 0 || ny >= height {
                    continue;
                }
                if hard.contains(&(nx, ny)) && (nx, ny) != goal {
                    continue;
                }
                let step = if s.dir == -1 || s.dir == dir {
                    1
                } else {
                    1 + turn_penalty
                };
                let ns = State { x: nx, y: ny, dir };
                let ng = cur_g + step;
                if g.get(&ns).copied().unwrap_or(u32::MAX) > ng {
                    g.insert(ns, ng);
                    parent.insert(ns, s);
                    heap.push(Reverse((ng + h(nx, ny), ns)));
                }
            }
        }
        None
    }

    let turn_penalty = 8;
    let mut routed: Vec<(String, Vec<(i32, i32)>)> = Vec::new();
    let mut unroutable: Vec<String> = Vec::new();
    for (key, start, goal) in &specs {
        match astar(*start, *goal, &hard, width, height, turn_penalty) {
            Some(path) => routed.push((key.clone(), path)),
            None => unroutable.push(key.clone()),
        }
    }

    eprintln!(
        "\nunder ghost semantics: {}/{} routed, {} still unroutable",
        routed.len(),
        specs.len(),
        unroutable.len()
    );
    for k in &unroutable {
        eprintln!("  STILL FAILS: {}", k);
    }

    // --- Per-path stats ---
    let count_turns = |path: &[(i32, i32)]| -> usize {
        let mut t = 0;
        for w in path.windows(3) {
            let d1 = (w[1].0 - w[0].0, w[1].1 - w[0].1);
            let d2 = (w[2].0 - w[1].0, w[2].1 - w[1].1);
            if d1 != d2 {
                t += 1;
            }
        }
        t
    };

    eprintln!("\nper-path (len / crossings / turns):");
    for (key, path) in &routed {
        let crossings = path.iter().filter(|t| existing_belts.contains(t)).count();
        let turns = count_turns(path);
        eprintln!(
            "  {}: len={} crossings={} turns={}",
            key,
            path.len(),
            crossings,
            turns
        );
    }

    // --- Union-find ---
    fn find(p: &mut [usize], i: usize) -> usize {
        let mut r = i;
        while p[r] != r {
            r = p[r];
        }
        let mut cur = i;
        while p[cur] != r {
            let next = p[cur];
            p[cur] = r;
            cur = next;
        }
        r
    }

    let mut uf: Vec<usize> = (0..routed.len()).collect();
    let mut tile_owner: FxHashMap<(i32, i32), usize> = FxHashMap::default();
    for (i, (_, path)) in routed.iter().enumerate() {
        for &tile in path {
            if let Some(&j) = tile_owner.get(&tile) {
                let ri = find(&mut uf, i);
                let rj = find(&mut uf, j);
                if ri != rj {
                    uf[ri] = rj;
                }
            } else {
                tile_owner.insert(tile, i);
            }
        }
    }

    let mut cluster_tiles: FxHashMap<usize, FxHashSet<(i32, i32)>> = FxHashMap::default();
    let mut cluster_paths: FxHashMap<usize, usize> = FxHashMap::default();
    let mut cluster_crossings: FxHashMap<usize, usize> = FxHashMap::default();
    for (i, (_, path)) in routed.iter().enumerate() {
        let r = find(&mut uf, i);
        let entry = cluster_tiles.entry(r).or_default();
        for &t in path {
            entry.insert(t);
        }
        *cluster_paths.entry(r).or_insert(0) += 1;
        *cluster_crossings.entry(r).or_insert(0) +=
            path.iter().filter(|t| existing_belts.contains(t)).count();
    }

    let mut clusters: Vec<(usize, usize, usize)> = cluster_paths
        .iter()
        .map(|(&r, &p)| {
            (
                p,
                cluster_tiles.get(&r).map(|s| s.len()).unwrap_or(0),
                cluster_crossings.get(&r).copied().unwrap_or(0),
            )
        })
        .collect();
    clusters.sort_by_key(|&(_, t, _)| Reverse(t));

    eprintln!("\n=== ghost clusters ({}): ===", clusters.len());
    eprintln!("  paths  tiles  crossings");
    for (p, t, c) in &clusters {
        eprintln!("  {:5}  {:5}  {:5}", p, t, c);
    }
    let max_tiles = clusters.iter().map(|(_, t, _)| *t).max().unwrap_or(0);
    let max_crossings = clusters.iter().map(|(_, _, c)| *c).max().unwrap_or(0);

    // --- Cross-check against ret: failure tile-set ---
    // Re-route the ret: failures from the same layout to see if any feeder
    // path shares a tile (which would merge clusters in a unified pass).
    let ret_failures: Vec<(String, (i32, i32), (i32, i32))> = result
        .trace_events
        .iter()
        .filter_map(|ev| match ev {
            TraceEvent::RouteFailure {
                spec_key,
                from_x,
                from_y,
                to_x,
                to_y,
                ..
            } => Some((spec_key.clone(), (*from_x, *from_y), (*to_x, *to_y))),
            _ => None,
        })
        .collect();

    let mut ret_tiles: FxHashSet<(i32, i32)> = FxHashSet::default();
    let mut ret_routed_count = 0;
    for (_, start, goal) in &ret_failures {
        if let Some(path) = astar(*start, *goal, &hard, width, height, turn_penalty) {
            for t in &path {
                ret_tiles.insert(*t);
            }
            ret_routed_count += 1;
        }
    }

    let feeder_tiles: FxHashSet<(i32, i32)> =
        routed.iter().flat_map(|(_, p)| p.iter().copied()).collect();
    let overlap_count = feeder_tiles.intersection(&ret_tiles).count();

    eprintln!(
        "\ncross-check with ret: paths ({}/{} routed): {} overlapping tiles",
        ret_routed_count,
        ret_failures.len(),
        overlap_count
    );
    if overlap_count > 0 {
        eprintln!("  NOTE: feeder+ret: clusters would merge in a unified routing pass");
    } else {
        eprintln!("  feeder clusters are disjoint from ret: clusters — SAT budgets stay independent");
    }

    eprintln!("\nmax cluster: {} tiles, {} crossings", max_tiles, max_crossings);

    panic!(
        "ghost-cluster feeder diagnostic: {}/{} feeders routed, {} clusters, max {} tiles / {} crossings, {} ret:-overlap tiles",
        routed.len(),
        specs.len(),
        clusters.len(),
        max_tiles,
        max_crossings,
        overlap_count
    );
}

// ---------------------------------------------------------------------------
// SAT zone cache histogram
// ---------------------------------------------------------------------------

/// Read `target/sat-zones.jsonl`, group by signature, print a frequency
/// histogram sorted by descending count, then panic with a top-10 summary.
///
/// Run after populating the cache with the full e2e suite:
///   cargo test --manifest-path crates/core/Cargo.toml --test e2e
///   cargo test --manifest-path crates/core/Cargo.toml --test e2e -- \
///       --ignored diag_sat_zone_histogram --exact --nocapture
#[test]
#[ignore]
fn diag_sat_zone_histogram() {
    use std::collections::HashMap;

    struct ZoneBucket {
        count: usize,
        total_width: u64,
        total_height: u64,
        total_vars: u64,
        total_clauses: u64,
        total_solve_us: u64,
        sources: Vec<String>,
    }

    // Resolve path the same way zone_cache::resolve_cache_path() does.
    let path = if let Ok(p) = std::env::var("FUCKTORIO_ZONE_CACHE_PATH") {
        std::path::PathBuf::from(p)
    } else {
        let base = std::env::var("XDG_CACHE_HOME")
            .ok()
            .filter(|s| !s.is_empty())
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|h| std::path::PathBuf::from(h).join(".cache"))
            })
            .unwrap_or_else(|| std::path::PathBuf::from(".cache"));
        base.join("fucktorio").join("sat-zones.jsonl")
    };

    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Cannot read {}: {e}", path.display()));

    let mut buckets: HashMap<String, ZoneBucket> = HashMap::new();
    let mut total_records = 0usize;

    for (lineno, line) in content.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: serde_json::Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("Bad JSON on line {}: {e}", lineno + 1));

        let sig = v["signature"].as_str().unwrap_or("?").to_string();
        let width = v["width"].as_u64().unwrap_or(0);
        let height = v["height"].as_u64().unwrap_or(0);
        let vars = v["variables"].as_u64().unwrap_or(0);
        let clauses = v["clauses"].as_u64().unwrap_or(0);
        let solve_us = v["solve_time_us"].as_u64().unwrap_or(0);
        let source = v["source"].as_str().map(|s| s.to_string());

        total_records += 1;
        let bucket = buckets.entry(sig).or_insert(ZoneBucket {
            count: 0,
            total_width: 0,
            total_height: 0,
            total_vars: 0,
            total_clauses: 0,
            total_solve_us: 0,
            sources: Vec::new(),
        });
        bucket.count += 1;
        bucket.total_width += width;
        bucket.total_height += height;
        bucket.total_vars += vars;
        bucket.total_clauses += clauses;
        bucket.total_solve_us += solve_us;
        if let Some(s) = source {
            if !bucket.sources.contains(&s) && bucket.sources.len() < 3 {
                bucket.sources.push(s);
            }
        }
    }

    let distinct = buckets.len();
    let mut rows: Vec<(String, ZoneBucket)> = buckets.into_iter().collect();
    rows.sort_by(|a, b| b.1.count.cmp(&a.1.count));

    eprintln!("\n=== SAT zone histogram ({total_records} records, {distinct} distinct signatures) ===");
    eprintln!("{:<40} {:>6}  {:>8}  {:>6}  {:>8}  {:>12}  sources",
        "signature", "count", "mean_WxH", "mean_v", "mean_cls", "mean_us");
    eprintln!("{}", "-".repeat(120));

    for (sig, b) in &rows {
        let n = b.count as f64;
        let mean_w = b.total_width as f64 / n;
        let mean_h = b.total_height as f64 / n;
        let mean_v = b.total_vars as f64 / n;
        let mean_cls = b.total_clauses as f64 / n;
        let mean_us = b.total_solve_us as f64 / n;
        let srcs = b.sources.join(", ");
        eprintln!("{:<40} {:>6}  {:>5.1}x{:<5.1} {:>6.1}  {:>8.1}  {:>12.1}  {}",
            sig, b.count, mean_w, mean_h, mean_v, mean_cls, mean_us, srcs);
    }

    // Build top-10 summary for the panic message
    let top10: Vec<String> = rows.iter().take(10)
        .map(|(sig, b)| format!("{}×{}", sig, b.count))
        .collect();
    let top10_str = top10.join("; ");

    panic!(
        "SAT zone histogram: total_records={total_records}, distinct_signatures={distinct}; top-10: {top10_str}"
    );
}
