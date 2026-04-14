//! Bus layout routing: trunk belt placement, tap-off coordination, balancer family stamping, and output mergers.
//!
//! Each item that flows between rows gets a dedicated vertical bus lane.
//! Lanes run SOUTH (top to bottom). At the consuming row, the lane turns
//! EAST into the row's input belt (tap-off). When a tap-off crosses another
//! lane's vertical segment, the tap-off goes underground (EAST) past it.
//!
//! Port of `src/bus/bus_router.py`:
//! - Lines 1-700: trunk placement + tap-off infrastructure (Phase 1)
//! - Lines 700-1400: N-to-M balancer family stamping, producer-to-input wiring (Phase 2)
//! - Lines 1400-1880: output mergers and N→1 Z-wrap balancing (Phase 3)

use std::cmp::Ordering;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::models::{SolverResult, PlacedEntity, EntityDirection};
use crate::bus::placer::RowSpan;

/// Per-lane capacity for each belt tier (half of total throughput).
const LANE_CAPACITY_TABLE: &[(&str, f64)] = &[
    ("transport-belt", 7.5),
    ("fast-transport-belt", 15.0),
    ("express-transport-belt", 22.5),
];

/// Entity names that occupy multiple tiles (sized by `machine_size()`).
pub(crate) const MACHINE_ENTITIES: &[&str] = &[
    "assembling-machine-1",
    "assembling-machine-2",
    "assembling-machine-3",
    "chemical-plant",
    "electric-furnace",
    "oil-refinery",
    "electromagnetic-plant",
    "cryogenic-plant",
    "foundry",
    "biochamber",
];

/// A single vertical lane on the main bus, carrying one item (or fluid) from its
/// source row(s) down to its consumer row(s). Lanes run SOUTH; at each consumer the
/// lane turns EAST via a tap-off. See `docs/ghost-pipeline-contracts.md` for the
/// phase-by-phase contract the router promises.
#[derive(Clone, Debug)]
pub struct BusLane {
    /// Item (or fluid) name this lane carries.
    pub item: String,
    /// Column (x-coordinate) assigned to this lane in the layout.
    pub x: i32,
    /// Y-coordinate where items enter this lane (0 for external inputs, or the
    /// producer row's output y for intermediate items).
    pub source_y: i32,
    /// Indices into the row-spans list for rows that consume from this lane.
    pub consumer_rows: Vec<usize>,
    /// Primary producer row index, or `None` for externally supplied items.
    pub producer_row: Option<usize>,
    /// Total throughput (items/s or fluid/s) for belt/pipe tier selection.
    pub rate: f64,
    /// Whether this lane carries a fluid (pipe + underground-pipe) instead of items.
    pub is_fluid: bool,
    /// Y-coordinates where tap-offs turn this lane EAST into a consumer row.
    pub tap_off_ys: Vec<i32>,
    /// Additional producer row indices beyond the primary (e.g. multiple sub-rows
    /// producing the same item).
    pub extra_producer_rows: Vec<usize>,
    /// Y-coordinate of the lane-balancer splitter, or `None` if no balancer is needed.
    pub balancer_y: Option<i32>,
    /// Index into the [`LaneFamily`] list when this lane is fed by an N-to-M balancer.
    pub family_id: Option<usize>,
    /// `(row_index, x, y)` of each pipe-to-ground exit connecting a fluid producer
    /// to this lane.
    pub fluid_port_positions: Vec<(usize, i32, i32)>,
    /// `(row_index, x, y)` of each fluid producer's output pipe port.
    pub fluid_output_port_positions: Vec<(usize, i32, i32)>,
    /// `(y_start, y_end)` inclusive — the full vertical range occupied by a
    /// balancer family block. Trunk segments inside this range are skipped.
    pub family_balancer_range: Option<(i32, i32)>,
}

impl BusLane {
    fn new(
        item: String,
        source_y: i32,
        consumer_rows: Vec<usize>,
        producer_row: Option<usize>,
        rate: f64,
        is_fluid: bool,
    ) -> Self {
        Self {
            item,
            x: 0,
            source_y,
            consumer_rows,
            producer_row,
            rate,
            is_fluid,
            tap_off_ys: Vec::new(),
            extra_producer_rows: Vec::new(),
            balancer_y: None,
            family_id: None,
            fluid_port_positions: Vec::new(),
            fluid_output_port_positions: Vec::new(),
            family_balancer_range: None,
        }
    }

    /// Collect all producer row indices for this lane.
    pub(crate) fn all_producers(&self) -> Vec<usize> {
        let mut rows = Vec::new();
        if let Some(pr) = self.producer_row {
            rows.push(pr);
        }
        rows.extend(&self.extra_producer_rows);
        rows
    }
}

/// An N-to-M balancer block that merges N producer outputs into M sibling trunk
/// lanes for one item, ensuring even distribution. Stamped as a pre-solved SAT
/// template from `balancer_library`.
#[derive(Clone, Debug)]
pub struct LaneFamily {
    /// Item name shared by all lanes in this family.
    pub item: String,
    /// `(N producers, M lanes)` — the balancer shape.
    pub shape: (usize, usize),
    /// Row indices of the N producers feeding into this balancer.
    pub producer_rows: Vec<usize>,
    /// X-coordinates of the M output lanes, populated after column assignment.
    pub lane_xs: Vec<i32>,
    /// Y-coordinate of the first (topmost) row of the balancer block.
    pub balancer_y_start: i32,
    /// Y-coordinate of the last row of the balancer block (inclusive).
    pub balancer_y_end: i32,
    /// Combined throughput across all lanes, used for belt tier selection.
    pub total_rate: f64,
}

/// Score a proposed lane ordering: the number of tap-off rays that have
/// to cross other lanes' active ranges. Lower is better. Also penalises
/// family-template input landing columns that overlap lanes to the right
/// of the family block, pushing family blocks rightmost.
pub(crate) fn score_lane_ordering(ordered: &[BusLane], row_spans: &[RowSpan]) -> usize {
    let mut score = 0;

    fn active_range(lane: &BusLane, row_spans: &[RowSpan]) -> (i32, i32) {
        let all_p = lane.all_producers();
        if !all_p.is_empty() && !lane.consumer_rows.is_empty() {
            let start = all_p.iter()
                .map(|&p| row_spans[p].output_belt_y)
                .min()
                .unwrap();
            let end = if !lane.tap_off_ys.is_empty() {
                lane.tap_off_ys.iter().copied().max().unwrap()
            } else {
                start
            };
            (start, end)
        } else if !lane.tap_off_ys.is_empty() {
            let end = lane.tap_off_ys.iter().copied().max().unwrap();
            (lane.source_y, end)
        } else {
            let end = all_p.iter()
                .map(|&p| row_spans[p].output_belt_y)
                .max()
                .unwrap_or(lane.source_y);
            (lane.source_y, end)
        }
    }

    let ranges: Vec<(i32, i32)> = ordered.iter().map(|ln| active_range(ln, row_spans)).collect();

    for (pos, lane) in ordered.iter().enumerate() {
        for &tap_y in &lane.tap_off_ys {
            for &(rs, re) in &ranges[(pos + 1)..] {
                if rs <= tap_y && tap_y <= re {
                    score += 1;
                }
            }
        }
        let all_producers = lane.all_producers();
        for &pri in &all_producers {
            let ret_y = row_spans[pri].output_belt_y;
            for &(rs, re) in &ranges[(pos + 1)..] {
                if rs <= ret_y && ret_y <= re {
                    score += 1;
                }
            }
        }
    }

    let templates = crate::bus::balancer_library::balancer_templates();
    let n = ordered.len();
    for (pos, lane) in ordered.iter().enumerate() {
        if let Some(fid) = lane.family_id {
            if pos > 0 && ordered[pos - 1].family_id == Some(fid) {
                continue;
            }
            let fam_count = ordered[pos..].iter()
                .take_while(|l| l.family_id == Some(fid))
                .count();
            let ox = pos + 1;
            let (fn_, fm) = {
                let all_p = lane.all_producers();
                (all_p.len().max(1), fam_count)
            };
            if let Some(tpl) = templates.get(&(fn_ as u32, fm as u32)) {
                for &(dx, _) in tpl.input_tiles {
                    let landing_x = (ox as i32) + dx + 1;
                    for rpos in (pos + fam_count)..n {
                        let rx = (rpos + 1) as i32;
                        if rx == landing_x {
                            score += 100;
                        }
                    }
                }
            }
        }
    }

    score
}

