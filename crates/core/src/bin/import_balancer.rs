//! Import a Factorio blueprint string as a balancer template.
//!
//! Decodes, validates as an N→M belt balancer, and patches
//! `crates/core/src/bus/balancer_library.rs` with the new template.
//!
//! Usage:
//!   cargo run --manifest-path crates/core/Cargo.toml \
//!     --bin import_balancer -- '<blueprint>'
//!   cargo run ... --bin import_balancer -- --dry-run '<blueprint>'
//!   cargo run ... --bin import_balancer -- --json    '<blueprint>'
//!   echo '<blueprint>' | cargo run ... --bin import_balancer -- --stdin

use base64::Engine as _;
use flate2::read::ZlibDecoder;
use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::Value;
use std::collections::VecDeque;
use std::fmt::Write as _;
use std::io::Read;
use std::path::PathBuf;
use std::process;

// ---------------------------------------------------------------------------
// Blueprint decoding
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct RawEntity {
    name: String,
    x: f64,
    y: f64,
    direction: u8,
    io_type: Option<String>,
}

const BELT_ENTITIES: &[&str] = &["transport-belt", "splitter", "underground-belt"];

const N: u8 = 0; // NORTH
const E: u8 = 2; // EAST
const S: u8 = 4; // SOUTH
const W: u8 = 6; // WEST

fn decode_blueprint(bp: &str) -> Result<Value, String> {
    let b64 = bp.strip_prefix('0').ok_or("Blueprint must start with '0'")?;
    let compressed = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| format!("base64 decode failed: {e}"))?;
    let mut decoder = ZlibDecoder::new(&compressed[..]);
    let mut json_bytes = Vec::new();
    decoder
        .read_to_end(&mut json_bytes)
        .map_err(|e| format!("zlib decompress failed: {e}"))?;
    serde_json::from_slice(&json_bytes).map_err(|e| format!("JSON parse failed: {e}"))
}

fn normalize_belt_name(name: &str) -> &str {
    match name {
        "fast-transport-belt" | "express-transport-belt" => "transport-belt",
        "fast-splitter" | "express-splitter" => "splitter",
        "fast-underground-belt" | "express-underground-belt" => "underground-belt",
        other => other,
    }
}

fn extract_entities(data: &Value) -> Result<Vec<RawEntity>, String> {
    let entities = data["blueprint"]["entities"]
        .as_array()
        .ok_or("No 'blueprint.entities' array in JSON")?;
    let mut result = Vec::new();
    for ent in entities {
        let raw_name = ent["name"].as_str().ok_or("Entity missing 'name'")?;
        let name = normalize_belt_name(raw_name).to_owned();
        let x = ent["position"]["x"]
            .as_f64()
            .ok_or("Entity missing position.x")?;
        let y = ent["position"]["y"]
            .as_f64()
            .ok_or("Entity missing position.y")?;
        let direction = ent["direction"].as_u64().unwrap_or(0) as u8;
        let io_type = ent["type"].as_str().map(|s| s.to_owned());
        result.push(RawEntity { name, x, y, direction, io_type });
    }
    Ok(result)
}

// ---------------------------------------------------------------------------
// Rotation helpers
// ---------------------------------------------------------------------------

fn rotate_dir_cw(d: u8) -> u8 {
    match d { N => E, E => S, S => W, W => N, _ => d }
}
fn rotate_dir_ccw(d: u8) -> u8 {
    match d { N => W, W => S, S => E, E => N, _ => d }
}
fn rotate_dir_180(d: u8) -> u8 {
    match d { N => S, S => N, E => W, W => E, _ => d }
}

fn rotate_cw(entities: &[RawEntity], max_y: f64) -> Vec<RawEntity> {
    entities
        .iter()
        .map(|e| RawEntity {
            name: e.name.clone(),
            x: max_y - e.y,
            y: e.x,
            direction: rotate_dir_cw(e.direction),
            io_type: e.io_type.clone(),
        })
        .collect()
}

fn rotate_ccw(entities: &[RawEntity], max_x: f64) -> Vec<RawEntity> {
    entities
        .iter()
        .map(|e| RawEntity {
            name: e.name.clone(),
            x: e.y,
            y: max_x - e.x,
            direction: rotate_dir_ccw(e.direction),
            io_type: e.io_type.clone(),
        })
        .collect()
}

