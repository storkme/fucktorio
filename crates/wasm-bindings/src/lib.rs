//! wasm-bindgen bindings for the Fucktorio pipeline.
//!
//! Thin wrapper around `fucktorio_core` that exposes the full pipeline to the
//! browser via WASM. Loaded by `web/src/engine.ts`.
//!
//! Build: `wasm-pack build crates/wasm-bindings --target web --out-dir ../../web/src/wasm-pkg`
//!
//! Exposed functions: `init`, `solve`, `layout`, `export_blueprint`, `validate`,
//! `get_all_items`, `get_recipes_for_item`, `parse_blueprint`.

use fucktorio_core::models::{LayoutResult, SolverResult};
use fucktorio_core::validate::{self, LayoutStyle, ValidationIssue};
use fucktorio_core::{blueprint, blueprint_parser, bus::layout::build_bus_layout, recipe_db, solver};
use rustc_hash::FxHashSet;
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
/// the structured `trace` events that drive the debug overlays. There
/// is no longer a separate "ghost" mode — the legacy direct router was
/// deleted and ghost routing is the only path. `layout_ghost` and
/// `layout_traced` both call `build_bus_layout_traced`; the alias is
/// kept for compatibility with `web/src/engine.ts` until the web app
/// is rebuilt.
#[wasm_bindgen]
pub fn layout_ghost(solver_result: SolverResult, max_belt_tier: Option<String>) -> Result<LayoutResult, JsError> {
    layout_traced(solver_result, max_belt_tier)
}

#[wasm_bindgen]
pub fn layout_traced(solver_result: SolverResult, max_belt_tier: Option<String>) -> Result<LayoutResult, JsError> {
    fucktorio_core::bus::layout::build_bus_layout_traced(&solver_result, max_belt_tier.as_deref())
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
