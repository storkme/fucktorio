//! Pre-generated N-to-M balancer templates.
//!
//! DO NOT EDIT MANUALLY. Regenerate with:
//!     uv run python scripts/generate_balancer_library.py
//!
//! Shapes are oriented for vertical SOUTH flow: inputs at the top
//! (facing SOUTH), outputs at the bottom (facing SOUTH).

use crate::models::{EntityDirection, PlacedEntity};
use rustc_hash::FxHashMap;
use std::sync::OnceLock;

/// A single entity within a balancer template.
///
/// Coordinates are relative to the template origin (top-left tile).
/// `direction` is a Factorio 1.0 8-way value: 0=N, 2=E, 4=S, 6=W.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BalancerTemplateEntity {
    /// Factorio entity name (e.g. "transport-belt", "splitter", "underground-belt").
    pub name: &'static str,
    /// X offset from template origin (top-left tile).
    pub x: i32,
    /// Y offset from template origin (top-left tile).
    pub y: i32,
    /// Factorio 1.0 direction: 0=N, 2=E, 4=S, 6=W.
    pub direction: u8,
    /// `Some("input")` or `Some("output")` for underground-belt, else `None`.
    pub io_type: Option<&'static str>,
}

impl BalancerTemplateEntity {
    /// Convert the Factorio 1.0 8-way direction to a Rust [`EntityDirection`].
    ///
    /// The 16-way system used by [`EntityDirection`] doubles each 8-way value.
    pub fn entity_direction(&self) -> EntityDirection {
        match self.direction {
            0 => EntityDirection::North,
            2 => EntityDirection::East,
            4 => EntityDirection::South,
            6 => EntityDirection::West,
            other => panic!("unknown Factorio 1.0 direction {other}"),
        }
    }

    /// Stamp this template entity as a [`PlacedEntity`] at `(origin_x + self.x, origin_y + self.y)`.
    ///
    /// Belt tier substitution:
    /// - `"transport-belt"` → `belt_name`
    /// - `"splitter"` → `splitter_name`
    /// - `"underground-belt"` → `ug_name`
    /// - anything else is kept as-is.
    pub fn stamp(
        &self,
        origin_x: i32,
        origin_y: i32,
        belt_name: &str,
        splitter_name: &str,
        ug_name: &str,
        item: Option<&str>,
    ) -> PlacedEntity {
        let name = match self.name {
            "transport-belt" => belt_name,
            "splitter" => splitter_name,
            "underground-belt" => ug_name,
            other => other,
        }
        .to_owned();

        PlacedEntity {
            name,
            x: origin_x + self.x,
            y: origin_y + self.y,
            direction: self.entity_direction(),
            recipe: None,
            io_type: self.io_type.map(|s| s.to_owned()),
            carries: item.map(|s| s.to_owned()),
            mirror: false,
            segment_id: None,
        }
    }
}

/// A pre-generated N-to-M balancer template.
#[derive(Debug, Clone)]
pub struct BalancerTemplate {
    pub n_inputs: u32,
    pub n_outputs: u32,
    pub width: u32,
    pub height: u32,
    pub entities: &'static [BalancerTemplateEntity],
    /// Input tile offsets (dx, dy) relative to template origin.
    pub input_tiles: &'static [(i32, i32)],
    /// Output tile offsets (dx, dy) relative to template origin.
    pub output_tiles: &'static [(i32, i32)],
    /// Source blueprint string (for debugging / regeneration).
    pub source_blueprint: &'static str,
}

impl BalancerTemplate {
    /// Stamp the template at the given origin, substituting belt tier names.
    /// See [`BalancerTemplateEntity::stamp`] for substitution rules.
    pub fn stamp(
        &self,
        origin_x: i32,
        origin_y: i32,
        belt_name: &str,
        splitter_name: &str,
        ug_name: &str,
        item: Option<&str>,
    ) -> Vec<PlacedEntity> {
        self.entities
            .iter()
            .map(|e| e.stamp(origin_x, origin_y, belt_name, splitter_name, ug_name, item))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Template data
// ---------------------------------------------------------------------------

static T_1_2_ENTITIES: &[BalancerTemplateEntity] = &[
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 2, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 0, y: 1, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 2, direction: 4, io_type: None },
];
static T_1_2_INPUT: &[(i32, i32)] = &[(1, 0)];
static T_1_2_OUTPUT: &[(i32, i32)] = &[(0, 2), (1, 2)];

static T_1_3_ENTITIES: &[BalancerTemplateEntity] = &[
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 1, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 3, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 4, direction: 6, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 8, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 1, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 1, y: 2, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "underground-belt", x: 1, y: 3, direction: 4, io_type: Some("input") },
    BalancerTemplateEntity { name: "splitter", x: 1, y: 4, direction: 6, io_type: None },
    BalancerTemplateEntity { name: "underground-belt", x: 1, y: 6, direction: 4, io_type: Some("output") },
    BalancerTemplateEntity { name: "splitter", x: 1, y: 7, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 8, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 1, direction: 2, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 2, direction: 0, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 3, direction: 0, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 4, direction: 0, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 5, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 6, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 7, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 8, direction: 4, io_type: None },
];
static T_1_3_INPUT: &[(i32, i32)] = &[(2, 0)];
static T_1_3_OUTPUT: &[(i32, i32)] = &[(0, 8), (1, 8), (2, 8)];

static T_1_4_ENTITIES: &[BalancerTemplateEntity] = &[
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 3, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 2, y: 2, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 3, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 1, y: 1, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 3, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 0, y: 2, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 3, direction: 4, io_type: None },
];
static T_1_4_INPUT: &[(i32, i32)] = &[(1, 0)];
static T_1_4_OUTPUT: &[(i32, i32)] = &[(0, 3), (1, 3), (2, 3), (3, 3)];

static T_2_1_ENTITIES: &[BalancerTemplateEntity] = &[
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 2, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 0, y: 1, direction: 4, io_type: None },
];
static T_2_1_INPUT: &[(i32, i32)] = &[(0, 0), (1, 0)];
static T_2_1_OUTPUT: &[(i32, i32)] = &[(1, 2)];

static T_2_2_ENTITIES: &[BalancerTemplateEntity] = &[
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 2, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 0, y: 1, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 2, direction: 4, io_type: None },
];
static T_2_2_INPUT: &[(i32, i32)] = &[(0, 0), (1, 0)];
static T_2_2_OUTPUT: &[(i32, i32)] = &[(0, 2), (1, 2)];

