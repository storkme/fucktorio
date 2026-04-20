//! wasm-bindgen bindings for the Fucktorio pipeline.
//!
//! Thin wrapper around `fucktorio_core` that exposes the full pipeline to the
//! browser via WASM. Loaded by `web/src/engine.ts`.
//!
//! Build: `wasm-pack build crates/wasm-bindings --target web --out-dir ../../web/src/wasm-pkg`
//!
//! Exposed functions: `init`, `solve`, `layout`, `export_blueprint`, `validate`,
//! `get_all_items`, `get_recipes_for_item`, `parse_blueprint`.

use fucktorio_core::models::{LayoutResult, PlacedEntity, SolverResult};
use fucktorio_core::validate::{self, LayoutStyle, ValidationIssue};
use fucktorio_core::{
    blueprint, blueprint_parser, bus::junction_cost::solution_cost,
    bus::layout::build_bus_layout, fixture as fixture_mod, recipe_db, sat, solver,
};
use rustc_hash::FxHashSet;
use serde::Serialize;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn init() {
    console_error_panic_hook::set_once();
}

#[wasm_bindgen]
pub fn solve(
    target_item: &str,
    target_rate: f64,
    available_inputs: Vec<String>,
    machine_entity: &str,
) -> Result<SolverResult, JsError> {
    let inputs: FxHashSet<String> = available_inputs.into_iter().collect();
    solver::solve(target_item, target_rate, &inputs, machine_entity)
        .map_err(|e| JsError::new(&e.to_string()))
}

#[wasm_bindgen]
pub fn all_producible_items() -> Vec<String> {
    recipe_db::all_producible_items()
}

#[wasm_bindgen]
pub fn all_producer_machines() -> Vec<String> {
    recipe_db::all_producer_machines()
}

#[wasm_bindgen]
pub fn default_machine_for_item(item: &str, fallback: &str) -> String {
    recipe_db::default_machine_for_item(item, fallback)
}

#[wasm_bindgen]
pub fn layout(solver_result: SolverResult, max_belt_tier: Option<String>) -> Result<LayoutResult, JsError> {
    build_bus_layout(&solver_result, max_belt_tier.as_deref()).map_err(|e| JsError::new(&e))
}

/// Traced variant of `layout()`. Returns the same `LayoutResult` plus
/// the structured `trace` events that drive the debug overlays. Ghost
/// routing is the only routing path — the legacy direct router was
/// deleted; both `layout()` and `layout_traced()` go through it.
#[wasm_bindgen]
pub fn layout_traced(solver_result: SolverResult, max_belt_tier: Option<String>) -> Result<LayoutResult, JsError> {
    fucktorio_core::bus::layout::build_bus_layout_traced(&solver_result, max_belt_tier.as_deref())
        .map_err(|e| JsError::new(&e))
}

/// Filter predicate — determines which TraceEvent variants are forwarded to
/// the streaming JS callback. The full event set remains collected in
/// `LayoutResult.trace` for post-hoc consumption.
///
/// v1 streams only `PhaseSnapshot` — enough for the renderer to progressively
/// commit entities as phases complete. Per-event overlays (SAT pulses, ghost
/// paths, etc.) are deferred to a follow-up using a shared-Graphics
/// draw-per-frame pattern; the naive "Graphics per event" approach saturated
/// Pixi's render tree on large layouts. Widening the filter is a one-line
/// change here when the renderer is ready.
fn streamable(evt: &fucktorio_core::trace::TraceEvent) -> bool {
    use fucktorio_core::trace::TraceEvent as T;
    matches!(evt, T::PhaseSnapshot { .. })
}