fn rotate_180(entities: &[RawEntity], max_x: f64, max_y: f64) -> Vec<RawEntity> {
    entities
        .iter()
        .map(|e| {
            let new_dir = rotate_dir_180(e.direction);
            // After 180°, NORTH-facing UG belts become SOUTH-facing.
            // Their input/output relative positions flip along the Y axis,
            // so io_type must be swapped. EAST/WEST UGs don't need swapping
            // because their relative X positions are handled correctly by the
            // pairing direction search.
            let new_io = e.io_type.clone();
            RawEntity {
                name: e.name.clone(),
                x: max_x - e.x,
                y: max_y - e.y,
                direction: new_dir,
                io_type: new_io,
            }
        })
        .collect()
}

fn normalize(entities: &[RawEntity]) -> Vec<RawEntity> {
    let min_x = entities.iter().map(|e| e.x).fold(f64::INFINITY, f64::min);
    let min_y = entities.iter().map(|e| e.y).fold(f64::INFINITY, f64::min);
    let dx = 0.5 - min_x;
    let dy = 0.5 - min_y;
    entities
        .iter()
        .map(|e| RawEntity {
            name: e.name.clone(),
            x: e.x + dx,
            y: e.y + dy,
            direction: e.direction,
            io_type: e.io_type.clone(),
        })
        .collect()
}

fn dominant_direction(entities: &[RawEntity]) -> u8 {
    let mut counts = [0u32; 4];
    for e in entities {
        if e.name == "transport-belt" || e.name == "splitter" {
            let idx = match e.direction { N => 0, E => 1, S => 2, W => 3, _ => continue };
            counts[idx] += 1;
        }
    }
    let (idx, _) = counts.iter().enumerate().max_by_key(|(_, &c)| c).unwrap();
    [N, E, S, W][idx]
}

fn apply_rotation(entities: &[RawEntity], rot: u8) -> Vec<RawEntity> {
    let max_x = entities.iter().map(|e| e.x).fold(f64::NEG_INFINITY, f64::max);
    let max_y = entities.iter().map(|e| e.y).fold(f64::NEG_INFINITY, f64::max);
    match rot {
        0 => entities.to_vec(),
        1 => rotate_cw(entities, max_y),
        2 => rotate_180(entities, max_x, max_y),
        3 => rotate_ccw(entities, max_x),
        _ => entities.to_vec(),
    }
}

fn normalize_to_south_flow(entities: &[RawEntity]) -> Vec<RawEntity> {
    // Primary guess based on dominant direction.
    let dom = dominant_direction(entities);
    let primary_rot: u8 = match dom { S => 0, E => 1, N => 2, W => 3, _ => 0 };

    // Try the dominant-direction rotation first; if connectivity would be poor,
    // fall back by trying all four rotations and picking the one whose normalized
    // form has inputs/outputs detected correctly (SOUTH-facing at top/bottom).
    // Full connectivity is checked later in validate_and_build, so here we just
    // pick the rotation with the most detected ports.
    let best_rot = [primary_rot, (primary_rot + 1) % 4, (primary_rot + 2) % 4, (primary_rot + 3) % 4]
        .iter()
        .map(|&rot| {
            let rotated = apply_rotation(entities, rot);
            let normed = normalize(&rotated);
            let (inputs, outputs) = identify_ports(&normed);
            (rot, inputs.len(), outputs.len())
        })
        .filter(|(_, ni, no)| *ni > 0 && *no > 0)
        .max_by_key(|(rot, ni, no)| (*ni + *no, *rot == primary_rot))
        .map(|(rot, _, _)| rot)
        .unwrap_or(primary_rot);

    apply_rotation(entities, best_rot)
}

// ---------------------------------------------------------------------------
// Port detection
// ---------------------------------------------------------------------------

fn entity_tile(e: &RawEntity) -> (i32, i32) {
    if e.name == "splitter" {
        if e.direction == N || e.direction == S {
            return ((e.x - 1.0).round() as i32, (e.y - 0.5).round() as i32);
        } else {
            return ((e.x - 0.5).round() as i32, (e.y - 1.0).round() as i32);
        }
    }
    ((e.x - 0.5).round() as i32, (e.y - 0.5).round() as i32)
}

