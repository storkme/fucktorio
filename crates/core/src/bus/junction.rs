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

use crate::models::{LayoutRegion, PortIo, PortPoint, RegionKind, RegionPort};

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

    /// Surface belt entity name for this tier.
    pub fn belt_name(self) -> &'static str {
        match self {
            Self::Yellow => "transport-belt",
            Self::Red => "fast-transport-belt",
            Self::Blue => "express-transport-belt",
        }
    }

    /// Ordering key for "which tier is higher throughput". Used when a
    /// junction carries specs at multiple tiers and we need to pick the
    /// dominant tier for the whole region.
    pub fn rank(self) -> u8 {
        match self {
            Self::Yellow => 0,
            Self::Red => 1,
            Self::Blue => 2,
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
    /// Each spec contributes two `RegionPort`s — one entry (`PortIo::Input`)
    /// and one exit (`PortIo::Output`) — with absolute positions and
    /// flow directions.
    pub fn to_layout_region(&self, kind: RegionKind) -> LayoutRegion {
        let mut ports: Vec<RegionPort> = Vec::with_capacity(self.specs.len() * 2);
        for spec in &self.specs {
            ports.push(RegionPort {
                point: spec.entry,
                io: PortIo::Input,
                item: Some(spec.item.clone()),
            });
            ports.push(RegionPort {
                point: spec.exit,
                io: PortIo::Output,
                item: Some(spec.item.clone()),
            });
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
