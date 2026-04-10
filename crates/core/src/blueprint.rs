//! LayoutResult → Factorio blueprint string.
//!
//! Format: `"0" + base64(zlib(JSON))`. The `"0"` is Factorio's version byte.

use std::io::Write;

use base64::Engine;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use serde::Serialize;

use crate::common::{is_machine_entity, machine_size};
use crate::models::LayoutResult;

#[derive(Serialize)]
struct Position {
    x: f64,
    y: f64,
}

#[derive(Serialize)]
struct BlueprintEntity<'a> {
    entity_number: usize,
    name: &'a str,
    position: Position,
    direction: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    recipe: Option<&'a str>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    io_type: Option<&'a str>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    mirror: bool,
}

#[derive(Serialize)]
struct Blueprint<'a> {
    icons: [(); 0],
    entities: Vec<BlueprintEntity<'a>>,
    item: &'static str,
    version: u64,
    label: &'a str,
}

#[derive(Serialize)]
struct BlueprintWrapper<'a> {
    blueprint: Blueprint<'a>,
}

/// Convert a `LayoutResult` into an importable Factorio blueprint string.
pub fn export(layout: &LayoutResult, label: &str) -> String {
    let entities: Vec<BlueprintEntity> = layout
        .entities
        .iter()
        .enumerate()
        .map(|(i, ent)| BlueprintEntity {
            entity_number: i + 1,
            name: &ent.name,
            position: {
                let size = if is_machine_entity(&ent.name) {
                    machine_size(&ent.name) as f64
                } else {
                    1.0
                };
                Position {
                    x: ent.x as f64 + size / 2.0,
                    y: ent.y as f64 + size / 2.0,
                }
            },
            direction: ent.direction as u8,
            recipe: ent.recipe.as_deref(),
            io_type: ent.io_type.as_deref(),
            mirror: ent.mirror,
        })
        .collect();

    let wrapper = BlueprintWrapper {
        blueprint: Blueprint {
            icons: [],
            entities,
            item: "blueprint",
            version: 562949955518464,
            label,
        },
    };

    let json = serde_json::to_vec(&wrapper).expect("blueprint serialization cannot fail");

    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&json).expect("zlib write cannot fail");
    let compressed = encoder.finish().expect("zlib finish cannot fail");

    let b64 = base64::engine::general_purpose::STANDARD.encode(&compressed);
    format!("0{b64}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{EntityDirection, PlacedEntity};
    use std::io::Read;

    #[test]
    fn round_trip_small_fixture() {
        let layout = LayoutResult {
            entities: vec![
                PlacedEntity {
                    name: "assembling-machine-3".into(),
                    x: 0,
                    y: 0,
                    direction: EntityDirection::North,
                    recipe: Some("iron-gear-wheel".into()),
                    io_type: None,
                    carries: None,
                    mirror: false,
                    segment_id: None,
                    ..Default::default()
                },
                PlacedEntity {
                    name: "transport-belt".into(),
                    x: 3,
                    y: 0,
                    direction: EntityDirection::East,
                    recipe: None,
                    io_type: None,
                    carries: None,
                    mirror: false,
                    segment_id: None,
                    ..Default::default()
                },
            ],
            width: 4,
            height: 1,
            ..Default::default()
        };
        let s = export(&layout, "test");

        assert!(s.starts_with('0'));
        let b64 = &s[1..];
        let compressed = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .unwrap();
        let mut decoder = flate2::read::ZlibDecoder::new(&compressed[..]);
        let mut json_str = String::new();
        decoder.read_to_string(&mut json_str).unwrap();

        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["blueprint"]["label"], "test");
        let ents = parsed["blueprint"]["entities"].as_array().unwrap();
        assert_eq!(ents.len(), 2);
        assert_eq!(ents[0]["name"], "assembling-machine-3");
        assert_eq!(ents[0]["recipe"], "iron-gear-wheel");
        assert_eq!(ents[0]["entity_number"], 1);
        assert_eq!(ents[0]["direction"], 0);
        assert_eq!(ents[1]["entity_number"], 2);
        assert_eq!(ents[1]["direction"], 4);
        // 3x3 assembler at (0,0) → center at (1.5, 1.5)
        assert_eq!(ents[0]["position"]["x"], 1.5);
        assert_eq!(ents[0]["position"]["y"], 1.5);
        // 1x1 belt at (3,0) → center at (3.5, 0.5)
        assert_eq!(ents[1]["position"]["x"], 3.5);
        // mirror should be absent when false
        assert!(ents[0].get("mirror").is_none());
        // recipe should be absent for belt
        assert!(ents[1].get("recipe").is_none());
    }
}
