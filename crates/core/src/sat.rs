//! SAT-based solver for bus crossing zones.
//!
//! When a horizontal tap-off crosses vertical trunk belts, the entities in
//! that small rectangular region form a constrained belt-routing problem.
//! We encode it as a Boolean satisfiability (SAT) problem and solve with
//! Varisat (a pure-Rust CDCL solver that also compiles to WASM).
//!
//! The encoding is a simplified subset of Factorio-SAT: no splitters, items
//! are known, and I/O ports are fixed.

use crate::models::{EntityDirection, PlacedEntity};
use varisat::{CnfFormula, ExtendFormula, Lit, Solver, Var};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A rectangular region where tap-offs cross foreign trunks.
#[derive(Debug, Clone)]
pub struct CrossingZone {
    /// World x of the zone's left column.
    pub x: i32,
    /// World y of the zone's top row.
    pub y: i32,
    /// Width in tiles.
    pub width: u32,
    /// Height in tiles.
    pub height: u32,
    /// Fixed belt entry/exit points on the zone boundary.
    pub boundaries: Vec<ZoneBoundary>,
    /// Tiles that must be empty (tap-off passage — underground belts pass
    /// through without surface entities).
    pub forced_empty: Vec<(i32, i32)>,
}

/// A fixed belt port on the boundary of a crossing zone.
#[derive(Debug, Clone)]
pub struct ZoneBoundary {
    /// World x of this port.
    pub x: i32,
    /// World y of this port.
    pub y: i32,
    /// Flow direction of the belt at this port.
    pub direction: EntityDirection,
    /// Item carried by this belt.
    pub item: String,
    /// True if the belt is entering the zone, false if leaving.
    pub is_input: bool,
}

/// Result of solving a crossing zone.
#[derive(Debug, Clone)]
pub struct CrossingZoneSolution {
    pub entities: Vec<PlacedEntity>,
    pub stats: CrossingZoneStats,
}

/// Solver statistics for a crossing zone.
#[derive(Debug, Clone)]
pub struct CrossingZoneStats {
    pub variables: u32,
    pub clauses: u32,
    pub solve_time_us: u64,
    pub zone_width: u32,
    pub zone_height: u32,
}

// ---------------------------------------------------------------------------
// Direction helpers
// ---------------------------------------------------------------------------

const DIR_N: usize = 0;
const DIR_E: usize = 1;
const DIR_S: usize = 2;
const DIR_W: usize = 3;
const ALL_DIRS: [usize; 4] = [DIR_N, DIR_E, DIR_S, DIR_W];

fn dir_delta(d: usize) -> (i32, i32) {
    match d {
        DIR_N => (0, -1),
        DIR_E => (1, 0),
        DIR_S => (0, 1),
        DIR_W => (-1, 0),
        _ => unreachable!(),
    }
}

fn entity_dir_to_idx(d: EntityDirection) -> usize {
    match d {
        EntityDirection::North => DIR_N,
        EntityDirection::East => DIR_E,
        EntityDirection::South => DIR_S,
        EntityDirection::West => DIR_W,
    }
}

fn idx_to_entity_dir(d: usize) -> EntityDirection {
    match d {
        DIR_N => EntityDirection::North,
        DIR_E => EntityDirection::East,
        DIR_S => EntityDirection::South,
        DIR_W => EntityDirection::West,
        _ => unreachable!(),
    }
}

// ---------------------------------------------------------------------------
// Per-tile variable block
// ---------------------------------------------------------------------------

/// SAT variables for one tile. All fields are `Copy` (Var is a u32 wrapper).
/// Uses fixed-size arrays (max 4 bits = 16 items).
#[derive(Debug, Clone, Copy)]
struct TileVars {
    is_belt: Var,
    is_ug_in: Var,
    is_ug_out: Var,
    /// Output direction (one-hot).
    out_dir: [Var; 4],
    /// Underground segment passing through in direction d.
    underground: [Var; 4],
    /// Surface item encoding (binary). Only first `n_item_bits` meaningful.
    item_bits: [Var; 4],
    /// Underground item for horizontal segments (East/West).
    ug_item_h: [Var; 4],
    /// Underground item for vertical segments (North/South).
    ug_item_v: [Var; 4],
}

// ---------------------------------------------------------------------------
// CNF builder helper (avoids borrow conflicts)
// ---------------------------------------------------------------------------

struct Cnf {
    formula: CnfFormula,
    count: u32,
}

