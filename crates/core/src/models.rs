use serde::{Deserialize, Serialize};

/// An item flowing at a certain rate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemFlow {
    pub item: String,
    pub rate: f64,
    pub is_fluid: bool,
}

/// One production step: which machine, which recipe, how many.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineSpec {
    pub entity: String,
    pub recipe: String,
    pub count: f64,
    pub inputs: Vec<ItemFlow>,
    pub outputs: Vec<ItemFlow>,
}

/// Everything the solver produces — no positional data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolverResult {
    pub machines: Vec<MachineSpec>,
    pub external_inputs: Vec<ItemFlow>,
    pub external_outputs: Vec<ItemFlow>,
    pub dependency_order: Vec<String>,
}

/// Matches Factorio's 16-way direction constants (we only use 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum EntityDirection {
    North = 0,
    East = 4,
    South = 8,
    West = 12,
}

impl Default for EntityDirection {
    fn default() -> Self {
        Self::North
    }
}

/// A single entity placed in the blueprint grid.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

/// Everything the layout engine produces — no rate data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutResult {
    pub entities: Vec<PlacedEntity>,
    #[serde(default)]
    pub width: i32,
    #[serde(default)]
    pub height: i32,
}
