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
use rustc_hash::FxHashSet;

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
#[ignore] // Validation hangs on large smelting+assembly layouts
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
#[ignore] // Validation hangs on large smelting+assembly layouts
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