static T_2_3_ENTITIES: &[BalancerTemplateEntity] = &[
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 2, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 3, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "underground-belt", x: 2, y: 5, direction: 4, io_type: Some("input") },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 6, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 7, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 8, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 9, direction: 6, io_type: None },
    BalancerTemplateEntity { name: "underground-belt", x: 2, y: 10, direction: 4, io_type: Some("output") },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 11, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 12, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 13, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 1, y: 1, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "underground-belt", x: 1, y: 2, direction: 4, io_type: Some("input") },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 3, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 1, y: 4, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 5, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 6, direction: 2, io_type: None },
    BalancerTemplateEntity { name: "underground-belt", x: 1, y: 7, direction: 4, io_type: Some("output") },
    BalancerTemplateEntity { name: "underground-belt", x: 1, y: 8, direction: 4, io_type: Some("input") },
    BalancerTemplateEntity { name: "splitter", x: 1, y: 9, direction: 6, io_type: None },
    BalancerTemplateEntity { name: "underground-belt", x: 1, y: 11, direction: 4, io_type: Some("output") },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 13, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 3, direction: 2, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 4, direction: 0, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 5, direction: 0, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 6, direction: 0, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 7, direction: 0, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 8, direction: 0, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 9, direction: 0, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 10, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 11, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 0, y: 12, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 13, direction: 4, io_type: None },
];
static T_2_3_INPUT: &[(i32, i32)] = &[(1, 0), (2, 0)];
static T_2_3_OUTPUT: &[(i32, i32)] = &[(0, 13), (1, 13), (2, 13)];

static T_2_4_ENTITIES: &[BalancerTemplateEntity] = &[
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 3, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 2, y: 2, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 3, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 1, y: 1, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 3, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 0, y: 2, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 3, direction: 4, io_type: None },
];
static T_2_4_INPUT: &[(i32, i32)] = &[(1, 0), (2, 0)];
static T_2_4_OUTPUT: &[(i32, i32)] = &[(0, 3), (1, 3), (2, 3), (3, 3)];

static T_2_5_ENTITIES: &[BalancerTemplateEntity] = &[
    BalancerTemplateEntity { name: "transport-belt", x: 4, y: 1, direction: 6, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 4, y: 2, direction: 0, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 4, y: 3, direction: 0, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 4, y: 4, direction: 0, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 4, y: 5, direction: 0, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 4, y: 9, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 1, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 3, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 5, direction: 2, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 7, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 3, y: 8, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 9, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 2, y: 2, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 2, y: 4, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 5, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 7, direction: 2, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 9, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 1, y: 1, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "underground-belt", x: 1, y: 2, direction: 4, io_type: Some("input") },
    BalancerTemplateEntity { name: "splitter", x: 1, y: 3, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 4, direction: 6, io_type: None },
    BalancerTemplateEntity { name: "underground-belt", x: 1, y: 5, direction: 4, io_type: Some("output") },
    BalancerTemplateEntity { name: "splitter", x: 1, y: 6, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 7, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 1, y: 8, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 9, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 4, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 5, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 6, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 7, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 8, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 9, direction: 4, io_type: None },
];
static T_2_5_INPUT: &[(i32, i32)] = &[(1, 0), (2, 0)];
static T_2_5_OUTPUT: &[(i32, i32)] = &[(0, 9), (1, 9), (2, 9), (3, 9), (4, 9)];

static T_3_1_ENTITIES: &[BalancerTemplateEntity] = &[
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 1, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 2, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 3, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 5, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 7, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 8, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "underground-belt", x: 1, y: 2, direction: 4, io_type: Some("input") },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 3, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 1, y: 4, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "underground-belt", x: 1, y: 5, direction: 4, io_type: Some("output") },
    BalancerTemplateEntity { name: "splitter", x: 1, y: 6, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 7, direction: 6, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 0, y: 1, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 3, direction: 2, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 4, direction: 0, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 5, direction: 0, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 6, direction: 0, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 7, direction: 0, io_type: None },
];
static T_3_1_INPUT: &[(i32, i32)] = &[(0, 0), (1, 0), (2, 0)];
static T_3_1_OUTPUT: &[(i32, i32)] = &[(2, 8)];

static T_3_2_ENTITIES: &[BalancerTemplateEntity] = &[
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 1, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 2, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "underground-belt", x: 2, y: 3, direction: 4, io_type: Some("input") },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 4, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 5, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 6, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 7, direction: 6, io_type: None },
    BalancerTemplateEntity { name: "underground-belt", x: 2, y: 8, direction: 4, io_type: Some("output") },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 10, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 11, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 13, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "underground-belt", x: 1, y: 2, direction: 4, io_type: Some("input") },
    BalancerTemplateEntity { name: "splitter", x: 1, y: 3, direction: 2, io_type: None },
    BalancerTemplateEntity { name: "underground-belt", x: 1, y: 5, direction: 4, io_type: Some("output") },
    BalancerTemplateEntity { name: "underground-belt", x: 1, y: 6, direction: 4, io_type: Some("input") },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 7, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 8, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 1, y: 9, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 10, direction: 6, io_type: None },
    BalancerTemplateEntity { name: "underground-belt", x: 1, y: 11, direction: 4, io_type: Some("output") },
    BalancerTemplateEntity { name: "splitter", x: 1, y: 12, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 13, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 0, y: 1, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 2, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 3, direction: 2, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 4, direction: 2, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 5, direction: 0, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 6, direction: 0, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 7, direction: 0, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 8, direction: 0, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 9, direction: 0, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 10, direction: 0, io_type: None },
];
static T_3_2_INPUT: &[(i32, i32)] = &[(0, 0), (1, 0), (2, 0)];
static T_3_2_OUTPUT: &[(i32, i32)] = &[(1, 13), (2, 13)];

