//! Factorio blueprint string → LayoutResult.
//!
//! Reverse of `blueprint.rs`. Decodes `"0" + base64(zlib(JSON))` and converts
//! the Factorio entity format (center-based float positions, raw direction ints)
//! into our tile-grid `LayoutResult`.

use std::io::Read;

use base64::Engine;
use flate2::read::ZlibDecoder;
use serde::Deserialize;

use crate::models::{EntityDirection, LayoutResult, PlacedEntity};

// ---- Raw Factorio blueprint JSON types ----

#[derive(Deserialize)]
struct BpRoot {
    blueprint: Option<BpData>,
    blueprint_book: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct BpData {
    #[serde(default)]
    entities: Vec<BpEntity>,
}

#[derive(Deserialize)]
struct BpEntity {
    name: String,
    position: BpPosition,
    #[serde(default)]
    direction: u8,
    recipe: Option<String>,
    /// "input" | "output" for underground belts / pipe-to-ground
    #[serde(rename = "type")]
    io_type: Option<String>,
}

#[derive(Deserialize)]
struct BpPosition {
    x: f64,
    y: f64,
}

// ---- Entity footprint lookup ----

/// Returns (width_tiles, height_tiles) for entities that aren't 1×1.
/// Direction is needed for splitters (2 tiles perpendicular to flow).
fn entity_footprint(name: &str, direction: EntityDirection) -> (i32, i32) {
    match name {
        "assembling-machine-1"
        | "assembling-machine-2"
        | "assembling-machine-3"
        | "chemical-plant"
        | "electric-furnace"
        | "centrifuge"
        | "lab"
        | "beacon"
        | "storage-tank"
        | "electric-mining-drill"
        | "biochamber" => (3, 3),

        "oil-refinery" | "foundry" | "biolab" | "cryogenic-plant" => (5, 5),
        "rocket-silo" => (9, 9),
        "big-electric-pole" | "substation" | "steel-furnace" => (2, 2),
        "electromagnetic-plant" => (4, 4),
        "recycler" => (2, 4),
        "crusher" => (2, 3),

        // Splitters: 2 tiles wide perpendicular to flow direction
        "splitter" | "fast-splitter" | "express-splitter" => {
            match direction {
                EntityDirection::North | EntityDirection::South => (2, 1),
                EntityDirection::East | EntityDirection::West => (1, 2),
            }
        }

        _ => (1, 1),
    }
}

// ---- Direction parsing ----

/// Map Factorio direction integer to EntityDirection.
///
/// Modern Factorio (≥0.17) uses 0/4/8/12 for N/E/S/W.
/// Older versions (0-7 eight-way): 0=N, 2=E, 4=S, 6=W.
/// We handle both by treating even values in 0-6 range as old-format
/// and values 8/12 as unambiguously modern.
fn parse_direction(d: u8) -> EntityDirection {
    match d {
        0 => EntityDirection::North,
        4 => EntityDirection::East,
        8 => EntityDirection::South,
        12 => EntityDirection::West,
        // Old 8-way format
        2 => EntityDirection::East,
        6 => EntityDirection::West,
        _ => EntityDirection::North,
    }
}

// ---- Public API ----

/// Parse a Factorio blueprint string into a `LayoutResult`.
///
/// The blueprint string must start with `'0'` (Factorio's version prefix).
/// Returns an error if the string is malformed or is a blueprint book.
///
/// Entity positions are normalized to start at (0, 0).
pub fn parse_blueprint_string(bp: &str) -> Result<LayoutResult, String> {
    let bp = bp.trim();
    if !bp.starts_with('0') {
        return Err("Blueprint string must start with '0'".into());
    }

    // Decode base64 → zlib decompress → JSON string
    let b64 = &bp[1..];
    let compressed = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| format!("base64 decode error: {e}"))?;

    let mut decoder = ZlibDecoder::new(&compressed[..]);
    let mut json_str = String::new();
    decoder
        .read_to_string(&mut json_str)
        .map_err(|e| format!("zlib decompress error: {e}"))?;

