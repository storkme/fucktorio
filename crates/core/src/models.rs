use serde::{Deserialize, Serialize};

/// An item flowing at a certain rate.
#[cfg_attr(feature = "wasm", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemFlow {
    pub item: String,
    pub rate: f64,
    pub is_fluid: bool,
}

/// One production step: which machine, which recipe, how many.
#[cfg_attr(feature = "wasm", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineSpec {
    pub entity: String,
    pub recipe: String,
    pub count: f64,
    pub inputs: Vec<ItemFlow>,
    pub outputs: Vec<ItemFlow>,
}

/// Everything the solver produces — no positional data.
#[cfg_attr(feature = "wasm", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolverResult {
    pub machines: Vec<MachineSpec>,
    pub external_inputs: Vec<ItemFlow>,
    pub external_outputs: Vec<ItemFlow>,
    pub dependency_order: Vec<String>,
}

/// Matches Factorio's 16-way direction constants (we only use 4).
#[cfg_attr(feature = "wasm", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
#[derive(Default)]
pub enum EntityDirection {
    #[default]
    North = 0,
    East = 4,
    South = 8,
    West = 12,
}


/// A module/item inserted into an entity (e.g. speed-module-3 × 2 in a beacon).
#[cfg_attr(feature = "wasm", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleItem {
    pub item: String,
    pub count: u32,
}

/// A single entity placed in the blueprint grid.
#[cfg_attr(feature = "wasm", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlacedEntity {
    pub name: String,
    #[serde(default)]
    pub x: i32,
    #[serde(default)]
    pub y: i32,
    #[serde(default)]
    pub direction: EntityDirection,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipe: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub io_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub carries: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mirror: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment_id: Option<String>,
    /// Throughput rate (items/s or fluid units/s) flowing through this entity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate: Option<f64>,
    /// Modules/items inserted into this entity (e.g. speed modules in a beacon).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<ModuleItem>,
}

/// Metadata about a SAT-solved region in the layout.
#[cfg_attr(feature = "wasm", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutRegion {
    pub kind: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub variables: u32,
    pub clauses: u32,
    pub solve_time_us: u64,
}

/// Everything the layout engine produces — no rate data.
#[cfg_attr(feature = "wasm", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LayoutResult {
    pub entities: Vec<PlacedEntity>,
    #[serde(default)]
    pub width: i32,
    #[serde(default)]
    pub height: i32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub regions: Vec<LayoutRegion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<Vec<crate::trace::TraceEvent>>,
}
