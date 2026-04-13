//! Junction types for the next-generation ghost routing resolution.
//!
//! See `docs/rfp-junction-solver.md`. A Junction is an abstract region where
//! two or more ghost-routed specs cross and need a deterministic resolution
//! (surface + underground belt arrangement).
//!
//! Currently used as the internal shape for ghost-router "unresolved"
//! junctions (per-tile 1×1 crossings that still need a template). The
//! junction is built first, then lowered to a `LayoutRegion` via
//! `to_layout_region` at the output boundary. A solver pass that actually
//! consumes `Junction` will land in a follow-up.
//!
//! Invariants:
//! - Each spec contributes exactly **1 entry + 1 exit**. No splitting or
//!   merging happens inside a junction — that's handled by pre-placed
//!   infrastructure (row templates, balancers, mergers).
//! - For each `(item, belt_tier)` tuple, `#inputs == #outputs` — flow is
//!   conserved by class.
//! - `forbidden` tiles inside the bbox are not routable (machines, poles,
//!   inserters, row template belts, pre-placed balancer entities, etc.).
//!   A strategy must not place belts on forbidden tiles.
//! - The `bbox` is the tight enclosing rectangle of the junction's routable
//!   area; strategies can assume any tile in `bbox \ forbidden` is free.

use rustc_hash::FxHashSet;

use crate::models::{EntityDirection, LayoutRegion, PortEdge, PortIo, PortSpec, RegionKind};

/// A rectangular bounding box in tile coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    #[allow(dead_code)]
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.x + self.w as i32 && y >= self.y && y < self.y + self.h as i32
    }
}

/// A single point on or inside a junction where a spec enters or exits.
/// `direction` is the flow direction at this tile — which way items are
/// physically moving. The edge of the bbox the port sits on is derivable
/// from `(x, y, direction)` combined with the bbox and whether this is an
/// entry or an exit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortPoint {
    pub x: i32,
    pub y: i32,
    pub direction: EntityDirection,
}

/// The belt tier a spec is routed at. Determines throughput class — two
/// specs can be paired inside a junction only if they have the same
/// `(item, belt_tier)`.
///
/// Mirrors the existing `belt_entity_for_rate` tiering in `common.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BeltTier {
    /// `transport-belt` — 15 items/s per lane
    Yellow,
    /// `fast-transport-belt` — 30 items/s per lane
    Red,
    /// `express-transport-belt` — 45 items/s per lane
    Blue,
}

impl BeltTier {
    /// Resolve a belt tier from a Factorio belt entity name. Accepts
    /// `transport-belt`, `fast-transport-belt`, `express-transport-belt`
    /// and their underground/splitter variants.
    pub fn from_name(name: &str) -> Option<Self> {
        if name.starts_with("express-") {
            Some(Self::Blue)
        } else if name.starts_with("fast-") {
            Some(Self::Red)
        } else if name.contains("transport-belt") || name.contains("underground-belt") || name.contains("splitter") {
            Some(Self::Yellow)
        } else {
            None
        }
    }
}

/// One spec crossing a junction. Exactly one entry + one exit per spec.
#[derive(Debug, Clone)]
pub struct SpecCrossing {
    pub item: String,
    pub belt_tier: BeltTier,
    pub entry: PortPoint,
    pub exit: PortPoint,
}

impl SpecCrossing {
    /// True when this spec passes straight through without turning.
    #[allow(dead_code)]
    pub fn is_straight(&self) -> bool {
        self.entry.direction == self.exit.direction
    }
}

/// An abstract junction: a rectangular bounding region with zero or more
/// non-routable carve-outs and a list of specs that need to be routed
/// through it.
///
/// This is the type a junction-solver strategy consumes. The solver's job
/// is to produce a valid placement of belt/underground entities inside
/// `bbox \ forbidden` that connects every spec's entry to its exit
/// without mixing items or violating belt physics.
#[derive(Debug, Clone)]
pub struct Junction {
    pub bbox: Rect,
    #[allow(dead_code)]
    pub forbidden: FxHashSet<(i32, i32)>,
    pub specs: Vec<SpecCrossing>,
}

impl Junction {
    /// Number of distinct `(item, belt_tier)` classes passing through.
    #[allow(dead_code)]
    pub fn class_count(&self) -> usize {
        let mut seen: FxHashSet<(&str, BeltTier)> = FxHashSet::default();
        for s in &self.specs {
            seen.insert((s.item.as_str(), s.belt_tier));
        }
        seen.len()
    }

    /// Lower this junction to a `LayoutRegion` for the pipeline output.
    /// Each spec produces two `PortSpec`s (one entry + one exit) whose
    /// edges are derived from the point's position inside the bbox and
    /// the flow direction.
    pub fn to_layout_region(&self, kind: RegionKind) -> LayoutRegion {
        let mut ports: Vec<PortSpec> = Vec::with_capacity(self.specs.len() * 2);
        for spec in &self.specs {
            ports.push(point_to_port(&self.bbox, &spec.entry, PortIo::Input, &spec.item));
            ports.push(point_to_port(&self.bbox, &spec.exit, PortIo::Output, &spec.item));
        }
        LayoutRegion {
            kind,
            x: self.bbox.x,
            y: self.bbox.y,
            width: self.bbox.w as i32,
            height: self.bbox.h as i32,
            ports,
        }
    }
}

/// Derive a `PortSpec` from a bbox + point + io + item. The edge is
/// picked from the point's position inside the bbox (W if on the west
/// edge, etc.). For a 1×1 bbox (the common case for per-tile
/// unresolved junctions), every position collapses to a corner; we
/// then use the flow direction to decide which edge the port
/// conceptually sits on: entries face the edge opposite their flow
/// (items flowing East enter from the west edge) and exits face the
/// edge matching their flow.
fn point_to_port(bbox: &Rect, p: &PortPoint, io: PortIo, item: &str) -> PortSpec {
    let lx = p.x - bbox.x;
    let ly = p.y - bbox.y;
    let w = bbox.w as i32;
    let h = bbox.h as i32;

    let on_north = ly == 0;
    let on_south = ly == h - 1;
    let on_west = lx == 0;
    let on_east = lx == w - 1;

    let edge = match (on_north, on_south, on_west, on_east) {
        (true, false, false, false) => PortEdge::N,
        (false, true, false, false) => PortEdge::S,
        (false, false, true, false) => PortEdge::W,
        (false, false, false, true) => PortEdge::E,
        // Corner (including 1×1 bbox where every edge overlaps) —
        // tiebreak by flow direction + io.
        _ => match (io, p.direction) {
            (PortIo::Input, EntityDirection::East) => PortEdge::W,
            (PortIo::Input, EntityDirection::West) => PortEdge::E,
            (PortIo::Input, EntityDirection::North) => PortEdge::S,
            (PortIo::Input, EntityDirection::South) => PortEdge::N,
            (PortIo::Output, EntityDirection::East) => PortEdge::E,
            (PortIo::Output, EntityDirection::West) => PortEdge::W,
            (PortIo::Output, EntityDirection::North) => PortEdge::N,
            (PortIo::Output, EntityDirection::South) => PortEdge::S,
        },
    };

    let offset = match edge {
        PortEdge::N | PortEdge::S => lx.max(0) as u32,
        PortEdge::E | PortEdge::W => ly.max(0) as u32,
    };

    PortSpec {
        edge,
        offset,
        io,
        item: Some(item.to_string()),
        direction: Some(p.direction),
    }
}