impl Cnf {
    fn new() -> Self {
        Self {
            formula: CnfFormula::new(),
            count: 0,
        }
    }

    fn add(&mut self, lits: &[Lit]) {
        self.formula.add_clause(lits);
        self.count += 1;
    }
}

// ---------------------------------------------------------------------------
// Encoder
// ---------------------------------------------------------------------------

struct CrossingEncoder {
    width: u32,
    height: u32,
    n_item_bits: u32,
    item_names: Vec<String>,
    tiles: Vec<TileVars>,
    total_vars: u32,
}

impl CrossingEncoder {
    fn new(width: u32, height: u32, item_names: Vec<String>) -> Self {
        let n_items = item_names.len().max(1);
        let n_item_bits = if n_items <= 1 {
            0
        } else {
            ((n_items as f64).log2().ceil() as u32).max(1)
        };

        // 3 type + 4 dir + 4 underground + 3 * n_item_bits (surface + ug_h + ug_v)
        let vars_per_tile: usize = 3 + 4 + 4 + 3 * n_item_bits as usize;
        let n_tiles = (width * height) as usize;
        let mut next: usize = 0;

        let mut tiles = Vec::with_capacity(n_tiles);
        for _ in 0..n_tiles {
            let base = next;
            next += vars_per_tile;

            let v = |offset: usize| -> Var { Var::from_index(base + offset) };

            let dummy = Var::from_index(0);
            let nb = n_item_bits as usize;

            let mut item_bits = [dummy; 4];
            let mut ug_item_h = [dummy; 4];
            let mut ug_item_v = [dummy; 4];
            for b in 0..nb {
                item_bits[b] = v(11 + b);
                ug_item_h[b] = v(11 + nb + b);
                ug_item_v[b] = v(11 + 2 * nb + b);
            }
            for b in nb..4 {
                item_bits[b] = v(0);
                ug_item_h[b] = v(0);
                ug_item_v[b] = v(0);
            }

            tiles.push(TileVars {
                is_belt: v(0),
                is_ug_in: v(1),
                is_ug_out: v(2),
                out_dir: [v(3), v(4), v(5), v(6)],
                underground: [v(7), v(8), v(9), v(10)],
                item_bits,
                ug_item_h,
                ug_item_v,
            });
        }

        CrossingEncoder {
            width,
            height,
            n_item_bits,
            item_names,
            tiles,
            total_vars: next as u32,
        }
    }

    fn idx(&self, x: u32, y: u32) -> usize {
        (y * self.width + x) as usize
    }

    fn in_bounds(&self, x: i32, y: i32) -> bool {
        x >= 0 && x < self.width as i32 && y >= 0 && y < self.height as i32
    }

    /// Build the full CNF formula.
    fn encode(&self, zone: &CrossingZone, max_ug_reach: u32) -> Cnf {
        let mut cnf = Cnf::new();
        self.encode_type_constraints(&mut cnf);
        self.encode_direction_constraints(&mut cnf);
        self.encode_adjacency(&mut cnf);
        self.encode_underground(&mut cnf, max_ug_reach);
        if self.n_item_bits > 0 {
            self.encode_item_transport(&mut cnf);
        }
        self.encode_boundaries(&mut cnf, zone);
        cnf
    }

    // -- Type: at most one of {belt, ug_in, ug_out} per tile ----------------

    fn encode_type_constraints(&self, cnf: &mut Cnf) {
        for t in &self.tiles {
            let types = [t.is_belt, t.is_ug_in, t.is_ug_out];
            for i in 0..types.len() {
                for j in (i + 1)..types.len() {
                    cnf.add(&[types[i].negative(), types[j].negative()]);
                }
            }
        }
    }

    // -- Direction constraints ----------------------------------------------

    fn encode_direction_constraints(&self, cnf: &mut Cnf) {
        for t in &self.tiles {
            // Direction AMO (at most one output direction).
            for i in 0..4usize {
                for j in (i + 1)..4 {
                    cnf.add(&[t.out_dir[i].negative(), t.out_dir[j].negative()]);
                }
            }

            // Any entity type -> at least one direction.
            for &type_var in &[t.is_belt, t.is_ug_in, t.is_ug_out] {
                cnf.add(&[
                    type_var.negative(),
                    t.out_dir[0].positive(),
                    t.out_dir[1].positive(),
                    t.out_dir[2].positive(),
                    t.out_dir[3].positive(),
                ]);
            }

            // No direction without entity: dir[d] -> at least one type.
            for d in 0..4 {
                cnf.add(&[
                    t.out_dir[d].negative(),
                    t.is_belt.positive(),
                    t.is_ug_in.positive(),
                    t.is_ug_out.positive(),
                ]);
            }
        }
    }

