//! Shared data models for the Fucktorio pipeline.
//!
//! Rust port of `src/models.py`. These types flow through every pipeline stage:
//! solver → layout → blueprint export → validation.
//!
//! Key types:
//! - [`ItemFlow`] — an item (or fluid) flowing at a given rate
//! - [`MachineSpec`] — one recipe step: machine type, count, inputs/outputs
//! - [`SolverResult`] — the full solved production graph
//! - [`PlacedEntity`] — a single entity placed on the tile grid (belt, machine, inserter, etc.)
//! - [`LayoutResult`] — the complete spatial layout ready for blueprint export

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
///
/// Represents any game entity (belt, inserter, machine, pipe, pole, etc.) at a
/// specific tile position with an orientation. Flows through layout → blueprint export.
#[cfg_attr(feature = "wasm", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PlacedEntity {
    /// Factorio entity prototype name (e.g. `"transport-belt"`, `"assembling-machine-2"`).
    pub name: String,
    /// Tile X coordinate (integer grid).
    #[serde(default)]
    pub x: i32,
    /// Tile Y coordinate (integer grid).
    #[serde(default)]
    pub y: i32,
    /// Facing direction (N/E/S/W). Corresponds to Factorio's 4-way direction
    /// constants (0/4/8/12).
    #[serde(default)]
    pub direction: EntityDirection,
    /// Recipe assigned to crafting machines (`None` for belts, inserters, etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipe: Option<String>,
    /// I/O role tag for bus entities: `"input"`, `"output"`, or `"passthrough"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub io_type: Option<String>,
    /// Item or fluid name this belt/pipe is currently carrying.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub carries: Option<String>,
    /// Factorio Space Age fluid-box mirroring. When `true`, flips fluid port
    /// positions along the entity's primary axis, giving 8 orientations (4
    /// rotations × 2 mirrors). Ignored in Factorio 1.1. See `CLAUDE.md`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mirror: bool,
    /// Optional identifier linking this entity to a layout segment or balancer
    /// group for debugging/analysis.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment_id: Option<String>,
    /// Throughput rate (items/s or fluid units/s) flowing through this entity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate: Option<f64>,
    /// Modules/items inserted into this entity (e.g. speed modules in a beacon).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<ModuleItem>,
}

/// Whether a boundary port is an input into the region or an output from it.
#[cfg_attr(feature = "wasm", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PortIo {
    Input,
    Output,
}

/// A point on or inside a region where a spec enters or exits. Stored in
/// absolute tile coordinates; `direction` encodes the flow direction at
/// that tile (which way items are physically moving).
#[cfg_attr(feature = "wasm", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortPoint {
    pub x: i32,
    pub y: i32,
    pub direction: EntityDirection,
}

/// A boundary port on a `LayoutRegion`: a point plus io/item metadata.
/// Replaces the old `PortSpec { edge, offset, … }` triple-encoding —
/// position is now stored in absolute coordinates and the edge is
/// derivable from the region's bbox when needed.
#[cfg_attr(feature = "wasm", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegionPort {
    pub point: PortPoint,
    pub io: PortIo,
    /// Item carried through this port.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
}

/// Origin/purpose of a `LayoutRegion`. Replaces the historical stringly-typed
/// `kind` field so classifier and histograms can exhaustive-match.
#[cfg_attr(feature = "wasm", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegionKind {
    /// Non-ghost SAT crossing-zone solver output.
    CrossingZone,
    /// Ghost-router corridor template (one horizontal spec crossing N trunks).
    CorridorTemplate,
    /// Ghost-router per-tile perpendicular crossing template.
    JunctionTemplate,
    /// Ghost-router crossing left for the junction solver to pick up.
    Unresolved,
}

/// Metadata about a resolved region in the layout (SAT crossing zone,
/// ghost-routed junction template, or unresolved placeholder).
#[cfg_attr(feature = "wasm", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutRegion {
    pub kind: RegionKind,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    /// Boundary ports. Each port records the absolute (x, y) position,
    /// flow direction, io (input/output), and item.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ports: Vec<RegionPort>,
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