/// Streaming variant — invokes `emit` synchronously for every filtered trace
/// event during the layout run. The JS callback fires on the worker thread;
/// use it to `postMessage` events to the main thread as the engine emits
/// them. Returns the completed `LayoutResult` with the *full* (unfiltered)
/// `trace` populated, so callers that ignore streaming still get a usable
/// result identical to `layout_traced`.
#[wasm_bindgen]
pub fn layout_streaming(
    solver_result: SolverResult,
    max_belt_tier: Option<String>,
    emit: &js_sys::Function,
) -> Result<LayoutResult, JsError> {
    let emit = emit.clone();
    let on_event: Box<dyn FnMut(&fucktorio_core::trace::TraceEvent)> = Box::new(move |evt| {
        if !streamable(evt) {
            return;
        }
        if let Ok(js_evt) = serde_wasm_bindgen::to_value(evt) {
            let _ = emit.call1(&JsValue::NULL, &js_evt);
        }
    });
    fucktorio_core::bus::layout::build_bus_layout_streaming(
        &solver_result,
        max_belt_tier.as_deref(),
        on_event,
    )
    .map_err(|e| JsError::new(&e))
}

#[wasm_bindgen]
pub fn export_blueprint(layout_result: LayoutResult, label: String) -> String {
    blueprint::export(&layout_result, &label)
}

#[wasm_bindgen]
pub fn parse_blueprint(bp_string: &str) -> Result<LayoutResult, JsError> {
    blueprint_parser::parse_blueprint_string(bp_string).map_err(|e| JsError::new(&e))
}

/// Response shape for `solve_fixture`. Serialised via
/// `serde_wasm_bindgen` — the TS side mirrors this in `engine.ts`.
#[derive(Serialize)]
struct SolveFixtureResponse {
    entities: Vec<PlacedEntity>,
    cost: u32,
    stats: SolveFixtureStats,
}

#[derive(Serialize)]
struct SolveFixtureStats {
    variables: u32,
    clauses: u32,
    solve_time_us: u64,
    zone_width: u32,
    zone_height: u32,
}

/// Solve a SAT-zone fixture, optionally with a set of painted entities
/// pinned as assumptions. Returns `null` (JS) on UNSAT or invalid pins.
///
/// `fixture_json` — JSON matching the v1 fixture schema (see
/// `crates/core/src/fixture.rs`).
/// `pins_json` — JSON array of `PlacedEntity` to assume; `"[]"` for an
/// unconstrained solve.
///
/// Used by the F2 SAT-zone editor to (a) validate the user's painted
/// state and (b) render a ghost-completion overlay showing how SAT
/// would extend the paint.
#[wasm_bindgen]
pub fn solve_fixture(fixture_json: &str, pins_json: &str) -> Result<JsValue, JsError> {
    let fixture: fixture_mod::Fixture = serde_json::from_str(fixture_json)
        .map_err(|e| JsError::new(&format!("fixture parse: {e}")))?;
    let pins: Vec<PlacedEntity> = serde_json::from_str(pins_json)
        .map_err(|e| JsError::new(&format!("pins parse: {e}")))?;

    let zone = fixture_mod::build_zone(&fixture);
    let (result, stats) = sat::solve_crossing_zone_with_pins(
        &zone,
        &pins,
        fixture.max_reach,
        &fixture.belt_tier,
        None,
    );

    let Some(entities) = result else {
        return Ok(JsValue::NULL);
    };

    let response = SolveFixtureResponse {
        cost: solution_cost(&entities),
        entities,
        stats: SolveFixtureStats {
            variables: stats.variables,
            clauses: stats.clauses,
            solve_time_us: stats.solve_time_us,
            zone_width: stats.zone_width,
            zone_height: stats.zone_height,
        },
    };

    serde_wasm_bindgen::to_value(&response).map_err(|e| JsError::new(&e.to_string()))
}

#[wasm_bindgen]
pub fn validate_layout(
    layout_result: LayoutResult,
    solver_result: Option<SolverResult>,
    layout_style: Option<LayoutStyle>,
) -> Result<Vec<ValidationIssue>, JsError> {
    let style = layout_style.unwrap_or_default();
    let solver_ref: Option<&SolverResult> = solver_result.as_ref();
    validate::validate(&layout_result, solver_ref, style)
        .map_err(|e| JsError::new(&e.to_string()))
}