    // -- Adjacency: belt flowing dir d requires compatible neighbor ----------

    fn encode_adjacency(&self, cnf: &mut Cnf) {
        for y in 0..self.height {
            for x in 0..self.width {
                let t = self.tiles[self.idx(x, y)];

                for &d in &ALL_DIRS {
                    let (dx, dy) = dir_delta(d);
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;

                    if !self.in_bounds(nx, ny) {
                        // Belt outputting off-grid is only allowed at
                        // boundary ports (handled in encode_boundaries).
                        // Non-boundary edge tiles cannot output off-grid.
                        // We'll enforce this below.
                        continue;
                    }

                    let n = self.tiles[self.idx(nx as u32, ny as u32)];

                    // belt AND out_dir[d] -> neighbor not empty
                    cnf.add(&[
                        t.is_belt.negative(),
                        t.out_dir[d].negative(),
                        n.is_belt.positive(),
                        n.is_ug_in.positive(),
                        n.is_ug_out.positive(),
                    ]);

                    // belt out d -> neighbor ug_in must face d (same direction)
                    cnf.add(&[
                        t.is_belt.negative(),
                        t.out_dir[d].negative(),
                        n.is_ug_in.negative(),
                        n.out_dir[d].positive(),
                    ]);

                    // belt out d -> neighbor can't be ug_out (UG outputs emit,
                    // don't receive from surface belts).
                    cnf.add(&[
                        t.is_belt.negative(),
                        t.out_dir[d].negative(),
                        n.is_ug_out.negative(),
                    ]);

                    // ug_out facing d -> neighbor not empty (items exit UG)
                    cnf.add(&[
                        t.is_ug_out.negative(),
                        t.out_dir[d].negative(),
                        n.is_belt.positive(),
                        n.is_ug_in.positive(),
                        n.is_ug_out.positive(),
                    ]);

                    // ug_out out d -> neighbor ug_in must face d
                    cnf.add(&[
                        t.is_ug_out.negative(),
                        t.out_dir[d].negative(),
                        n.is_ug_in.negative(),
                        n.out_dir[d].positive(),
                    ]);

                    // ug_out out d -> neighbor can't be another ug_out
                    cnf.add(&[
                        t.is_ug_out.negative(),
                        t.out_dir[d].negative(),
                        n.is_ug_out.negative(),
                    ]);

                    // No U-turn: if belt A outputs toward B, belt B can't
                    // output back toward A (direction opposite(d)).
                    let opp = (d + 2) % 4;
                    cnf.add(&[
                        t.is_belt.negative(),
                        t.out_dir[d].negative(),
                        n.is_belt.negative(),
                        n.out_dir[opp].negative(),
                    ]);
                    // Same for ug_out feeding into belt
                    cnf.add(&[
                        t.is_ug_out.negative(),
                        t.out_dir[d].negative(),
                        n.is_belt.negative(),
                        n.out_dir[opp].negative(),
                    ]);
                }
            }
        }
    }

    // -- Underground belt pairing and propagation ---------------------------

