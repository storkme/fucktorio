// Build: wasm-pack build crates/wasm-bindings --target web --out-dir ../../web/src/wasm-pkg

use fucktorio_core::models::{LayoutResult, SolverResult};
use fucktorio_core::{blueprint, recipe_db, solver};
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
    let available: FxHashSet<String> = available_inputs.into_iter().collect();
    solver::solve(target_item, target_rate, &available, machine_entity)
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
pub fn layout(_solver: SolverResult) -> Result<LayoutResult, JsError> {
    // Layout engine not yet ported — see docs/port-plan.md (bus-* units).
    Ok(LayoutResult {
        entities: vec![],
        width: 0,
        height: 0,
    })
}

#[wasm_bindgen]
pub fn export_blueprint(layout: LayoutResult, label: String) -> String {
    blueprint::export(&layout, &label)
}