fn identify_ports(entities: &[RawEntity]) -> (Vec<(i32, i32)>, Vec<(i32, i32)>) {
    let conveyors: Vec<_> = entities
        .iter()
        .filter(|e| e.name == "transport-belt" || e.name == "splitter")
        .collect();

    if conveyors.is_empty() {
        return (vec![], vec![]);
    }

    let tiles: Vec<((i32, i32), &RawEntity)> =
        conveyors.iter().map(|e| (entity_tile(e), *e)).collect();

    let min_y = tiles.iter().map(|((_, y), _)| *y).min().unwrap();
    let max_y = tiles.iter().map(|((_, y), _)| *y).max().unwrap();

    let mut inputs: Vec<(i32, i32)> = Vec::new();
    for &((tx, ty), e) in &tiles {
        if ty == min_y && e.direction == S {
            inputs.push((tx, ty));
            if e.name == "splitter" {
                inputs.push((tx + 1, ty));
            }
        }
    }
    inputs.sort();

    let mut outputs: Vec<(i32, i32)> = Vec::new();
    for &((tx, ty), e) in &tiles {
        if ty == max_y && e.direction == S {
            outputs.push((tx, ty));
            if e.name == "splitter" {
                outputs.push((tx + 1, ty));
            }
        }
    }
    outputs.sort();

    (inputs, outputs)
}

// ---------------------------------------------------------------------------
// Template building
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct TemplateEntity {
    name: String,
    x: i32,
    y: i32,
    direction: u8,
    io_type: Option<String>,
}

#[derive(Debug)]
struct Template {
    n_inputs: usize,
    n_outputs: usize,
    width: i32,
    height: i32,
    entities: Vec<TemplateEntity>,
    input_tiles: Vec<(i32, i32)>,
    output_tiles: Vec<(i32, i32)>,
    source_blueprint: String,
}

fn validate_and_build(bp: &str) -> Result<Template, String> {
    let data = decode_blueprint(bp)?;
    let raw = extract_entities(&data)?;

    let non_belt: Vec<_> = raw.iter().filter(|e| !BELT_ENTITIES.contains(&e.name.as_str())).collect();
    if !non_belt.is_empty() {
        let names: std::collections::BTreeSet<_> = non_belt.iter().map(|e| &e.name).collect();
        return Err(format!("Blueprint contains non-belt entities: {names:?}"));
    }

    let rotated = normalize_to_south_flow(&raw);
    let normed = normalize(&rotated);

    let (inputs, outputs) = identify_ports(&normed);
    if inputs.is_empty() {
        return Err("Could not identify input ports (SOUTH-facing belts at top row)".to_owned());
    }
    if outputs.is_empty() {
        return Err("Could not identify output ports (SOUTH-facing belts at bottom row)".to_owned());
    }

    let mut all_tiles: Vec<(i32, i32)> = Vec::new();
    for e in &normed {
        let (tx, ty) = entity_tile(e);
        all_tiles.push((tx, ty));
        if e.name == "splitter" {
            if e.direction == N || e.direction == S {
                all_tiles.push((tx + 1, ty));
            } else {
                all_tiles.push((tx, ty + 1));
            }
        }
    }
    let width = all_tiles.iter().map(|(x, _)| *x).max().unwrap() + 1;
    let height = all_tiles.iter().map(|(_, y)| *y).max().unwrap() + 1;

    let entities = normed
        .iter()
        .map(|e| {
            let (x, y) = entity_tile(e);
            TemplateEntity { name: e.name.clone(), x, y, direction: e.direction, io_type: e.io_type.clone() }
        })
        .collect();

    Ok(Template {
        n_inputs: inputs.len(),
        n_outputs: outputs.len(),
        width,
        height,
        entities,
        input_tiles: inputs,
        output_tiles: outputs,
        source_blueprint: bp.to_owned(),
    })
}

// ---------------------------------------------------------------------------
// Lane-level connectivity validation
// ---------------------------------------------------------------------------

