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
use fucktorio_core::trace;
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
        trace::drain_events(),
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

    let result = E2EResult {
        solver_result,
        layout,
        parsed,
        issues,
        analysis,
    };

    // Dump snapshot if there are errors or if env var is set.
    let has_errors = result.issues.iter().any(|i| i.severity == Severity::Error);
    if has_errors || should_dump_snapshots() {
        dump_snapshot(test_name, &run_params, &result);
    }

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
    let warnings: Vec<_> = result
        .issues
        .iter()
        .filter(|i| i.severity == Severity::Warning)
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
#[ignore] // Layout warnings: belt-direction dead spots, copper-plate input-rate-delivery to assembler rows (rate propagation doesn't reach y=29), power network (31 disconnected poles)
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
    assert_no_warnings(&result);
    assert_produces(&result, "electronic-circuit", 10.0);
    assert_round_trip(&result);
}

#[test]
#[ignore] // Remaining errors: belt-dead-end, lane-throughput (6× on yellow belt).
          // NOTE: entity-overlap and belt-item-isolation were eliminated by the
          // dropped-bridge retry loop in build_bus_layout (see BridgeDropped trace).
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
    assert_no_warnings(&result);
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

    let checks: Vec<(&str, Box<dyn FnOnce() -> Vec<ValidationIssue>>)> = vec![
        ("power_coverage", Box::new(|| power::check_power_coverage(lr))),
        ("pole_network_connectivity", Box::new(|| power::check_pole_network_connectivity(lr))),
        ("inserter_chains", Box::new(|| inserters::check_inserter_chains(lr, Some(sr)))),
        ("inserter_direction", Box::new(|| inserters::check_inserter_direction(lr))),
        ("pipe_isolation", Box::new(|| validate::check_pipe_isolation(lr))),
        ("fluid_port_connectivity", Box::new(|| validate::check_fluid_port_connectivity(lr, LayoutStyle::Bus))),
        ("belt_connectivity", Box::new(|| belt_flow::check_belt_connectivity(lr, Some(sr)))),
        ("belt_flow_path", Box::new(|| belt_flow::check_belt_flow_path(lr, Some(sr), LayoutStyle::Bus))),
        ("belt_direction_continuity", Box::new(|| belt_flow::check_belt_direction_continuity(lr))),
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
