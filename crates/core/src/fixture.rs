//! SAT-zone fixture schema (v1).
//!
//! Shared between the regression test harness
//! (`crates/core/tests/sat_fixtures.rs`), the WASM `solve_fixture`
//! binding, and the in-canvas SAT-zone editor. See
//! `crates/core/tests/sat_fixtures/README.md` for the schema docs and
//! the workflow for adding new fixtures.

use crate::models::EntityDirection;
use crate::sat::{CrossingZone, ZoneBoundary};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Fixture {
    pub version: u32,
    pub name: String,
    pub notes: Option<String>,
    pub source_url: Option<String>,
    pub seed: [i32; 2],
    pub bbox: FixtureBbox,
    #[serde(default)]
    pub forbidden: Vec<[i32; 2]>,
    pub belt_tier: String,
    pub max_reach: u32,
    pub boundaries: Vec<FixtureBoundary>,
    pub expected: FixtureExpected,
    /// Informational only in v1 — carried along but not consumed.
    pub context: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct FixtureBbox {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

#[derive(Debug, Deserialize)]
pub struct FixtureBoundary {
    pub x: i32,
    pub y: i32,
    pub dir: String,
    pub item: String,
    #[serde(rename = "in")]
    pub is_input: bool,
    #[serde(default)]
    pub interior: bool,
}

#[derive(Debug, Deserialize)]
pub struct FixtureExpected {
    pub mode: String,
    /// Optional quality bar for the `solve` mode: the test fails if
    /// the solver's `solution_cost` exceeds this. Absent on legacy
    /// fixtures — they only get the mode-based check.
    #[serde(default)]
    pub max_cost: Option<u32>,
}

pub fn parse_direction(dir: &str) -> EntityDirection {
    match dir {
        "North" => EntityDirection::North,
        "East" => EntityDirection::East,
        "South" => EntityDirection::South,
        "West" => EntityDirection::West,
        other => panic!("unknown direction in fixture: {other:?}"),
    }
}

pub fn build_zone(fixture: &Fixture) -> CrossingZone {
    let boundaries: Vec<ZoneBoundary> = fixture
        .boundaries
        .iter()
        .map(|b| ZoneBoundary {
            x: b.x,
            y: b.y,
            direction: parse_direction(&b.dir),
            item: b.item.clone(),
            is_input: b.is_input,
            interior: b.interior,
        })
        .collect();

    let forced_empty: Vec<(i32, i32)> = fixture
        .forbidden
        .iter()
        .map(|&[x, y]| (x, y))
        .collect();

    CrossingZone {
        x: fixture.bbox.x,
        y: fixture.bbox.y,
        width: fixture.bbox.w,
        height: fixture.bbox.h,
        boundaries,
        forced_empty,
    }
}