    fn encode_underground(&self, cnf: &mut Cnf, max_reach: u32) {
        for y in 0..self.height {
            for x in 0..self.width {
                let t = self.tiles[self.idx(x, y)];

                // Underground passages coexist with surface entities (belts
                // travel underneath). The only conflict: a UG entrance/exit
                // in the SAME direction as an ongoing underground segment
                // would create ambiguous pairing. Block that:
                // underground[d] AND ug_in facing d -> false
                // underground[d] AND ug_out facing d -> false
                for &d in &ALL_DIRS {
                    cnf.add(&[
                        t.underground[d].negative(),
                        t.is_ug_in.negative(),
                        t.out_dir[d].negative(),
                    ]);
                    cnf.add(&[
                        t.underground[d].negative(),
                        t.is_ug_out.negative(),
                        t.out_dir[d].negative(),
                    ]);
                }

                for &d in &ALL_DIRS {
                    let (dx, dy) = dir_delta(d);
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;

                    if self.in_bounds(nx, ny) {
                        let n = self.tiles[self.idx(nx as u32, ny as u32)];

                        // ug_in facing d -> next has underground[d] OR
                        // next is ug_out facing d (adjacent pair, distance=1)
                        cnf.add(&[
                            t.is_ug_in.negative(),
                            t.out_dir[d].negative(),
                            n.underground[d].positive(),
                            n.is_ug_out.positive(),
                        ]);

                        // If next is ug_out (from the clause above), it must face d.
                        cnf.add(&[
                            t.is_ug_in.negative(),
                            t.out_dir[d].negative(),
                            n.is_ug_out.negative(),
                            n.out_dir[d].positive(),
                        ]);

                        // underground[d] propagation: next has underground[d]
                        // OR next is ug_out facing d.
                        cnf.add(&[
                            t.underground[d].negative(),
                            n.underground[d].positive(),
                            n.is_ug_out.positive(),
                        ]);
                        cnf.add(&[
                            t.underground[d].negative(),
                            n.is_ug_out.negative(),
                            n.out_dir[d].positive(),
                        ]);
                    } else {
                        // Edge: ug_in can't face off-grid (no room for pair)
                        cnf.add(&[t.is_ug_in.negative(), t.out_dir[d].negative()]);
                        // Edge: underground can't continue off-grid
                        cnf.add(&[t.underground[d].negative()]);
                    }

                    // Backward: underground[d] must have a source.
                    // prev tile must have underground[d] OR be ug_in facing d.
                    let px = x as i32 - dx;
                    let py = y as i32 - dy;
                    if self.in_bounds(px, py) {
                        let p = self.tiles[self.idx(px as u32, py as u32)];
                        cnf.add(&[
                            t.underground[d].negative(),
                            p.underground[d].positive(),
                            p.is_ug_in.positive(),
                        ]);
                        // Tighten: if prev is ug_in (not underground[d]),
                        // it must face direction d.
                        cnf.add(&[
                            t.underground[d].negative(),
                            p.underground[d].positive(),
                            p.out_dir[d].positive(),
                        ]);
                    } else {
                        // No predecessor: underground[d] impossible here.
                        cnf.add(&[t.underground[d].negative()]);
                    }
                }
            }
        }

        // Max reach: at most max_reach tiles of underground[d] in a row.
        for &d in &ALL_DIRS {
            let (dx, dy) = dir_delta(d);
            for y in 0..self.height as i32 {
                for x in 0..self.width as i32 {
                    let mut clause = Vec::new();
                    for i in 0..=(max_reach as i32) {
                        let cx = x + dx * i;
                        let cy = y + dy * i;
                        if !self.in_bounds(cx, cy) {
                            break;
                        }
                        let t = self.tiles[self.idx(cx as u32, cy as u32)];
                        clause.push(t.underground[d].negative());
                    }
                    if clause.len() == (max_reach + 1) as usize {
                        cnf.add(&clause);
                    }
                }
            }
        }
    }

    // -- Item transport consistency -----------------------------------------
    //
    // Three channels per tile:
    //   item_bits   — surface belt item
    //   ug_item_h   — item traveling underground horizontally (E/W)
    //   ug_item_v   — item traveling underground vertically (N/S)

    fn ug_channel(t: &TileVars, d: usize) -> &[Var; 4] {
        if d == DIR_N || d == DIR_S {
            &t.ug_item_v
        } else {
            &t.ug_item_h
        }
    }

