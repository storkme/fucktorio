//! Lightweight append-only cache of SAT crossing-zone shapes.
//!
//! Writes one JSONL record per solved zone to:
//!   1. `$FUCKTORIO_ZONE_CACHE_PATH`            — if that env var is set (full path override)
//!   2. `$XDG_CACHE_HOME/fucktorio/sat-zones.jsonl`
//!   3. `$HOME/.cache/fucktorio/sat-zones.jsonl` — final fallback
//!
//! Used purely for sizing telemetry — not production code.
//!
//! Gated behind `#[cfg(not(target_arch = "wasm32"))]` so it compiles to nothing
//! in WASM builds.

#![cfg(not(target_arch = "wasm32"))]

use crate::models::{LayoutRegion, PortIo, RegionPort};

/// Internal edge classification used only for the canonical signature
/// computation. The public `RegionPort` uses absolute positions instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Edge {
    N,
    E,
    S,
    W,
}

/// Derive `(edge, offset)` for a port sitting on the boundary of a
/// `(width, height)` zone anchored at `(zx, zy)`. Returns `None` if the
/// port point is outside the zone bbox.
fn edge_and_offset(zx: i32, zy: i32, w: u32, h: u32, port: &RegionPort) -> Option<(Edge, u32)> {
    let lx = port.point.x - zx;
    let ly = port.point.y - zy;
    if lx < 0 || ly < 0 || lx as u32 >= w || ly as u32 >= h {
        return None;
    }
    let lxu = lx as u32;
    let lyu = ly as u32;
    let on_north = lyu == 0;
    let on_south = lyu == h - 1;
    let on_west = lxu == 0;
    let on_east = lxu == w - 1;
    let edge = match (on_north, on_south, on_west, on_east) {
        (true, false, false, false) => Edge::N,
        (false, true, false, false) => Edge::S,
        (false, false, true, false) => Edge::W,
        (false, false, false, true) => Edge::E,
        // Corner/degenerate — break ties with flow direction.
        _ => {
            use crate::models::EntityDirection;
            match port.point.direction {
                EntityDirection::North => Edge::N,
                EntityDirection::South => Edge::S,
                EntityDirection::West => Edge::W,
                EntityDirection::East => Edge::E,
            }
        }
    };
    let offset = match edge {
        Edge::N | Edge::S => lxu,
        Edge::E | Edge::W => lyu,
    };
    Some((edge, offset))
}

/// SAT solver stats associated with a zone record. Mirrors
/// `sat::SolutionStats` but kept local so `zone_cache` doesn't depend on
/// the full `sat` module.
pub struct ZoneStats {
    pub variables: u32,
    pub clauses: u32,
    pub solve_time_us: u64,
}
use std::cell::RefCell;
use std::io::Write as _;
use std::time::{SystemTime, UNIX_EPOCH};

// Thread-local source tag so parallel tests each carry their own label
// without stomping on each other via a process-global env var.
thread_local! {
    static ZONE_SOURCE: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Set the per-thread zone source tag. Call this at the start of a test body.
pub fn set_thread_source(source: Option<&str>) {
    ZONE_SOURCE.with(|s| *s.borrow_mut() = source.map(|s| s.to_string()));
}

/// Resolve the JSONL output path.
///
/// Priority:
/// 1. `FUCKTORIO_ZONE_CACHE_PATH` env var (full path override)
/// 2. `$XDG_CACHE_HOME/fucktorio/sat-zones.jsonl`
/// 3. `$HOME/.cache/fucktorio/sat-zones.jsonl`
fn resolve_cache_path() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("FUCKTORIO_ZONE_CACHE_PATH") {
        return std::path::PathBuf::from(p);
    }
    let base = std::env::var("XDG_CACHE_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| std::path::PathBuf::from(h).join(".cache"))
        })
        .unwrap_or_else(|| std::path::PathBuf::from(".cache"));
    base.join("fucktorio").join("sat-zones.jsonl")
}

// ---------------------------------------------------------------------------
// Canonical orientation-invariant signature
// ---------------------------------------------------------------------------

/// A single transform in the dihedral group D4 applied to a port position.
///
/// For a rectangle of size (w, h) the 8 dihedral symmetries are:
///   rotations by 0°, 90°, 180°, 270° and their reflections (flip).
/// Rotating 90° maps (w,h) → (h,w) and port (edge,offset) according to the
/// standard boundary walk.
///
/// We represent each port as `(edge_idx, offset)` where edge_idx encodes
/// N=0, E=1, S=2, W=3 (clockwise from top).
fn transform_port(
    edge: u8,    // 0=N,1=E,2=S,3=W
    offset: u32,
    io: bool,    // true=Input
    w: u32,
    h: u32,
    rotation: u8,   // 0..4
    reflect: bool,
) -> (u8, u32, bool, u32, u32) {
    // After the full transform, what are (new_w, new_h)?
    let (tw, th) = if rotation.is_multiple_of(2) { (w, h) } else { (h, w) };

    // Apply reflection first (flip along vertical axis before rotation).
    // Reflection: N stays N but offset mirrors; E<->W; S stays S but mirrors.
    let (edge, offset) = if reflect {
        match edge {
            0 => (0u8, w.saturating_sub(1).saturating_sub(offset)), // N: mirror offset
            1 => (3u8, offset),  // E -> W
            2 => (2u8, w.saturating_sub(1).saturating_sub(offset)), // S: mirror offset
            3 => (1u8, offset),  // W -> E
            _ => (edge, offset),
        }
    } else {
        (edge, offset)
    };

    // Apply rotation: each 90° CW maps edge (N->E->S->W->N) and transforms offset.
    let (edge, offset) = {
        let mut e = edge;
        let mut o = offset;
        let mut cur_w = w;
        let mut cur_h = h;
        for _ in 0..rotation {
            // 90° CW: N->E, E->S, S->W, W->N
            // N(offset) -> E(offset), E(offset) -> S(cur_w-1-offset),
            // S(offset) -> W(cur_h-1-offset), W(offset) -> N(cur_w-1-offset)
            let (ne, no) = match e {
                0 => (1u8, o),                              // N -> E
                1 => (2u8, cur_w.saturating_sub(1).saturating_sub(o)), // E -> S
                2 => (3u8, cur_h.saturating_sub(1).saturating_sub(o)), // S -> W
                3 => (0u8, o),                              // W -> N
                _ => (e, o),
            };
            e = ne;
            o = no;
            // After 90° CW, dimensions swap
            std::mem::swap(&mut cur_w, &mut cur_h);
        }
        (e, o)
    };

    (edge, offset, io, tw, th)
}