fn family_contiguous(ordered: &[BusLane]) -> bool {
    let mut seen_ranges: FxHashMap<usize, (usize, usize)> = FxHashMap::default();
    for (i, ln) in ordered.iter().enumerate() {
        if let Some(fid) = ln.family_id {
            let (lo, hi) = seen_ranges.get(&fid).copied().unwrap_or((i, i));
            seen_ranges.insert(fid, (lo.min(i), hi.max(i)));
        }
    }
    let mut counts: FxHashMap<usize, usize> = FxHashMap::default();
    for ln in ordered {
        if let Some(fid) = ln.family_id {
            *counts.entry(fid).or_insert(0) += 1;
        }
    }
    seen_ranges.iter().all(|(fid, (lo, hi))| hi - lo + 1 == counts[fid])
}

fn find_best_permutation(solid: &[BusLane], row_spans: &[RowSpan]) -> Vec<BusLane> {
    if solid.is_empty() {
        return Vec::new();
    }
    let n = solid.len();
    let mut indices: Vec<usize> = (0..n).collect();
    let mut best_order: Vec<usize> = indices.clone();
    let mut best_score = score_lane_ordering(
        &indices.iter().map(|&i| solid[i].clone()).collect::<Vec<_>>(),
        row_spans,
    );
    let mut c = vec![0; n];
    let mut i = 0;
    while i < n {
        if c[i] < i {
            if i % 2 == 0 {
                indices.swap(0, i);
            } else {
                indices.swap(c[i], i);
            }
            let ordered: Vec<BusLane> = indices.iter().map(|&idx| solid[idx].clone()).collect();
            if family_contiguous(&ordered) {
                let score = score_lane_ordering(&ordered, row_spans);
                if score < best_score {
                    best_score = score;
                    best_order = indices.clone();
                }
            }
            c[i] += 1;
            i = 0;
        } else {
            c[i] = 0;
            i += 1;
        }
    }
    best_order.iter().map(|&i| solid[i].clone()).collect()
}

fn hill_climb_lane_order(solid: &[BusLane], row_spans: &[RowSpan]) -> Vec<BusLane> {
    let mut order = solid.to_vec();
    order.sort_by_key(|ln| {
        let fid = ln.family_id.unwrap_or(usize::MAX) as i32;
        let y = ln.tap_off_ys.iter().min().copied().map(|y| -y).unwrap_or(9999);
        (fid, y)
    });
    let n = order.len();
    let mut best_score = score_lane_ordering(&order, row_spans);
    loop {
        let mut improved = false;
        'outer: for i in 0..n {
            for j in (i + 1)..n {
                order.swap(i, j);
                if family_contiguous(&order) {
                    let score = score_lane_ordering(&order, row_spans);
                    if score < best_score {
                        best_score = score;
                        improved = true;
                        continue 'outer;
                    }
                }
                order.swap(i, j);
            }
        }
        if !improved { break; }
    }
    order
}

/// Optimize the left-to-right ordering of lanes to minimise tap-off /
/// return crossings while keeping family lanes contiguous. Exact search
/// for ≤7 solid lanes, hill-climbing above. Fluid lanes are appended
/// unchanged at the right.
pub(crate) fn optimize_lane_order(lanes: &[BusLane], row_spans: &[RowSpan]) -> Vec<BusLane> {
    if lanes.len() <= 1 {
        return lanes.to_vec();
    }
    let solid: Vec<BusLane> = lanes.iter().filter(|ln| !ln.is_fluid).cloned().collect();
    let fluid: Vec<BusLane> = lanes.iter().filter(|ln| ln.is_fluid).cloned().collect();
    let best_solid = if solid.len() <= 7 {
        find_best_permutation(&solid, row_spans)
    } else {
        hill_climb_lane_order(&solid, row_spans)
    };
    let mut result = best_solid;
    result.extend(fluid);
    let crossing_score = score_lane_ordering(&result, row_spans);
    crate::trace::emit(crate::trace::TraceEvent::LaneOrderOptimized {
        ordering: result.iter().map(|ln| ln.item.clone()).collect(),
        crossing_score,
    });
    result
}

