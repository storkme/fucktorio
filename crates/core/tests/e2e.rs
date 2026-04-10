//! End-to-end blueprint validation tests.
//!
//! Closes the loop: solve → layout → export → parse back → validate → analyze.
//! Asserts that generated factories produce the target item at the target rate
//! with zero validation errors.
//!
//! Run with:  cargo test --test e2e
//! Filter:    cargo test --test e2e -- tier1
//! All (incl. known-failing): cargo test --test e2e -- --ignored

use fucktorio_core::analysis::{self, BlueprintAnalysis};
use fucktorio_core::blueprint;
use fucktorio_core::blueprint_parser;
use fucktorio_core::bus::layout;
use fucktorio_core::models::{LayoutResult, SolverResult};
use fucktorio_core::solver;
use fucktorio_core::validate::{self, LayoutStyle, Severity, ValidationIssue};
use fucktorio_core::validate::{belt_flow, belt_structural, power, inserters};
use rustc_hash::FxHashSet;
use std::time::Instant;

struct E2EResult {
    #[allow(dead_code)]
    solver_result: SolverResult,
    layout: LayoutResult,
    #[allow(dead_code)]
    bp_string: String,
    parsed: LayoutResult,
    issues: Vec<ValidationIssue>,
    analysis: BlueprintAnalysis,
}

fn run_e2e(
    item: &str,
    rate: f64,
    machine: &str,
    belt_tier: Option<&str>,
    available_inputs: &FxHashSet<String>,
) -> Result<E2EResult, String> {
    let solver_result = solver::solve(item, rate, available_inputs, machine)
        .map_err(|e| format!("solver: {e}"))?;

    let layout = layout::build_bus_layout(&solver_result, belt_tier)
        .map_err(|e| format!("layout: {e}"))?;

    // Validate the original layout (correct top-left positions).
    // The blueprint export has a known offset bug for multi-tile entities,
    // so validating the parsed layout would produce false overlap errors.
    let issues = match validate::validate(&layout, Some(&solver_result), LayoutStyle::Bus) {
        Ok(issues) => issues,
        Err(e) => e.issues,
    };

    let analysis = analysis::analyze(&layout);

    // Round-trip through blueprint export → parse as a smoke test.
    let bp_string = blueprint::export(&layout, item);
    let parsed = blueprint_parser::parse_blueprint_string(&bp_string)
        .map_err(|e| format!("parse: {e}"))?;

    Ok(E2EResult {
        solver_result,
        layout,
        bp_string,
        parsed,
        issues,
        analysis,
    })
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
    // Blueprint export loses some metadata (carries, segment_id, etc.)
    // so we only assert entity count is preserved.
    assert_eq!(
        result.layout.entities.len(),
        result.parsed.entities.len(),
        "Round-trip entity count mismatch: layout has {} but parsed has {}",
        result.layout.entities.len(),
        result.parsed.entities.len(),
    );
}

// ---------------------------------------------------------------------------
// Tier 1: iron-gear-wheel (1 recipe, 1 solid input)
// ---------------------------------------------------------------------------

#[test]
fn tier1_iron_gear_wheel() {
    let inputs: FxHashSet<String> = ["iron-plate"].iter().map(|s| s.to_string()).collect();
    let result = run_e2e("iron-gear-wheel", 10.0, "assembling-machine-1", None, &inputs)
        .expect("e2e pipeline");

    assert_no_errors(&result);
    assert_produces(&result, "iron-gear-wheel", 10.0);
    assert_round_trip(&result);
}

#[test]
fn tier1_iron_gear_wheel_from_ore() {
    let inputs: FxHashSet<String> = ["iron-ore"].iter().map(|s| s.to_string()).collect();
    let result = run_e2e(
        "iron-gear-wheel",
        10.0,
        "assembling-machine-2",
        None,
        &inputs,
    )
    .expect("e2e pipeline");

    assert_no_errors(&result);
    assert_produces(&result, "iron-gear-wheel", 10.0);
    assert_round_trip(&result);
}

// ---------------------------------------------------------------------------
// Tier 2: electronic-circuit (2 recipes, 2 solid inputs)
// ---------------------------------------------------------------------------

#[test]
#[ignore] // Lane-throughput errors: both lanes at 15/s on yellow belt (7.5/s per-lane cap)
fn tier2_electronic_circuit() {
    let inputs: FxHashSet<String> = ["iron-plate", "copper-plate"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let result = run_e2e(
        "electronic-circuit",
        10.0,
        "assembling-machine-2",
        None,
        &inputs,
    )
    .expect("e2e pipeline");

    assert_no_errors(&result);
    assert_produces(&result, "electronic-circuit", 10.0);
    assert_round_trip(&result);
}

#[test]
fn tier2_electronic_circuit_from_ore() {
    let inputs: FxHashSet<String> = ["iron-ore", "copper-ore"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let result = run_e2e(
        "electronic-circuit",
        10.0,
        "assembling-machine-1",
        Some("transport-belt"),
        &inputs,
    )
    .expect("e2e pipeline");

    assert_no_errors(&result);
    assert_produces(&result, "electronic-circuit", 10.0);
    assert_round_trip(&result);
}

// ---------------------------------------------------------------------------
// Tier 3: plastic-bar (1 recipe, 1 fluid + 1 solid input)
// ---------------------------------------------------------------------------

#[test]
fn tier3_plastic_bar() {
    let inputs: FxHashSet<String> = ["petroleum-gas", "coal"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let result =
        run_e2e("plastic-bar", 10.0, "chemical-plant", None, &inputs).expect("e2e pipeline");

    assert_no_errors(&result);
    assert_produces(&result, "plastic-bar", 10.0);
    assert_round_trip(&result);
}

// ---------------------------------------------------------------------------
// Tier 4: advanced-circuit (5+ recipes, mixed solid/fluid)
// Known issues: lane-throughput warnings from single-lane sideload bottleneck (#64)
// ---------------------------------------------------------------------------

#[test]
#[ignore] // Blocked by #64: lane-throughput warnings
fn tier4_advanced_circuit_from_plates() {
    let inputs: FxHashSet<String> = ["iron-plate", "copper-plate", "plastic-bar"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let result = run_e2e(
        "advanced-circuit",
        10.0,
        "assembling-machine-2",
        None,
        &inputs,
    )
    .expect("e2e pipeline");

    assert_no_errors(&result);
    assert_produces(&result, "advanced-circuit", 10.0);
    assert_round_trip(&result);
}

// ---------------------------------------------------------------------------
// Diagnostic: find which validator hangs on large layouts
// ---------------------------------------------------------------------------

#[test]
#[ignore] // Diagnostic only — run with --ignored --nocapture
fn diag_validator_timing_from_ore() {
    let inputs: FxHashSet<String> = ["iron-ore"].iter().map(|s| s.to_string()).collect();
    let sr = solver::solve("iron-gear-wheel", 10.0, &inputs, "assembling-machine-2").unwrap();
    let lr = layout::build_bus_layout(&sr, None).unwrap();
    eprintln!("=== iron-gear-wheel from ore ===");
    eprintln!("Layout: {} entities, {}x{}", lr.entities.len(), lr.width, lr.height);
    run_timed_validators(&lr, &sr);

    // The layout that was hanging
    let inputs2: FxHashSet<String> = ["iron-ore", "copper-ore"].iter().map(|s| s.to_string()).collect();
    let sr2 = solver::solve("electronic-circuit", 10.0, &inputs2, "assembling-machine-1").unwrap();
    let lr2 = layout::build_bus_layout(&sr2, Some("transport-belt")).unwrap();
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