/// Format a single port tuple as e.g. `N2I` or `E0O`.
fn format_port(edge: u8, offset: u32, is_input: bool) -> String {
    let e = match edge {
        0 => 'N',
        1 => 'E',
        2 => 'S',
        3 => 'W',
        _ => '?',
    };
    let io = if is_input { 'I' } else { 'O' };
    format!("{}{}{}", e, offset, io)
}

/// Compute the canonical orientation-invariant signature for a crossing zone.
///
/// Tries all 8 dihedral symmetries (4 rotations × 2 reflections), formats each
/// as `"{W}x{H}:{sorted_port_tuples}"`, and returns the lexicographically
/// smallest — so geometrically identical zones (rotated or mirrored) collapse
/// to the same bucket.
pub fn canonical_signature(zx: i32, zy: i32, width: u32, height: u32, ports: &[RegionPort]) -> String {
    let mut candidates: Vec<String> = Vec::with_capacity(8);

    // Derive (edge, offset, io) tuples once; then apply D4 transforms to each.
    let base: Vec<(u8, u32, bool)> = ports
        .iter()
        .filter_map(|p| {
            let (edge, offset) = edge_and_offset(zx, zy, width, height, p)?;
            let edge_idx = match edge {
                Edge::N => 0,
                Edge::E => 1,
                Edge::S => 2,
                Edge::W => 3,
            };
            let is_input = matches!(p.io, PortIo::Input);
            Some((edge_idx, offset, is_input))
        })
        .collect();

    for rotation in 0u8..4 {
        for &reflect in &[false, true] {
            let transformed: Vec<(u8, u32, bool)> = base
                .iter()
                .map(|&(edge_idx, offset, is_input)| {
                    let (ne, no, nio, _tw, _th) = transform_port(
                        edge_idx, offset, is_input, width, height, rotation, reflect,
                    );
                    (ne, no, nio)
                })
                .collect();

            // Derive transformed dimensions
            let (tw, th) = if rotation.is_multiple_of(2) {
                (width, height)
            } else {
                (height, width)
            };

            let mut port_strs: Vec<String> = transformed
                .iter()
                .map(|&(e, o, io)| format_port(e, o, io))
                .collect();
            port_strs.sort_unstable();

            candidates.push(format!("{}x{}:{}", tw, th, port_strs.join(",")));
        }
    }

    candidates.into_iter().min().unwrap_or_default()
}

/// Append one zone record to the cache JSONL file.
///
/// The effective source is resolved in priority order:
/// 1. The thread-local set by [`set_thread_source`] (handles parallel tests).
/// 2. The `source` argument passed by the caller.
/// 3. The `FUCKTORIO_ZONE_SOURCE` environment variable (legacy / single-threaded).
///
/// Silently no-ops on any I/O error — this is telemetry, not correctness.
pub fn record_zone(region: &LayoutRegion, stats: ZoneStats, source: Option<&str>) {
    // Resolve source: thread-local wins, then arg, then env var.
    let thread_src = ZONE_SOURCE.with(|s| s.borrow().clone());
    let effective_source: Option<String> = thread_src
        .or_else(|| source.map(|s| s.to_string()))
        .or_else(|| std::env::var("FUCKTORIO_ZONE_SOURCE").ok());

    let signature = canonical_signature(
        region.x,
        region.y,
        region.width as u32,
        region.height as u32,
        &region.ports,
    );

    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Serialize with serde_json — build a serde_json::Value directly to avoid
    // needing a separate named struct with its own derives.
    let record = serde_json::json!({
        "ts": ts,
        "signature": signature,
        "width": region.width as u32,
        "height": region.height as u32,
        "variables": stats.variables,
        "clauses": stats.clauses,
        "solve_time_us": stats.solve_time_us,
        "source": effective_source,
    });

    let mut line = serde_json::to_string(&record).unwrap_or_default();
    line.push('\n');

    let path = resolve_cache_path();

    // Ensure the directory exists.
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // Single write_all — atomic for writes < PIPE_BUF (4 KB) on POSIX.
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&path)
    {
        let _ = f.write_all(line.as_bytes());
    }
}