    let root: BpRoot =
        serde_json::from_str(&json_str).map_err(|e| format!("JSON parse error: {e}"))?;

    if root.blueprint_book.is_some() {
        return Err("Blueprint books are not supported — extract individual blueprints first".into());
    }

    let bp_data = root
        .blueprint
        .ok_or("not a blueprint (missing 'blueprint' key)")?;

    // Convert entities
    let mut entities: Vec<PlacedEntity> = Vec::with_capacity(bp_data.entities.len());

    for raw in bp_data.entities {
        let dir = parse_direction(raw.direction);
        let (w, h) = entity_footprint(&raw.name, dir);

        // Factorio stores center position; convert to top-left tile
        let x = (raw.position.x - w as f64 / 2.0).round() as i32;
        let y = (raw.position.y - h as f64 / 2.0).round() as i32;

        entities.push(PlacedEntity {
            name: raw.name,
            x,
            y,
            direction: dir,
            recipe: raw.recipe,
            io_type: raw.io_type,
            carries: None,
            mirror: false,
            segment_id: None,
            rate: None,
        });
    }

    if entities.is_empty() {
        return Ok(LayoutResult::default());
    }

    // Compute bounding box and normalize to (0, 0)
    let min_x = entities.iter().map(|e| e.x).min().unwrap_or(0);
    let min_y = entities.iter().map(|e| e.y).min().unwrap_or(0);

    for e in &mut entities {
        e.x -= min_x;
        e.y -= min_y;
    }

    // Width/height from the adjusted max extents
    let max_x = entities
        .iter()
        .map(|e| {
            let (w, _) = entity_footprint(&e.name, e.direction);
            e.x + w - 1
        })
        .max()
        .unwrap_or(0);
    let max_y = entities
        .iter()
        .map(|e| {
            let (_, h) = entity_footprint(&e.name, e.direction);
            e.y + h - 1
        })
        .max()
        .unwrap_or(0);

    Ok(LayoutResult {
        entities,
        width: max_x + 1,
        height: max_y + 1,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blueprint;
    use crate::models::{EntityDirection, PlacedEntity};

    #[test]
    fn round_trip_simple() {
        let layout = LayoutResult {
            entities: vec![
                PlacedEntity {
                    name: "assembling-machine-2".into(),
                    x: 0,
                    y: 0,
                    direction: EntityDirection::North,
                    recipe: Some("iron-gear-wheel".into()),
                    ..Default::default()
                },
                PlacedEntity {
                    name: "transport-belt".into(),
                    x: 3,
                    y: 1,
                    direction: EntityDirection::East,
                    ..Default::default()
                },
                PlacedEntity {
                    name: "underground-belt".into(),
                    x: 4,
                    y: 1,
                    direction: EntityDirection::East,
                    io_type: Some("input".into()),
                    ..Default::default()
                },
            ],
            width: 5,
            height: 3,
            ..Default::default()
        };

        let bp_string = blueprint::export(&layout, "test");
        let parsed = parse_blueprint_string(&bp_string).expect("should parse");

        // After round-trip, entities should be at the same positions
        // (origin may shift if export uses center positions for multi-tile)
        assert_eq!(parsed.entities.len(), 3);

        // Find the assembling machine
        let machine = parsed
            .entities
            .iter()
            .find(|e| e.name == "assembling-machine-2")
            .expect("should have assembling machine");
        assert_eq!(machine.recipe.as_deref(), Some("iron-gear-wheel"));

        // Find the underground belt
        let ug = parsed
            .entities
            .iter()
            .find(|e| e.name == "underground-belt")
            .expect("should have underground belt");
        assert_eq!(ug.io_type.as_deref(), Some("input"));
        assert!(matches!(ug.direction, EntityDirection::East));
    }

    #[test]
    fn rejects_non_blueprint() {
        assert!(parse_blueprint_string("1invalidstring").is_err());
        assert!(parse_blueprint_string("").is_err());
    }
}