static T_3_3_ENTITIES: &[BalancerTemplateEntity] = &[
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "underground-belt", x: 3, y: 2, direction: 4, io_type: Some("input") },
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 3, direction: 6, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 4, direction: 0, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 5, direction: 0, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 6, direction: 0, io_type: None },
    BalancerTemplateEntity { name: "underground-belt", x: 3, y: 7, direction: 4, io_type: Some("output") },
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 9, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 2, y: 1, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 2, direction: 6, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 3, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "underground-belt", x: 2, y: 5, direction: 4, io_type: Some("input") },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 6, direction: 2, io_type: None },
    BalancerTemplateEntity { name: "underground-belt", x: 2, y: 7, direction: 4, io_type: Some("output") },
    BalancerTemplateEntity { name: "splitter", x: 2, y: 8, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 9, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "underground-belt", x: 1, y: 1, direction: 4, io_type: Some("input") },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 2, direction: 6, io_type: None },
    BalancerTemplateEntity { name: "underground-belt", x: 1, y: 3, direction: 4, io_type: Some("output") },
    BalancerTemplateEntity { name: "splitter", x: 1, y: 4, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 6, direction: 2, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 7, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 8, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 9, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 2, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 3, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 4, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 0, y: 5, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 6, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 7, direction: 2, io_type: None },
];
static T_3_3_INPUT: &[(i32, i32)] = &[(1, 0), (2, 0), (3, 0)];
static T_3_3_OUTPUT: &[(i32, i32)] = &[(1, 9), (2, 9), (3, 9)];

static T_3_4_ENTITIES: &[BalancerTemplateEntity] = &[
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "underground-belt", x: 3, y: 2, direction: 4, io_type: Some("input") },
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 3, direction: 6, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 4, direction: 0, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 5, direction: 0, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 6, direction: 0, io_type: None },
    BalancerTemplateEntity { name: "underground-belt", x: 3, y: 7, direction: 4, io_type: Some("output") },
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 9, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 11, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 2, y: 1, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 2, direction: 6, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 3, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "underground-belt", x: 2, y: 5, direction: 4, io_type: Some("input") },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 6, direction: 2, io_type: None },
    BalancerTemplateEntity { name: "underground-belt", x: 2, y: 7, direction: 4, io_type: Some("output") },
    BalancerTemplateEntity { name: "splitter", x: 2, y: 8, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 2, y: 10, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 11, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "underground-belt", x: 1, y: 1, direction: 4, io_type: Some("input") },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 2, direction: 6, io_type: None },
    BalancerTemplateEntity { name: "underground-belt", x: 1, y: 3, direction: 4, io_type: Some("output") },
    BalancerTemplateEntity { name: "splitter", x: 1, y: 4, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 6, direction: 2, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 1, y: 9, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 11, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 2, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 3, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 4, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 0, y: 5, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 6, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 7, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 8, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 9, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 0, y: 10, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 11, direction: 4, io_type: None },
];
static T_3_4_INPUT: &[(i32, i32)] = &[(1, 0), (2, 0), (3, 0)];
static T_3_4_OUTPUT: &[(i32, i32)] = &[(0, 11), (1, 11), (2, 11), (3, 11)];

static T_4_1_ENTITIES: &[BalancerTemplateEntity] = &[
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 2, y: 1, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 3, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 1, y: 2, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 0, y: 1, direction: 4, io_type: None },
];
static T_4_1_INPUT: &[(i32, i32)] = &[(0, 0), (1, 0), (2, 0), (3, 0)];
static T_4_1_OUTPUT: &[(i32, i32)] = &[(2, 3)];

static T_4_2_ENTITIES: &[BalancerTemplateEntity] = &[
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 2, y: 1, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 3, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 1, y: 2, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 3, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 0, y: 1, direction: 4, io_type: None },
];
static T_4_2_INPUT: &[(i32, i32)] = &[(0, 0), (1, 0), (2, 0), (3, 0)];
static T_4_2_OUTPUT: &[(i32, i32)] = &[(1, 3), (2, 3)];

static T_4_3_ENTITIES: &[BalancerTemplateEntity] = &[
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 2, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "underground-belt", x: 3, y: 4, direction: 4, io_type: Some("input") },
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 5, direction: 6, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 6, direction: 0, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 7, direction: 0, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 8, direction: 0, io_type: None },
    BalancerTemplateEntity { name: "underground-belt", x: 3, y: 9, direction: 4, io_type: Some("output") },
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 11, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 2, y: 1, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 2, y: 3, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "underground-belt", x: 2, y: 4, direction: 4, io_type: Some("input") },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 5, direction: 6, io_type: None },
    BalancerTemplateEntity { name: "underground-belt", x: 2, y: 6, direction: 4, io_type: Some("output") },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 8, direction: 2, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 9, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 2, y: 10, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 11, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 1, y: 2, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 5, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 1, y: 7, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "underground-belt", x: 1, y: 8, direction: 4, io_type: Some("input") },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 9, direction: 2, io_type: None },
    BalancerTemplateEntity { name: "underground-belt", x: 1, y: 10, direction: 4, io_type: Some("output") },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 11, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 0, y: 1, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 2, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 3, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 4, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 5, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 0, y: 6, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 7, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 8, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 9, direction: 2, io_type: None },
];
static T_4_3_INPUT: &[(i32, i32)] = &[(0, 0), (1, 0), (2, 0), (3, 0)];
static T_4_3_OUTPUT: &[(i32, i32)] = &[(1, 11), (2, 11), (3, 11)];

static T_4_4_ENTITIES: &[BalancerTemplateEntity] = &[
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 2, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 3, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 4, direction: 6, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 6, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 7, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 9, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 2, y: 1, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "underground-belt", x: 2, y: 3, direction: 4, io_type: Some("input") },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 4, direction: 6, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 6, direction: 2, io_type: None },
    BalancerTemplateEntity { name: "underground-belt", x: 2, y: 7, direction: 4, io_type: Some("output") },
    BalancerTemplateEntity { name: "splitter", x: 2, y: 8, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 9, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 1, y: 2, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "underground-belt", x: 1, y: 3, direction: 4, io_type: Some("input") },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 4, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 6, direction: 2, io_type: None },
    BalancerTemplateEntity { name: "underground-belt", x: 1, y: 7, direction: 4, io_type: Some("output") },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 9, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 0, y: 1, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 2, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 3, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 4, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 0, y: 5, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 6, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 7, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 0, y: 8, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 9, direction: 4, io_type: None },
];
static T_4_4_INPUT: &[(i32, i32)] = &[(0, 0), (1, 0), (2, 0), (3, 0)];
static T_4_4_OUTPUT: &[(i32, i32)] = &[(0, 9), (1, 9), (2, 9), (3, 9)];