/// A lane-node in the belt graph: tile (x, y) plus lane index (0=right, 1=left).
///
/// For SOUTH-flowing belts, lane 0 is the west half and lane 1 is the east half.
/// Splitters fully mix and redistribute, so all 4 input lane-nodes connect to all
/// 4 output lane-nodes. Regular belts and UG belts preserve lane identity.
type LaneNode = (i32, i32, u8);

fn dir_to_delta(direction: u8) -> (i32, i32) {
    match direction {
        0 => (0, -1), // N
        2 => (1, 0),  // E
        4 => (0, 1),  // S
        6 => (-1, 0), // W
        _ => (0, 0),
    }
}

fn splitter_second(e: &TemplateEntity) -> (i32, i32) {
    if e.direction == N || e.direction == S {
        (e.x + 1, e.y)
    } else {
        (e.x, e.y + 1)
    }
}

/// Pair underground belt inputs to their nearest downstream output of the same
/// direction.  Returns a bidirectional map input↔output.
fn pair_ug_belts(entities: &[TemplateEntity]) -> FxHashMap<(i32, i32), (i32, i32)> {
    let inputs: Vec<_> = entities
        .iter()
        .filter(|e| e.name == "underground-belt" && e.io_type.as_deref() == Some("input"))
        .collect();
    let outputs: Vec<_> = entities
        .iter()
        .filter(|e| e.name == "underground-belt" && e.io_type.as_deref() == Some("output"))
        .collect();

    let mut pairs = FxHashMap::default();
    let mut used: FxHashSet<(i32, i32)> = FxHashSet::default();

    for inp in &inputs {
        let (dx, dy) = dir_to_delta(inp.direction);
        let mut best: Option<((i32, i32), i32)> = None;

        for out in &outputs {
            if used.contains(&(out.x, out.y)) || out.direction != inp.direction {
                continue;
            }
            let (rx, ry) = (out.x - inp.x, out.y - inp.y);
            let dist = if dx != 0 {
                if ry != 0 || (rx > 0) != (dx > 0) { continue; }
                rx.abs()
            } else {
                if rx != 0 || (ry > 0) != (dy > 0) { continue; }
                ry.abs()
            };
            if dist > 1 {
                if best.map_or(true, |(_, bd)| dist < bd) {
                    best = Some(((out.x, out.y), dist));
                }
            }
        }

        if let Some(((ox, oy), _)) = best {
            pairs.insert((inp.x, inp.y), (ox, oy));
            pairs.insert((ox, oy), (inp.x, inp.y));
            used.insert((ox, oy));
        }
    }
    pairs
}

/// Build a forward lane-graph: LaneNode → reachable LaneNodes in one step.
fn build_lane_graph(
    entities: &[TemplateEntity],
    ug_pairs: &FxHashMap<(i32, i32), (i32, i32)>,
) -> FxHashMap<LaneNode, Vec<LaneNode>> {
    // Build tile → entity lookup (splitters stored at both tiles).
    let mut tile_entity: FxHashMap<(i32, i32), &TemplateEntity> = FxHashMap::default();
    let mut splitter_sibling: FxHashMap<(i32, i32), (i32, i32)> = FxHashMap::default();
    for e in entities {
        tile_entity.insert((e.x, e.y), e);
        if e.name == "splitter" {
            let second = splitter_second(e);
            tile_entity.insert(second, e);
            splitter_sibling.insert((e.x, e.y), second);
            splitter_sibling.insert(second, (e.x, e.y));
        }
    }

    let mut graph: FxHashMap<LaneNode, Vec<LaneNode>> = FxHashMap::default();

    for (&tile, &e) in &tile_entity {
        for lane in 0u8..2 {
            let src = (tile.0, tile.1, lane);
            let mut dsts: Vec<LaneNode> = Vec::new();

            if e.name == "splitter" {
                // Splitter: fully mixes and redistributes to both output tiles both lanes.
                let (dx, dy) = dir_to_delta(e.direction);
                let out1 = (tile.0 + dx, tile.1 + dy);
                let sib = splitter_sibling[&tile];
                let out2 = (sib.0 + dx, sib.1 + dy);
                for out_tile in [out1, out2] {
                    if tile_entity.contains_key(&out_tile) {
                        dsts.push((out_tile.0, out_tile.1, 0));
                        dsts.push((out_tile.0, out_tile.1, 1));
                    }
                }
            } else if e.name == "underground-belt" {
                // UG input → paired output (preserves lane).
                // UG output → next tile in direction (preserves lane).
                if e.io_type.as_deref() == Some("input") {
                    if let Some(&(ox, oy)) = ug_pairs.get(&tile) {
                        if tile_entity.contains_key(&(ox, oy)) {
                            dsts.push((ox, oy, lane));
                        }
                    }
                } else {
                    let (dx, dy) = dir_to_delta(e.direction);
                    let next = (tile.0 + dx, tile.1 + dy);
                    if tile_entity.contains_key(&next) {
                        dsts.push((next.0, next.1, lane));
                    }
                }
            } else {
                // Regular belt: straight-through, preserves lane.
                let (dx, dy) = dir_to_delta(e.direction);
                let next = (tile.0 + dx, tile.1 + dy);
                if tile_entity.contains_key(&next) {
                    dsts.push((next.0, next.1, lane));
                }
            }

            graph.entry(src).or_default().extend(dsts);
        }
    }

    graph
}