/// Lanes are ordered so that lanes tapping off at earlier (higher) rows
/// are placed on the LEFT, reducing tap-off crossings.
///
/// Returns (lanes, families) — `families` is the list of balancer blocks
/// (possibly empty); each lane's `family_id` indexes into it.
pub fn plan_bus_lanes(
    solver_result: &SolverResult,
    row_spans: &[RowSpan],
    max_belt_tier: Option<&str>,
) -> Result<(Vec<BusLane>, Vec<LaneFamily>), String> {
    let mut lanes: Vec<BusLane> = Vec::new();
    let mut seen_items: FxHashSet<String> = FxHashSet::default();

    // Build item_to_consumers map
    let mut item_to_consumers: FxHashMap<String, Vec<usize>> = FxHashMap::default();
    for (idx, rs) in row_spans.iter().enumerate() {
        for inp in &rs.spec.inputs {
            item_to_consumers.entry(inp.item.clone()).or_default().push(idx);
        }
    }

    // External inputs (solid AND fluid).
    let mut fluid_source_y = 0;
    for ext in &solver_result.external_inputs {
        if seen_items.contains(&ext.item) {
            continue;
        }
        let consumers = item_to_consumers.get(&ext.item).cloned().unwrap_or_default();
        if !consumers.is_empty() {
            let src_y = if ext.is_fluid {
                let ys = fluid_source_y;
                fluid_source_y += 1;
                ys
            } else {
                0
            };
            lanes.push(BusLane::new(
                ext.item.clone(),
                src_y,
                consumers,
                None,
                ext.rate,
                ext.is_fluid,
            ));
            seen_items.insert(ext.item.clone());
        }
    }

    // Intermediate items (solid AND fluid).
    let mut item_to_producers: FxHashMap<String, Vec<usize>> = FxHashMap::default();
    let mut item_to_rate: FxHashMap<String, f64> = FxHashMap::default();
    let mut item_is_fluid: FxHashMap<String, bool> = FxHashMap::default();

    for (idx, rs) in row_spans.iter().enumerate() {
        for out in &rs.spec.outputs {
            item_to_producers.entry(out.item.clone()).or_default().push(idx);
            *item_to_rate.entry(out.item.clone()).or_insert(0.0) += out.rate * rs.machine_count as f64;
            item_is_fluid.insert(out.item.clone(), out.is_fluid);
        }
    }

    for (item, producer_rows) in item_to_producers.iter() {
        if seen_items.contains(item) {
            continue;
        }
        let consumers = item_to_consumers.get(item).cloned().unwrap_or_default();
        if consumers.is_empty() {
            continue;
        }
        let first_producer = producer_rows[0];
        let rate = item_to_rate.get(item).copied().unwrap_or(0.0);
        let is_fluid = item_is_fluid.get(item).copied().unwrap_or(false);
        lanes.push(BusLane {
            item: item.clone(),
            x: 0,
            source_y: row_spans[first_producer].output_belt_y,
            consumer_rows: consumers,
            producer_row: Some(first_producer),
            rate,
            is_fluid,
            extra_producer_rows: producer_rows[1..].to_vec(),
            ..Default::default()
        });
        seen_items.insert(item.clone());
    }

    // Split lanes that exceed max belt tier capacity
    let (mut lanes, mut families) = split_overflowing_lanes(&lanes, row_spans, max_belt_tier)?;

    // Pre-compute tap-off ys before sorting
    for lane in &mut lanes {
        lane.tap_off_ys = find_tap_off_ys(lane, row_spans);
        if lane.is_fluid {
            // Collect fluid port pipe positions for tap-off routing.
            // Filter by lane.item so rows with multiple fluid ports (e.g. oil-refinery)
            // only contribute the port(s) for this specific fluid.
            for &ri in &lane.consumer_rows {
                let rs = &row_spans[ri];
                for (ref item, px, py) in &rs.fluid_port_pipes {
                    if *item == lane.item {
                        lane.fluid_port_positions.push((ri, *px, *py));
                    }
                }
            }
            // Collect producer-side output port pipes (also filtered by item).
            let mut producer_rows = Vec::new();
            if let Some(pr) = lane.producer_row {
                producer_rows.push(pr);
            }
            producer_rows.extend(&lane.extra_producer_rows);
            for ri in producer_rows {
                let rs = &row_spans[ri];
                for (ref item, px, py) in &rs.fluid_output_port_pipes {
                    if *item == lane.item {
                        lane.fluid_output_port_positions.push((ri, *px, *py));
                    }
                }
            }
        }
    }

    // Tighten fluid-external source_y
    for lane in &mut lanes {
        if !lane.is_fluid || lane.producer_row.is_some() {
            continue;
        }
        let mut port_ys: Vec<i32> = lane.tap_off_ys.clone();
        port_ys.extend(lane.fluid_port_positions.iter().map(|(_, _, py)| *py));
        port_ys.extend(lane.fluid_output_port_positions.iter().map(|(_, _, py)| *py));
        if !port_ys.is_empty() {
            let min_y = *port_ys.iter().min().unwrap();
            lane.source_y = (min_y - 1).max(0);
        }
    }

    // Compute lane balancer positions for intermediate solid lanes
    for lane in &mut lanes {
        if lane.is_fluid {
            continue;
        }
        if !lane.consumer_rows.is_empty() {
            continue;
        }
        let all_producers = lane.all_producers();

        if all_producers.len() <= 1 {
            continue;
        }

        let last_sideload_y = all_producers.iter()
            .map(|&pri| row_spans[pri].output_belt_y)
            .max()
            .unwrap();
        let bal_y = last_sideload_y + 1;
        let tap_set: FxHashSet<i32> = lane.tap_off_ys.iter().copied().collect();
        if !tap_set.contains(&bal_y) && !tap_set.contains(&(bal_y + 1)) {
            lane.balancer_y = Some(bal_y);
        }
    }

    // Optimize lane left-to-right ordering
    lanes = optimize_lane_order(&lanes, row_spans);

    // Assign x-columns with 1-tile spacing
    for (i, lane) in lanes.iter_mut().enumerate() {
        lane.x = (i + 1) as i32;
    }

    // Fill in lane_xs on each family
    for (fid, fam) in families.iter_mut().enumerate() {
        fam.lane_xs = lanes.iter()
            .filter(|ln| ln.family_id == Some(fid))
            .map(|ln| ln.x)
            .collect();
        fam.lane_xs.sort_unstable();

        // Verify contiguous columns
        if !fam.lane_xs.is_empty() {
            let expected: Vec<i32> = (fam.lane_xs[0]..fam.lane_xs[0] + fam.lane_xs.len() as i32).collect();
            if fam.lane_xs != expected {
                return Err(format!(
                    "Balancer for item {} shape {:?} needs contiguous lane columns, but lane x's are {:?}",
                    fam.item, fam.shape, fam.lane_xs
                ));
            }
        }
    }

    // Resolve balancer_y_end from actual template heights and propagate the
    // full balancer zone to each lane so trunks skip the entire zone.
    let templates = crate::bus::balancer_library::balancer_templates();
    for fam in &mut families {
        let (n, m) = (fam.shape.0 as u32, fam.shape.1 as u32);
        // Find the effective template height: direct match or decomposed.
        let tpl_height = templates.get(&(n, m)).map(|t| t.height)
            .or_else(|| {
                // Decomposition: find divisor g where (n/g, m/g) has a template.
                (1..=n).rev().find_map(|g| {
                    if n % g == 0 && m % g == 0 {
                        templates.get(&(n / g, m / g)).map(|t| t.height)
                    } else {
                        None
                    }
                })
            });
        if let Some(h) = tpl_height {
            fam.balancer_y_end = fam.balancer_y_start + h as i32 - 1;
            let range = (fam.balancer_y_start, fam.balancer_y_end);
            for lane in lanes.iter_mut() {
                if lane.family_id.is_some() && lane.item == fam.item {
                    lane.family_balancer_range = Some(range);
                }
            }
        }
    }

    crate::trace::emit(crate::trace::TraceEvent::LanesPlanned {
        lanes: lanes.iter().map(|l| crate::trace::LaneInfo {
            item: l.item.clone(),
            x: l.x,
            rate: l.rate,
            is_fluid: l.is_fluid,
            source_y: l.source_y,
            tap_off_ys: l.tap_off_ys.clone(),
            consumer_rows: l.consumer_rows.clone(),
            producer_row: l.producer_row,
            family_id: l.family_id,
        }).collect(),
        families: families.iter().map(|f| crate::trace::FamilyInfo {
            item: f.item.clone(),
            shape: f.shape,
            lane_xs: f.lane_xs.clone(),
            balancer_y_start: f.balancer_y_start,
            balancer_y_end: f.balancer_y_end,
            total_rate: f.total_rate,
            producer_rows: f.producer_rows.clone(),
        }).collect(),
        bus_width: lanes.iter().map(|l| l.x).max().map(|x| x + 1).unwrap_or(0),
    });

    Ok((lanes, families))
}

impl Default for BusLane {
    fn default() -> Self {
        Self {
            item: String::new(),
            x: 0,
            source_y: 0,
            consumer_rows: Vec::new(),
            producer_row: None,
            rate: 0.0,
            is_fluid: false,
            tap_off_ys: Vec::new(),
            extra_producer_rows: Vec::new(),
            balancer_y: None,
            family_id: None,
            fluid_port_positions: Vec::new(),
            fluid_output_port_positions: Vec::new(),
            family_balancer_range: None,
        }
    }
}

/// Splitter name mapping by belt tier.
const SPLITTER_MAP: &[(&str, &str)] = &[
    ("transport-belt", "splitter"),
    ("fast-transport-belt", "fast-splitter"),
    ("express-transport-belt", "express-splitter"),
];

/// Underground belt name mapping by belt tier.
const UNDERGROUND_MAP: &[(&str, &str)] = &[
    ("transport-belt", "underground-belt"),
    ("fast-transport-belt", "fast-underground-belt"),
    ("express-transport-belt", "express-underground-belt"),
];

pub(crate) fn splitter_for_belt(belt: &str) -> &'static str {
    SPLITTER_MAP.iter()
        .find(|(b, _)| *b == belt)
        .map(|(_, s)| *s)
        .unwrap_or("splitter")
}

fn underground_for_belt(belt: &str) -> &'static str {
    UNDERGROUND_MAP.iter()
        .find(|(b, _)| *b == belt)
        .map(|(_, u)| *u)
        .unwrap_or("underground-belt")
}