static T_5_2_ENTITIES: &[BalancerTemplateEntity] = &[
    BalancerTemplateEntity { name: "transport-belt", x: 4, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 4, y: 3, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 4, y: 4, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 4, y: 6, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 4, y: 7, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 4, y: 8, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 4, y: 10, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 3, y: 1, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 2, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 3, direction: 2, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 4, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 3, y: 5, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 7, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 3, y: 9, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 3, y: 10, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 2, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 3, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 4, direction: 2, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 5, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 2, y: 6, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 2, y: 8, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 2, y: 9, direction: 6, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "splitter", x: 1, y: 1, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 3, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "underground-belt", x: 1, y: 4, direction: 4, io_type: Some("input") },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 5, direction: 2, io_type: None },
    BalancerTemplateEntity { name: "underground-belt", x: 1, y: 6, direction: 4, io_type: Some("output") },
    BalancerTemplateEntity { name: "splitter", x: 1, y: 7, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 1, y: 9, direction: 6, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 0, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 1, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 2, direction: 4, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 3, direction: 2, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 5, direction: 2, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 6, direction: 0, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 7, direction: 0, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 8, direction: 0, io_type: None },
    BalancerTemplateEntity { name: "transport-belt", x: 0, y: 9, direction: 0, io_type: None },
];
static T_5_2_INPUT: &[(i32, i32)] = &[(0, 0), (1, 0), (2, 0), (3, 0), (4, 0)];
static T_5_2_OUTPUT: &[(i32, i32)] = &[(3, 10), (4, 10)];

// ---------------------------------------------------------------------------
// Global registry
// ---------------------------------------------------------------------------

/// Lazily-initialised map from (n_inputs, n_outputs) to [`BalancerTemplate`].
pub fn balancer_templates() -> &'static FxHashMap<(u32, u32), BalancerTemplate> {
    static MAP: OnceLock<FxHashMap<(u32, u32), BalancerTemplate>> = OnceLock::new();
    MAP.get_or_init(build_templates)
}