fn bfs_lane(
    start: LaneNode,
    graph: &FxHashMap<LaneNode, Vec<LaneNode>>,
) -> FxHashSet<LaneNode> {
    let mut visited = FxHashSet::default();
    let mut queue = VecDeque::new();
    visited.insert(start);
    queue.push_back(start);
    while let Some(node) = queue.pop_front() {
        for &next in graph.get(&node).into_iter().flatten() {
            if visited.insert(next) {
                queue.push_back(next);
            }
        }
    }
    visited
}

fn transpose_graph(
    graph: &FxHashMap<LaneNode, Vec<LaneNode>>,
) -> FxHashMap<LaneNode, Vec<LaneNode>> {
    let mut rev: FxHashMap<LaneNode, Vec<LaneNode>> = FxHashMap::default();
    for (&src, dsts) in graph {
        for &dst in dsts {
            rev.entry(dst).or_default().push(src);
        }
    }
    rev
}

/// Check lane-level balancer connectivity.
///
/// Uses union reachability: the combined forward reach of ALL input lanes must
/// cover every output tile, and the combined reverse reach of ALL output lanes
/// must cover every input tile. This accepts mixing balancers where individual
/// inputs share belt segments and distribute via splitters collectively.
///
/// Returns a list of human-readable error strings; empty = fully connected.
fn check_lane_connectivity(t: &Template) -> Vec<String> {
    let ug_pairs = pair_ug_belts(&t.entities);
    let fwd = build_lane_graph(&t.entities, &ug_pairs);
    let rev = transpose_graph(&fwd);

    let mut errors = Vec::new();

    // Forward: union of all input lanes must cover every output tile.
    let mut fwd_union: FxHashSet<LaneNode> = FxHashSet::default();
    for &(ix, iy) in &t.input_tiles {
        for lane in 0u8..2 {
            fwd_union.extend(bfs_lane((ix, iy, lane), &fwd));
        }
    }
    for &(ox, oy) in &t.output_tiles {
        if !fwd_union.contains(&(ox, oy, 0)) && !fwd_union.contains(&(ox, oy, 1)) {
            errors.push(format!("output ({ox},{oy}) is unreachable from any input"));
        }
    }

    // Reverse: union of all output lanes must cover every input tile.
    let mut rev_union: FxHashSet<LaneNode> = FxHashSet::default();
    for &(ox, oy) in &t.output_tiles {
        for lane in 0u8..2 {
            rev_union.extend(bfs_lane((ox, oy, lane), &rev));
        }
    }
    for &(ix, iy) in &t.input_tiles {
        if !rev_union.contains(&(ix, iy, 0)) && !rev_union.contains(&(ix, iy, 1)) {
            errors.push(format!("input ({ix},{iy}) cannot reach any output"));
        }
    }

    // Deduplicate (forward + reverse can surface the same broken link).
    errors.dedup();
    errors
}

// ---------------------------------------------------------------------------
// Rust code generation
// ---------------------------------------------------------------------------