    fn encode_item_transport(&self, cnf: &mut Cnf) {
        let nb = self.n_item_bits as usize;

        for y in 0..self.height {
            for x in 0..self.width {
                let t = self.tiles[self.idx(x, y)];

                for &d in &ALL_DIRS {
                    let (dx, dy) = dir_delta(d);
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;
                    if !self.in_bounds(nx, ny) {
                        continue;
                    }
                    let n = self.tiles[self.idx(nx as u32, ny as u32)];

                    // 1. Surface belt → surface: belt out d → neighbor
                    //    surface item matches.
                    for bit in 0..nb {
                        cnf.add(&[
                            t.is_belt.negative(),
                            t.out_dir[d].negative(),
                            t.item_bits[bit].negative(),
                            n.item_bits[bit].positive(),
                        ]);
                        cnf.add(&[
                            t.is_belt.negative(),
                            t.out_dir[d].negative(),
                            t.item_bits[bit].positive(),
                            n.item_bits[bit].negative(),
                        ]);
                    }

                    // 2. UG input → underground: ug_in facing d →
                    //    neighbor's underground channel matches this
                    //    tile's surface item.
                    let n_ug = Self::ug_channel(&n, d);
                    for bit in 0..nb {
                        cnf.add(&[
                            t.is_ug_in.negative(),
                            t.out_dir[d].negative(),
                            t.item_bits[bit].negative(),
                            n_ug[bit].positive(),
                        ]);
                        cnf.add(&[
                            t.is_ug_in.negative(),
                            t.out_dir[d].negative(),
                            t.item_bits[bit].positive(),
                            n_ug[bit].negative(),
                        ]);
                    }

                    // 3. Underground propagation: underground[d] →
                    //    neighbor's underground channel matches this
                    //    tile's underground channel.
                    let t_ug = Self::ug_channel(&t, d);
                    let n_ug = Self::ug_channel(&n, d);
                    for bit in 0..nb {
                        cnf.add(&[
                            t.underground[d].negative(),
                            t_ug[bit].negative(),
                            n_ug[bit].positive(),
                        ]);
                        cnf.add(&[
                            t.underground[d].negative(),
                            t_ug[bit].positive(),
                            n_ug[bit].negative(),
                        ]);
                    }
                }

                // 4. UG output → surface: ug_out facing d → this tile's
                //    surface item matches this tile's underground channel
                //    for the incoming direction (d).
                for &d in &ALL_DIRS {
                    let t_ug = Self::ug_channel(&t, d);
                    for bit in 0..nb {
                        cnf.add(&[
                            t.is_ug_out.negative(),
                            t.out_dir[d].negative(),
                            t_ug[bit].negative(),
                            t.item_bits[bit].positive(),
                        ]);
                        cnf.add(&[
                            t.is_ug_out.negative(),
                            t.out_dir[d].negative(),
                            t_ug[bit].positive(),
                            t.item_bits[bit].negative(),
                        ]);
                    }
                }
            }
        }
    }

    // -- Boundary conditions ------------------------------------------------

    fn encode_boundaries(&self, cnf: &mut Cnf, zone: &CrossingZone) {
        // Track which local tiles have boundary conditions.
        let mut boundary_tiles = std::collections::HashSet::new();

        for b in &zone.boundaries {
            let lx = (b.x - zone.x) as u32;
            let ly = (b.y - zone.y) as u32;
            if lx >= self.width || ly >= self.height {
                continue;
            }

            boundary_tiles.insert((lx, ly));
            let t = self.tiles[self.idx(lx, ly)];
            let d = entity_dir_to_idx(b.direction);

            if b.is_input {
                // Input: items enter zone flowing dir d. Tile can be
                // belt (surface) or ug_in (items enter underground).
                cnf.add(&[t.is_belt.positive(), t.is_ug_in.positive()]);
                cnf.add(&[t.out_dir[d].positive()]);
            } else {
                // Output: items exit zone flowing dir d. Tile can be
                // belt or ug_out.
                cnf.add(&[t.is_belt.positive(), t.is_ug_out.positive()]);
                cnf.add(&[t.out_dir[d].positive()]);
            }

            // Fix item bits.
            let item_idx = self
                .item_names
                .iter()
                .position(|n| *n == b.item)
                .unwrap_or(0);
            for bit in 0..self.n_item_bits as usize {
                let val = (item_idx >> bit) & 1;
                if val == 1 {
                    cnf.add(&[t.item_bits[bit].positive()]);
                } else {
                    cnf.add(&[t.item_bits[bit].negative()]);
                }
            }
        }

        // Non-boundary edge tiles: block output toward off-grid.
        for y in 0..self.height {
            for x in 0..self.width {
                if boundary_tiles.contains(&(x, y)) {
                    continue;
                }
                let t = self.tiles[self.idx(x, y)];
                for &d in &ALL_DIRS {
                    let (dx, dy) = dir_delta(d);
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;
                    if !self.in_bounds(nx, ny) {
                        // Non-boundary tile can't output off-grid.
                        cnf.add(&[t.is_belt.negative(), t.out_dir[d].negative()]);
                        cnf.add(&[t.is_ug_out.negative(), t.out_dir[d].negative()]);
                    }
                }
            }
        }

        // Forced-empty tiles: no surface entity allowed (tap-off passage).
        for &(ex, ey) in &zone.forced_empty {
            let lx = (ex - zone.x) as u32;
            let ly = (ey - zone.y) as u32;
            if lx >= self.width || ly >= self.height {
                continue;
            }
            let t = self.tiles[self.idx(lx, ly)];
            cnf.add(&[t.is_belt.negative()]);
            cnf.add(&[t.is_ug_in.negative()]);
            cnf.add(&[t.is_ug_out.negative()]);
        }
    }