/// Split lanes whose rate exceeds the available belt's per-lane capacity.
fn split_overflowing_lanes(
    lanes: &[BusLane],
    row_spans: &[RowSpan],
    max_belt_tier: Option<&str>,
) -> Result<(Vec<BusLane>, Vec<LaneFamily>), String> {
    let default_cap = LANE_CAPACITY_TABLE.last().map(|(_, c)| *c).unwrap_or(15.0);
    let max_lane_cap = if let Some(tier) = max_belt_tier {
        LANE_CAPACITY_TABLE.iter()
            .find(|(name, _)| *name == tier)
            .map(|(_, cap)| *cap)
            .unwrap_or(default_cap)
    } else {
        default_cap
    };

    let mut result: Vec<BusLane> = Vec::new();
    let mut families: Vec<LaneFamily> = Vec::new();

    for lane in lanes {
        if lane.is_fluid {
            result.push(lane.clone());
            continue;
        }

        let n_splits = if lane.rate > max_lane_cap {
            ((lane.rate / max_lane_cap).ceil() as usize).max(1)
        } else {
            1
        };
        // External input lanes (no producer) can share a trunk across multiple
        // consumer rows — split only by capacity.  Intermediate lanes still need
        // 1-per-consumer because route_intermediate_lane only handles tap_off_ys[0].
        let is_external_input =
            lane.producer_row.is_none() && lane.extra_producer_rows.is_empty();
        let n_splits = if is_external_input {
            n_splits
        } else {
            n_splits.max(lane.consumer_rows.len())
        };

        // External inputs serving multiple consumers via fewer trunks: consolidation
        if is_external_input && lane.consumer_rows.len() > n_splits {
            crate::trace::emit(crate::trace::TraceEvent::LaneConsolidated {
                item: lane.item.clone(),
                rate: lane.rate,
                consumer_count: lane.consumer_rows.len(),
                n_trunk_lanes: n_splits,
                rate_per_lane: lane.rate / n_splits as f64,
            });
        }

        if n_splits <= 1 {
            result.push(lane.clone());
            continue;
        }

        crate::trace::emit(crate::trace::TraceEvent::LaneSplit {
            item: lane.item.clone(),
            rate: lane.rate,
            max_lane_cap,
            n_splits,
        });

        // Distribute consumer rows round-robin
        let mut consumers_per_split: Vec<Vec<usize>> = vec![Vec::new(); n_splits];
        for (i, &ri) in lane.consumer_rows.iter().enumerate() {
            consumers_per_split[i % n_splits].push(ri);
        }

        // Distribute producer rows by rate
        let all_producer_rows = lane.all_producers();

        let mut producers_per_split: Vec<Vec<usize>> = vec![Vec::new(); n_splits];
        let mut split_prod_rate: Vec<f64> = vec![0.0; n_splits];

        for &pri in &all_producer_rows {
            let rs = &row_spans[pri];
            let prod_rate: f64 = rs.spec.outputs.iter()
                .filter(|o| o.item == lane.item)
                .map(|o| o.rate * rs.machine_count as f64)
                .sum();
            let target = split_prod_rate.iter()
                .enumerate()
                .min_by(|(_, &a), (_, &b)| a.partial_cmp(&b).unwrap_or(Ordering::Equal))
                .map(|(i, _)| i)
                .unwrap_or(0);
            producers_per_split[target].push(pri);
            split_prod_rate[target] += prod_rate;
        }

        let is_collector = lane.consumer_rows.is_empty();

        // Detect N-to-M balancer requirement
        let n_lanes_with_consumers = if is_collector {
            n_splits
        } else {
            consumers_per_split.iter().filter(|c| !c.is_empty()).count()
        };
        let n_producers = all_producer_rows.len();

        let mut family_id: Option<usize> = None;
        let mut family_source_y: Option<i32> = None;

        // Create a family balancer for any multi-lane case where the
        // number of producers is <= the number of lanes. The original
        // condition (`n_producers < n_lanes`) only handled fan-out
        // (1→N, 2→3, etc). It missed the parallel case (e.g. 2→2),
        // which tried to route each producer to its own trunk via a
        // `ret:` sideload — but inner lanes get boxed in by adjacent
        // trunks and there's no clean tile for the sideload to land
        // on. Always using a family for multi-lane puts a balancer
        // block between the producers and the trunks, which handles
        // the merge cleanly. The `n_producers > n_lanes` (fan-in)
        // path is left unchanged for now.
        if n_producers >= 1
            && n_lanes_with_consumers >= 2
            && n_producers <= n_lanes_with_consumers
        {
            let shape = (n_producers, n_lanes_with_consumers);
            family_id = Some(families.len());

            let balancer_y_start = if n_producers == 1 {
                row_spans[all_producer_rows[0]].output_belt_y
            } else {
                all_producer_rows.iter()
                    .map(|&p| row_spans[p].y_end)
                    .max()
                    .unwrap_or(0)
            };

            families.push(LaneFamily {
                item: lane.item.clone(),
                shape,
                producer_rows: all_producer_rows.to_vec(),
                lane_xs: Vec::new(),
                balancer_y_start,
                balancer_y_end: balancer_y_start,
                total_rate: lane.rate,
            });
            family_source_y = Some(balancer_y_start + 1);
        }

        // Create split lanes
        for si in 0..n_splits {
            let consumers = consumers_per_split[si].clone();
            if consumers.is_empty() && !is_collector && si > 0 {
                continue;  // skip empty splits
            }
            let split_rate = lane.rate / n_splits as f64;

            if let Some(fid) = family_id {
                result.push(BusLane {
                    item: lane.item.clone(),
                    x: 0,
                    source_y: family_source_y.unwrap_or(0),
                    consumer_rows: consumers,
                    producer_row: None,
                    rate: split_rate,
                    is_fluid: false,
                    family_id: Some(fid),
                    ..Default::default()
                });
                continue;
            }

            let prods = &producers_per_split[si];
            let first_prod = if prods.is_empty() { None } else { Some(prods[0]) };
            let extra_prods = if prods.len() > 1 { prods[1..].to_vec() } else { Vec::new() };
            let split_source_y = if prods.is_empty() {
                lane.source_y
            } else {
                prods.iter()
                    .map(|&p| row_spans[p].output_belt_y)
                    .min()
                    .unwrap_or(lane.source_y)
            };

            result.push(BusLane {
                item: lane.item.clone(),
                x: 0,
                source_y: split_source_y,
                consumer_rows: consumers,
                producer_row: first_prod,
                rate: split_rate,
                is_fluid: false,
                extra_producer_rows: extra_prods,
                ..Default::default()
            });
        }
    }

    Ok((result, families))
}

/// Find y-coordinates where this lane taps off into consumer rows.
fn find_tap_off_ys(lane: &BusLane, row_spans: &[RowSpan]) -> Vec<i32> {
    let mut tap_ys: Vec<i32> = Vec::new();

    for &ri in &lane.consumer_rows {
        let rs = &row_spans[ri];
        if lane.is_fluid {
            // Fluid lanes tap off at the fluid port y positions
            if !rs.fluid_port_ys.is_empty() {
                tap_ys.push(rs.fluid_port_ys[0]);
            }
        } else {
            // Solid lanes
            let solid_inputs: Vec<_> = rs.spec.inputs.iter()
                .filter(|f| !f.is_fluid)
                .collect();
            for (input_idx, inp) in solid_inputs.iter().enumerate() {
                if inp.item == lane.item && input_idx < rs.input_belt_y.len() {
                    tap_ys.push(rs.input_belt_y[input_idx]);
                    break;
                }
            }
        }
    }

    tap_ys
}

/// Return the total bus width needed for the given lanes.
pub fn bus_width_for_lanes(lanes: &[BusLane]) -> i32 {
    if lanes.is_empty() {
        2
    } else {
        (lanes.len() + 2) as i32
    }
}

/// Stamp a balancer template at the family's origin position.
///
/// Template entity tiles are offset by the family's stamp origin
/// (x = min(lane_xs), y = balancer_y_start). The item each entity
/// carries is set to the family's item. Belt and splitter tiers are
/// chosen from the family's total rate so the balancer matches its
/// sibling trunks.
pub(crate) fn stamp_family_balancer(
    family: &LaneFamily,
    max_belt_tier: Option<&str>,
) -> Result<Vec<PlacedEntity>, String> {
    use crate::bus::balancer_library::balancer_templates;
    use crate::common::belt_entity_for_rate;

    let templates = balancer_templates();
    let (n, m) = (family.shape.0 as u32, family.shape.1 as u32);
    let template_key = (n, m);

    if family.lane_xs.is_empty() {
        return Err(format!("LaneFamily for item {} has no lane_xs assigned", family.item));
    }

    let belt_tier = belt_entity_for_rate(family.total_rate, max_belt_tier);
    let splitter_name = splitter_for_belt(belt_tier);
    let ug_name = underground_for_belt(belt_tier);
    let balancer_seg_id = Some(format!("balancer:{}", family.item));

    if let Some(template) = templates.get(&template_key) {
        // Direct template match.
        let origin_x = *family.lane_xs.iter().min().unwrap();
        let origin_y = family.balancer_y_start;

        let mut entities = template.stamp(
            origin_x, origin_y, belt_tier, splitter_name, ug_name,
            Some(&family.item),
        );
        for ent in &mut entities {
            ent.segment_id = balancer_seg_id.clone();
        }
        return Ok(entities);
    }

    // Decomposition fallback: try to split (N, M) into groups that have
    // templates. Search for a divisor g of N where (N/g, M/g) has a template.
    // E.g., (6,8) → g=2 → 2 copies of (3,4). (5,10) → g=5 → 5 copies of (1,2).
    for g in (1..=n).rev() {
        if n % g != 0 || m % g != 0 {
            continue;
        }
        let sub_n = n / g;
        let sub_m = m / g;
        if let Some(sub_template) = templates.get(&(sub_n, sub_m)) {
            let mut all_entities = Vec::new();
            let _producers_per_group = sub_n as usize;
            let lanes_per_group = sub_m as usize;

            for gi in 0..(g as usize) {
                let lane_start = gi * lanes_per_group;
                let lane_end = (lane_start + lanes_per_group).min(family.lane_xs.len());
                let lane_chunk = &family.lane_xs[lane_start..lane_end];
                if lane_chunk.is_empty() {
                    continue;
                }
                let sub_origin_x = *lane_chunk.iter().min().unwrap();
                let sub_origin_y = family.balancer_y_start;

                let mut ents = sub_template.stamp(
                    sub_origin_x, sub_origin_y, belt_tier, splitter_name, ug_name,
                    Some(&family.item),
                );
                for ent in &mut ents {
                    ent.segment_id = Some(format!("balancer:{}:{}", family.item, gi));
                }
                all_entities.extend(ents);
            }
            return Ok(all_entities);
        }
    }

    // No template and no decomposition possible — skip.
    Ok(Vec::new())
}