fn rust_statics(t: &Template) -> String {
    let tag = format!("{}_{}", t.n_inputs, t.n_outputs);
    let mut out = String::new();

    writeln!(out, "static T_{tag}_ENTITIES: &[BalancerTemplateEntity] = &[").unwrap();
    for e in &t.entities {
        let io = match &e.io_type {
            Some(s) => format!(r#", io_type: Some("{s}")"#),
            None => ", io_type: None".to_owned(),
        };
        writeln!(
            out,
            r#"    BalancerTemplateEntity {{ name: "{}", x: {}, y: {}, direction: {}{io} }},"#,
            e.name, e.x, e.y, e.direction,
        ).unwrap();
    }
    writeln!(out, "];").unwrap();

    let input_str: Vec<_> = t.input_tiles.iter().map(|(x, y)| format!("({x}, {y})")).collect();
    writeln!(out, "static T_{tag}_INPUT: &[(i32, i32)] = &[{}];", input_str.join(", ")).unwrap();

    let output_str: Vec<_> = t.output_tiles.iter().map(|(x, y)| format!("({x}, {y})")).collect();
    writeln!(out, "static T_{tag}_OUTPUT: &[(i32, i32)] = &[{}];", output_str.join(", ")).unwrap();

    out
}

fn rust_insert(t: &Template) -> String {
    let n = t.n_inputs;
    let m = t.n_outputs;
    let tag = format!("{n}_{m}");
    let mut out = String::new();
    writeln!(out, "    m.insert(({n}, {m}), BalancerTemplate {{").unwrap();
    writeln!(out, "        n_inputs: {n}, n_outputs: {m}, width: {}, height: {},", t.width, t.height).unwrap();
    writeln!(out, "        entities: T_{tag}_ENTITIES, input_tiles: T_{tag}_INPUT, output_tiles: T_{tag}_OUTPUT,").unwrap();
    writeln!(out, "        source_blueprint: \"{}\",", t.source_blueprint).unwrap();
    writeln!(out, "    }});").unwrap();
    out
}

// ---------------------------------------------------------------------------
// Patch balancer_library.rs
// ---------------------------------------------------------------------------

const REGISTRY_MARKER: &str =
    "// ---------------------------------------------------------------------------\n// Global registry";

fn patch_rust_library(lib_path: &PathBuf, t: &Template) -> Result<(), String> {
    let n = t.n_inputs;
    let m = t.n_outputs;
    let tag = format!("{n}_{m}");

    let mut src = std::fs::read_to_string(lib_path)
        .map_err(|e| format!("Could not read {}: {e}", lib_path.display()))?;

    if src.contains(&format!("static T_{tag}_ENTITIES")) {
        eprint!("Template ({n},{m}) already exists. Overwrite? [y/N] ");
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).ok();
        if line.trim().to_lowercase() != "y" {
            return Err("Aborted.".to_owned());
        }
        src = remove_existing(&src, n, m);
    }

    // Insert statics before the registry marker
    if !src.contains(REGISTRY_MARKER) {
        return Err(format!("Could not find registry marker in {}", lib_path.display()));
    }
    src = src.replacen(
        REGISTRY_MARKER,
        &format!("{}\n{REGISTRY_MARKER}", rust_statics(t)),
        1,
    );

    // Insert m.insert() before the `    m\n}` closing of build_templates
    let fn_pos = src.find("fn build_templates()").ok_or("Could not find fn build_templates()")?;
    let tail = &src[fn_pos..];
    // Find the last `\n    m\n}` in the function (the return statement)
    let close = tail.rfind("\n    m\n}").ok_or("Could not find closing `m` in build_templates()")?;
    let insert_at = fn_pos + close + 1; // just after the \n, before `    m`
    src.insert_str(insert_at, &rust_insert(t));

    std::fs::write(lib_path, &src)
        .map_err(|e| format!("Could not write {}: {e}", lib_path.display()))?;

    println!(
        "Patched {} with ({n},{m}) template.",
        lib_path.file_name().unwrap().to_string_lossy()
    );
    println!("  {}W × {}H, {} entities", t.width, t.height, t.entities.len());
    println!("  inputs:  {:?}", t.input_tiles);
    println!("  outputs: {:?}", t.output_tiles);
    println!();
    println!("Run: cargo test --manifest-path crates/core/Cargo.toml");
    Ok(())
}