    // -- Solution extraction ------------------------------------------------

    fn extract_solution(
        &self,
        model: &[Lit],
        zone: &CrossingZone,
        belt_tier: &str,
    ) -> Vec<PlacedEntity> {
        let model_set: std::collections::HashSet<Lit> = model.iter().copied().collect();
        let is_true = |v: Var| model_set.contains(&v.positive());

        let mut entities = Vec::new();

        for y in 0..self.height {
            for x in 0..self.width {
                let t = self.tiles[self.idx(x, y)];

                let belt = is_true(t.is_belt);
                let ug_in = is_true(t.is_ug_in);
                let ug_out = is_true(t.is_ug_out);

                if !belt && !ug_in && !ug_out {
                    continue;
                }

                let dir = ALL_DIRS
                    .iter()
                    .find(|&&d| is_true(t.out_dir[d]))
                    .copied()
                    .unwrap_or(DIR_S);

                let item_idx = if self.n_item_bits > 0 {
                    let mut idx = 0usize;
                    for bit in 0..self.n_item_bits as usize {
                        if is_true(t.item_bits[bit]) {
                            idx |= 1 << bit;
                        }
                    }
                    idx.min(self.item_names.len().saturating_sub(1))
                } else {
                    0
                };

                let item_name = self.item_names.get(item_idx).cloned();

                let (entity_name, io_type) = if belt {
                    (belt_tier.to_string(), None)
                } else if ug_in {
                    (
                        ug_name_for_tier(belt_tier).to_string(),
                        Some("input".to_string()),
                    )
                } else {
                    (
                        ug_name_for_tier(belt_tier).to_string(),
                        Some("output".to_string()),
                    )
                };

                entities.push(PlacedEntity {
                    name: entity_name,
                    x: zone.x + x as i32,
                    y: zone.y + y as i32,
                    direction: idx_to_entity_dir(dir),
                    recipe: None,
                    carries: item_name,
                    io_type,
                    segment_id: Some(format!("crossing:{}:{}", zone.x, zone.y)),
                    rate: None,
                    mirror: false,
                });
            }
        }

        entities
    }
}