/// Render path entities from A*-routed belts and underground segments.
///
/// Gaps in the path (manhattan distance > 1 between consecutive tiles)
/// indicate underground belt jumps — UG entry at the first tile, UG exit
/// at the second. Surface tiles get regular belt entities.
///
/// For single-tile paths, `direction_hint` determines the belt direction.
pub(crate) fn render_path(
    path: &[(i32, i32)],
    item: &str,
    belt_name: &str,
    direction_hint: EntityDirection,
    segment_id: Option<String>,
    rate: Option<f64>,
) -> Vec<PlacedEntity> {
    let mut entities: Vec<PlacedEntity> = Vec::new();
    if path.is_empty() {
        return entities;
    }

    if path.len() == 1 {
        entities.push(PlacedEntity {
            name: belt_name.to_string(),
            x: path[0].0,
            y: path[0].1,
            direction: direction_hint,
            carries: Some(item.to_string()),
            ..Default::default()
        });
        return entities;
    }

    let ug_name = underground_for_belt(belt_name);
    let mut i = 0;
    while i < path.len() {
        let (x, y) = path[i];
        if i + 1 < path.len() {
            let (nx, ny) = path[i + 1];
            let dx = nx - x;
            let dy = ny - y;
            let dist = (dx.abs() + dy.abs()) as usize;

            if dist > 1 {
                // Underground jump: entry at (x,y), exit at (nx,ny)
                let ug_dir = if dx != 0 {
                    if dx > 0 { EntityDirection::East } else { EntityDirection::West }
                } else if dy > 0 { EntityDirection::South } else { EntityDirection::North };
                entities.push(PlacedEntity {
                    name: ug_name.to_string(),
                    x,
                    y,
                    direction: ug_dir,
                    io_type: Some("input".to_string()),
                    carries: Some(item.to_string()),
                    ..Default::default()
                });
                entities.push(PlacedEntity {
                    name: ug_name.to_string(),
                    x: nx,
                    y: ny,
                    direction: ug_dir,
                    io_type: Some("output".to_string()),
                    carries: Some(item.to_string()),
                    ..Default::default()
                });
                i += 2;
                continue;
            }

            // Surface belt: determine direction from movement
            let direction = if dx != 0 {
                if dx > 0 { EntityDirection::East } else { EntityDirection::West }
            } else if dy != 0 {
                if dy > 0 { EntityDirection::South } else { EntityDirection::North }
            } else {
                direction_hint // shouldn't happen
            };

            entities.push(PlacedEntity {
                name: belt_name.to_string(),
                x,
                y,
                direction,
                carries: Some(item.to_string()),
                ..Default::default()
            });
            i += 1;
        } else {
            // Last tile
            entities.push(PlacedEntity {
                name: belt_name.to_string(),
                x,
                y,
                direction: direction_hint,
                carries: Some(item.to_string()),
                ..Default::default()
            });
            i += 1;
        }
    }

    if segment_id.is_some() || rate.is_some() {
        for ent in &mut entities {
            if segment_id.is_some() {
                ent.segment_id = segment_id.clone();
            }
            if rate.is_some() {
                ent.rate = rate;
            }
        }
    }

    entities
}

/// Wire each producer's WEST output belt to its designated template input.
///
/// The template places SOUTH-facing input tiles at its top row. The
/// horizontal WEST feeder segment (from the row's leftmost output belt
/// at `x=bw` to `(input_x+1, out_y)`) is A*-routed via the negotiator.
///
/// The SOUTH descent column (from the feeder row down to the balancer's
/// top row) is placed manually since it sits inside the balancer's
/// reserved x-columns.
///
/// Producer-to-input assignment: topmost producer (smallest out_y)
/// maps to leftmost input tile (smallest dx). This keeps the per-
/// producer SOUTH columns non-crossing.
pub(crate) fn merge_output_rows(
    output_rows: &[usize],
    item: &str,
    row_spans: &[RowSpan],
    merge_start_y: i32,
    max_belt_tier: Option<&str>,
) -> (Vec<PlacedEntity>, i32, i32) {
    use crate::common::belt_entity_for_rate;

    let mut entities: Vec<PlacedEntity> = Vec::new();
    let n = output_rows.len();
    if n == 0 {
        return (entities, merge_start_y, 0);
    }
    let merger_seg_id = Some(format!("merger:{}", item));

    // Calculate total rate
    let total_rate = output_rows.iter()
        .map(|&ri| {
            if ri >= row_spans.len() {
                0.0
            } else {
                row_spans[ri].spec.outputs.iter()
                    .filter(|o| o.item == item)
                    .map(|o| o.rate * row_spans[ri].machine_count as f64)
                    .sum::<f64>()
            }
        })
        .sum::<f64>();

    let belt_name = belt_entity_for_rate(total_rate * 2.0, max_belt_tier);
    let splitter_name = splitter_for_belt(belt_name);

    let merge_x = output_rows.iter()
        .map(|&ri| if ri < row_spans.len() { row_spans[ri].row_width } else { 0 })
        .max()
        .unwrap_or(0) + 1;

    for (idx, &ri) in output_rows.iter().enumerate() {
        if ri >= row_spans.len() {
            continue;
        }
        let out_y = row_spans[ri].output_belt_y;
        let col_x = merge_x + (n - 1 - idx) as i32; // first row rightmost, last row at merge_x

        // Extend EAST belts from the row's rightmost tile to the merge column.
        let rw = row_spans[ri].row_width;
        for x in rw..col_x {
            entities.push(PlacedEntity {
                name: belt_name.to_string(),
                x,
                y: out_y,
                direction: EntityDirection::East,
                carries: Some(item.to_string()),
                segment_id: merger_seg_id.clone(),
                rate: Some(total_rate),
                ..Default::default()
            });
        }

        // SOUTH column from out_y to merge_start_y.
        for y in out_y..merge_start_y {
            entities.push(PlacedEntity {
                name: belt_name.to_string(),
                x: col_x,
                y,
                direction: EntityDirection::South,
                carries: Some(item.to_string()),
                segment_id: merger_seg_id.clone(),
                rate: Some(total_rate),
                ..Default::default()
            });
        }
    }

    // Sequential splitter cascade merging N south columns into 1.
    // Columns are at x = merge_x (row n-1) through merge_x + n-1 (row 0).
    //
    // At each step we place a SOUTH splitter that merges two adjacent columns.
    // A SOUTH splitter at (x, y) spans tiles (x, y) and (x+1, y), accepting
    // input from (x, y-1) and (x+1, y-1), outputting at (x, y+1) and (x+1, y+1).
    // We use the left output (x) and discard the right.
    //
    // Between steps, ALL surviving columns need a continuation belt at each row
    // so they stay connected through to the next splitter.
    let mut y_cursor = merge_start_y;
    // Active columns, sorted left-to-right.
    let mut active: Vec<i32> = (0..n as i32).map(|i| merge_x + i).collect();

    while active.len() > 1 {
        let right_x = active.pop().unwrap();
        let left_x = *active.last().unwrap();

        // Splitter merging left_x and left_x+1 (right_x should equal left_x+1)
        // If not adjacent, route right column west first.
        if right_x != left_x + 1 {
            for x in ((left_x + 2)..=right_x).rev() {
                entities.push(PlacedEntity {
                    name: belt_name.to_string(),
                    x,
                    y: y_cursor,
                    direction: EntityDirection::West,
                    carries: Some(item.to_string()),
                    segment_id: merger_seg_id.clone(),
                    rate: Some(total_rate),
                    ..Default::default()
                });
            }
        }
        // Pass-through belts at the splitter row for uninvolved columns.
        // The splitter occupies (left_x, y_cursor) and (left_x+1, y_cursor).
        for &ax in &active {
            if ax != left_x && ax != left_x + 1 {
                entities.push(PlacedEntity {
                    name: belt_name.to_string(),
                    x: ax,
                    y: y_cursor,
                    direction: EntityDirection::South,
                    carries: Some(item.to_string()),
                    segment_id: merger_seg_id.clone(),
                    rate: Some(total_rate),
                    ..Default::default()
                });
            }
        }
        entities.push(PlacedEntity {
            name: splitter_name.to_string(),
            x: left_x,
            y: y_cursor,
            direction: EntityDirection::South,
            carries: Some(item.to_string()),
            segment_id: merger_seg_id.clone(),
            rate: Some(total_rate),
            ..Default::default()
        });
        y_cursor += 1;

        // Continuation belts below the splitter for all surviving columns.
        for &ax in &active {
            entities.push(PlacedEntity {
                name: belt_name.to_string(),
                x: ax,
                y: y_cursor,
                direction: EntityDirection::South,
                carries: Some(item.to_string()),
                segment_id: merger_seg_id.clone(),
                rate: Some(total_rate),
                ..Default::default()
            });
        }
        y_cursor += 1;
    }

    (entities, y_cursor, merge_x + n as i32)
}