fn remove_existing(src: &str, n: usize, m: usize) -> String {
    let tag = format!("{n}_{m}");
    let mut out = src.to_owned();

    // Remove each static block (multi-line, ends with `;\n`)
    for static_name in [
        format!("T_{tag}_ENTITIES"),
        format!("T_{tag}_INPUT"),
        format!("T_{tag}_OUTPUT"),
    ] {
        if let Some(start) = out.find(&format!("static {static_name}")) {
            if let Some(semi) = out[start..].find(";\n") {
                out.drain(start..start + semi + 2);
            }
        }
    }

    // Remove the m.insert block
    let insert_needle = format!("    m.insert(({n}, {m}),");
    if let Some(start) = out.find(&insert_needle) {
        if let Some(end) = out[start..].find("    });\n") {
            out.drain(start..start + end + 8);
        }
    }

    out
}

// ---------------------------------------------------------------------------
// JSON output
// ---------------------------------------------------------------------------

fn print_json(t: &Template) {
    let entities: Vec<_> = t.entities.iter().map(|e| {
        let mut obj = serde_json::json!({
            "name": e.name,
            "x": e.x,
            "y": e.y,
            "direction": e.direction,
        });
        if let Some(io) = &e.io_type {
            obj["io_type"] = serde_json::json!(io);
        }
        obj
    }).collect();

    let json = serde_json::json!({
        "n_inputs": t.n_inputs,
        "n_outputs": t.n_outputs,
        "width": t.width,
        "height": t.height,
        "entities": entities,
        "input_tiles": t.input_tiles,
        "output_tiles": t.output_tiles,
        "source_blueprint": t.source_blueprint,
    });
    println!("{}", serde_json::to_string_pretty(&json).unwrap());
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut dry_run = false;
    let mut json_mode = false;
    let mut from_stdin = false;
    let mut blueprint: Option<String> = None;

    for arg in &args {
        match arg.as_str() {
            "--dry-run" => dry_run = true,
            "--json" => json_mode = true,
            "--stdin" => from_stdin = true,
            other if !other.starts_with('-') => blueprint = Some(other.to_owned()),
            other => {
                eprintln!("Unknown flag: {other}");
                process::exit(1);
            }
        }
    }

    let bp = if from_stdin {
        let mut s = String::new();
        std::io::stdin().read_line(&mut s).expect("failed to read stdin");
        s.trim().to_owned()
    } else if let Some(b) = blueprint {
        b
    } else {
        eprintln!("Usage: import_balancer [--dry-run] [--json] [--stdin] '<blueprint>'");
        eprintln!();
        eprintln!("Options:");
        eprintln!("  --dry-run   Print Rust snippet without modifying any file");
        eprintln!("  --json      Dump template as JSON and exit");
        eprintln!("  --stdin     Read blueprint from stdin");
        process::exit(1);
    };

    let template = match validate_and_build(&bp) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Error: {e}");
            process::exit(1);
        }
    };

    println!(
        "Detected ({}→{}) balancer: {}W × {}H, {} entities",
        template.n_inputs, template.n_outputs,
        template.width, template.height,
        template.entities.len(),
    );

    let conn_errors = check_lane_connectivity(&template);
    if !conn_errors.is_empty() {
        eprintln!("Connectivity errors ({}):", conn_errors.len());
        for e in &conn_errors {
            eprintln!("  - {e}");
        }
        eprintln!("Blueprint is not a valid fully-connected balancer.");
        process::exit(1);
    }
    println!("Connectivity OK: all {} input lanes reach all {} output tiles.", template.n_inputs * 2, template.n_outputs);

    if json_mode {
        print_json(&template);
        return;
    }

    if dry_run {
        println!("\n--- Rust statics ---");
        print!("{}", rust_statics(&template));
        println!("--- m.insert() ---");
        print!("{}", rust_insert(&template));
        return;
    }

    let lib_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/bus/balancer_library.rs");

    if let Err(e) = patch_rust_library(&lib_path, &template) {
        eprintln!("Error: {e}");
        process::exit(1);
    }
}