fn ug_name_for_tier(belt_tier: &str) -> &str {
    match belt_tier {
        "fast-transport-belt" => "fast-underground-belt",
        "express-transport-belt" => "express-underground-belt",
        _ => "underground-belt",
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Solve a crossing zone, returning placed entities or None if unsatisfiable.
pub fn solve_crossing_zone(
    zone: &CrossingZone,
    max_ug_reach: u32,
    belt_tier: &str,
) -> Option<CrossingZoneSolution> {
    let item_names: Vec<String> = {
        let mut names: Vec<String> = zone.boundaries.iter().map(|b| b.item.clone()).collect();
        names.sort();
        names.dedup();
        names
    };

    let encoder = CrossingEncoder::new(zone.width, zone.height, item_names);
    let cnf = encoder.encode(zone, max_ug_reach);

    let variables = encoder.total_vars;
    let clauses = cnf.count;

    let start = std::time::Instant::now();

    let mut solver = Solver::new();
    solver.add_formula(&cnf.formula);

    let sat = solver.solve().ok()?;
    let solve_time_us = start.elapsed().as_micros() as u64;

    if !sat {
        return None;
    }

    let model: Vec<Lit> = solver.model().unwrap_or_default().to_vec();
    let entities = encoder.extract_solution(&model, zone, belt_tier);

    Some(CrossingZoneSolution {
        entities,
        stats: CrossingZoneStats {
            variables,
            clauses,
            solve_time_us,
            zone_width: zone.width,
            zone_height: zone.height,
        },
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn simple_crossing_zone(width: u32, height: u32) -> CrossingZone {
        let mid_x = width / 2;
        let mid_y = height / 2;

        CrossingZone {
            x: 0,
            y: 0,
            width,
            height,
            boundaries: vec![
                // Trunk in (top)
                ZoneBoundary {
                    x: mid_x as i32,
                    y: 0,
                    direction: EntityDirection::South,
                    item: "iron-plate".into(),
                    is_input: true,
                },
                // Trunk out (bottom)
                ZoneBoundary {
                    x: mid_x as i32,
                    y: (height - 1) as i32,
                    direction: EntityDirection::South,
                    item: "iron-plate".into(),
                    is_input: false,
                },
                // Tap-off in (left)
                ZoneBoundary {
                    x: 0,
                    y: mid_y as i32,
                    direction: EntityDirection::East,
                    item: "copper-plate".into(),
                    is_input: true,
                },
                // Tap-off out (right)
                ZoneBoundary {
                    x: (width - 1) as i32,
                    y: mid_y as i32,
                    direction: EntityDirection::East,
                    item: "copper-plate".into(),
                    is_input: false,
                },
            ],
            forced_empty: vec![],
        }
    }

    #[test]
    fn test_3x3_crossing_solvable() {
        let zone = simple_crossing_zone(3, 3);
        let result = solve_crossing_zone(&zone, 4, "transport-belt");
        assert!(result.is_some(), "3x3 crossing should be solvable");

        let solution = result.unwrap();
        assert!(!solution.entities.is_empty());

        // No overlapping positions.
        let mut positions: Vec<(i32, i32)> =
            solution.entities.iter().map(|e| (e.x, e.y)).collect();
        let total = positions.len();
        positions.sort();
        positions.dedup();
        assert_eq!(total, positions.len(), "No duplicate positions");

        eprintln!(
            "3x3 solution ({} vars, {} clauses, {}µs):",
            solution.stats.variables, solution.stats.clauses, solution.stats.solve_time_us
        );
        for e in &solution.entities {
            eprintln!(
                "  ({},{}) {} {:?} carries={:?} io={:?}",
                e.x, e.y, e.name, e.direction, e.carries, e.io_type
            );
        }
    }

    #[test]
    fn test_5x3_crossing_solvable() {
        let zone = CrossingZone {
            x: 10,
            y: 20,
            width: 5,
            height: 3,
            boundaries: vec![
                ZoneBoundary {
                    x: 12,
                    y: 20,
                    direction: EntityDirection::South,
                    item: "iron-plate".into(),
                    is_input: true,
                },
                ZoneBoundary {
                    x: 12,
                    y: 22,
                    direction: EntityDirection::South,
                    item: "iron-plate".into(),
                    is_input: false,
                },
                ZoneBoundary {
                    x: 10,
                    y: 21,
                    direction: EntityDirection::East,
                    item: "copper-plate".into(),
                    is_input: true,
                },
                ZoneBoundary {
                    x: 14,
                    y: 21,
                    direction: EntityDirection::East,
                    item: "copper-plate".into(),
                    is_input: false,
                },
            ],
            forced_empty: vec![],
        };

        let result = solve_crossing_zone(&zone, 4, "transport-belt");
        assert!(result.is_some(), "5x3 crossing should be solvable");

        let solution = result.unwrap();
        eprintln!(
            "5x3 solution ({} vars, {} clauses, {}µs):",
            solution.stats.variables, solution.stats.clauses, solution.stats.solve_time_us
        );
        for e in &solution.entities {
            eprintln!(
                "  ({},{}) {} {:?} carries={:?} io={:?}",
                e.x, e.y, e.name, e.direction, e.carries, e.io_type
            );
        }
    }

    #[test]
    fn test_impossible_zone_returns_none() {
        // 1x1 zone with two conflicting boundary requirements.
        let zone = CrossingZone {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
            boundaries: vec![
                ZoneBoundary {
                    x: 0,
                    y: 0,
                    direction: EntityDirection::South,
                    item: "iron-plate".into(),
                    is_input: true,
                },
                ZoneBoundary {
                    x: 0,
                    y: 0,
                    direction: EntityDirection::East,
                    item: "copper-plate".into(),
                    is_input: true,
                },
            ],
            forced_empty: vec![],
        };

        let result = solve_crossing_zone(&zone, 4, "transport-belt");
        assert!(result.is_none(), "Conflicting 1x1 should be UNSAT");
    }

    #[test]
    fn test_stats_populated() {
        let zone = simple_crossing_zone(3, 3);
        let result = solve_crossing_zone(&zone, 4, "transport-belt").unwrap();
        assert!(result.stats.variables > 0);
        assert!(result.stats.clauses > 0);
        assert_eq!(result.stats.zone_width, 3);
        assert_eq!(result.stats.zone_height, 3);
    }
}
