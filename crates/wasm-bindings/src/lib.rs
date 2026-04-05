// Build: wasm-pack build crates/wasm-bindings --target web --out-dir ../../web/src/wasm-pkg

use fucktorio_core::models::{LayoutResult, SolverResult};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn init() {
    console_error_panic_hook::set_once();
}

#[wasm_bindgen]
pub fn solve(
    _target_item: &str,
    _target_rate: f64,
    _available_inputs: Vec<String>,
    _machine_entity: &str,
) -> Result<SolverResult, JsError> {
    Ok(SolverResult {
        machines: vec![],
        external_inputs: vec![],
        external_outputs: vec![],
        dependency_order: vec![],
    })
}

#[wasm_bindgen]
pub fn all_producible_items() -> Vec<String> {
    vec![]
}

#[wasm_bindgen]
pub fn all_producer_machines() -> Vec<String> {
    vec![]
}

#[wasm_bindgen]
pub fn layout(_solver: SolverResult) -> Result<LayoutResult, JsError> {
    Ok(LayoutResult {
        entities: vec![],
        width: 0,
        height: 0,
    })
}

#[wasm_bindgen]
pub fn export_blueprint(_layout: LayoutResult, _label: String) -> String {
    String::new()
}