/// Merge N parallel trunk lanes into M output belts using splitters.
///
/// M = ceil(total_rate / full_belt_capacity). The merger block is placed
/// below the last row at merge_start_y. Extends each trunk downward from
/// its end_y to merge_start_y so items can flow into the merger.
///
/// Returns (entities, end_y).
pub(crate) fn is_intermediate(lane: &BusLane) -> bool {
    let has_producers = lane.producer_row.is_some() || !lane.extra_producer_rows.is_empty();
    let has_consumers = !lane.consumer_rows.is_empty();
    has_producers && has_consumers
}

/// Route a solid-item bus lane with belts (external input or collector).
///
/// Port of Python `_route_belt_lane`.
pub(crate) fn trunk_segments(start_y: i32, end_y: i32, skip_ys: &FxHashSet<i32>) -> Vec<(i32, i32)> {
    let mut segments: Vec<(i32, i32)> = Vec::new();
    let mut seg_start: Option<i32> = None;
    for y in start_y..=end_y {
        if skip_ys.contains(&y) {
            if let Some(ss) = seg_start.take() {
                segments.push((ss, y - 1));
            }
        } else if seg_start.is_none() {
            seg_start = Some(y);
        }
    }
    if let Some(ss) = seg_start {
        segments.push((ss, end_y));
    }
    segments
}

// NOTE(cleanup): foreign_trunk_skip_ys removed along with direct-mode.
// It delegated to crate::bus::plan::compute_foreign_yields_for_lane,
// which only direct-mode's route_belt_lane/route_intermediate_lane
// used to decide UG-bridge vs sideload at cross-lane conflicts. Gone
// with the rest of direct-mode below.

