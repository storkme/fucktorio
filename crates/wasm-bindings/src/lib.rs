// Build: wasm-pack build crates/wasm-bindings --target web --out-dir ../../web/src/wasm-pkg

use fucktorio_core::models::{LayoutResult, SolverResult};
use fucktorio_core::{blueprint, bus::layout::build_bus_layout, recipe_db, solver};
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

#[wasm_bindgen]
pub fn export_blueprint(layout_result: LayoutResult, label: String) -> String {
    blueprint::export(&layout_result, &label)
}
