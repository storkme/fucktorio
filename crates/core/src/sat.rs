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
        self.encode_single_incoming(&mut cnf);
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

                // ug_out pairing: ug_out facing d must receive underground[d]
                // from its "tail" tile — the tile in direction -d.  Without
                // this, an orphaned ug_out can appear with no matching ug_in.
                for &d in &ALL_DIRS {
                    let (dx, dy) = dir_delta(d);
                    let px = x as i32 - dx;
                    let py = y as i32 - dy;
                    if self.in_bounds(px, py) {
                        let p = self.tiles[self.idx(px as u32, py as u32)];
                        cnf.add(&[
                            t.is_ug_out.negative(),
                            t.out_dir[d].negative(),
                            p.underground[d].positive(),
                        ]);
                    } else {
                        // No underground can arrive from off-grid.
                        cnf.add(&[t.is_ug_out.negative(), t.out_dir[d].negative()]);
                    }
                }

                for &d in &ALL_DIRS {
                    let (dx, dy) = dir_delta(d);
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;

                    if self.in_bounds(nx, ny) {
                        let n = self.tiles[self.idx(nx as u32, ny as u32)];

                        // ug_in facing d -> next tile MUST have underground[d].
                        // Distance-1 pairs (ug_in directly adjacent to ug_out with
                        // no underground passage) are forbidden — they're pointless
                        // and confuse the validator's UG pairing algorithm.
                        cnf.add(&[
                            t.is_ug_in.negative(),
                            t.out_dir[d].negative(),
                            n.underground[d].positive(),
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

    // -- At most one incoming surface edge per tile --------------------------
    //
    // Prevents closed loops (A→B→C→A) and spurious item merges.  For every
    // pair of distinct directions d1, d2, the two upstream tiles p1 and p2
    // cannot both be outputting toward this tile simultaneously.
    //
    // This is valid for pure routing (no item splits/merges) and is safe for
    // crossing zones where each path is a simple chain with no merging.

    fn encode_single_incoming(&self, cnf: &mut Cnf) {
        for y in 0..self.height {
            for x in 0..self.width {
                // Collect (type_var, out_dir_var) pairs for every neighbor
                // that *could* output toward (x,y).
                // A neighbor at (x-dx, y-dy) facing direction d = (dx,dy)
                // sends items toward (x,y).
                let mut feeders: Vec<(Var, Var)> = Vec::new();
                for &d in &ALL_DIRS {
                    let (dx, dy) = dir_delta(d);
                    let px = x as i32 - dx;
                    let py = y as i32 - dy;
                    if !self.in_bounds(px, py) {
                        continue;
                    }
                    let p = self.tiles[self.idx(px as u32, py as u32)];
                    // Both surface belts and ug_out can output toward us.
                    feeders.push((p.is_belt, p.out_dir[d]));
                    feeders.push((p.is_ug_out, p.out_dir[d]));
                }
                // Pairwise AMO: at most one (type ∧ dir) pair active.
                for i in 0..feeders.len() {
                    for j in (i + 1)..feeders.len() {
                        let (ti, di) = feeders[i];
                        let (tj, dj) = feeders[j];
                        cnf.add(&[
                            ti.negative(),
                            di.negative(),
                            tj.negative(),
                            dj.negative(),
                        ]);
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

                    // 1a. UG-out → surface neighbor: a UG-out emits into
                    //     its downstream tile and the surface item must
                    //     propagate, same as a surface belt. This clause
                    //     was missing from the original encoder and lets
                    //     the solver emit uoc → iron-plate belt chains
                    //     on larger zones.
                    for bit in 0..nb {
                        cnf.add(&[
                            t.is_ug_out.negative(),
                            t.out_dir[d].negative(),
                            t.item_bits[bit].negative(),
                            n.item_bits[bit].positive(),
                        ]);
                        cnf.add(&[
                            t.is_ug_out.negative(),
                            t.out_dir[d].negative(),
                            t.item_bits[bit].positive(),
                            n.item_bits[bit].negative(),
                        ]);
                    }

                    // 2. UG input → underground: ug_in facing d →
                    //    neighbor's underground channel matches this
                    //    tile's surface item.
                    let n_ug = Self::ug_channel(&n, d);
                    for (tb, nb_var) in t.item_bits[..nb].iter().zip(&n_ug[..nb]) {
                        cnf.add(&[
                            t.is_ug_in.negative(),
                            t.out_dir[d].negative(),
                            tb.negative(),
                            nb_var.positive(),
                        ]);
                        cnf.add(&[
                            t.is_ug_in.negative(),
                            t.out_dir[d].negative(),
                            tb.positive(),
                            nb_var.negative(),
                        ]);
                    }

                    // 3. Underground propagation: underground[d] →
                    //    neighbor's underground channel matches this
                    //    tile's underground channel.
                    let t_ug = Self::ug_channel(&t, d);
                    let n_ug = Self::ug_channel(&n, d);
                    for (tu, nu) in t_ug[..nb].iter().zip(&n_ug[..nb]) {
                        cnf.add(&[
                            t.underground[d].negative(),
                            tu.negative(),
                            nu.positive(),
                        ]);
                        cnf.add(&[
                            t.underground[d].negative(),
                            tu.positive(),
                            nu.negative(),
                        ]);
                    }
                }

                // 4. UG output → surface: ug_out facing d → this tile's
                //    surface item matches this tile's underground channel
                //    for the incoming direction (d).
                for &d in &ALL_DIRS {
                    let t_ug = Self::ug_channel(&t, d);
                    for (tu, tb) in t_ug[..nb].iter().zip(&t.item_bits[..nb]) {
                        cnf.add(&[
                            t.is_ug_out.negative(),
                            t.out_dir[d].negative(),
                            tu.negative(),
                            tb.positive(),
                        ]);
                        cnf.add(&[
                            t.is_ug_out.negative(),
                            t.out_dir[d].negative(),
                            tu.positive(),
                            tb.negative(),
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

                // No in-grid entity may output toward an input boundary.
                // Items at an input boundary enter from outside the zone;
                // allowing in-grid paths to flow back into the input would
                // create loops (items circling around an input tile).
                for &fd in &ALL_DIRS {
                    let (fdx, fdy) = dir_delta(fd);
                    let px = lx as i32 - fdx;
                    let py = ly as i32 - fdy;
                    if self.in_bounds(px, py) {
                        let p = self.tiles[self.idx(px as u32, py as u32)];
                        cnf.add(&[p.is_belt.negative(), p.out_dir[fd].negative()]);
                        cnf.add(&[p.is_ug_out.negative(), p.out_dir[fd].negative()]);
                    }
                }
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
                    items: Vec::new(),
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

    #[cfg(not(target_arch = "wasm32"))]
    let start = std::time::Instant::now();

    let mut solver = Solver::new();
    solver.add_formula(&cnf.formula);

    let sat = solver.solve().ok()?;

    #[cfg(not(target_arch = "wasm32"))]
    let solve_time_us = start.elapsed().as_micros() as u64;
    #[cfg(target_arch = "wasm32")]
    let solve_time_us = 0u64;

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

    /// Build a realistic band-shaped zone: a horizontal band that contains
    /// `n_trunks` vertical trunks crossing `n_horizontals` horizontal specs.
    /// Trunks are spaced evenly along the x-axis; horizontal specs run at
    /// distinct y values inside the band. Items are shared across trunks:
    /// real bus layouts carry ≤16 distinct items total, and a given band
    /// usually has fewer (encoder cap is 16 items).
    fn band_zone(width: u32, height: u32, n_trunks: usize, n_horizontals: usize) -> CrossingZone {
        assert!(width >= 4 && height >= 3);
        assert!(n_trunks >= 1 && n_horizontals >= 1);
        assert!(n_horizontals <= (height as usize).saturating_sub(2));

        // Share items across trunks; encoder supports ≤16 items.
        let n_trunk_items = n_trunks.min(6);
        let n_horiz_items = n_horizontals.clamp(1, 4);

        let mut boundaries = Vec::new();

        for i in 0..n_trunks {
            let trunk_x =
                1 + ((i as u32 * (width - 2)) / n_trunks.max(1) as u32) as i32;
            let item = format!("trunk-item-{}", i % n_trunk_items);
            boundaries.push(ZoneBoundary {
                x: trunk_x,
                y: 0,
                direction: EntityDirection::South,
                item: item.clone(),
                is_input: true,
            });
            boundaries.push(ZoneBoundary {
                x: trunk_x,
                y: (height - 1) as i32,
                direction: EntityDirection::South,
                item,
                is_input: false,
            });
        }

        for j in 0..n_horizontals {
            let horiz_y = 1 + j as i32;
            let item = format!("horiz-item-{}", j % n_horiz_items);
            boundaries.push(ZoneBoundary {
                x: 0,
                y: horiz_y,
                direction: EntityDirection::East,
                item: item.clone(),
                is_input: true,
            });
            boundaries.push(ZoneBoundary {
                x: (width - 1) as i32,
                y: horiz_y,
                direction: EntityDirection::East,
                item,
                is_input: false,
            });
        }

        CrossingZone {
            x: 0,
            y: 0,
            width,
            height,
            boundaries,
            forced_empty: vec![],
        }
    }

    /// Validate a solved band: overlap check, port-adjacency carry check,
    /// and flow-trace from each input port to a matching output port.
    ///
    /// Returns a list of human-readable problems. Empty list = valid.
    fn validate_band_solution(
        zone: &CrossingZone,
        solution: &CrossingZoneSolution,
    ) -> Vec<String> {
        use std::collections::HashMap;
        let mut problems = Vec::new();

        let mut by_pos: HashMap<(i32, i32), &PlacedEntity> = HashMap::new();
        for e in &solution.entities {
            if let Some(prev) = by_pos.insert((e.x, e.y), e) {
                problems.push(format!(
                    "overlap: two entities at ({},{}): {} vs {}",
                    e.x, e.y, prev.name, e.name
                ));
            }
        }

        // For each input port, verify there's an entity at the port position
        // carrying the right item.
        for b in &zone.boundaries {
            let ent = match by_pos.get(&(b.x, b.y)) {
                Some(e) => e,
                None => {
                    problems.push(format!(
                        "missing entity at {} port ({},{}) item={}",
                        if b.is_input { "input" } else { "output" },
                        b.x,
                        b.y,
                        b.item
                    ));
                    continue;
                }
            };
            match ent.carries.as_deref() {
                Some(c) if c == b.item => {}
                Some(c) => problems.push(format!(
                    "{} port ({},{}) expected item={} but entity carries {}",
                    if b.is_input { "input" } else { "output" },
                    b.x,
                    b.y,
                    b.item,
                    c
                )),
                None => problems.push(format!(
                    "{} port ({},{}) expected item={} but entity {} carries nothing",
                    if b.is_input { "input" } else { "output" },
                    b.x,
                    b.y,
                    b.item,
                    ent.name
                )),
            }
        }

        // Flow trace: follow each input port through the belt graph until
        // we exit the zone. Verify we land on an output port with a
        // matching item.
        for b in zone.boundaries.iter().filter(|b| b.is_input) {
            let out = trace_flow(zone, &by_pos, b);
            match out {
                Ok((ox, oy, item)) => {
                    let matched = zone.boundaries.iter().any(|p| {
                        !p.is_input && p.x == ox && p.y == oy && p.item == item
                    });
                    if !matched {
                        problems.push(format!(
                            "input ({},{}) item={} traced to ({},{}) item={} — no matching output port",
                            b.x, b.y, b.item, ox, oy, item
                        ));
                    }
                }
                Err(why) => problems.push(format!(
                    "input ({},{}) item={} trace failed: {}",
                    b.x, b.y, b.item, why
                )),
            }
        }

        problems
    }

    fn step(dir: EntityDirection) -> (i32, i32) {
        match dir {
            EntityDirection::North => (0, -1),
            EntityDirection::East => (1, 0),
            EntityDirection::South => (0, 1),
            EntityDirection::West => (-1, 0),
        }
    }

    fn trace_flow(
        zone: &CrossingZone,
        by_pos: &std::collections::HashMap<(i32, i32), &PlacedEntity>,
        input: &ZoneBoundary,
    ) -> Result<(i32, i32, String), String> {
        let mut visited = std::collections::HashSet::new();
        let mut x = input.x;
        let mut y = input.y;
        let x_lo = zone.x;
        let x_hi = zone.x + zone.width as i32 - 1;
        let y_lo = zone.y;
        let y_hi = zone.y + zone.height as i32 - 1;

        for _ in 0..10_000 {
            if !visited.insert((x, y)) {
                return Err(format!("loop at ({},{})", x, y));
            }
            let ent = match by_pos.get(&(x, y)) {
                Some(e) => e,
                None => return Err(format!("no entity at ({},{})", x, y)),
            };
            let item = ent
                .carries
                .as_ref()
                .ok_or_else(|| format!("entity at ({},{}) carries nothing", x, y))?
                .clone();
            if item != input.item {
                return Err(format!(
                    "item mismatch at ({},{}): expected {} got {}",
                    x, y, input.item, item
                ));
            }
            let next_dir = ent.direction;
            let (dx, dy) = step(next_dir);

            // Underground-belt input: jump to its matching output along the
            // direction, skipping intermediate tiles.
            if ent.name.ends_with("underground-belt")
                && ent.io_type.as_deref() == Some("input")
            {
                let mut jx = x + dx;
                let mut jy = y + dy;
                let mut jumped = false;
                for _ in 0..6 {
                    if let Some(peer) = by_pos.get(&(jx, jy)) {
                        if peer.name == ent.name
                            && peer.io_type.as_deref() == Some("output")
                            && peer.direction == next_dir
                        {
                            x = jx;
                            y = jy;
                            jumped = true;
                            break;
                        }
                    }
                    jx += dx;
                    jy += dy;
                }
                if !jumped {
                    return Err(format!("UG-in at ({},{}) has no matching UG-out", x, y));
                }
                // Continue tracing from the UG-out tile.
            }

            let nx = x + dx;
            let ny = y + dy;

            // Exit: next tile is outside the zone. Done.
            if nx < x_lo || nx > x_hi || ny < y_lo || ny > y_hi {
                return Ok((x, y, item));
            }

            x = nx;
            y = ny;
        }
        Err("trace exceeded 10000 steps".into())
    }

    fn render_band_with_items(
        zone: &CrossingZone,
        solution: &CrossingZoneSolution,
    ) -> String {
        use std::collections::HashMap;
        let w = zone.width as usize;
        let h = zone.height as usize;
        let mut by_pos: HashMap<(i32, i32), &PlacedEntity> = HashMap::new();
        for e in &solution.entities {
            by_pos.insert((e.x, e.y), e);
        }
        let mut out = String::new();
        for y in 0..h {
            out.push_str("    ");
            for x in 0..w {
                if let Some(e) = by_pos.get(&(x as i32 + zone.x, y as i32 + zone.y)) {
                    let tag = match e.name.as_str() {
                        n if n.ends_with("underground-belt") => {
                            match e.io_type.as_deref() {
                                Some("input") => "ui",
                                Some("output") => "uo",
                                _ => "u?",
                            }
                        }
                        _ => match e.direction {
                            EntityDirection::North => " N",
                            EntityDirection::East => " E",
                            EntityDirection::South => " S",
                            EntityDirection::West => " W",
                        },
                    };
                    let item_label = e.carries.as_deref().map(|c| c.chars().next().unwrap_or('?')).unwrap_or('·');
                    out.push_str(&format!("{}{} ", tag, item_label));
                } else {
                    out.push_str(" .. ");
                }
            }
            out.push('\n');
        }
        out
    }

    #[allow(clippy::needless_range_loop)]
    fn render_band_solution(
        zone: &CrossingZone,
        solution: &CrossingZoneSolution,
    ) -> String {
        use std::collections::HashMap;
        let w = zone.width as usize;
        let h = zone.height as usize;
        let mut grid = vec![vec!['.'; w]; h];

        let mut by_pos: HashMap<(i32, i32), &PlacedEntity> = HashMap::new();
        for e in &solution.entities {
            by_pos.insert((e.x, e.y), e);
        }

        for y in 0..h {
            for x in 0..w {
                if let Some(e) = by_pos.get(&(x as i32 + zone.x, y as i32 + zone.y)) {
                    let glyph = if e.name.ends_with("underground-belt") {
                        match (e.direction, e.io_type.as_deref()) {
                            (EntityDirection::North, Some("input")) => '↥',
                            (EntityDirection::North, Some("output")) => '▲',
                            (EntityDirection::East, Some("input")) => '↦',
                            (EntityDirection::East, Some("output")) => '▶',
                            (EntityDirection::South, Some("input")) => '↧',
                            (EntityDirection::South, Some("output")) => '▼',
                            (EntityDirection::West, Some("input")) => '↤',
                            (EntityDirection::West, Some("output")) => '◀',
                            _ => '?',
                        }
                    } else {
                        match e.direction {
                            EntityDirection::North => '↑',
                            EntityDirection::East => '→',
                            EntityDirection::South => '↓',
                            EntityDirection::West => '←',
                        }
                    };
                    grid[y][x] = glyph;
                }
            }
        }

        let mut out = String::new();
        for row in &grid {
            out.push_str("    ");
            out.extend(row.iter());
            out.push('\n');
        }
        out
    }

    fn time_band(
        label: &str,
        width: u32,
        height: u32,
        n_trunks: usize,
        n_horizontals: usize,
        render: bool,
    ) -> Option<CrossingZoneSolution> {
        let zone = band_zone(width, height, n_trunks, n_horizontals);
        let n_ports = zone.boundaries.len();
        let t = std::time::Instant::now();
        let result = solve_crossing_zone(&zone, 4, "transport-belt");
        let elapsed = t.elapsed();
        match result {
            Some(sol) => {
                let problems = validate_band_solution(&zone, &sol);
                let verdict = if problems.is_empty() {
                    "VALID".to_string()
                } else {
                    format!("INVALID ({} issues)", problems.len())
                };
                eprintln!(
                    "  {label}: {width}x{height} trunks={n_trunks} horiz={n_horizontals} ports={n_ports}  {vars} vars  {clauses} clauses  solver={solver_us}µs  wall={wall_ms:.1}ms  {verdict}",
                    vars = sol.stats.variables,
                    clauses = sol.stats.clauses,
                    solver_us = sol.stats.solve_time_us,
                    wall_ms = elapsed.as_secs_f64() * 1e3,
                );
                for p in problems.iter().take(6) {
                    eprintln!("      ✗ {}", p);
                }
                if render {
                    eprint!("{}", render_band_solution(&zone, &sol));
                }
                Some(sol)
            }
            None => {
                eprintln!(
                    "  {label}: {width}x{height} trunks={n_trunks} horiz={n_horizontals} ports={n_ports}  wall={wall_ms:.1}ms  UNSAT",
                    wall_ms = elapsed.as_secs_f64() * 1e3,
                );
                None
            }
        }
    }

    #[test]
    #[ignore]
    fn validate_existing_small_tests() {
        // Re-run the existing small tests through the trace validator to
        // see if they are actually valid or just getting lucky.
        eprintln!("validating existing small cases via flow trace:");

        let cases = [
            ("3x3", simple_crossing_zone(3, 3)),
            ("5x5", simple_crossing_zone(5, 5)),
            ("7x5", simple_crossing_zone(7, 5)),
            ("9x5", simple_crossing_zone(9, 5)),
            ("11x5", simple_crossing_zone(11, 5)),
            ("15x5", simple_crossing_zone(15, 5)),
            ("21x5", simple_crossing_zone(21, 5)),
        ];
        for (label, zone) in cases {
            match solve_crossing_zone(&zone, 4, "transport-belt") {
                Some(sol) => {
                    let problems = validate_band_solution(&zone, &sol);
                    let verdict = if problems.is_empty() {
                        "VALID".to_string()
                    } else {
                        format!("INVALID ({} issues)", problems.len())
                    };
                    eprintln!("  {label}: {verdict}");
                    for p in problems.iter().take(3) {
                        eprintln!("      ✗ {}", p);
                    }
                    if !problems.is_empty() || label == "3x3" {
                        eprint!("{}", render_band_solution(&zone, &sol));
                        if label == "5x5" {
                            eprint!("   items+dirs for 5x5:\n{}", render_band_with_items(&zone, &sol));
                        }
                    }
                }
                None => eprintln!("  {label}: UNSAT"),
            }
        }
    }

    #[test]
    #[ignore]
    fn band_regions_sat_bench() {
        eprintln!("band-regions SAT benchmark:");
        time_band("baseline 5x5", 5, 5, 1, 1, true);
        time_band("small band ", 30, 5, 4, 2, true);
        time_band("medium band", 50, 5, 8, 2, false);
        time_band("tier2 band ", 90, 5, 12, 2, false);
        time_band("tier2 wide ", 90, 5, 16, 3, false);
        time_band("tier4 merged-3", 90, 9, 12, 3, false);
        time_band("tier4 wide ", 124, 7, 14, 3, false);
        time_band("stress big ", 124, 9, 20, 4, false);
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

    /// 4×4 routing for the broken electronic-circuit tap-off zone.
    ///
    /// World coords x:3-6, y:6-9.  Two items cross:
    ///   - copper-plate: enters top-left (3,6) South, exits right-mid (6,8) East
    ///   - copper-cable: enters right (6,7) West, exits bottom (4,9) South
    ///
    /// The broken layout had belt-W at (5,7) feeding ug-in-S at (4,7) — illegal.
    /// The solver must find a path that turns copper-cable South before the UG entrance.
    #[test]
    fn test_4x4_electronic_circuit_routing() {
        let zone = CrossingZone {
            x: 3,
            y: 6,
            width: 4,
            height: 4,
            boundaries: vec![
                // IN1: copper-plate enters top-left, flowing South into grid
                ZoneBoundary {
                    x: 3,
                    y: 6,
                    direction: EntityDirection::South,
                    item: "copper-plate".into(),
                    is_input: true,
                },
                // IN2: copper-cable enters right column y=7, flowing West into grid
                ZoneBoundary {
                    x: 6,
                    y: 7,
                    direction: EntityDirection::West,
                    item: "copper-cable".into(),
                    is_input: true,
                },
                // OUT1: copper-plate exits right column y=8, flowing East
                ZoneBoundary {
                    x: 6,
                    y: 8,
                    direction: EntityDirection::East,
                    item: "copper-plate".into(),
                    is_input: false,
                },
                // OUT2: copper-cable exits bottom row x=4, flowing South
                ZoneBoundary {
                    x: 4,
                    y: 9,
                    direction: EntityDirection::South,
                    item: "copper-cable".into(),
                    is_input: false,
                },
            ],
            forced_empty: vec![],
        };

        let result = solve_crossing_zone(&zone, 4, "fast-transport-belt");
        assert!(result.is_some(), "4×4 electronic-circuit routing should be solvable");

        let solution = result.unwrap();

        // Verify no overlapping positions.
        let mut positions: Vec<(i32, i32)> =
            solution.entities.iter().map(|e| (e.x, e.y)).collect();
        let total = positions.len();
        positions.sort();
        positions.dedup();
        assert_eq!(total, positions.len(), "No duplicate positions");

        eprintln!(
            "\n4×4 solution: {} entities ({} vars, {} clauses, {}µs)",
            solution.entities.len(),
            solution.stats.variables,
            solution.stats.clauses,
            solution.stats.solve_time_us,
        );

        // Print a grid so we can eyeball it.
        let by_pos: std::collections::HashMap<(i32, i32), &crate::models::PlacedEntity> =
            solution.entities.iter().map(|e| ((e.x, e.y), e)).collect();

        eprintln!("     x=3        x=4        x=5        x=6");
        for wy in 6..=9 {
            eprint!("y={wy} ");
            for wx in 3..=6 {
                if let Some(e) = by_pos.get(&(wx, wy)) {
                    let sym = match (&e.direction, &e.io_type) {
                        (_, Some(t)) if t == "input" => "UG↓in".to_string(),
                        (_, Some(_)) => "UG↓out".to_string(),
                        (EntityDirection::North, _) => format!("↑({})", &e.carries.as_deref().unwrap_or("?")[..2]),
                        (EntityDirection::South, _) => format!("↓({})", &e.carries.as_deref().unwrap_or("?")[..2]),
                        (EntityDirection::East,  _) => format!("→({})", &e.carries.as_deref().unwrap_or("?")[..2]),
                        (EntityDirection::West,  _) => format!("←({})", &e.carries.as_deref().unwrap_or("?")[..2]),
                    };
                    eprint!("{sym:<10} ");
                } else {
                    eprint!(".          ");
                }
            }
            eprintln!();
        }
        eprintln!();

        for e in &solution.entities {
            eprintln!(
                "  ({},{}) {} {:?} carries={:?} io={:?}",
                e.x, e.y, e.name, e.direction, e.carries, e.io_type
            );
        }
    }

    /// 3×4 grown-region experiment for the tier2_electronic_circuit
    /// sideload bug. Bbox x:3-5, y:9-12. Three item pairs cross:
    ///   - iron-plate: enters left (3,10) East, exits right (5,10) East
    ///   - copper-cable col-3: enters top (3,9) South, exits bottom (3,12) South
    ///   - copper-cable col-4: enters top (4,9) South, exits right (5,11) East
    ///
    /// The question is whether the existing SAT encoder can find a
    /// routing that avoids sideloading (3,9) south-belt into a putative
    /// iron-plate UG input at (3,10). A valid solution exists by
    /// undergrounding col-3 copper-cable around the iron-plate crossing.
    #[test]
    fn test_3x4_tier2_ec_grown_region() {
        let zone = CrossingZone {
            x: 3,
            y: 9,
            width: 3,
            height: 4,
            boundaries: vec![
                ZoneBoundary {
                    x: 3, y: 10,
                    direction: EntityDirection::East,
                    item: "iron-plate".into(),
                    is_input: true,
                },
                ZoneBoundary {
                    x: 5, y: 10,
                    direction: EntityDirection::East,
                    item: "iron-plate".into(),
                    is_input: false,
                },
                ZoneBoundary {
                    x: 3, y: 9,
                    direction: EntityDirection::South,
                    item: "copper-cable".into(),
                    is_input: true,
                },
                ZoneBoundary {
                    x: 3, y: 12,
                    direction: EntityDirection::South,
                    item: "copper-cable".into(),
                    is_input: false,
                },
                ZoneBoundary {
                    x: 4, y: 9,
                    direction: EntityDirection::South,
                    item: "copper-cable".into(),
                    is_input: true,
                },
                ZoneBoundary {
                    x: 5, y: 11,
                    direction: EntityDirection::East,
                    item: "copper-cable".into(),
                    is_input: false,
                },
            ],
            forced_empty: vec![],
        };

        let result = solve_crossing_zone(&zone, 7, "fast-transport-belt");
        match &result {
            Some(sol) => eprintln!(
                "\n3×4 SAT SOLVED: {} entities ({} vars, {} clauses, {}µs)",
                sol.entities.len(),
                sol.stats.variables,
                sol.stats.clauses,
                sol.stats.solve_time_us,
            ),
            None => eprintln!("\n3×4 SAT returned None (UNSAT or encoder limitation)"),
        }
        let solution = result.expect("3×4 grown region should be solvable");

        let by_pos: std::collections::HashMap<(i32, i32), &crate::models::PlacedEntity> =
            solution.entities.iter().map(|e| ((e.x, e.y), e)).collect();

        eprintln!("       x=3        x=4        x=5");
        for wy in 9..=12 {
            eprint!("y={wy:<2} ");
            for wx in 3..=5 {
                if let Some(e) = by_pos.get(&(wx, wy)) {
                    let carry = e.carries.as_deref().unwrap_or("??");
                    let tag = &carry[..2];
                    let sym = match (&e.direction, &e.io_type) {
                        (d, Some(t)) if t == "input" => {
                            let arrow = match d {
                                EntityDirection::North => "↑",
                                EntityDirection::East => "→",
                                EntityDirection::South => "↓",
                                EntityDirection::West => "←",
                            };
                            format!("UGin{arrow}{tag}")
                        }
                        (d, Some(_)) => {
                            let arrow = match d {
                                EntityDirection::North => "↑",
                                EntityDirection::East => "→",
                                EntityDirection::South => "↓",
                                EntityDirection::West => "←",
                            };
                            format!("UGot{arrow}{tag}")
                        }
                        (EntityDirection::North, _) => format!("↑({tag})"),
                        (EntityDirection::South, _) => format!("↓({tag})"),
                        (EntityDirection::East,  _) => format!("→({tag})"),
                        (EntityDirection::West,  _) => format!("←({tag})"),
                    };
                    eprint!("{sym:<10} ");
                } else {
                    eprint!(".          ");
                }
            }
            eprintln!();
        }
        eprintln!();

        for e in &solution.entities {
            eprintln!(
                "  ({},{}) {} {:?} carries={:?} io={:?}",
                e.x, e.y, e.name, e.direction, e.carries, e.io_type
            );
        }
    }
}