/// Extra y-rows to add to trunk skip set so UG-pair tiles don't collide.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ItemFlow, MachineSpec};
    use crate::bus::placer::RowSpan;

    fn make_test_row_span(
        recipe: &str,
        y_start: i32,
        inputs: Vec<ItemFlow>,
        outputs: Vec<ItemFlow>,
        machine_count: usize,
        input_belt_y: Vec<i32>,
    ) -> RowSpan {
        RowSpan {
            y_start,
            y_end: y_start + 3,
            spec: MachineSpec {
                entity: "assembling-machine-3".to_string(),
                recipe: recipe.to_string(),
                count: machine_count as f64,
                inputs,
                outputs,
            },
            machine_count,
            input_belt_y,
            output_belt_y: y_start + 2,
            row_width: 10,
            fluid_port_ys: Vec::new(),
            fluid_port_pipes: Vec::new(),
            fluid_output_port_pipes: Vec::new(),
        }
    }

    #[test]
    fn test_bus_width_for_lanes_empty() {
        assert_eq!(bus_width_for_lanes(&[]), 2);
    }

    #[test]
    fn test_bus_width_for_lanes_single() {
        let lane = BusLane {
            item: "iron-ore".to_string(),
            ..Default::default()
        };
        assert_eq!(bus_width_for_lanes(&[lane]), 3);
    }

    #[test]
    fn test_bus_width_for_lanes_three() {
        let lanes = vec![
            BusLane { item: "iron-ore".to_string(), ..Default::default() },
            BusLane { item: "copper-ore".to_string(), ..Default::default() },
            BusLane { item: "coal".to_string(), ..Default::default() },
        ];
        assert_eq!(bus_width_for_lanes(&lanes), 5);
    }

    #[test]
    fn test_find_tap_off_ys_single_consumer() {
        let lane = BusLane {
            item: "iron-ore".to_string(),
            consumer_rows: vec![0],
            is_fluid: false,
            ..Default::default()
        };

        let row_span = make_test_row_span(
            "iron-plate",
            0,
            vec![ItemFlow { item: "iron-ore".to_string(), rate: 1.0, is_fluid: false }],
            vec![ItemFlow { item: "iron-plate".to_string(), rate: 1.0, is_fluid: false }],
            1,
            vec![1],
        );

        let tap_ys = find_tap_off_ys(&lane, &[row_span]);
        assert_eq!(tap_ys, vec![1]);
    }

    #[test]
    fn test_score_lane_ordering_with_crossing() {
        let lanes = vec![
            BusLane {
                item: "iron-ore".to_string(),
                consumer_rows: vec![0],
                tap_off_ys: vec![1],
                producer_row: None,
                source_y: 0,
                ..Default::default()
            },
            BusLane {
                item: "copper-ore".to_string(),
                consumer_rows: vec![1],
                tap_off_ys: vec![5],
                producer_row: None,
                source_y: 0,
                ..Default::default()
            },
        ];

        let row_spans = vec![
            make_test_row_span(
                "iron-plate",
                0,
                vec![ItemFlow { item: "iron-ore".to_string(), rate: 1.0, is_fluid: false }],
                vec![],
                1,
                vec![1],
            ),
            make_test_row_span(
                "copper-plate",
                4,
                vec![ItemFlow { item: "copper-ore".to_string(), rate: 1.0, is_fluid: false }],
                vec![],
                1,
                vec![5],
            ),
        ];

        let score = score_lane_ordering(&lanes, &row_spans);
        // Iron-ore taps at y=1, copper-ore is active from y=0 to y=5, so 1 crossing
        assert_eq!(score, 1);
    }

    #[test]
    fn test_score_lane_ordering_no_crossing() {
        let lanes = vec![
            BusLane {
                item: "iron-ore".to_string(),
                consumer_rows: vec![0],
                tap_off_ys: vec![10],
                producer_row: None,
                source_y: 0,
                ..Default::default()
            },
            BusLane {
                item: "copper-ore".to_string(),
                consumer_rows: vec![1],
                tap_off_ys: vec![5],
                producer_row: None,
                source_y: 0,
                ..Default::default()
            },
        ];

        let row_spans = vec![
            make_test_row_span(
                "iron-plate",
                8,
                vec![ItemFlow { item: "iron-ore".to_string(), rate: 1.0, is_fluid: false }],
                vec![],
                1,
                vec![10],
            ),
            make_test_row_span(
                "copper-plate",
                4,
                vec![ItemFlow { item: "copper-ore".to_string(), rate: 1.0, is_fluid: false }],
                vec![],
                1,
                vec![5],
            ),
        ];

        let score = score_lane_ordering(&lanes, &row_spans);
        // Iron-ore taps at y=10, copper-ore is only active from y=0 to y=5, no crossing
        assert_eq!(score, 0);
    }

    #[test]
    fn test_stamp_family_balancer() {
        let family = LaneFamily {
            item: "iron-plate".to_string(),
            shape: (1, 2),  // 1 producer, 2 lanes
            producer_rows: vec![0],
            lane_xs: vec![1, 2],
            balancer_y_start: 10,
            balancer_y_end: 11,
            total_rate: 20.0,  // should use fast-transport-belt
        };

        let entities = stamp_family_balancer(&family, None);
        assert!(entities.is_ok());

        let entities = entities.unwrap();
        assert!(!entities.is_empty());
        // Verify that the stamped entities have the correct origin and item
        for e in &entities {
            assert_eq!(e.carries, Some("iron-plate".to_string()));
            assert!(e.x >= 1);  // origin_x should be >= 1 (min of lane_xs)
            assert!(e.y >= 10); // origin_y should be >= 10
        }
    }

    #[test]
    fn test_render_path_single_tile() {
        let path = vec![(5, 10)];
        let entities = render_path(&path, "iron-plate", "transport-belt", EntityDirection::East, None, None);

        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].name, "transport-belt");
        assert_eq!(entities[0].x, 5);
        assert_eq!(entities[0].y, 10);
        assert_eq!(entities[0].direction, EntityDirection::East);
        assert_eq!(entities[0].carries, Some("iron-plate".to_string()));
    }

    #[test]
    fn test_render_path_east_movement() {
        let path = vec![(5, 10), (6, 10), (7, 10)];
        let entities = render_path(&path, "iron-plate", "transport-belt", EntityDirection::East, None, None);

        assert_eq!(entities.len(), 3);
        for e in &entities {
            assert_eq!(e.name, "transport-belt");
            assert_eq!(e.direction, EntityDirection::East);
            assert_eq!(e.carries, Some("iron-plate".to_string()));
        }
    }

    #[test]
    fn test_render_path_south_movement() {
        let path = vec![(5, 10), (5, 11), (5, 12)];
        let entities = render_path(&path, "copper-ore", "transport-belt", EntityDirection::South, None, None);

        assert_eq!(entities.len(), 3);
        for e in &entities {
            assert_eq!(e.name, "transport-belt");
            assert_eq!(e.direction, EntityDirection::South);
            assert_eq!(e.carries, Some("copper-ore".to_string()));
        }
    }

    #[test]
    fn test_render_path_with_underground_jump() {
        // Gap of 3 tiles = underground jump
        let path = vec![(5, 10), (8, 10)];  // x moves from 5 to 8, distance = 3
        let entities = render_path(&path, "iron-plate", "transport-belt", EntityDirection::East, None, None);

        assert_eq!(entities.len(), 2);
        assert_eq!(entities[0].name, "underground-belt");
        assert_eq!(entities[0].x, 5);
        assert_eq!(entities[0].y, 10);
        assert_eq!(entities[1].name, "underground-belt");
        assert_eq!(entities[1].x, 8);
        assert_eq!(entities[1].y, 10);
    }

    #[test]
    fn test_merge_output_rows_single_row() {
        let row_span = make_test_row_span(
            "iron-plate",
            0,
            vec![],
            vec![ItemFlow { item: "iron-plate".to_string(), rate: 10.0, is_fluid: false }],
            1,
            vec![],
        );

        let output_rows = vec![0];
        let (entities, _end_y, _merge_max_x) = merge_output_rows(&output_rows, "iron-plate", &[row_span], 20, None);

        // Single row should extend EAST and SOUTH without splitters
        assert!(!entities.is_empty());
        assert!(entities.iter().all(|e| e.carries.as_deref() == Some("iron-plate")));
    }

    #[test]
    fn test_merge_output_rows_multiple_rows() {
        let row_span1 = make_test_row_span(
            "iron-plate",
            0,
            vec![],
            vec![ItemFlow { item: "iron-plate".to_string(), rate: 10.0, is_fluid: false }],
            1,
            vec![],
        );
        let row_span2 = make_test_row_span(
            "iron-plate",
            0,
            vec![],
            vec![ItemFlow { item: "iron-plate".to_string(), rate: 10.0, is_fluid: false }],
            1,
            vec![],
        );

        let output_rows = vec![0, 1];
        let (entities, _end_y, _merge_max_x) = merge_output_rows(&output_rows, "iron-plate", &[row_span1, row_span2], 20, None);

        // Multiple rows should include splitters
        let splitters = entities.iter().filter(|e| e.name.contains("splitter")).count();
        assert!(splitters > 0, "Expected splitters for multiple rows");
    }

    fn make_solver_result_iron_gear_wheel() -> crate::models::SolverResult {
        crate::models::SolverResult {
            machines: vec![MachineSpec {
                entity: "assembling-machine-3".to_string(),
                recipe: "iron-gear-wheel".to_string(),
                count: 1.0,
                inputs: vec![ItemFlow { item: "iron-plate".to_string(), rate: 2.0, is_fluid: false }],
                outputs: vec![ItemFlow { item: "iron-gear-wheel".to_string(), rate: 1.0, is_fluid: false }],
            }],
            external_inputs: vec![ItemFlow { item: "iron-plate".to_string(), rate: 2.0, is_fluid: false }],
            external_outputs: vec![ItemFlow { item: "iron-gear-wheel".to_string(), rate: 1.0, is_fluid: false }],
            dependency_order: vec!["iron-gear-wheel".to_string()],
        }
    }

    fn make_solver_result_plastic_bar() -> crate::models::SolverResult {
        crate::models::SolverResult {
            machines: vec![MachineSpec {
                entity: "assembling-machine-3".to_string(),
                recipe: "plastic-bar".to_string(),
                count: 1.0,
                inputs: vec![
                    ItemFlow { item: "coal".to_string(), rate: 1.5, is_fluid: false },
                    ItemFlow { item: "petroleum-gas".to_string(), rate: 2.0, is_fluid: true },
                ],
                outputs: vec![ItemFlow { item: "plastic-bar".to_string(), rate: 2.0, is_fluid: false }],
            }],
            external_inputs: vec![
                ItemFlow { item: "coal".to_string(), rate: 1.5, is_fluid: false },
                ItemFlow { item: "petroleum-gas".to_string(), rate: 2.0, is_fluid: true },
            ],
            external_outputs: vec![ItemFlow { item: "plastic-bar".to_string(), rate: 2.0, is_fluid: false }],
            dependency_order: vec!["plastic-bar".to_string()],
        }
    }

    #[test]
    fn test_plan_bus_lanes_iron_gear_wheel_single_solid_input() {
        let sr = make_solver_result_iron_gear_wheel();

        // One consumer row for the iron-gear-wheel machine.
        let row_span = make_test_row_span(
            "iron-gear-wheel",
            5,
            vec![ItemFlow { item: "iron-plate".to_string(), rate: 2.0, is_fluid: false }],
            vec![ItemFlow { item: "iron-gear-wheel".to_string(), rate: 1.0, is_fluid: false }],
            1,
            vec![6],  // input belt at y=6
        );

        let (lanes, families) = plan_bus_lanes(&sr, &[row_span], None)
            .expect("plan_bus_lanes should succeed for iron-gear-wheel");

        // Should have exactly 1 lane for iron-plate
        assert_eq!(lanes.len(), 1, "Expected exactly 1 lane (iron-plate), got {:?}", lanes.iter().map(|l| &l.item).collect::<Vec<_>>());
        assert_eq!(lanes[0].item, "iron-plate");
        assert!(!lanes[0].is_fluid, "iron-plate lane should not be fluid");
        assert_eq!(families.len(), 0, "No balancer family needed for 1 external input");

        // Lane x should be assigned (>= 1)
        assert!(lanes[0].x >= 1, "Lane x should be >= 1 after assignment");
    }

    #[test]
    fn test_plan_bus_lanes_iron_gear_wheel_lane_count() {
        let sr = make_solver_result_iron_gear_wheel();

        let row_span = make_test_row_span(
            "iron-gear-wheel",
            5,
            vec![ItemFlow { item: "iron-plate".to_string(), rate: 2.0, is_fluid: false }],
            vec![ItemFlow { item: "iron-gear-wheel".to_string(), rate: 1.0, is_fluid: false }],
            1,
            vec![6],
        );

        let (lanes, _families) = plan_bus_lanes(&sr, &[row_span], None).unwrap();

        // iron-gear-wheel is the final output, not consumed internally, so no lane for it
        // Only iron-plate (the external input) needs a lane
        let item_names: Vec<&str> = lanes.iter().map(|l| l.item.as_str()).collect();
        assert!(item_names.contains(&"iron-plate"), "iron-plate lane expected");
        assert!(!item_names.contains(&"iron-gear-wheel"), "iron-gear-wheel is final output, should not get a bus lane");
    }

    #[test]
    fn test_plan_bus_lanes_plastic_bar_fluid_lane_created() {
        let sr = make_solver_result_plastic_bar();

        let row_span = make_test_row_span(
            "plastic-bar",
            5,
            vec![
                ItemFlow { item: "coal".to_string(), rate: 1.5, is_fluid: false },
                ItemFlow { item: "petroleum-gas".to_string(), rate: 2.0, is_fluid: true },
            ],
            vec![ItemFlow { item: "plastic-bar".to_string(), rate: 2.0, is_fluid: false }],
            1,
            vec![6, 7],  // two input belt y positions
        );

        let (lanes, _families) = plan_bus_lanes(&sr, &[row_span], None)
            .expect("plan_bus_lanes should succeed for plastic-bar");

        // Should have lanes for coal and petroleum-gas (plastic-bar is final output)
        let item_names: Vec<&str> = lanes.iter().map(|l| l.item.as_str()).collect();
        assert!(item_names.contains(&"coal"), "coal lane expected");
        assert!(item_names.contains(&"petroleum-gas"), "petroleum-gas lane expected");

        // petroleum-gas lane must be fluid
        let pg_lane = lanes.iter().find(|l| l.item == "petroleum-gas")
            .expect("petroleum-gas lane must exist");
        assert!(pg_lane.is_fluid, "petroleum-gas lane must have is_fluid=true");

        // coal lane must not be fluid
        let coal_lane = lanes.iter().find(|l| l.item == "coal")
            .expect("coal lane must exist");
        assert!(!coal_lane.is_fluid, "coal lane must have is_fluid=false");
    }

    #[test]
    fn test_plan_bus_lanes_fluid_not_first() {
        // Solid lanes should come before fluid lanes in the ordering
        let sr = make_solver_result_plastic_bar();

        let row_span = make_test_row_span(
            "plastic-bar",
            5,
            vec![
                ItemFlow { item: "coal".to_string(), rate: 1.5, is_fluid: false },
                ItemFlow { item: "petroleum-gas".to_string(), rate: 2.0, is_fluid: true },
            ],
            vec![ItemFlow { item: "plastic-bar".to_string(), rate: 2.0, is_fluid: false }],
            1,
            vec![6, 7],
        );

        let (lanes, _families) = plan_bus_lanes(&sr, &[row_span], None).unwrap();

        // optimize_lane_order puts solid before fluid
        let fluid_indices: Vec<usize> = lanes.iter().enumerate()
            .filter(|(_, l)| l.is_fluid)
            .map(|(i, _)| i)
            .collect();
        let solid_indices: Vec<usize> = lanes.iter().enumerate()
            .filter(|(_, l)| !l.is_fluid)
            .map(|(i, _)| i)
            .collect();

        if !fluid_indices.is_empty() && !solid_indices.is_empty() {
            let last_solid = *solid_indices.iter().max().unwrap();
            let first_fluid = *fluid_indices.iter().min().unwrap();
            assert!(last_solid < first_fluid, "All solid lanes should come before fluid lanes");
        }
    }

    #[test]
    fn test_plan_bus_lanes_consumer_row_must_have_tap_off_y() {
        let sr = make_solver_result_iron_gear_wheel();

        let row_span = make_test_row_span(
            "iron-gear-wheel",
            5,
            vec![ItemFlow { item: "iron-plate".to_string(), rate: 2.0, is_fluid: false }],
            vec![ItemFlow { item: "iron-gear-wheel".to_string(), rate: 1.0, is_fluid: false }],
            1,
            vec![6],
        );

        let (lanes, _families) = plan_bus_lanes(&sr, &[row_span], None).unwrap();

        // The iron-plate lane has consumer row 0, so it should have a tap-off y
        let iron_plate_lane = lanes.iter().find(|l| l.item == "iron-plate").unwrap();
        assert!(!iron_plate_lane.consumer_rows.is_empty(), "iron-plate lane should have consumer rows");
        assert!(!iron_plate_lane.tap_off_ys.is_empty(), "iron-plate lane should have tap-off y after plan");
    }

    // -----------------------------------------------------------------------
    // route_lane / route_belt_lane tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_merge_output_rows_two_rows_have_splitters_and_correct_item() {
        // Two rows producing iron-gear-wheel: the merger must emit splitters and
        // all entities must carry iron-gear-wheel.
        let row0 = {
            let mut rs = make_test_row_span(
                "iron-gear-wheel",
                0,
                vec![],
                vec![ItemFlow { item: "iron-gear-wheel".to_string(), rate: 5.0, is_fluid: false }],
                2,
                vec![],
            );
            rs.output_belt_y = 2;
            rs.row_width = 8;
            rs
        };
        let row1 = {
            let mut rs = make_test_row_span(
                "iron-gear-wheel",
                5,
                vec![],
                vec![ItemFlow { item: "iron-gear-wheel".to_string(), rate: 5.0, is_fluid: false }],
                2,
                vec![],
            );
            rs.output_belt_y = 7;
            rs.row_width = 8;
            rs
        };

        let (entities, end_y, merge_max_x) = merge_output_rows(
            &[0, 1],
            "iron-gear-wheel",
            &[row0, row1],
            15,
            None,
        );

        // Splitters must be present
        let splitters: Vec<_> = entities.iter()
            .filter(|e| e.name.contains("splitter"))
            .collect();
        assert!(!splitters.is_empty(), "Expected splitter(s) in merger for 2 rows");

        // Every entity must carry the correct item
        for e in &entities {
            assert_eq!(
                e.carries.as_deref(),
                Some("iron-gear-wheel"),
                "All merger entities should carry iron-gear-wheel, got {:?}",
                e
            );
        }

        // end_y and merge_max_x should be sane
        assert!(end_y > 15, "end_y should be greater than merge_start_y");
        assert!(merge_max_x > 0, "merge_max_x should be positive");
    }

    #[test]
    fn test_merge_output_rows_splitters_face_south() {
        // Splitters produced by merge_output_rows should face SOUTH (merging
        // parallel SOUTH-flowing trunks).
        let row0 = make_test_row_span(
            "electronic-circuit",
            0,
            vec![],
            vec![ItemFlow { item: "electronic-circuit".to_string(), rate: 5.0, is_fluid: false }],
            1,
            vec![],
        );
        let row1 = make_test_row_span(
            "electronic-circuit",
            5,
            vec![],
            vec![ItemFlow { item: "electronic-circuit".to_string(), rate: 5.0, is_fluid: false }],
            1,
            vec![],
        );

        let (entities, _end_y, _merge_max_x) = merge_output_rows(
            &[0, 1],
            "electronic-circuit",
            &[row0, row1],
            20,
            None,
        );

        let splitters: Vec<_> = entities.iter().filter(|e| e.name.contains("splitter")).collect();
        for s in &splitters {
            assert_eq!(s.direction, EntityDirection::South, "Merger splitters should face SOUTH");
        }
    }

    // -----------------------------------------------------------------------
    // plan_bus_lanes via solver - integration
    // -----------------------------------------------------------------------

    #[test]
    fn test_plan_bus_lanes_via_solver_iron_gear_wheel() {
        use crate::solver::solve;
        use rustc_hash::FxHashSet;

        let available: FxHashSet<String> = ["iron-plate"]
            .iter()
            .map(|s| s.to_string())
            .collect();

        let sr = solve("iron-gear-wheel", 10.0, &available, "assembling-machine-3")
            .expect("solver should succeed");

        // Build minimal row spans from solver machines
        let row_spans: Vec<RowSpan> = sr.machines.iter().enumerate().map(|(i, m)| {
            let input_belt_y: Vec<i32> = m.inputs.iter().enumerate()
                .filter(|(_, f)| !f.is_fluid)
                .map(|(idx, _)| (i * 5 + idx) as i32)
                .collect();
            RowSpan {
                y_start: (i * 5) as i32,
                y_end: (i * 5 + 3) as i32,
                spec: m.clone(),
                machine_count: m.count.ceil() as usize,
                input_belt_y,
                output_belt_y: (i * 5 + 2) as i32,
                row_width: 10,
                fluid_port_ys: Vec::new(),
                fluid_port_pipes: Vec::new(),
                fluid_output_port_pipes: Vec::new(),
            }
        }).collect();

        let (lanes, _families) = plan_bus_lanes(&sr, &row_spans, None)
            .expect("plan_bus_lanes should succeed");

        // Must have at least one lane
        assert!(!lanes.is_empty(), "Expected at least one bus lane");

        // Each lane must have its x assigned (>= 1)
        for lane in &lanes {
            assert!(lane.x >= 1, "Lane x must be assigned >= 1, got x={} for item={}", lane.x, lane.item);
        }

        // No two lanes should share the same x column
        let xs: Vec<i32> = lanes.iter().map(|l| l.x).collect();
        let xs_set: std::collections::HashSet<i32> = xs.iter().copied().collect();
        assert_eq!(xs.len(), xs_set.len(), "All lane x columns must be unique");
    }
}