fn build_templates() -> FxHashMap<(u32, u32), BalancerTemplate> {
    let mut m = FxHashMap::with_capacity_and_hasher(17, Default::default());

    m.insert((1, 2), BalancerTemplate {
        n_inputs: 1, n_outputs: 2, width: 2, height: 3,
        entities: T_1_2_ENTITIES, input_tiles: T_1_2_INPUT, output_tiles: T_1_2_OUTPUT,
        source_blueprint: "0eNqtkOtqwzAMhV+l6HcW4iyXtq8yQslFFEGiGFsdDcHvPs0L26BhbLB/R8dH5xNeoRtvaB2xwPmwAvUze1UvK3i6cjtGVxaLKoAEJ0gOwO0UZ29HEkEHQU3iAe/qmtAkW1QjX/VqvqLzNLP6+dEUdXGqq9pkVVnpG7KQEG7wOC0Xvk2d1mupJuzsNRHXV3gnZWmp9vKhwre7xLXs7ezkqcMxkgdy2G/LuUYfCfkeIf9PwvMewXwSTJqFvb/9VXfx8/Xmb9c3IbwB0pGkzg==",
    });

    m.insert((1, 3), BalancerTemplate {
        n_inputs: 1, n_outputs: 3, width: 3, height: 9,
        entities: T_1_3_ENTITIES, input_tiles: T_1_3_INPUT, output_tiles: T_1_3_OUTPUT,
        source_blueprint: "0eNqtlOFugyAQx1+l4XPXACJqX2VpFm1JQ6JgEJcZw7vv5szWbCdZOz55HMf97nL3dyZNO6reaePJcTcTfbZmAOt5JoO+mrpdvH7qFRhEe9WR/Y6YulvOQ99q75UjAZzaXNQbeFk47ddQCPlOD85X5QZtDfh5yUQhqkIWjMpcwp0yXnutVvhyml7M2DWQHpJCRG8HiFiez+SDRA85uKdPK9zU5V1tht46/9SodiFftFPn9TGH0N8EjhFYSkKGEbKUBIERxGMEgRJyjFCm7EHG58D+TygwAr8h0IDt+J9yl/EZ/6h+BMm4q7PwxeuH85f2TD96gkIrDJofKA7daggfOKNYcpmiIzv6zZYYKvgizZAYj29xgh1jWXyNeRSBpxTxveX3VC1xRB5f3xQIGf9HpUAUuB4eQmyMt4yrIgWiiisgAYLTuA7uRJxCeAde3ooy",
    });

    m.insert((1, 4), BalancerTemplate {
        n_inputs: 1, n_outputs: 4, width: 4, height: 4,
        entities: T_1_4_ENTITIES, input_tiles: T_1_4_INPUT, output_tiles: T_1_4_OUTPUT,
        source_blueprint: "0eNqtkd1qwzAMhV+l+LoLsfPX9VVKKUkriiBRjK2WhuB3n2ZCV2gIG9mddHys71geVdPewDokVvvNqPDck5fqMCqPV6rbqPJgQQqFDJ3abhTVXey9bZEZnAoiIl3gIaoOx+1kFcvPeBHv4Dz2JLrZ6bzKP6uy0mlZlHIGxMgIEzx2w4luXSPjZag4bO/FEa+P6puUJYXIg1RpUoSXXOxq8rZ3/NFAG8kXdHCeLhuxvhPMHME8CTpJw9zLfzU7W06v16fP5wjpk2DWE4o5gn4hrNhPubyff0hfLf9utib9bjl99rf0xxC+AJD9GFk=",
    });

    m.insert((2, 1), BalancerTemplate {
        n_inputs: 2, n_outputs: 1, width: 2, height: 3,
        entities: T_2_1_ENTITIES, input_tiles: T_2_1_INPUT, output_tiles: T_2_1_OUTPUT,
        source_blueprint: "0eNqtkdEKwjAMRX9F8qyyzrmpvyIimwYJbFlpM3GM/ruxDhQcvujbze3NPS0doKo7tI5YYDcbgE4te1X7ATxduKyjK71FFUCCDcxnwGUTZ29rEkEHQU3iM97UNeEwH6MaedWreUXnqWX1043Jimxb5IVJ8nWuZ8hCQjjC49QfuWsqrddSTdjWayKuD/AgJcu12v1Thbd7iSvZ29bJosI6ks/k8DQupxr9JKRThPSfhNX3N5jfCdkUwbwRkjD1ex/dhxDuAAqkzA==",
    });

    m.insert((2, 2), BalancerTemplate {
        n_inputs: 2, n_outputs: 2, width: 2, height: 3,
        entities: T_2_2_ENTITIES, input_tiles: T_2_2_INPUT, output_tiles: T_2_2_OUTPUT,
        source_blueprint: "0eNqtketqwzAMhV+l6Hdb4jSXtq9SSslFDEGiGFstDcHvXs0LtLBQNrZ/0vHR+WR7grq7onXEAsfVBNQM7LU6TeDpg6suqjJa1AJIsIf1CrjqY+9tRyLoIKhI3OJdVRPO69mqlme8ijd0ngZWPd2brMwOZVGapMgLPUMWEsIZHrvxwte+1ngNVYcdvDri+ASfpGSbqzx+VeFlL3EVezs42dTYRXJLDpt5OFXrd0K6REj/k7B7fwfzd0K2RDAvhCQs/d6PsvP37/PL7c8hPAC7gMLO",
    });

    m.insert((2, 3), BalancerTemplate {
        n_inputs: 2, n_outputs: 3, width: 3, height: 14,
        entities: T_2_3_ENTITIES, input_tiles: T_2_3_INPUT, output_tiles: T_2_3_OUTPUT,
        source_blueprint: "0eNq1lu1ugyAUhm+l4Xe3cFBReytLs/SDNCQWDeKyxnjvO3PN1mRHXLvjL+F44OHrfaEX+6ozjbcuiM2qF/ZQuxZLL71o7cntqjEaLo3BgrDBnMV6JdzuPNbbprIhGC8GDFp3NO8YhWG7vqZiyk/3GHwzvrW1w7gqIM3TMtc5SJ1p/GdcsMGaK3ysXV5dd95j99gpZjR1ixlj8158kuRzhuHLV2m4GVfwO9c2tQ9Pe1ON5KP15nBtrDD1N0FRBMVJSChCwklIKUI2Rehww/zJ1/ilGVj/3nnXdEGQ0IyCas5paYqQcxJyilBwEgqKUD5GSElCSRFAcux+3YXJ7QdJcoFz8YCUP7CqE0gDAFZ9QhK3MWBAkB4ANwg5UA7+t86zuEnCIgYDOm6cHKtGGkDKtGpF3JY5xl/GTTiOoO9DGXddWMZOFMSteJkjptSEeUoaO3UAaGdWyYxFLrWW6YylMZw8lcXFqR44eTouRnXPqDWNyOOS5EAUcUlyIMq4RBkQiYzLkQMB8acRB0LNvI0Uw+t+TuQcjHTmHaT+c08l2Yxd3DmD7TB8ANO/YuA=",
    });

    m.insert((2, 4), BalancerTemplate {
        n_inputs: 2, n_outputs: 4, width: 4, height: 4,
        entities: T_2_4_ENTITIES, input_tiles: T_2_4_INPUT, output_tiles: T_2_4_OUTPUT,
        source_blueprint: "0eNqtkV1qwzAQhK8S9JwES/5LepUSgp0sZcFeC2lTaozu3q0headGBJy31Wg03zJogra7o7HEHk67Ceg6sJPpYwJHn9x0UfWjQRmAPPaw3wE3fTw705H3aCGISHzDh6gqnPezVSx/8SJ+oXU0sOj6oIq6ONZVrbKqrOQO2ZMnnOHxNF743rcSL6HiMIMTR3w+wQ8pey9FHn+nsNjL24adGax/a7GL5BtZvM6PtVj/E3SaoLYT8jWCWhCysNftU9nFWnb+yu3LdD96O6FaI+gFYUM/dbqfF2x/SPeTbycc0/8nf7afcwjfKm82Vg==",
    });

    m.insert((2, 5), BalancerTemplate {
        n_inputs: 2, n_outputs: 5, width: 5, height: 10,
        entities: T_2_5_ENTITIES, input_tiles: T_2_5_INPUT, output_tiles: T_2_5_OUTPUT,
        source_blueprint: "0eNqtlttqwzAMQH+l+LkbkePc9iujjB5MMaROcJyxEvLu07KyFabaa6s81VYdndSyTj2KXT3ozhnrxctqFGbf2h5Hr6PozdFu6znqT53GgTBeN2K9EnbbzPO+q4332okJg8Ye9AdGYdqsz0txyW96DL5r15vWYlyWoApVFXkBSZ7l+J223nijz/B5dnqzQ7PD9JgUV3Rtjyvmx0fxRUqeMwyfv0fj1bq829qubZx/2ul6Ih+M0/vLx4CpfwlAEeAWocfNuqNr8EkzcP7z12zbe0FCUwqaPratjCRkFCF7jKBIQk4Rck6CogiKk1BQhILj8Jve3zz9kqJWnKKuKIKUnAiZhK0pGRCk++UVIhmpW+l/xSPGl/M9KCM25/hFWdiFcpHrS+ZhZ4Y3RpdUYSvKZawoyRugZJIYaXSZMFWPmZxBXhAxOTAgIiaHRRQMEffDfPdDxP2wjKIh0vlhjuYgYny43/igwt1w3nqLiEc4BFyGpZQyIKqwlBgQaRKWDQdChntVOuekUwgrk2P9abhFcSCycDviQORhv3EgVKTvzTvpmKfv3MFmHD8BBYSXeA==",
    });

    m.insert((3, 1), BalancerTemplate {
        n_inputs: 3, n_outputs: 1, width: 3, height: 9,
        entities: T_3_1_ENTITIES, input_tiles: T_3_1_INPUT, output_tiles: T_3_1_OUTPUT,
        source_blueprint: "0eNqtlOFuhCAMx1+lwufbIpyi7lWWy6J35EKiYACXGcO7r3Nmu2xdMyOfbGvpD8q/zKztRjU4bQJ7OsxMX6zxYD3PzOubabolGqZBgcF0UD07Hphp+sX3Q6dDUI5FCGpzVW8Q5fF8XFMh5bs8BF+V89oaiIuK52Vel7LkmSwk/FMm6KDVCl+86cWMfQvloShkDNZDxrJ8Zh+k7LGA8PRpxbt9BdcYP1gXHlrVLeSrduqyLhaQ+psgMAJPSThhBJGSkGOEU0pCgRGKlASJEcqUhBIjVCkJFa1Wvp9Q01r6QRhhNN3NWfjiDPC/ZtwMY2AolGe0vhKci6ODnt8hsog9QP8rLmj17uiaHcPfbUMHXyY6U07Py6Y7yXFEQctZJLh2Sb++YleLSlq29P7xkhUt000tkTiipsWaACEyWpgpEJyW50bEOcZ3cM2oWg==",
    });

    m.insert((3, 2), BalancerTemplate {
        n_inputs: 3, n_outputs: 2, width: 3, height: 14,
        entities: T_3_2_ENTITIES, input_tiles: T_3_2_INPUT, output_tiles: T_3_2_OUTPUT,
        source_blueprint: "0eNq1lttugzAMQH9l+bobkePc9iujjB5MMaROcJyxEvLu07KyFabaa6s81VYdndSyTj2KXT3ozhnrxctqFGbf2h5Hr6PozdFu6znqT53GgTBeN2K9EnbbzPO+q4332okJg8Ye9AdGYdqsz0txyW96DL5r15vWYlyWoApVFXkBSZ7l+J223nijz/B5dnqzQ7PD9JgUV3Rtjyvmx0fxRUqeMwyfv0fj1bq829qubZx/2ul6Ih+M0/vLx4CpfwlAEeAWocfNuqNr8EkzcP7z12zbe0FCUwqaPratjCRkFCF7jKBIQk4Rck6CogiKk1BQhILj8Jve3zz9kqJWnKKuKIKUnAiZhK0pGRCk++UVIhmpW+l/xSPGl/M9KCM25/hFWdiFcpHrS+ZhZ4Y3RpdUYSvKZawoyRugZJIYaXSZMFWPmZxBXhAxOTAgIiaHRRQMEffDfPdDxP2wjKIh0vlhjuYgYny43/igwt1w3nqLiEc4BFyGpZQyIKqwlBgQaRKWDQdChntVOuekUwgrk2P9abhFcSCycDviQORhv3EgVKTvzTvpmKfv3MFmHD8BBYSXeA==",
    });

    m.insert((3, 3), BalancerTemplate {
        n_inputs: 3, n_outputs: 3, width: 4, height: 10,
        entities: T_3_3_ENTITIES, input_tiles: T_3_3_INPUT, output_tiles: T_3_3_OUTPUT,
        source_blueprint: "0eNq1ldtugzAMQH9lyvU24XDsXmWqph6iKhINKIRpFeLd57FqqzTX2Yq5IjHGhxCfMKh93ds2OB/VZjUod2h8h6OXQXXu5Hf1FI2X1uJAuWjPar1Sfnee5l1buxhtUCMGnT/ad4zqcbu+pmLKT3kMvtnQucZjHEqdFmlV5IVO8izHe9ZHF529wqfZ5dX35z2Wx6KY0TYdZkyPD+qTlDxnGL58jcab94ph57u2CfFpb+uJfHTBHq4PA6b+JgBFgHuEHhcbTqHBK83A+fdX820fFQk1FNQ8tqyUJKQUIX2MkJOEjCJkkoScIuSShIIiFBKb3/Tx7u6XFLWSbOqK10bPJ+iEQugbRDJSJ8bfimteSj3fDw28ghKfyPCG6EWOFp3y1vALo0tmvCZ6GU006X8p1GIFL6HE/pe8hSCAqHgLYZEWg4TXE+brCZrXE5ZpOQD+twlzWg4Mbyb830xIeTMFWgwy3kIJRM67KIEo+JY1AoiSb1kJRMW3pwDCJPxPy8wxwGjeAIn3B94IHjFux/EDHmcBYA==",
    });

    m.insert((3, 4), BalancerTemplate {
        n_inputs: 3, n_outputs: 4, width: 4, height: 12,
        entities: T_3_4_ENTITIES, input_tiles: T_3_4_INPUT, output_tiles: T_3_4_OUTPUT,
        source_blueprint: "0eNq1ld1ugzAMhl+lyvU24XDsXmWqph6iKhINKIRpFeLd57FqqzTX2Yq5IjHGn0P824Pa1b1pvXVBPa8GZfeN63D1MqjOHt22nqzh3BpcKBvMST2slNuepn3X1jYE49WIRusO5h2tMG4eLq7o8hMejW/Gd7ZxaNcVZGW2LosSkiIv8J1xwQZrLvBpd351/WmH4TEoerRNhx7T54P6JCVPOZrPX6vxKq/gt65rGx8ed6aeyAfrzf7ysUbX3wRNEbQkIaUI2S1Cj7/TH32DT5qB++97cW0fFAnNKGh+37EykpBThOI+QkESCopQShJKilBJEiqKsJa4/KYPN29/TVEBJKsaEl6aIIAg1Q9XiGSkutLfgpPCT4WCRzQPi2geIqKH+aKHiOphmXqGghcqfzI6ZMkrU6J+SfFDIlRjMZELnEBHRK4FEMAPYD3nH2nNK0Ii/5SfUvPyz/i614v0EZ3z0tD/V5suIlLQyzQOXUZEIlEBFS+SVACx5idhOqfI0oRXoED+KfDDVgKh+ZErgUj5biKByPj5Ou+ic75VSeQfGdQSiMjg5hHjZhw/AMqYknc=",
    });

    m.insert((4, 1), BalancerTemplate {
        n_inputs: 4, n_outputs: 1, width: 4, height: 4,
        entities: T_4_1_ENTITIES, input_tiles: T_4_1_INPUT, output_tiles: T_4_1_OUTPUT,
        source_blueprint: "0eNqtkd1qwzAMhV+l6LobsfPX9VVGGUkrhiBRjK2OheB3n+YFFlgIhexOPj4+30GeoO3u6DyxwPkwAV0HDjq9ThDonZsuqTI61AFIsIfjAbjp0zm4jkTQQ1SR+Iafqpp4Oc5WtfzGq/iBPtDAqtuTKeripa5qk1VlpXfIQkI4w9NpfON732q8hqrDDUEd6fkE36TsuVR5/Jniopf4hoMbvDy12CXyjTxe58dWrX8Jdptg9hPyNYJZELK4ttuHsou17Pw/25fb+7H7CdUawS4IO/ZTb7fP97c/bf9u/mj7S4xfCWoYTg==",
    });

    m.insert((4, 2), BalancerTemplate {
        n_inputs: 4, n_outputs: 2, width: 4, height: 4,
        entities: T_4_2_ENTITIES, input_tiles: T_4_2_INPUT, output_tiles: T_4_2_OUTPUT,
        source_blueprint: "0eNqtkl1qwzAQhK8S9jktlvyX5ColBDtZyoK9FpJSYozu3q1qqKFGBJy31Wg03zJogra7o7HEHk67Ceg6sJPpYwJHn9x0UfWjQRmAPPaw3wE3fTw705H3aCGISHzDh6gqnPezVSx/8SJ+oXU0sOj6oIq6ONZVrbKqrOQO2ZMnnOHxNF743rcSL6HiMIMTR3w+wQ8pey9FHn+nsNjL24adGax/a7GL5BtZvM6PtVj/E3SaoLYT8jWCWhCysNbtU9nFWnb+yu3LdD96O6FaI+gFYUM/dbqfF2x/SPeTbycc0/8nf7afcwjfKm82Vg==",
    });

    m.insert((4, 3), BalancerTemplate {
        n_inputs: 4, n_outputs: 3, width: 4, height: 12,
        entities: T_4_3_ENTITIES, input_tiles: T_4_3_INPUT, output_tiles: T_4_3_OUTPUT,
        source_blueprint: "0eNq1ldtugzAMhl+lyvU24XDsXmWqph6iKhINKIRpFeLd57FqqzTX2Yq5IjHGn0P824Pa1b1pvXVBPa8GZfeN63D1MqjOHt22nqzh3BpcKBvMST2slNuepn3X1jYE49WIRusO5h2tMG4eLq7o8hMejW/Gd7ZxaNcVZGW2LosSkiIv8J1xwQZrLvBpd351/WmH4TEoerRNhx7T54P6JCVPOZrPX6vxKq/gt65rGx8ed6aeyAfrzf7ysUbX3wRNEbQkIaUI2S1Cj7/TH32DT5qB++97cW0fFAnNKGh+37EykpBThOI+QkESCopQShJKilBJEiqKsJa4/KYPN29/TVEBJKsaEl6aIIAg1Q9XiGSkutLfgpPCT4WCRzQPi2geIqKH+aKHiOphmXqGghcqfzI6ZMkrU6J+SfFDIlRjMZELnEBHRK4FEMAPYD3nH2nNK0Ii/5SfUvPyz/i614v0EZ3z0tD/V5suIlLQyzQOXUZEIlEBFS+SVACx5idhOqfI0oRXoED+KfDDVgKh+ZErgUj5biKByPj5Ou+ic75VSeQfGdQSiMjg5hHjZhw/AMqYknc=",
    });

    m.insert((4, 4), BalancerTemplate {
        n_inputs: 4, n_outputs: 4, width: 4, height: 10,
        entities: T_4_4_ENTITIES, input_tiles: T_4_4_INPUT, output_tiles: T_4_4_OUTPUT,
        source_blueprint: "0eNq1ld1ugzAMhV+lynU3EQcS2KtM1dSfqIoEAYUwrUK8+zxWbdWWeetwr0iM8WfMOWEUu3qwXXA+iofVKNy+9T2uHkfRu6Pf1nM0njqLC+GibcR6Jfy2mfd9V7sYbRATBp0/2BeMymmzPqdiymd5DD7b0LvWYxxKmZu8MtrITBca71kfXXT2DJ93pyc/NDssj0Uxo2t7zJgfH8UbKbsvMHx6X00XfcWw9X3Xhni3s/VMPrhg9+eHAVO/EyBFAE6CShEUJyFPEfL/EfIkoUgRNOc76BTBcBJMilBxEkparXI5oUoR5AUhm1I+/VNtmdFC/dL+gL4Px9DiNf0CuP84QHw3RJGmSlq8crl4JdDqpRHpkoqW64JZtUP8eVhJp5dM37+gDcIgX6lphwADwtAHOiwaUUlbBG5jkYq2CMPUIKMtAtdbBCRtEbiNRQBoFXMMS9EqVgyInD7o1RIVQ0FbhKN/TRuFA2FoV3AgknYvmL5CRVuOoX+V0RbkQEj6j7RoROoXL1/Z/2aaXgF1hPVJ",
    });

    m.insert((5, 2), BalancerTemplate {
        n_inputs: 5, n_outputs: 2, width: 5, height: 11,
        entities: T_5_2_ENTITIES, input_tiles: T_5_2_INPUT, output_tiles: T_5_2_OUTPUT,
        source_blueprint: "0eNqtlttugzAMQH+lynM35UYC+5WpmnqJqkg0oBCmVYh/n8eqrdpSbxQ/NZjg46Q+Sga2q3vXRh8Se1oNzO+b0MHoeWCdP4ZtPUXTuXUwYD65E1uvWNiepueurX1KLrIRgj4c3BtExbhZX6bClO/0EHx1sfNNgLgshba6ssYKbgoD71xIPnl3gU9P55fQn3aQHpLCjLbpYMb0+cA+SPyxgPD5czRe1ZXiNnRtE9PDztUT+eCj218+ljD1N0HmCIqSoHIETUnQOYKhJBQ5gqUkmByhpCTYHEGQNlOJt6tYTqiyi7gi8DEn6r9yC55LLinLFwLXDUfkU0rcL4qqswoXRLuucbco6s/qWxHVb/7wimIBFhdLEiBKvPkpEBXe/AQIyXEZ5Hy/pMCbn6JqiR9gckl/SoWfLMuSa9ysWZuj84gC731FsP8GP1XUoi2yeNdT1F/iXf8D0cONNR5jA795CDx/XX1D298So8LFUPNdUxwXYcFCmj7dXIkS+BG06P9XEldELVdEKVwRTXCJ17giFIgCP4EoEAZ3Ud/Rsha34J6UJW7BrI0weUSFdzwBQnP83KFACFytmYjNOL4DrQXooQ==",
    });

    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_template_count() {
        let templates = balancer_templates();
        // 17 templates in the Python reference
        assert_eq!(templates.len(), 17);
    }

    #[test]
    fn test_2_1_template_shape() {
        let t = balancer_templates().get(&(2, 1)).expect("(2,1) template missing");
        assert_eq!(t.n_inputs, 2);
        assert_eq!(t.n_outputs, 1);
        assert_eq!(t.width, 2);
        assert_eq!(t.height, 3);
        assert_eq!(t.input_tiles, &[(0, 0), (1, 0)]);
        assert_eq!(t.output_tiles, &[(1, 2)]);
    }

    #[test]
    fn test_2_1_entities_count() {
        let t = balancer_templates().get(&(2, 1)).expect("(2,1) template missing");
        assert_eq!(t.entities.len(), 4);
    }

    #[test]
    fn test_2_1_stamp_entity_count() {
        let t = balancer_templates().get(&(2, 1)).expect("(2,1) template missing");
        let entities = t.stamp(10, 20, "transport-belt", "splitter", "underground-belt", Some("iron-plate"));
        assert_eq!(entities.len(), 4);
    }

    #[test]
    fn test_2_1_stamp_positions() {
        let t = balancer_templates().get(&(2, 1)).expect("(2,1) template missing");
        let entities = t.stamp(5, 3, "transport-belt", "splitter", "underground-belt", None);
        for (ent, tmpl) in entities.iter().zip(t.entities.iter()) {
            assert_eq!(ent.x, 5 + tmpl.x, "x mismatch for entity {}", tmpl.name);
            assert_eq!(ent.y, 3 + tmpl.y, "y mismatch for entity {}", tmpl.name);
        }
    }

    #[test]
    fn test_2_1_stamp_directions() {
        let t = balancer_templates().get(&(2, 1)).expect("(2,1) template missing");
        let entities = t.stamp(0, 0, "transport-belt", "splitter", "underground-belt", None);
        for ent in &entities {
            assert_eq!(ent.direction, EntityDirection::South, "expected South for {}", ent.name);
        }
    }

    #[test]
    fn test_direction_conversion() {
        let north = BalancerTemplateEntity { name: "transport-belt", x: 0, y: 0, direction: 0, io_type: None };
        let east  = BalancerTemplateEntity { name: "transport-belt", x: 0, y: 0, direction: 2, io_type: None };
        let south = BalancerTemplateEntity { name: "transport-belt", x: 0, y: 0, direction: 4, io_type: None };
        let west  = BalancerTemplateEntity { name: "transport-belt", x: 0, y: 0, direction: 6, io_type: None };
        assert_eq!(north.entity_direction(), EntityDirection::North);
        assert_eq!(east.entity_direction(),  EntityDirection::East);
        assert_eq!(south.entity_direction(), EntityDirection::South);
        assert_eq!(west.entity_direction(),  EntityDirection::West);
    }

    #[test]
    fn test_stamp_carries_item() {
        let t = balancer_templates().get(&(2, 1)).expect("(2,1) template missing");
        let entities = t.stamp(0, 0, "transport-belt", "splitter", "underground-belt", Some("copper-plate"));
        for ent in &entities {
            assert_eq!(ent.carries.as_deref(), Some("copper-plate"));
        }
    }

    #[test]
    fn test_stamp_belt_tier_substitution() {
        let t = balancer_templates().get(&(2, 1)).expect("(2,1) template missing");
        let entities = t.stamp(0, 0, "fast-transport-belt", "fast-splitter", "fast-underground-belt", None);
        for ent in &entities {
            assert!(
                ent.name == "fast-transport-belt" || ent.name == "fast-splitter" || ent.name == "fast-underground-belt",
                "unexpected name: {}",
                ent.name
            );
        }
    }

    #[test]
    fn test_all_templates_have_correct_io_counts() {
        let templates = balancer_templates();
        for ((n, m), t) in templates {
            assert_eq!(t.n_inputs, *n, "n_inputs mismatch for ({n},{m})");
            assert_eq!(t.n_outputs, *m, "n_outputs mismatch for ({n},{m})");
            assert_eq!(
                t.input_tiles.len() as u32, *n,
                "input_tiles.len mismatch for ({n},{m})"
            );
            assert_eq!(
                t.output_tiles.len() as u32, *m,
                "output_tiles.len mismatch for ({n},{m})"
            );
        }
    }

    #[test]
    fn test_1_2_template_shape() {
        let t = balancer_templates().get(&(1, 2)).expect("(1,2) template missing");
        assert_eq!(t.n_inputs, 1);
        assert_eq!(t.n_outputs, 2);
        assert_eq!(t.width, 2);
        assert_eq!(t.height, 3);
        assert_eq!(t.input_tiles, &[(1, 0)]);
        assert_eq!(t.output_tiles, &[(0, 2), (1, 2)]);
        assert_eq!(t.entities.len(), 4);
    }

    #[test]
    fn test_4_4_template_shape() {
        let t = balancer_templates().get(&(4, 4)).expect("(4,4) template missing");
        assert_eq!(t.width, 4);
        assert_eq!(t.height, 10);
        assert_eq!(t.entities.len(), 32);
    }

    #[test]
    fn test_underground_belt_io_type_preserved() {
        // (2,3) template has underground belts with io_type
        let t = balancer_templates().get(&(2, 3)).expect("(2,3) template missing");
        let ug_entities: Vec<_> = t.entities.iter().filter(|e| e.name == "underground-belt").collect();
        assert!(!ug_entities.is_empty(), "should have underground belts");
        for e in &ug_entities {
            assert!(e.io_type.is_some(), "underground belt missing io_type");
        }
    }

    #[test]
    fn test_stamp_underground_io_type_preserved() {
        let t = balancer_templates().get(&(2, 3)).expect("(2,3) template missing");
        let stamped = t.stamp(0, 0, "transport-belt", "splitter", "underground-belt", None);
        let ug: Vec<_> = stamped.iter().filter(|e| e.name == "underground-belt").collect();
        assert!(!ug.is_empty());
        for e in &ug {
            assert!(e.io_type.is_some());
        }
    }

    #[test]
    fn test_missing_template_returns_none() {
        let templates = balancer_templates();
        assert!(templates.get(&(99, 99)).is_none());
    }

    #[test]
    fn test_source_blueprint_nonempty() {
        for (key, t) in balancer_templates() {
            assert!(!t.source_blueprint.is_empty(), "source_blueprint empty for {key:?}");
        }
    }
}
