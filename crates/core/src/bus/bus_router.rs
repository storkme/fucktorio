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
use crate::common::belt_entity_for_rate;

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
];

/// A single vertical lane on the bus.
#[derive(Clone, Debug)]
pub struct BusLane {
    pub item: String,
    pub x: i32,  // column in the layout
    pub source_y: i32,  // where items enter (0 for external, output_y for intermediate)
    pub consumer_rows: Vec<usize>,  // indices into row_spans
    pub producer_row: Option<usize>,  // index or None for external
    pub rate: f64,  // total throughput for belt tier selection
    pub is_fluid: bool,
    pub tap_off_ys: Vec<i32>,
    pub extra_producer_rows: Vec<usize>,  // additional sub-rows
    pub balancer_y: Option<i32>,  // y of lane balancer splitter (None = no balancer)
    pub family_id: Option<usize>,  // index into LaneFamily list if fed by N-to-M balancer
    pub fluid_port_positions: Vec<(usize, i32, i32)>,  // (row_index, x, y) of pipe-to-ground exit
    pub fluid_output_port_positions: Vec<(usize, i32, i32)>,  // (row_index, x, y) of producer output ports
    pub family_balancer_range: Option<(i32, i32)>,  // (y_start, y_end) inclusive — full balancer zone to skip
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
}

/// An N-to-M balancer block that feeds M sibling trunk lanes for one item.
#[derive(Clone, Debug)]
pub struct LaneFamily {
    pub item: String,
    pub shape: (usize, usize),  // (N producers, M lanes)
    pub producer_rows: Vec<usize>,
    pub lane_xs: Vec<i32>,  // filled in after x-assignment
    pub balancer_y_start: i32,
    pub balancer_y_end: i32,  // inclusive
    pub total_rate: f64,  // sum across all lanes
}

/// Determine which items need bus lanes and assign x-columns.
///
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
            // Collect fluid port pipe positions for tap-off routing
            for &ri in &lane.consumer_rows {
                let rs = &row_spans[ri];
                for &(px, py) in &rs.fluid_port_pipes {
                    lane.fluid_port_positions.push((ri, px, py));
                }
            }
            // Collect producer-side output port pipes
            let mut producer_rows = Vec::new();
            if let Some(pr) = lane.producer_row {
                producer_rows.push(pr);
            }
            producer_rows.extend(&lane.extra_producer_rows);
            for ri in producer_rows {
                let rs = &row_spans[ri];
                for &(px, py) in &rs.fluid_output_port_pipes {
                    lane.fluid_output_port_positions.push((ri, px, py));
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
        let all_producers = lane_all_producers(lane);

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
        let shape_key = (fam.shape.0 as u32, fam.shape.1 as u32);
        if let Some(tpl) = templates.get(&shape_key) {
            fam.balancer_y_end = fam.balancer_y_start + tpl.height as i32 - 1;
            // Store the full zone on each family lane so route_belt_lane can skip it.
            let range = (fam.balancer_y_start, fam.balancer_y_end);
            for lane in lanes.iter_mut() {
                if lane.family_id.is_some() && lane.item == fam.item {
                    lane.family_balancer_range = Some(range);
                }
            }
        }
        // If no template exists, don't set family_balancer_range — lanes operate
        // independently with no zone to skip.
    }

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

/// Factorio direction IDs mapped to EntityDirection
#[allow(dead_code)]
const FACTORIO_DIR_TO_ENTITY: &[(usize, EntityDirection)] = &[
    (0, EntityDirection::North),
    (2, EntityDirection::East),
    (4, EntityDirection::South),
    (6, EntityDirection::West),
];

#[allow(dead_code)]
fn splitter_for_belt(belt: &str) -> &'static str {
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

/// Collect all producer row indices for a lane.
fn lane_all_producers(lane: &BusLane) -> Vec<usize> {
    let mut rows = Vec::new();
    if let Some(pr) = lane.producer_row {
        rows.push(pr);
    }
    rows.extend(&lane.extra_producer_rows);
    rows
}

/// Count total underground crossings for a given lane ordering.
fn score_lane_ordering(ordered: &[BusLane], row_spans: &[RowSpan]) -> usize {
    let mut score = 0;

    fn active_range(lane: &BusLane, row_spans: &[RowSpan]) -> (i32, i32) {
        let all_p = lane_all_producers(lane);

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
        // EAST tap-off crossings
        for &tap_y in &lane.tap_off_ys {
            for &(rs, re) in &ranges[(pos + 1)..] {
                if rs <= tap_y && tap_y <= re {
                    score += 1;
                }
            }
        }

        // WEST output return crossings
        let all_producers = lane_all_producers(lane);
        for &pri in &all_producers {
            let ret_y = row_spans[pri].output_belt_y;
            for &(rs, re) in &ranges[(pos + 1)..] {
                if rs <= ret_y && ret_y <= re {
                    score += 1;
                }
            }
        }
    }

    score
}

/// Optimize lane left-to-right ordering to minimize underground crossings.
fn optimize_lane_order(lanes: &[BusLane], row_spans: &[RowSpan]) -> Vec<BusLane> {
    if lanes.len() <= 1 {
        return lanes.to_vec();
    }

    let solid: Vec<BusLane> = lanes.iter().filter(|ln| !ln.is_fluid).cloned().collect();
    let fluid: Vec<BusLane> = lanes.iter().filter(|ln| ln.is_fluid).cloned().collect();

    let best_solid = if solid.len() <= 10 {
        // Enumerate all permutations
        find_best_permutation(&solid, row_spans)
    } else {
        // Heuristic: sort by family_id then by negative min tap_off_y
        let mut sorted = solid.clone();
        sorted.sort_by_key(|ln| {
            let fid = ln.family_id.unwrap_or(usize::MAX) as i32;
            let y = if !ln.tap_off_ys.is_empty() {
                -*ln.tap_off_ys.iter().min().unwrap()
            } else {
                9999
            };
            (fid, y)
        });
        sorted
    };

    let mut result = best_solid;
    result.extend(fluid);
    result
}

/// Find best permutation of solid lanes that respects family contiguity.
fn find_best_permutation(solid: &[BusLane], row_spans: &[RowSpan]) -> Vec<BusLane> {
    if solid.is_empty() {
        return Vec::new();
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

    // Use iterative approach instead of recursive permutations for small sets
    let n = solid.len();
    let mut indices: Vec<usize> = (0..n).collect();
    let mut best_order: Vec<usize> = indices.clone();
    let mut best_score = score_lane_ordering(
        &indices.iter().map(|&i| solid[i].clone()).collect::<Vec<_>>(),
        row_spans,
    );

    // Heap's algorithm for permutation generation
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
        let n_splits = n_splits.max(lane.consumer_rows.len());

        if n_splits <= 1 {
            result.push(lane.clone());
            continue;
        }

        // Distribute consumer rows round-robin
        let mut consumers_per_split: Vec<Vec<usize>> = vec![Vec::new(); n_splits];
        for (i, &ri) in lane.consumer_rows.iter().enumerate() {
            consumers_per_split[i % n_splits].push(ri);
        }

        // Distribute producer rows by rate
        let all_producer_rows = lane_all_producers(lane);

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

        if n_producers >= 1 && n_producers < n_lanes_with_consumers {
            let shape = (n_producers, n_lanes_with_consumers);
            // TODO: Check if template exists and create LaneFamily
            // For now, we skip balancer creation as it's Phase 2
            family_id = Some(families.len());

            // Placeholder: we would stamp the balancer here in Phase 2
            let balancer_y_start = if n_producers == 1 {
                row_spans[all_producer_rows[0]].output_belt_y
            } else {
                all_producer_rows.iter()
                    .map(|&p| row_spans[p].y_end)
                    .max()
                    .unwrap_or(0)
            };

            // For now, we create a placeholder family
            families.push(LaneFamily {
                item: lane.item.clone(),
                shape,
                producer_rows: all_producer_rows.to_vec(),
                lane_xs: Vec::new(),
                balancer_y_start,
                balancer_y_end: balancer_y_start,  // Placeholder
                total_rate: lane.rate,
            });
            family_source_y = Some(balancer_y_start + 1);  // Placeholder
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
    let template_key = (family.shape.0 as u32, family.shape.1 as u32);
    let template = match templates.get(&template_key) {
        Some(t) => t,
        None => return Ok(Vec::new()), // No template — skip balancer, lanes operate independently
    };

    if family.lane_xs.is_empty() {
        return Err(format!("LaneFamily for item {} has no lane_xs assigned", family.item));
    }

    let origin_x = *family.lane_xs.iter().min().unwrap();
    let origin_y = family.balancer_y_start;

    let belt_tier = belt_entity_for_rate(family.total_rate, max_belt_tier);
    let splitter_name = splitter_for_belt(belt_tier);
    let ug_name = underground_for_belt(belt_tier);

    let balancer_seg_id = Some(format!("balancer:{}", family.item));
    let mut entities = template.stamp(
        origin_x,
        origin_y,
        belt_tier,
        splitter_name,
        ug_name,
        Some(&family.item),
    );
    for ent in &mut entities {
        ent.segment_id = balancer_seg_id.clone();
    }

    Ok(entities)
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
pub(crate) fn render_family_input_paths(
    family: &LaneFamily,
    row_spans: &[RowSpan],
    belt_tier: &str,
    routed_paths: Option<&FxHashMap<String, Vec<(i32, i32)>>>,
) -> Result<Vec<PlacedEntity>, String> {
    use crate::bus::balancer_library::balancer_templates;

    let templates = balancer_templates();
    let template_key = (family.shape.0 as u32, family.shape.1 as u32);
    let template = match templates.get(&template_key) {
        Some(t) => t,
        None => return Ok(Vec::new()), // No template — skip feeder paths
    };

    if family.lane_xs.is_empty() {
        return Ok(Vec::new());
    }

    let origin_x = *family.lane_xs.iter().min().unwrap();
    let origin_y = family.balancer_y_start;
    let default_paths = FxHashMap::default();
    let paths = routed_paths.unwrap_or(&default_paths);

    // Sort producers top-to-bottom, input tiles left-to-right
    let mut producers = family.producer_rows.clone();
    producers.sort_by_key(|&p| row_spans[p].output_belt_y);

    let mut inputs: Vec<(i32, i32)> = template.input_tiles.to_vec();
    inputs.sort_by_key(|t| t.0);

    let mut entities: Vec<PlacedEntity> = Vec::new();

    for (producer_row_idx, input_tile) in producers.iter().zip(inputs.iter()) {
        let out_y = row_spans[*producer_row_idx].output_belt_y;
        let input_x = origin_x + input_tile.0;

        // Horizontal WEST feeder: A*-routed by the negotiator
        let feeder_key = format!("feeder:{}:{}:{}", family.item, input_x, out_y);
        if let Some(feeder_path) = paths.get(&feeder_key) {
            let per_producer_rate = family.total_rate / family.producer_rows.len().max(1) as f64;
            let feeder_entities = render_path(feeder_path, &family.item, belt_tier, EntityDirection::West, Some(format!("family-input:{}", family.item)), Some(per_producer_rate));
            entities.extend(feeder_entities);
        }

        if out_y == origin_y {
            // N == 1 case: template's input tile is the turn point
            continue;
        }

        // Turn: SOUTH belt at (input_x, out_y), then descend to (input_x, origin_y - 1)
        let fam_input_seg_id = Some(format!("family-input:{}", family.item));
        let per_producer_rate = family.total_rate / family.producer_rows.len().max(1) as f64;
        entities.push(PlacedEntity {
            name: belt_tier.to_string(),
            x: input_x,
            y: out_y,
            direction: EntityDirection::South,
            carries: Some(family.item.clone()),
            segment_id: fam_input_seg_id.clone(),
            rate: Some(per_producer_rate),
            ..Default::default()
        });

        for y in (out_y + 1)..origin_y {
            entities.push(PlacedEntity {
                name: belt_tier.to_string(),
                x: input_x,
                y,
                direction: EntityDirection::South,
                carries: Some(family.item.clone()),
                segment_id: fam_input_seg_id.clone(),
                rate: Some(per_producer_rate),
                ..Default::default()
            });
        }
    }

    Ok(entities)
}

/// Vector direction (dx, dy) to entity direction.
#[allow(dead_code)]
fn vec_to_entity_dir(dx: i32, dy: i32) -> EntityDirection {
    if dx > 0 {
        EntityDirection::East
    } else if dx < 0 {
        EntityDirection::West
    } else if dy > 0 {
        EntityDirection::South
    } else {
        EntityDirection::North
    }
}

/// Merge EAST-flowing output belts from multiple rows at the bottom-right.
///
/// Each output row's belt flows EAST and collects items at its rightmost
/// tile. This function extends shorter rows to a common merge column,
/// places SOUTH columns, and merges them with a splitter tree.
///
/// Returns (entities, max_y, merge_max_x).
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

    // Merge columns sit just past the widest output row.
    // Earlier rows (lower idx, higher up in the layout) get farther-right
    // columns so their SOUTH columns don't block later rows' EAST extensions.
    let merge_x = output_rows.iter()
        .map(|&ri| if ri < row_spans.len() { row_spans[ri].row_width } else { 0 })
        .max()
        .unwrap_or(0);

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
pub(crate) fn place_merger_block(
    trunk_lanes: &[BusLane],
    row_spans: &[RowSpan],
    merge_start_y: i32,
    existing_entities: &[PlacedEntity],
    max_belt_tier: Option<&str>,
) -> (Vec<PlacedEntity>, i32) {
    use crate::common::{belt_entity_for_rate, belt_throughput};

    let mut entities: Vec<PlacedEntity> = Vec::new();

    let total_rate: f64 = trunk_lanes.iter().map(|ln| ln.rate).sum();

    // Determine belt tier and capacity
    let belt_name = belt_entity_for_rate(total_rate * 2.0, max_belt_tier);
    let full_cap = belt_throughput(belt_name);
    let target_m = (total_rate / full_cap).ceil().max(1.0) as usize;

    let mut trunk_xs: Vec<i32> = trunk_lanes.iter().map(|ln| ln.x).collect();
    trunk_xs.sort_unstable();
    let n = trunk_xs.len();

    if n <= target_m {
        return (entities, merge_start_y);
    }

    let splitter_name = splitter_for_belt(belt_name);
    let item = &trunk_lanes[0].item;
    let merger_seg_id = Some(format!("merger:{}", item));

    // Build set of already-occupied positions to avoid overlaps
    let occupied: FxHashSet<(i32, i32)> = existing_entities.iter()
        .map(|e| (e.x, e.y))
        .collect();

    // Extend each trunk from its current end_y to merge_start_y
    for lane in trunk_lanes {
        let mut all_ys = lane.tap_off_ys.clone();
        for &pri in &lane.extra_producer_rows {
            if pri < row_spans.len() {
                all_ys.push(row_spans[pri].output_belt_y);
            }
        }
        let end_y = all_ys.iter().max().copied().unwrap_or(lane.source_y);

        for y in (end_y + 1)..merge_start_y {
            if !occupied.contains(&(lane.x, y)) {
                entities.push(PlacedEntity {
                    name: belt_name.to_string(),
                    x: lane.x,
                    y,
                    direction: EntityDirection::South,
                    carries: Some(item.clone()),
                    segment_id: merger_seg_id.clone(),
                    rate: Some(lane.rate),
                    ..Default::default()
                });
            }
        }
    }

    let mut y_cursor = merge_start_y;
    let mut current_xs = trunk_xs.clone();

    while current_xs.len() > target_m {
        // How many pairs to merge this stage (at most half, enough to reach target)
        let pairs_needed = std::cmp::min(current_xs.len() - target_m, current_xs.len() / 2);
        let mut next_xs: Vec<i32> = Vec::new();
        let mut i = 0;
        let mut pairs_done = 0;

        while i < current_xs.len() {
            if pairs_done < pairs_needed && i + 1 < current_xs.len() {
                let left_x = current_xs[i];
                let right_x = current_xs[i + 1];

                // Route right trunk to left_x + 1 using horizontal WEST belts
                for rx in (left_x + 1..=right_x).rev() {
                    entities.push(PlacedEntity {
                        name: belt_name.to_string(),
                        x: rx,
                        y: y_cursor,
                        direction: EntityDirection::West,
                        carries: Some(item.clone()),
                        segment_id: merger_seg_id.clone(),
                        rate: Some(total_rate),
                        ..Default::default()
                    });
                }

                // Continue left trunk straight down
                entities.push(PlacedEntity {
                    name: belt_name.to_string(),
                    x: left_x,
                    y: y_cursor,
                    direction: EntityDirection::South,
                    carries: Some(item.clone()),
                    segment_id: merger_seg_id.clone(),
                    rate: Some(total_rate),
                    ..Default::default()
                });

                // Splitter (SOUTH-facing, occupies left_x and left_x+1)
                entities.push(PlacedEntity {
                    name: splitter_name.to_string(),
                    x: left_x,
                    y: y_cursor + 1,
                    direction: EntityDirection::South,
                    carries: Some(item.clone()),
                    segment_id: merger_seg_id.clone(),
                    rate: Some(total_rate),
                    ..Default::default()
                });

                // Output belt on the left side only (right side empty → all items go left)
                entities.push(PlacedEntity {
                    name: belt_name.to_string(),
                    x: left_x,
                    y: y_cursor + 2,
                    direction: EntityDirection::South,
                    carries: Some(item.clone()),
                    segment_id: merger_seg_id.clone(),
                    rate: Some(total_rate),
                    ..Default::default()
                });

                next_xs.push(left_x);
                pairs_done += 1;
                i += 2;
            } else {
                // Passthrough — extend this trunk down through the merge stage
                let px = current_xs[i];
                for dy in 0..3 {
                    entities.push(PlacedEntity {
                        name: belt_name.to_string(),
                        x: px,
                        y: y_cursor + dy,
                        direction: EntityDirection::South,
                        carries: Some(item.clone()),
                        segment_id: merger_seg_id.clone(),
                        rate: Some(total_rate),
                        ..Default::default()
                    });
                }
                next_xs.push(px);
                i += 1;
            }
        }

        y_cursor += 3; // each stage is 3 rows: route + splitter + output
        current_xs = next_xs;
    }

    (entities, y_cursor)
}

/// Route a single bus lane: dispatches to fluid, intermediate, or belt routing.
///
/// Port of Python `_route_lane`.
pub(crate) fn route_lane(
    entities: &mut Vec<PlacedEntity>,
    lane: &BusLane,
    all_lanes: &[BusLane],
    row_spans: &[RowSpan],
    bw: i32,
    max_belt_tier: Option<&str>,
    routed_paths: Option<&FxHashMap<String, Vec<(i32, i32)>>>,
    crossing_tiles: &CrossingTileSet,
) {
    if lane.is_fluid {
        entities.extend(route_fluid_lane(lane));
    } else if is_intermediate(lane) {
        route_intermediate_lane(entities, lane, all_lanes, row_spans, bw, max_belt_tier, routed_paths, crossing_tiles);
    } else {
        route_belt_lane(entities, lane, all_lanes, row_spans, max_belt_tier, routed_paths, crossing_tiles);
    }
}

fn is_intermediate(lane: &BusLane) -> bool {
    let has_producers = lane.producer_row.is_some() || !lane.extra_producer_rows.is_empty();
    let has_consumers = !lane.consumer_rows.is_empty();
    has_producers && has_consumers
}

/// Route a solid-item bus lane with belts (external input or collector).
///
/// Port of Python `_route_belt_lane`.
fn route_belt_lane(
    entities: &mut Vec<PlacedEntity>,
    lane: &BusLane,
    all_lanes: &[BusLane],
    row_spans: &[RowSpan],
    max_belt_tier: Option<&str>,
    routed_paths: Option<&FxHashMap<String, Vec<(i32, i32)>>>,
    crossing_tiles: &CrossingTileSet,
) {
    let x = lane.x;
    let tap_off_set: FxHashSet<i32> = lane.tap_off_ys.iter().copied().collect();
    let empty: FxHashMap<String, Vec<(i32, i32)>> = FxHashMap::default();
    let paths: &FxHashMap<String, Vec<(i32, i32)>> = routed_paths.unwrap_or(&empty);

    let start_y = lane.source_y;
    let mut all_ys: Vec<i32> = lane.tap_off_ys.clone();
    for &pri in &lane.extra_producer_rows {
        if pri < row_spans.len() {
            all_ys.push(row_spans[pri].output_belt_y);
        }
    }
    let end_y = all_ys.iter().copied().max().unwrap_or(start_y);
    let end_y = if let Some(bal_y) = lane.balancer_y {
        end_y.max(bal_y + 1)
    } else {
        end_y
    };

    let belt_name = if lane.balancer_y.is_some() {
        belt_entity_for_rate(lane.rate, max_belt_tier)
    } else {
        belt_entity_for_rate(lane.rate * 2.0, max_belt_tier)
    };
    let horiz_belt = belt_entity_for_rate(lane.rate * 2.0, max_belt_tier);
    let pre_bal_belt = if lane.balancer_y.is_some() {
        belt_entity_for_rate(lane.rate * 2.0, max_belt_tier)
    } else {
        belt_name
    };

    let foreign_skips = foreign_trunk_skip_ys(lane, all_lanes, row_spans, start_y, end_y);
    let mut skip_ys = tap_off_set.clone();
    skip_ys.extend(lane.balancer_y);
    // Skip the entire family balancer zone (not just one tile).
    if let Some((by_start, by_end)) = lane.family_balancer_range {
        for y in by_start..=by_end {
            skip_ys.insert(y);
        }
    }
    skip_ys.extend(foreign_skip_ug_tiles(&foreign_skips).iter().copied());
    // Also skip any y-rows that SAT crossing zones own at this x.
    for &(cx, cy) in crossing_tiles.iter() {
        if cx == x {
            skip_ys.insert(cy);
        }
    }

    // UG-pair bridges over foreign skip y's — skip if SAT solved this zone.
    let trunk_seg_id = Some(format!("trunk:{}", lane.item));
    let ug_name = underground_for_belt(belt_name);
    for &fy in &foreign_skips {
        if crossing_tiles.contains(&(x, fy)) {
            // SAT solver owns this crossing — skip the heuristic bridge.
            continue;
        }
        // Remove any previously placed entity at the bridge input position so the
        // UG input can replace it (e.g. a balancer-stamp surface belt at fy-1).
        entities.retain(|e| !(e.x == x && e.y == fy - 1 && !crossing_tiles.contains(&(e.x, e.y))));
        entities.push(PlacedEntity {
            name: ug_name.to_string(),
            x,
            y: fy - 1,
            direction: EntityDirection::South,
            io_type: Some("input".to_string()),
            carries: Some(lane.item.clone()),
            segment_id: trunk_seg_id.clone(),
            rate: Some(lane.rate),
            ..Default::default()
        });
        entities.push(PlacedEntity {
            name: ug_name.to_string(),
            x,
            y: fy + 1,
            direction: EntityDirection::South,
            io_type: Some("output".to_string()),
            carries: Some(lane.item.clone()),
            segment_id: trunk_seg_id.clone(),
            rate: Some(lane.rate),
            ..Default::default()
        });
    }

    // Vertical trunk
    let bal_y = lane.balancer_y;
    for (seg_start, seg_end) in trunk_segments(start_y, end_y, &skip_ys) {
        let tier = if let Some(by) = bal_y {
            if seg_start < by { pre_bal_belt } else { belt_name }
        } else {
            belt_name
        };
        let trunk_key = format!("trunk:{}:{}:{}:{}", lane.item, x, seg_start, seg_end);
        if let Some(trunk_path) = paths.get(&trunk_key) {
            entities.extend(render_path(trunk_path, &lane.item, tier, EntityDirection::South, trunk_seg_id.clone(), Some(lane.rate)));
        } else {
            for y in seg_start..=seg_end {
                entities.push(PlacedEntity {
                    name: tier.to_string(),
                    x,
                    y,
                    direction: EntityDirection::South,
                    carries: Some(lane.item.clone()),
                    segment_id: trunk_seg_id.clone(),
                    rate: Some(lane.rate),
                    ..Default::default()
                });
            }
        }
    }

    // Lane balancer
    if let Some(by) = lane.balancer_y {
        let splitter_name = splitter_for_belt(belt_name);
        entities.push(PlacedEntity {
            name: splitter_name.to_string(),
            x: x - 1,
            y: by,
            direction: EntityDirection::South,
            carries: Some(lane.item.clone()),
            segment_id: trunk_seg_id.clone(),
            rate: Some(lane.rate),
            ..Default::default()
        });
        entities.push(PlacedEntity {
            name: belt_name.to_string(),
            x: x - 1,
            y: by + 1,
            direction: EntityDirection::East,
            carries: Some(lane.item.clone()),
            segment_id: trunk_seg_id.clone(),
            rate: Some(lane.rate),
            ..Default::default()
        });
    }

    // Tap-offs
    let tapoff_seg_id = Some(format!("tapoff:{}", lane.item));
    for &tap_y in &lane.tap_off_ys {
        let tap_key = format!("tap:{}:{}:{}", lane.item, x, tap_y);
        if let Some(tap_path) = paths.get(&tap_key) {
            entities.extend(render_path(tap_path, &lane.item, horiz_belt, EntityDirection::East, tapoff_seg_id.clone(), Some(lane.rate)));
        } else {
            entities.push(PlacedEntity {
                name: horiz_belt.to_string(),
                x,
                y: tap_y,
                direction: EntityDirection::East,
                carries: Some(lane.item.clone()),
                segment_id: tapoff_seg_id.clone(),
                rate: Some(lane.rate),
                ..Default::default()
            });
        }
        // Also render the post-zone segment if the tap-off was split by SAT.
        let tap_post_key = format!("tap:{}:{}:{}_post", lane.item, x, tap_y);
        if let Some(tap_path) = paths.get(&tap_post_key) {
            entities.extend(render_path(tap_path, &lane.item, horiz_belt, EntityDirection::East, tapoff_seg_id.clone(), Some(lane.rate)));
        }
    }
}

/// Route an intermediate lane (has both producers and consumers).
///
/// Port of Python `_route_intermediate_lane`.
fn route_intermediate_lane(
    entities: &mut Vec<PlacedEntity>,
    lane: &BusLane,
    all_lanes: &[BusLane],
    row_spans: &[RowSpan],
    bw: i32,
    max_belt_tier: Option<&str>,
    routed_paths: Option<&FxHashMap<String, Vec<(i32, i32)>>>,
    crossing_tiles: &CrossingTileSet,
) {
    let x = lane.x;
    // Both belt_name and horiz_belt use the same tier for intermediate lanes
    let belt_name = belt_entity_for_rate(lane.rate * 2.0, max_belt_tier);
    let horiz_belt = belt_name;
    let empty: FxHashMap<String, Vec<(i32, i32)>> = FxHashMap::default();
    let paths = routed_paths.unwrap_or(&empty);
    let trunk_seg_id = Some(format!("trunk:{}", lane.item));
    let tapoff_seg_id = Some(format!("tapoff:{}", lane.item));

    let mut all_producers: Vec<usize> = Vec::new();
    if let Some(pr) = lane.producer_row {
        all_producers.push(pr);
    }
    all_producers.extend(&lane.extra_producer_rows);

    let tap_y = if !lane.tap_off_ys.is_empty() {
        lane.tap_off_ys[0]
    } else if let Some(&ri) = lane.consumer_rows.first() {
        if ri < row_spans.len() {
            row_spans[ri].input_belt_y.first().copied().unwrap_or(0)
        } else {
            0
        }
    } else {
        return;
    };

    let producer_out_ys: Vec<i32> = all_producers.iter()
        .filter(|&&p| p < row_spans.len())
        .map(|&p| row_spans[p].output_belt_y)
        .collect();
    let start_y = producer_out_ys.iter().copied().min().unwrap_or(lane.source_y);

    // Determine balance_y for splitter lane-balancing
    let balance_y: Option<i32> = if all_producers.len() >= 2 && x > 1 {
        all_producers.last()
            .and_then(|&last_pri| {
                if last_pri < row_spans.len() {
                    Some(row_spans[last_pri].output_belt_y)
                } else {
                    None
                }
            })
    } else {
        None
    };

    // Output returns — skip when lane has a family balancer (feeders handle routing)
    if lane.family_balancer_range.is_none() {
        for &pri in &all_producers {
            if pri >= row_spans.len() {
                continue;
            }
            let out_y = row_spans[pri].output_belt_y;
            if Some(out_y) == balance_y {
                // Splitter for lane balancing
                let splitter_name = splitter_for_belt(horiz_belt);
                entities.push(PlacedEntity {
                    name: splitter_name.to_string(),
                    x: bw,
                    y: out_y - 1,
                    direction: EntityDirection::West,
                    carries: Some(lane.item.clone()),
                    segment_id: trunk_seg_id.clone(),
                    rate: Some(lane.rate),
                    ..Default::default()
                });
                // Normal return via A*-routed path
                let ret_key = format!("ret:{}:{}:{}", lane.item, x, out_y);
                if let Some(ret_path) = paths.get(&ret_key) {
                    entities.extend(render_path(ret_path, &lane.item, horiz_belt, EntityDirection::West, Some(format!("trunk:{}", lane.item)), Some(lane.rate)));
                }
                // Balance route
                let split_y = out_y - 1;
                let bal_key = format!("bal:{}:{}:{}", lane.item, x, split_y);
                if let Some(bal_path) = paths.get(&bal_key) {
                    let mut bal_entities = render_path(bal_path, &lane.item, horiz_belt, EntityDirection::West, Some(format!("trunk:{}", lane.item)), Some(lane.rate));
                    if let Some(last) = bal_entities.last_mut() {
                        last.direction = EntityDirection::East;
                    }
                    entities.extend(bal_entities);
                }
            } else {
                // Normal return
                let ret_key = format!("ret:{}:{}:{}", lane.item, x, out_y);
                if let Some(ret_path) = paths.get(&ret_key) {
                    entities.extend(render_path(ret_path, &lane.item, horiz_belt, EntityDirection::West, Some(format!("trunk:{}", lane.item)), Some(lane.rate)));
                }
            }
        }
    }

    // Vertical trunk
    let foreign_skips = foreign_trunk_skip_ys(lane, all_lanes, row_spans, start_y, tap_y - 1);
    let mut skip_ys: FxHashSet<i32> = producer_out_ys.iter().copied().collect();
    skip_ys.extend(foreign_skip_ug_tiles(&foreign_skips).iter().copied());
    // Skip any y-rows owned by SAT crossing zones at this x.
    for &(cx, cy) in crossing_tiles.iter() {
        if cx == x {
            skip_ys.insert(cy);
        }
    }

    // Surface belt at each producer output y (return junction points)
    for &out_y in &producer_out_ys {
        if out_y < tap_y && !crossing_tiles.contains(&(x, out_y)) {
            entities.push(PlacedEntity {
                name: belt_name.to_string(),
                x,
                y: out_y,
                direction: EntityDirection::South,
                carries: Some(lane.item.clone()),
                segment_id: trunk_seg_id.clone(),
                rate: Some(lane.rate),
                ..Default::default()
            });
        }
    }

    // UG-pair bridges over foreign skip y's — skip if SAT solved this zone.
    let ug_name = underground_for_belt(belt_name);
    for &fy in &foreign_skips {
        if crossing_tiles.contains(&(x, fy)) {
            continue;
        }
        // Remove any previously placed entity at the bridge input position (e.g. a
        // balancer-stamp surface belt) so the UG input can take that tile.
        entities.retain(|e| !(e.x == x && e.y == fy - 1 && !crossing_tiles.contains(&(e.x, e.y))));
        entities.push(PlacedEntity {
            name: ug_name.to_string(),
            x,
            y: fy - 1,
            direction: EntityDirection::South,
            io_type: Some("input".to_string()),
            carries: Some(lane.item.clone()),
            segment_id: trunk_seg_id.clone(),
            rate: Some(lane.rate),
            ..Default::default()
        });
        entities.push(PlacedEntity {
            name: ug_name.to_string(),
            x,
            y: fy + 1,
            direction: EntityDirection::South,
            io_type: Some("output".to_string()),
            carries: Some(lane.item.clone()),
            segment_id: trunk_seg_id.clone(),
            rate: Some(lane.rate),
            ..Default::default()
        });
    }

    for (seg_start, seg_end) in trunk_segments(start_y, tap_y - 1, &skip_ys) {
        let trunk_key = format!("trunk:{}:{}:{}:{}", lane.item, x, seg_start, seg_end);
        if let Some(trunk_path) = paths.get(&trunk_key) {
            entities.extend(render_path(trunk_path, &lane.item, belt_name, EntityDirection::South, trunk_seg_id.clone(), Some(lane.rate)));
        } else {
            for y in seg_start..=seg_end {
                entities.push(PlacedEntity {
                    name: belt_name.to_string(),
                    x,
                    y,
                    direction: EntityDirection::South,
                    carries: Some(lane.item.clone()),
                    segment_id: trunk_seg_id.clone(),
                    rate: Some(lane.rate),
                    ..Default::default()
                });
            }
        }
    }

    // Tap-off
    let tap_key = format!("tap:{}:{}:{}", lane.item, x, tap_y);
    if let Some(tap_path) = paths.get(&tap_key) {
        entities.extend(render_path(tap_path, &lane.item, belt_name, EntityDirection::East, tapoff_seg_id.clone(), Some(lane.rate)));
    } else {
        entities.push(PlacedEntity {
            name: belt_name.to_string(),
            x,
            y: tap_y,
            direction: EntityDirection::East,
            carries: Some(lane.item.clone()),
            segment_id: tapoff_seg_id.clone(),
            rate: Some(lane.rate),
            ..Default::default()
        });
    }
    // Also render the post-zone segment if the tap-off was split by SAT.
    let tap_post_key = format!("tap:{}:{}:{}_post", lane.item, x, tap_y);
    if let Some(tap_path) = paths.get(&tap_post_key) {
        entities.extend(render_path(tap_path, &lane.item, belt_name, EntityDirection::East, tapoff_seg_id.clone(), Some(lane.rate)));
    }
}

/// Split [start_y, end_y] into contiguous segments excluding skip_ys.
fn trunk_segments(start_y: i32, end_y: i32, skip_ys: &FxHashSet<i32>) -> Vec<(i32, i32)> {
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

/// Y-rows where this trunk must bridge underground so other items can use the
/// surface tile at that y.  Two cases handled:
///
/// 1. The immediate western neighbor produces items whose output-return belt lands
///    here — that y must be surface-free so the return belt can enter the trunk.
/// 2. Any lane to the left taps off at a y that falls inside this trunk's range
///    and that tap-off crosses this column.  The tap-off's UG input sits on this
///    column, so the trunk tile must be free.
///    Guard: if the bridge output (tap_y + 1) would land on this trunk's own
///    tap-off belt, skip it — the geometry is handled differently there.
fn foreign_trunk_skip_ys(
    lane: &BusLane,
    all_lanes: &[BusLane],
    row_spans: &[RowSpan],
    trunk_start_y: i32,
    trunk_end_y: i32,
) -> FxHashSet<i32> {
    let mut foreign: FxHashSet<i32> = FxHashSet::default();

    // Case 1: west neighbor's output-return rows need a free landing tile here.
    let west_col = lane.x - 1;
    if let Some(neighbor) = all_lanes.iter().find(|other| {
        !other.is_fluid && !std::ptr::eq(*other, lane) && other.x == west_col
    }) {
        let mut producer_rows: Vec<usize> = Vec::new();
        if let Some(pr) = neighbor.producer_row {
            producer_rows.push(pr);
        }
        producer_rows.extend(&neighbor.extra_producer_rows);
        for p in producer_rows {
            if p >= row_spans.len() {
                continue;
            }
            let y = row_spans[p].output_belt_y;
            if trunk_start_y < y && y < trunk_end_y {
                foreign.insert(y);
            }
        }
    }

    // Case 2: any left-lane tap-off that would cross this trunk column and whose
    // UG input would sit here.  Only add if the bridge output (tap_y + 1) doesn't
    // collide with this trunk's own tap-off belt at that position.
    let own_tap_set: FxHashSet<i32> = lane.tap_off_ys.iter().copied().collect();
    for other in all_lanes {
        if other.is_fluid || std::ptr::eq(other, lane) || other.x >= lane.x {
            continue;
        }
        for &tap_y in &other.tap_off_ys {
            if !(trunk_start_y < tap_y && tap_y < trunk_end_y) {
                continue;
            }
            if own_tap_set.contains(&(tap_y + 1)) {
                continue;
            }
            // Only bridge if the tap-off travels surface all the way to this
            // trunk.  If any intermediate lane has a surface belt at tap_y, the
            // tap-off already went underground before reaching lane.x — no
            // bridge needed here.
            let all_intermediate_clear = all_lanes.iter()
                .filter(|mid| !mid.is_fluid && mid.x > other.x && mid.x < lane.x)
                .all(|mid| {
                    // mid has no surface belt at tap_y when:
                    // (a) mid skips tap_y (tap_y is one of its own tap-offs), OR
                    // (b) mid's trunk doesn't reach tap_y (all its tap_off_ys are above)
                    mid.tap_off_ys.contains(&tap_y)
                        || mid.tap_off_ys.iter().all(|&y| y < tap_y)
                });
            if all_intermediate_clear {
                foreign.insert(tap_y);
            }
        }
    }

    foreign
}

/// Extra y-rows to add to trunk skip set so UG-pair tiles don't collide.
fn foreign_skip_ug_tiles(foreign_skip_ys: &FxHashSet<i32>) -> FxHashSet<i32> {
    let mut result: FxHashSet<i32> = FxHashSet::default();
    for &y in foreign_skip_ys {
        result.insert(y - 1);
        result.insert(y);
        result.insert(y + 1);
    }
    result
}

// ---------------------------------------------------------------------------
// SAT-based crossing zone solver
// ---------------------------------------------------------------------------

use crate::sat::{self, CrossingZone, CrossingZoneSolution, ZoneBoundary};

/// A solved crossing zone: the SAT solution plus its origin.
#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct SolvedCrossing {
    pub zone: CrossingZone,
    pub solution: CrossingZoneSolution,
}

/// A SAT-solved region that the A* should route around.
/// The tap-off at `tap_y` from lane at `tap_x` is handled by SAT
/// from `x_min` to `x_max` (inclusive).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct SatCrossingRegion {
    pub tap_x: i32,    // x of the tapping lane
    pub x_min: i32,
    pub x_max: i32,
    pub tap_y: i32,
    pub tap_item: String,
}

/// Tile set of all (x, y) positions owned by solved crossing zones.
pub(crate) type CrossingTileSet = FxHashSet<(i32, i32)>;

/// Extract crossing zones from the lane plan and solve them via SAT.
///
/// Returns (solved_crossings, tile_set) where tile_set contains all (x,y)
/// positions owned by crossing zone entities.
#[allow(dead_code)]
pub(crate) fn extract_and_solve_crossings(
    lanes: &[BusLane],
    _row_spans: &[RowSpan],
    max_belt_tier: Option<&str>,
) -> (Vec<SolvedCrossing>, CrossingTileSet, Vec<SatCrossingRegion>) {
    let effective_belt = belt_entity_for_rate(f64::MAX, max_belt_tier);
    let max_reach = crate::common::ug_max_reach(effective_belt);

    // For each left-lane tap-off, find which right-lane trunks it crosses on
    // the surface. Group contiguous crossed columns into zones.
    let mut zone_specs: Vec<(String, i32, i32, Vec<(i32, String)>)> = Vec::new();
    // zone_specs: (tap_item, tap_x, tap_y, vec of (trunk_x, trunk_item))

    for tapping_lane in lanes {
        if tapping_lane.is_fluid {
            continue;
        }
        for &tap_y in &tapping_lane.tap_off_ys {
            let mut crossed_trunks: Vec<(i32, String)> = Vec::new();

            // Find trunk columns the tap-off reaches on the surface
            // (same intermediate-clear check as foreign_trunk_skip_ys Case 2).
            for trunk_lane in lanes {
                if trunk_lane.is_fluid
                    || std::ptr::eq(trunk_lane, tapping_lane)
                    || trunk_lane.x <= tapping_lane.x
                {
                    continue;
                }
                let trunk_extends = trunk_lane.tap_off_ys.iter().any(|&y| y >= tap_y);
                let trunk_skips_tap_y = trunk_lane.tap_off_ys.contains(&tap_y);
                if !trunk_extends || trunk_skips_tap_y {
                    continue;
                }
                let all_clear = lanes.iter()
                    .filter(|mid| {
                        !mid.is_fluid
                            && mid.x > tapping_lane.x
                            && mid.x < trunk_lane.x
                    })
                    .all(|mid| {
                        mid.tap_off_ys.contains(&tap_y)
                            || mid.tap_off_ys.iter().all(|&y| y < tap_y)
                    });
                if all_clear {
                    crossed_trunks.push((trunk_lane.x, trunk_lane.item.clone()));
                }
            }

            if !crossed_trunks.is_empty() {
                crossed_trunks.sort_by_key(|(x, _)| *x);
                zone_specs.push((
                    tapping_lane.item.clone(),
                    tapping_lane.x,
                    tap_y,
                    crossed_trunks,
                ));
            }
        }
    }

    // Deduplicate: if the same tap_y has overlapping zones from multiple
    // tapping lanes, merge them (take the union of crossed trunks).
    // For now, just solve each independently.

    let mut solved = Vec::new();
    let mut tile_set = FxHashSet::default();

    #[allow(unused_variables)]
    for (tap_item, tap_x, tap_y, crossed) in &zone_specs {
        let x_min = crossed.first().unwrap().0;
        let x_max = crossed.last().unwrap().0;
        let zone_width = (x_max - x_min + 1) as u32;
        let zone_height: u32 = 3;
        let zone_x = x_min;
        let zone_y = tap_y - 1;

        let mut boundaries = Vec::new();

        // Trunk boundaries only: each crossed column enters top, exits bottom.
        // Tap-off tiles are forced empty (the A* routes the tap-off underground).
        let mut forced_empty = Vec::new();
        for (trunk_x, trunk_item) in crossed {
            boundaries.push(ZoneBoundary {
                x: *trunk_x,
                y: zone_y,
                direction: EntityDirection::South,
                item: trunk_item.clone(),
                is_input: true,
            });
            boundaries.push(ZoneBoundary {
                x: *trunk_x,
                y: zone_y + zone_height as i32 - 1,
                direction: EntityDirection::South,
                item: trunk_item.clone(),
                is_input: false,
            });
            // Force the tap-off row empty so the trunk must bridge around it.
            forced_empty.push((*trunk_x, *tap_y));
        }

        let zone = CrossingZone {
            x: zone_x,
            y: zone_y,
            width: zone_width,
            height: zone_height,
            boundaries,
            forced_empty,
        };

        if let Some(solution) = sat::solve_crossing_zone(&zone, max_reach, effective_belt) {
            // Register SAT entity positions in the tile set.
            for e in &solution.entities {
                tile_set.insert((e.x, e.y));
            }
            solved.push(SolvedCrossing { zone, solution });
        }
    }

    // Build SatCrossingRegions for spec-splitting in negotiate_and_route.
    let mut regions: Vec<SatCrossingRegion> = Vec::new();
    for (tap_item, tap_x, tap_y, crossed) in &zone_specs {
        if crossed.is_empty() {
            continue;
        }
        let x_min = crossed.first().unwrap().0;
        let x_max = crossed.last().unwrap().0;
        // Only create region if SAT actually solved this zone.
        let zone_origin = (x_min, tap_y - 1);
        if solved.iter().any(|sc| sc.zone.x == zone_origin.0 && sc.zone.y == zone_origin.1) {
            regions.push(SatCrossingRegion {
                tap_x: *tap_x,
                x_min,
                x_max,
                tap_y: *tap_y,
                tap_item: tap_item.clone(),
            });
        }
    }

    (solved, tile_set, regions)
}

/// Maximum tiles between PTG input and output positions.
const PTG_MAX_SPAN: i32 = 10;

fn chain_ptg_pairs_vertical(
    entities: &mut Vec<PlacedEntity>,
    x: i32,
    start_y: i32,
    end_y: i32,
    item: &str,
) {
    let mut cur = start_y + 1;
    while cur < end_y {
        let remaining = end_y - cur;
        if remaining == 1 {
            entities.push(PlacedEntity {
                name: "pipe".to_string(),
                x,
                y: cur,
                carries: Some(item.to_string()),
                ..Default::default()
            });
            return;
        }
        let out_pos = std::cmp::min(cur + PTG_MAX_SPAN, end_y - 1);
        entities.push(PlacedEntity {
            name: "pipe-to-ground".to_string(),
            x,
            y: cur,
            direction: EntityDirection::South,
            io_type: Some("input".to_string()),
            carries: Some(item.to_string()),
            ..Default::default()
        });
        entities.push(PlacedEntity {
            name: "pipe-to-ground".to_string(),
            x,
            y: out_pos,
            direction: EntityDirection::South,
            io_type: Some("output".to_string()),
            carries: Some(item.to_string()),
            ..Default::default()
        });
        cur = out_pos + 1;
    }
}

/// Fill the gap between two surface pipes at (start_x, y) and (end_x, y)
/// with PTG pairs (or surface pipe for 1-tile gaps). start_x < end_x.
fn chain_ptg_pairs_horizontal(
    entities: &mut Vec<PlacedEntity>,
    y: i32,
    start_x: i32,
    end_x: i32,
    item: &str,
) {
    let mut cur = start_x + 1;
    while cur < end_x {
        let remaining = end_x - cur;
        if remaining == 1 {
            entities.push(PlacedEntity {
                name: "pipe".to_string(),
                x: cur,
                y,
                carries: Some(item.to_string()),
                ..Default::default()
            });
            return;
        }
        let out_pos = std::cmp::min(cur + PTG_MAX_SPAN, end_x - 1);
        entities.push(PlacedEntity {
            name: "pipe-to-ground".to_string(),
            x: cur,
            y,
            direction: EntityDirection::East,
            io_type: Some("input".to_string()),
            carries: Some(item.to_string()),
            ..Default::default()
        });
        entities.push(PlacedEntity {
            name: "pipe-to-ground".to_string(),
            x: out_pos,
            y,
            direction: EntityDirection::East,
            io_type: Some("output".to_string()),
            carries: Some(item.to_string()),
            ..Default::default()
        });
        cur = out_pos + 1;
    }
}

/// Route a fluid bus lane with PTG-segmented trunks and tap-offs.
///
/// Surface pipes exist only at explicit connection points (trunk source,
/// consumer tap-off y's, producer output y's, and port-pipe tiles). The
/// gaps between them are filled with pipe-to-ground pairs so adjacent
/// fluid trunks at 1-tile spacing don't merge.
pub(crate) fn route_fluid_lane(
    lane: &BusLane,
) -> Vec<PlacedEntity> {
    let mut entities: Vec<PlacedEntity> = Vec::new();
    let x = lane.x;

    // Collect every y where the trunk needs a surface connection
    let mut connection_ys: FxHashSet<i32> = FxHashSet::default();
    connection_ys.insert(lane.source_y);
    for &tap_y in &lane.tap_off_ys {
        connection_ys.insert(tap_y);
    }
    for &(_ri, _px, py) in &lane.fluid_output_port_positions {
        connection_ys.insert(py);
    }

    // Vertical trunk: surface pipe at each connection y, PTG pairs between
    let mut sorted_ys: Vec<i32> = connection_ys.iter().copied().collect();
    sorted_ys.sort_unstable();

    for &y in &sorted_ys {
        entities.push(PlacedEntity {
            name: "pipe".to_string(),
            x,
            y,
            carries: Some(lane.item.clone()),
            rate: Some(lane.rate),
            ..Default::default()
        });
    }

    for i in 0..(sorted_ys.len().saturating_sub(1)) {
        chain_ptg_pairs_vertical(&mut entities, x, sorted_ys[i], sorted_ys[i + 1], &lane.item);
    }

    // Horizontal tap-offs: group ports by y (consumer inputs + producer
    // outputs all connect the same way — a PTG chain from trunk to port
    // pipes along the port y-row). Port pipes are placed by templates.
    let mut port_xs_by_y: FxHashMap<i32, FxHashSet<i32>> = FxHashMap::default();

    for &(_ri, port_x, port_y) in &lane.fluid_port_positions {
        port_xs_by_y.entry(port_y).or_default().insert(port_x);
    }

    for &(_ri, port_x, port_y) in &lane.fluid_output_port_positions {
        port_xs_by_y.entry(port_y).or_default().insert(port_x);
    }

    for (port_y, xs) in port_xs_by_y.iter() {
        // Chain: trunk(x) → first_port → second_port → ... → last_port
        let mut anchors: Vec<i32> = vec![x];
        anchors.extend(xs.iter().copied());
        anchors.sort_unstable();

        for i in 0..(anchors.len().saturating_sub(1)) {
            chain_ptg_pairs_horizontal(&mut entities, *port_y, anchors[i], anchors[i + 1], &lane.item);
        }
    }

    entities
}

// ---------------------------------------------------------------------------
// Negotiated lane routing
// ---------------------------------------------------------------------------

/// Port of Python `_negotiate_and_route`.
///
/// Collects all fixed obstacles (machine tiles, balancer footprints, feeder
/// descent columns, fluid-lane tiles), builds `LaneSpec` objects for every
/// bus segment (trunks, tap-offs, returns, feeders, mergers), runs
/// `negotiate_lanes` (congestion-aware A*), and returns a map from string
/// key to routed path.
///
/// Keys:
/// - `"trunk:{item}:{x}:{start_y}:{end_y}"`
/// - `"tap:{item}:{x}:{y}"`
/// - `"ret:{item}:{x}:{y}"`
/// - `"bal:{item}:{x}:{y}"`
/// - `"feeder:{item}:{input_x}:{out_y}"`
pub(crate) fn negotiate_and_route(
    lanes: &[BusLane],
    row_spans: &[RowSpan],
    total_height: i32,
    bw: i32,
    row_entities: &[PlacedEntity],
    solver_result: &SolverResult,
    families: &[LaneFamily],
    max_belt_tier: Option<&str>,
    sat_regions: &[SatCrossingRegion],
    _sat_obstacles: &FxHashSet<(i32, i32)>,
) -> FxHashMap<String, Vec<(i32, i32)>> {
    use crate::astar::{LaneSpec, negotiate_lanes};
    use crate::bus::balancer_library::balancer_templates;

    // Build item → numeric ID mapping (include output items for merger routing)
    let mut items_set: std::collections::BTreeSet<String> = lanes
        .iter()
        .filter(|ln| !ln.is_fluid)
        .map(|ln| ln.item.clone())
        .collect();
    for ext in &solver_result.external_outputs {
        if !ext.is_fluid {
            items_set.insert(ext.item.clone());
        }
    }
    let item_to_id: FxHashMap<String, u16> = items_set
        .iter()
        .enumerate()
        .map(|(i, item)| (item.clone(), i as u16))
        .collect();

    // Map spec id → string key for result lookup
    let mut id_to_key: FxHashMap<u32, String> = FxHashMap::default();
    let mut specs: Vec<LaneSpec> = Vec::new();
    let mut lane_id: u32 = 0;

    // Compute flow direction from first→last waypoint; None if diagonal/Z.
    let flow_dir = |waypoints: &[(i16, i16)]| -> Option<(i8, i8)> {
        if waypoints.len() < 2 {
            return None;
        }
        let dx = waypoints.last().unwrap().0 - waypoints[0].0;
        let dy = waypoints.last().unwrap().1 - waypoints[0].1;
        let sx = dx.signum() as i8;
        let sy = dy.signum() as i8;
        if sx != 0 && sy != 0 { return None; }
        if sx == 0 && sy == 0 { return None; }
        Some((sx, sy))
    };

    // --- Collect fixed obstacles ---
    let mut obstacles: FxHashSet<(i16, i16)> = FxHashSet::default();

    // SAT crossing zone tiles are NOT added as hard obstacles: the A* tap-off
    // specs are split around SAT zones (they don't route through), and trunk
    // specs already skip SAT-owned y-rows via crossing_tiles in route_belt_lane.

    for e in row_entities {
        if MACHINE_ENTITIES.contains(&e.name.as_str()) {
            let sz = crate::common::machine_size(&e.name) as i32;
            for dx in 0..sz {
                for dy in 0..sz {
                    obstacles.insert(((e.x + dx) as i16, (e.y + dy) as i16));
                }
            }
        } else {
            obstacles.insert((e.x as i16, e.y as i16));
        }
    }

    // Block balancer footprints + feeder descent columns; emit feeder specs.
    // Both obstacle collection and spec generation iterate the same per-family
    // sorted producer/input pairs, so they're merged into one loop.
    let templates = balancer_templates();
    for fam in families {
        if fam.lane_xs.is_empty() {
            continue;
        }
        let shape_key = (fam.shape.0 as u32, fam.shape.1 as u32);
        let template = match templates.get(&shape_key) {
            Some(t) => t,
            None => continue,
        };
        let ox = fam.lane_xs.iter().copied().min().unwrap();
        let oy = fam.balancer_y_start;

        // Balancer body tiles (obstacle)
        for te in template.entities {
            obstacles.insert(((ox + te.x) as i16, (oy + te.y) as i16));
            if te.name == "splitter" {
                if te.direction == 0 || te.direction == 4 {
                    obstacles.insert(((ox + te.x + 1) as i16, (oy + te.y) as i16));
                } else {
                    obstacles.insert(((ox + te.x) as i16, (oy + te.y + 1) as i16));
                }
            }
        }

        // Sort producers top-to-bottom, input tiles left-to-right (used for
        // both obstacle descent columns and feeder LaneSpecs below).
        let mut producers_sorted: Vec<usize> = fam.producer_rows.clone();
        producers_sorted.sort_by_key(|&p| {
            if p < row_spans.len() { row_spans[p].output_belt_y } else { 0 }
        });
        let mut inputs_sorted: Vec<(i32, i32)> = template.input_tiles.to_vec();
        inputs_sorted.sort_by_key(|t| t.0);

        let item_id = item_to_id.get(&fam.item).copied().unwrap_or(0);
        for (&producer_row_idx, &(input_dx, _)) in producers_sorted.iter().zip(inputs_sorted.iter()) {
            if producer_row_idx >= row_spans.len() {
                continue;
            }
            let out_y = row_spans[producer_row_idx].output_belt_y;
            let input_x = ox + input_dx;

            // SOUTH descent column: block it so A* routes around it.
            // (WEST feeder row is A*-routed via feeder: spec below — not blocked.)
            if out_y != fam.balancer_y_start {
                for y in (out_y + 1)..fam.balancer_y_start {
                    obstacles.insert((input_x as i16, y as i16));
                }
            }

            // If a foreign trunk occupies the feeder landing column (input_x+1),
            // mark it as a static obstacle so the trunk is forced underground,
            // leaving the surface free for the feeder to land.
            let landing_x = input_x + 1;
            let has_foreign_trunk_at_landing = lanes.iter().any(|l| {
                !l.is_fluid && l.item != fam.item && l.x == landing_x
            });
            if has_foreign_trunk_at_landing && landing_x < bw {
                obstacles.insert((landing_x as i16, out_y as i16));
            }

            // Feeder spec: A* horizontal WEST, priority=4
            if input_x < bw - 1 {
                let feeder_key = format!("feeder:{}:{}:{}", fam.item, input_x, out_y);
                id_to_key.insert(lane_id, feeder_key);
                let wps = vec![(bw as i16 - 1, out_y as i16), (input_x as i16 + 1, out_y as i16)];
                let fd = flow_dir(&wps);
                specs.push(LaneSpec {
                    id: lane_id,
                    item_id,
                    waypoints: wps,
                    strategy: 2,
                    priority: 4,
                    y_constraint: Some(out_y as i16),
                    x_constraint: None,
                    flow_dir: fd,
                });
                lane_id += 1;
            }
        }
    }

    // Block fluid-lane tiles (pipes + PTG): belt tap-offs must tunnel past them.
    for lane in lanes {
        if !lane.is_fluid {
            continue;
        }
        let mut connection_ys: FxHashSet<i32> = FxHashSet::default();
        connection_ys.insert(lane.source_y);
        for &tap_y in &lane.tap_off_ys {
            connection_ys.insert(tap_y);
        }
        for &(_ri, _px, py) in &lane.fluid_output_port_positions {
            connection_ys.insert(py);
        }
        if !connection_ys.is_empty() {
            let trunk_start = connection_ys.iter().copied().min().unwrap();
            let trunk_end = connection_ys.iter().copied().max().unwrap();
            for y in trunk_start..=trunk_end {
                obstacles.insert((lane.x as i16, y as i16));
            }
        }
        // Horizontal tap-off x-range
        let mut port_xs_by_y: FxHashMap<i32, FxHashSet<i32>> = FxHashMap::default();
        for &(_ri, px, py) in &lane.fluid_port_positions {
            port_xs_by_y.entry(py).or_default().insert(px);
        }
        for &(_ri, px, py) in &lane.fluid_output_port_positions {
            port_xs_by_y.entry(py).or_default().insert(px);
        }
        for (py, xs) in &port_xs_by_y {
            let last_x = xs.iter().copied().max().unwrap();
            for fx in (lane.x + 1)..=last_x {
                obstacles.insert((fx as i16, *py as i16));
            }
        }
    }

    // --- Build demand specs ---

    for lane in lanes {
        if lane.is_fluid {
            continue;
        }
        let item_id = item_to_id.get(&lane.item).copied().unwrap_or(0);
        let x = lane.x;

        let mut all_producers: Vec<usize> = Vec::new();
        if let Some(pr) = lane.producer_row {
            all_producers.push(pr);
        }
        all_producers.extend(&lane.extra_producer_rows);

        if is_intermediate(lane) {
            // Intermediate lane: has both producers and consumers.
            let producer_out_ys: Vec<i32> = all_producers.iter()
                .filter(|&&p| p < row_spans.len())
                .map(|&p| row_spans[p].output_belt_y)
                .collect();
            let start_y = producer_out_ys.iter().copied().min().unwrap_or(lane.source_y);
            let last_tap_y = lane.tap_off_ys.iter().copied().max().unwrap_or(start_y);

            // Trunk segments (vertical A*, priority=5)
            let foreign_skips = foreign_trunk_skip_ys(lane, lanes, row_spans, start_y, last_tap_y - 1);
            let mut skip_ys: FxHashSet<i32> = producer_out_ys.iter().copied().collect();
            skip_ys.extend(foreign_skip_ug_tiles(&foreign_skips).iter().copied());
            for (seg_start, seg_end) in trunk_segments(start_y, last_tap_y - 1, &skip_ys) {
                let trunk_key = format!("trunk:{}:{}:{}:{}", lane.item, x, seg_start, seg_end);
                id_to_key.insert(lane_id, trunk_key);
                let wps = vec![(x as i16, seg_start as i16), (x as i16, seg_end as i16)];
                let fd = flow_dir(&wps);
                specs.push(LaneSpec {
                    id: lane_id,
                    item_id,
                    waypoints: wps,
                    strategy: 2,
                    priority: 5,
                    x_constraint: Some(x as i16),
                    y_constraint: None,
                    flow_dir: fd,
                });
                lane_id += 1;
            }

            // Output returns: horizontal WEST, priority=4
            // Skip when lane has a family balancer — feeders handle routing instead.
            if lane.family_balancer_range.is_none() {
                for &pri in &all_producers {
                    if pri >= row_spans.len() {
                        continue;
                    }
                    let out_y = row_spans[pri].output_belt_y;
                    let ret_key = format!("ret:{}:{}:{}", lane.item, x, out_y);
                    id_to_key.insert(lane_id, ret_key);
                    let wps = vec![(bw as i16 - 1, out_y as i16), (x as i16 + 1, out_y as i16)];
                    let fd = flow_dir(&wps);
                    specs.push(LaneSpec {
                        id: lane_id,
                        item_id,
                        waypoints: wps,
                        strategy: 2,
                        priority: 4,
                        y_constraint: Some(out_y as i16),
                        x_constraint: None,
                        flow_dir: fd,
                    });
                    lane_id += 1;
                }
            }

            // Splitter balance return (Z-wrap), priority=4
            if lane.family_balancer_range.is_none() && all_producers.len() >= 2 && x > 1 {
                if let Some(&last_pri) = all_producers.last() {
                    if last_pri < row_spans.len() {
                        let last_out_y = row_spans[last_pri].output_belt_y;
                        let split_y = last_out_y - 1;
                        let sideload_y = last_out_y;
                        let bal_key = format!("bal:{}:{}:{}", lane.item, x, split_y);
                        id_to_key.insert(lane_id, bal_key);
                        let wps = vec![(bw as i16 - 1, split_y as i16), (x as i16 - 1, sideload_y as i16)];
                        // No flow_dir — allow vertical movement for Z-turn
                        specs.push(LaneSpec {
                            id: lane_id,
                            item_id,
                            waypoints: wps,
                            strategy: 2,
                            priority: 4,
                            y_constraint: None,
                            x_constraint: None,
                            flow_dir: None,
                        });
                        lane_id += 1;
                    }
                }
            }

            // Tap-off: horizontal EAST, priority=6
            // If a SAT crossing zone covers part of this tap-off, split the
            // spec into before-zone and after-zone segments.
            let tap_y = if !lane.tap_off_ys.is_empty() {
                lane.tap_off_ys[0]
            } else {
                last_tap_y
            };
            if x < bw {
                let sat_region = sat_regions.iter().find(|r| {
                    r.tap_x == x && r.tap_y == tap_y && r.tap_item == lane.item && r.x_min > x && r.x_max < bw - 1
                });
                if let Some(region) = sat_region {
                    // Before zone: lane.x → zone.x_min - 1
                    if x < region.x_min {
                        let key = format!("tap:{}:{}:{}", lane.item, x, tap_y);
                        id_to_key.insert(lane_id, key);
                        let wps = vec![(x as i16, tap_y as i16), ((region.x_min - 1) as i16, tap_y as i16)];
                        let fd = flow_dir(&wps);
                        specs.push(LaneSpec {
                            id: lane_id,
                            item_id,
                            waypoints: wps,
                            strategy: 2,
                            priority: 6,
                            y_constraint: Some(tap_y as i16),
                            x_constraint: None,
                            flow_dir: fd,
                        });
                        lane_id += 1;
                    }
                    // After zone: zone.x_max + 1 → bw - 1
                    if region.x_max + 1 < bw {
                        let key = format!("tap:{}:{}:{}_post", lane.item, x, tap_y);
                        id_to_key.insert(lane_id, key);
                        let wps = vec![((region.x_max + 1) as i16, tap_y as i16), (bw as i16 - 1, tap_y as i16)];
                        let fd = flow_dir(&wps);
                        specs.push(LaneSpec {
                            id: lane_id,
                            item_id,
                            waypoints: wps,
                            strategy: 2,
                            priority: 6,
                            y_constraint: Some(tap_y as i16),
                            x_constraint: None,
                            flow_dir: fd,
                        });
                        lane_id += 1;
                    }
                } else {
                    // No SAT zone — normal full-width tap-off spec.
                    let tap_key = format!("tap:{}:{}:{}", lane.item, x, tap_y);
                    id_to_key.insert(lane_id, tap_key);
                    let wps = vec![(x as i16, tap_y as i16), (bw as i16 - 1, tap_y as i16)];
                    let fd = flow_dir(&wps);
                    specs.push(LaneSpec {
                        id: lane_id,
                        item_id,
                        waypoints: wps,
                        strategy: 2,
                        priority: 6,
                        y_constraint: Some(tap_y as i16),
                        x_constraint: None,
                        flow_dir: fd,
                    });
                    lane_id += 1;
                }
            }
        } else if !lane.consumer_rows.is_empty() {
            // External input lane: trunk from source to tap-off, then tap-off EAST.
            let tap_y = if !lane.tap_off_ys.is_empty() {
                lane.tap_off_ys[0]
            } else {
                lane.source_y
            };

            // Trunk segments (vertical A*, priority=5)
            let mut skip_ys: FxHashSet<i32> = lane.tap_off_ys.iter().copied().collect();
            if let Some(bal_y) = lane.balancer_y {
                skip_ys.insert(bal_y);
            }
            // Skip the entire family balancer zone.
            if let Some((by_start, by_end)) = lane.family_balancer_range {
                for y in by_start..=by_end {
                    skip_ys.insert(y);
                }
            }
            let mut end_y = tap_y;
            if let Some(bal_y) = lane.balancer_y {
                end_y = end_y.max(bal_y + 1);
            }
            if let Some((_, by_end)) = lane.family_balancer_range {
                end_y = end_y.max(by_end + 1);
            }
            let foreign_skips = foreign_trunk_skip_ys(lane, lanes, row_spans, lane.source_y, end_y);
            skip_ys.extend(foreign_skip_ug_tiles(&foreign_skips).iter().copied());
            for (seg_start, seg_end) in trunk_segments(lane.source_y, end_y, &skip_ys) {
                let trunk_key = format!("trunk:{}:{}:{}:{}", lane.item, x, seg_start, seg_end);
                id_to_key.insert(lane_id, trunk_key);
                let wps = vec![(x as i16, seg_start as i16), (x as i16, seg_end as i16)];
                let fd = flow_dir(&wps);
                specs.push(LaneSpec {
                    id: lane_id,
                    item_id,
                    waypoints: wps,
                    strategy: 2,
                    priority: 5,
                    x_constraint: Some(x as i16),
                    y_constraint: None,
                    flow_dir: fd,
                });
                lane_id += 1;
            }

            // Tap-off: horizontal EAST, priority=6
            // Split around SAT crossing zones if present.
            if x < bw {
                let sat_region = sat_regions.iter().find(|r| {
                    r.tap_x == x && r.tap_y == tap_y && r.tap_item == lane.item && r.x_min > x && r.x_max < bw - 1
                });
                if let Some(region) = sat_region {
                    if x < region.x_min {
                        let key = format!("tap:{}:{}:{}", lane.item, x, tap_y);
                        id_to_key.insert(lane_id, key);
                        let wps = vec![(x as i16, tap_y as i16), ((region.x_min - 1) as i16, tap_y as i16)];
                        let fd = flow_dir(&wps);
                        specs.push(LaneSpec {
                            id: lane_id, item_id, waypoints: wps, strategy: 2,
                            priority: 6, y_constraint: Some(tap_y as i16),
                            x_constraint: None, flow_dir: fd,
                        });
                        lane_id += 1;
                    }
                    if region.x_max + 1 < bw {
                        let key = format!("tap:{}:{}:{}_post", lane.item, x, tap_y);
                        id_to_key.insert(lane_id, key);
                        let wps = vec![((region.x_max + 1) as i16, tap_y as i16), (bw as i16 - 1, tap_y as i16)];
                        let fd = flow_dir(&wps);
                        specs.push(LaneSpec {
                            id: lane_id, item_id, waypoints: wps, strategy: 2,
                            priority: 6, y_constraint: Some(tap_y as i16),
                            x_constraint: None, flow_dir: fd,
                        });
                        lane_id += 1;
                    }
                } else {
                    let tap_key = format!("tap:{}:{}:{}", lane.item, x, tap_y);
                    id_to_key.insert(lane_id, tap_key);
                    let wps = vec![(x as i16, tap_y as i16), (bw as i16 - 1, tap_y as i16)];
                    let fd = flow_dir(&wps);
                    specs.push(LaneSpec {
                        id: lane_id, item_id, waypoints: wps, strategy: 2,
                        priority: 6, y_constraint: Some(tap_y as i16),
                        x_constraint: None, flow_dir: fd,
                    });
                    lane_id += 1;
                }
            }
        } else {
            // Collector lane (output/collector only): trunk + output returns.
            let mut all_ys: Vec<i32> = lane.tap_off_ys.clone();
            for &pri in &all_producers {
                if pri < row_spans.len() {
                    all_ys.push(row_spans[pri].output_belt_y);
                }
            }
            let mut end_y = all_ys.iter().copied().max().unwrap_or(lane.source_y);
            if let Some(bal_y) = lane.balancer_y {
                end_y = end_y.max(bal_y + 1);
            }

            // Trunk segments (vertical A*, priority=5)
            let mut skip_ys: FxHashSet<i32> = lane.tap_off_ys.iter().copied().collect();
            if let Some(bal_y) = lane.balancer_y {
                skip_ys.insert(bal_y);
            }
            let foreign_skips = foreign_trunk_skip_ys(lane, lanes, row_spans, lane.source_y, end_y);
            skip_ys.extend(foreign_skip_ug_tiles(&foreign_skips).iter().copied());
            for (seg_start, seg_end) in trunk_segments(lane.source_y, end_y, &skip_ys) {
                let trunk_key = format!("trunk:{}:{}:{}:{}", lane.item, x, seg_start, seg_end);
                id_to_key.insert(lane_id, trunk_key);
                let wps = vec![(x as i16, seg_start as i16), (x as i16, seg_end as i16)];
                let fd = flow_dir(&wps);
                specs.push(LaneSpec {
                    id: lane_id,
                    item_id,
                    waypoints: wps,
                    strategy: 2,
                    priority: 5,
                    x_constraint: Some(x as i16),
                    y_constraint: None,
                    flow_dir: fd,
                });
                lane_id += 1;
            }

            // Output returns: horizontal WEST, priority=4
            // Skip when lane has a family balancer — feeders handle routing.
            if lane.family_balancer_range.is_none() {
                for &pri in &all_producers {
                    if pri >= row_spans.len() {
                        continue;
                    }
                    let out_y = row_spans[pri].output_belt_y;
                    let ret_key = format!("ret:{}:{}:{}", lane.item, x, out_y);
                    id_to_key.insert(lane_id, ret_key);
                    let wps = vec![(bw as i16 - 1, out_y as i16), (x as i16 + 1, out_y as i16)];
                    let fd = flow_dir(&wps);
                    specs.push(LaneSpec {
                        id: lane_id,
                        item_id,
                        waypoints: wps,
                        strategy: 2,
                        priority: 4,
                        y_constraint: Some(out_y as i16),
                        x_constraint: None,
                        flow_dir: fd,
                    });
                    lane_id += 1;
                }
            }
        }
    }

    // --- Merger segments (axis-aligned, highest priority=8) ---
    let mut item_lane_groups: FxHashMap<String, Vec<&BusLane>> = FxHashMap::default();
    for lane in lanes {
        if !lane.is_fluid {
            item_lane_groups.entry(lane.item.clone()).or_default().push(lane);
        }
    }
    for (item, group) in &item_lane_groups {
        if group.len() <= 1 || group.iter().all(|ln| !ln.consumer_rows.is_empty()) {
            continue;
        }
        let item_id = item_to_id.get(item.as_str()).copied().unwrap_or(0);
        let mut trunk_xs: Vec<i32> = group.iter().map(|ln| ln.x).collect();
        trunk_xs.sort_unstable();
        let merge_y = total_height;

        for ln in group {
            let wps = vec![(ln.x as i16, ln.source_y as i16), (ln.x as i16, merge_y as i16 + 3)];
            specs.push(LaneSpec {
                id: lane_id,
                item_id,
                waypoints: wps,
                strategy: 0,
                priority: 8,
                x_constraint: None,
                y_constraint: None,
                flow_dir: None,
            });
            lane_id += 1;
        }

        let mut i = 0;
        while i + 1 < trunk_xs.len() {
            let left_x = trunk_xs[i];
            let right_x = trunk_xs[i + 1];
            let wps = vec![(right_x as i16, merge_y as i16), (left_x as i16 + 1, merge_y as i16)];
            specs.push(LaneSpec {
                id: lane_id,
                item_id,
                waypoints: wps,
                strategy: 0,
                priority: 8,
                x_constraint: None,
                y_constraint: None,
                flow_dir: None,
            });
            lane_id += 1;
            i += 2;
        }
    }

    if specs.is_empty() {
        return FxHashMap::default();
    }

    let max_extent = (bw.max(total_height) + 50) as i16;
    let effective_belt = crate::common::belt_entity_for_rate(f64::MAX, max_belt_tier);
    let reach = crate::common::ug_max_reach(effective_belt) as i16;
    let routed = negotiate_lanes(
        &specs,
        &obstacles,
        /* max_iterations */ 20,
        max_extent,
        /* allow_underground */ true,
        reach,
        /* history_factor */ 0.5,
        /* present_factor */ 1.0,
    );

    // Build result map: string key → path (cast i16 coords back to i32)
    let mut result: FxHashMap<String, Vec<(i32, i32)>> = FxHashMap::default();
    for r in &routed {
        if let Some(key) = id_to_key.get(&r.id) {
            if !r.path.is_empty() {
                let path: Vec<(i32, i32)> = r.path.iter().map(|&(x, y)| (x as i32, y as i32)).collect();
                result.insert(key.clone(), path);
            }
        }
    }
    result
}

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
    fn test_render_family_input_paths_no_lane_xs() {
        let family = LaneFamily {
            item: "iron-plate".to_string(),
            shape: (1, 2),
            producer_rows: vec![0],
            lane_xs: vec![],  // empty
            balancer_y_start: 10,
            balancer_y_end: 11,
            total_rate: 20.0,
        };

        let row_span = make_test_row_span(
            "iron-plate",
            8,
            vec![],
            vec![ItemFlow { item: "iron-plate".to_string(), rate: 20.0, is_fluid: false }],
            1,
            vec![],
        );

        let result = render_family_input_paths(&family, &[row_span], "transport-belt", None);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 0);
    }

    #[test]
    fn test_render_family_input_paths_n_equals_1() {
        // N=1 case: no descent column needed
        let family = LaneFamily {
            item: "iron-plate".to_string(),
            shape: (1, 2),
            producer_rows: vec![0],
            lane_xs: vec![1, 2],
            balancer_y_start: 10,  // Same as producer output_belt_y
            balancer_y_end: 11,
            total_rate: 20.0,
        };

        let row_span = make_test_row_span(
            "iron-plate",
            8,
            vec![],
            vec![ItemFlow { item: "iron-plate".to_string(), rate: 20.0, is_fluid: false }],
            1,
            vec![],
        );

        // Manually override output_belt_y to match balancer_y_start
        let mut row_span = row_span;
        row_span.output_belt_y = 10;

        let result = render_family_input_paths(&family, &[row_span], "transport-belt", None);
        assert!(result.is_ok());

        // For N=1 with out_y == origin_y, no descent column is needed
        // We expect only the feeder path (if provided) or nothing
        let entities = result.unwrap();
        // No descent column should be added
        assert!(entities.iter().all(|e| e.y != 10 || e.name != "transport-belt" || e.direction != EntityDirection::South));
    }

    #[test]
    fn test_render_family_input_paths_n_greater_than_1() {
        // N>1 case: descent column needed
        let family = LaneFamily {
            item: "iron-plate".to_string(),
            shape: (2, 2),
            producer_rows: vec![0, 1],
            lane_xs: vec![1, 2],
            balancer_y_start: 15,
            balancer_y_end: 18,
            total_rate: 20.0,
        };

        let row_span1 = make_test_row_span(
            "iron-plate",
            5,
            vec![],
            vec![ItemFlow { item: "iron-plate".to_string(), rate: 10.0, is_fluid: false }],
            1,
            vec![],
        );

        let row_span2 = make_test_row_span(
            "iron-plate",
            10,
            vec![],
            vec![ItemFlow { item: "iron-plate".to_string(), rate: 10.0, is_fluid: false }],
            1,
            vec![],
        );

        let result = render_family_input_paths(&family, &[row_span1, row_span2], "transport-belt", None);
        assert!(result.is_ok());

        let entities = result.unwrap();
        // Should have descent columns
        let descent_belts: Vec<_> = entities.iter()
            .filter(|e| e.direction == EntityDirection::South)
            .collect();
        assert!(!descent_belts.is_empty());
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

    #[test]
    fn test_place_merger_block_no_merge_needed() {
        let lane = BusLane {
            item: "iron-plate".to_string(),
            x: 0,
            source_y: 0,
            consumer_rows: vec![0],
            producer_row: None,
            rate: 10.0,
            is_fluid: false,
            tap_off_ys: vec![5],
            extra_producer_rows: vec![],
            balancer_y: None,
            family_id: None,
            fluid_port_positions: vec![],
            fluid_output_port_positions: vec![],
            family_balancer_range: None,
        };

        let row_span = make_test_row_span(
            "iron-plate",
            0,
            vec![],
            vec![ItemFlow { item: "iron-plate".to_string(), rate: 10.0, is_fluid: false }],
            1,
            vec![],
        );

        // Single lane: no merging needed (n <= target_m)
        let (entities, end_y) = place_merger_block(&[lane], &[row_span], 15, &[], None);
        assert_eq!(entities.len(), 0); // No entities added if no merge needed
        assert_eq!(end_y, 15);
    }

    #[test]
    fn test_place_merger_block_multiple_lanes() {
        let lanes = vec![
            BusLane {
                item: "iron-plate".to_string(),
                x: 0,
                source_y: 0,
                consumer_rows: vec![0],
                producer_row: None,
                rate: 10.0,
                is_fluid: false,
                tap_off_ys: vec![5],
                extra_producer_rows: vec![],
                balancer_y: None,
                family_id: None,
                fluid_port_positions: vec![],
                fluid_output_port_positions: vec![],
            family_balancer_range: None,
            },
            BusLane {
                item: "iron-plate".to_string(),
                x: 1,
                source_y: 0,
                consumer_rows: vec![0],
                producer_row: None,
                rate: 10.0,
                is_fluid: false,
                tap_off_ys: vec![5],
                extra_producer_rows: vec![],
                balancer_y: None,
                family_id: None,
                fluid_port_positions: vec![],
                fluid_output_port_positions: vec![],
            family_balancer_range: None,
            },
        ];

        let row_span = make_test_row_span(
            "iron-plate",
            0,
            vec![],
            vec![ItemFlow { item: "iron-plate".to_string(), rate: 20.0, is_fluid: false }],
            1,
            vec![],
        );

        // Two lanes of 10.0 each = 20.0 total. With transport-belt (15.0 cap), target_m = 2.
        // But we have 2 lanes already, so no merge needed.
        let (_entities, _end_y) = place_merger_block(&lanes, &[row_span], 15, &[], None);
        // Merge only needed if n > target_m
        // For 20.0 rate with transport-belt (15.0 cap): target_m = ceil(20.0/15.0) = 2
        // So no merge needed for 2 lanes
    }


    #[test]
    fn test_route_fluid_lane_basic() {
        let lane = BusLane {
            item: "water".to_string(),
            x: 5,
            source_y: 0,
            consumer_rows: vec![0],
            producer_row: None,
            rate: 10.0,
            is_fluid: true,
            tap_off_ys: vec![10],
            extra_producer_rows: vec![],
            balancer_y: None,
            family_id: None,
            fluid_port_positions: vec![],
            fluid_output_port_positions: vec![],
            family_balancer_range: None,
        };

        let entities = route_fluid_lane(&lane);

        // Should have surface pipes at source and tap-off y
        let pipe_entities: Vec<_> = entities
            .iter()
            .filter(|e| e.name == "pipe" && e.carries.as_deref() == Some("water"))
            .collect();
        assert_eq!(pipe_entities.len(), 2); // source_y=0 and tap_off_y=10

        // Check positions
        let pipe_ys: Vec<i32> = pipe_entities
            .iter()
            .map(|e| e.y)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        assert_eq!(pipe_ys, vec![0, 10]);

        // Should have PTG pairs to fill the gap between 0 and 10
        let ptg_entities: Vec<_> = entities
            .iter()
            .filter(|e| e.name == "pipe-to-ground")
            .collect();
        assert!(!ptg_entities.is_empty());
    }

    #[test]
    fn test_chain_ptg_pairs_vertical_single_gap() {
        let mut entities = Vec::new();
        chain_ptg_pairs_vertical(&mut entities, 5, 0, 2, "water");

        // Gap of 1 tile should result in a single surface pipe
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].name, "pipe");
        assert_eq!(entities[0].x, 5);
        assert_eq!(entities[0].y, 1);
    }

    #[test]
    fn test_chain_ptg_pairs_vertical_multi_gap() {
        let mut entities = Vec::new();
        chain_ptg_pairs_vertical(&mut entities, 5, 0, 15, "water");

        // Gap of 14 tiles requires PTG pairs
        // With PTG_MAX_SPAN=10, first pair should be at y=1 (input) and y=10 (output)
        // Then gap from 10 to 14 remaining (4 tiles), another pair at y=11 (input) and y=14 (output)
        let ptg_inputs: Vec<_> = entities
            .iter()
            .filter(|e| e.name == "pipe-to-ground" && e.io_type.as_deref() == Some("input"))
            .collect();
        assert_eq!(ptg_inputs.len(), 2);
    }

    #[test]
    fn test_chain_ptg_pairs_horizontal_single_gap() {
        let mut entities = Vec::new();
        chain_ptg_pairs_horizontal(&mut entities, 5, 0, 2, "water");

        // Gap of 1 tile should result in a single surface pipe
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].name, "pipe");
        assert_eq!(entities[0].y, 5);
        assert_eq!(entities[0].x, 1);
    }

    #[test]
    fn test_route_fluid_lane_with_port_positions() {
        let lane = BusLane {
            item: "crude-oil".to_string(),
            x: 5,
            source_y: 0,
            consumer_rows: vec![0],
            producer_row: None,
            rate: 20.0,
            is_fluid: true,
            tap_off_ys: vec![10],
            extra_producer_rows: vec![],
            balancer_y: None,
            family_id: None,
            fluid_port_positions: vec![(0, 7, 10), (0, 8, 10)],
            fluid_output_port_positions: vec![],
            family_balancer_range: None,
        };

        let entities = route_fluid_lane(&lane);

        // Should have pipes at trunk and for connecting ports
        let pipe_entities: Vec<_> = entities
            .iter()
            .filter(|e| e.name == "pipe")
            .collect();
        assert!(!pipe_entities.is_empty());

        // All entities should carry crude-oil
        for entity in &entities {
            assert_eq!(entity.carries, Some("crude-oil".to_string()));
        }
    }

    #[test]
    fn test_vec_to_entity_dir() {
        assert_eq!(vec_to_entity_dir(1, 0), EntityDirection::East);
        assert_eq!(vec_to_entity_dir(-1, 0), EntityDirection::West);
        assert_eq!(vec_to_entity_dir(0, 1), EntityDirection::South);
        assert_eq!(vec_to_entity_dir(0, -1), EntityDirection::North);
    }

    // -----------------------------------------------------------------------
    // plan_bus_lanes tests
    // -----------------------------------------------------------------------

    fn make_solver_result_iron_gear_wheel() -> crate::models::SolverResult {
        // Iron-gear-wheel: 1 recipe, 1 solid input (iron-plate).
        // We construct it by hand to avoid recipe_db dependency in the test module.
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
        // Plastic-bar: 1 recipe with coal (solid) + petroleum-gas (fluid).
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
    fn test_route_belt_lane_external_input_produces_south_trunk() {
        // A simple external-input lane: source at top, one consumer below.
        // The trunk segment between source_y and tap_off_y should be SOUTH belts.
        let lane = BusLane {
            item: "iron-plate".to_string(),
            x: 1,
            source_y: 0,
            consumer_rows: vec![0],
            producer_row: None,
            rate: 2.0,
            is_fluid: false,
            tap_off_ys: vec![5],
            extra_producer_rows: vec![],
            balancer_y: None,
            family_id: None,
            fluid_port_positions: vec![],
            fluid_output_port_positions: vec![],
            family_balancer_range: None,
        };

        let row_span = make_test_row_span(
            "iron-gear-wheel",
            3,
            vec![ItemFlow { item: "iron-plate".to_string(), rate: 2.0, is_fluid: false }],
            vec![ItemFlow { item: "iron-gear-wheel".to_string(), rate: 1.0, is_fluid: false }],
            1,
            vec![5],
        );

        let mut entities: Vec<PlacedEntity> = Vec::new();
        route_lane(&mut entities, &lane, std::slice::from_ref(&lane), &[row_span], 3, None, None, &FxHashSet::default());

        // Should have produced some entities
        assert!(!entities.is_empty(), "route_lane must produce entities");

        // All belt entities must carry the item
        for e in &entities {
            if e.name.contains("belt") || e.name == "splitter" {
                assert_eq!(
                    e.carries.as_deref(),
                    Some("iron-plate"),
                    "All entities should carry iron-plate, got: {:?}",
                    e
                );
            }
        }

        // Trunk segment (x=1, y=0..4) should be SOUTH belts
        let trunk_belts: Vec<_> = entities.iter()
            .filter(|e| e.x == 1 && e.y < 5 && e.name.contains("belt") && !e.name.contains("underground"))
            .collect();
        assert!(!trunk_belts.is_empty(), "Expected SOUTH trunk belts at x=1");
        for b in &trunk_belts {
            assert_eq!(b.direction, EntityDirection::South, "Trunk belts should face SOUTH");
        }
    }

    #[test]
    fn test_route_belt_lane_tap_off_is_east() {
        // The tap-off belt at tap_y should face EAST.
        let lane = BusLane {
            item: "copper-plate".to_string(),
            x: 2,
            source_y: 0,
            consumer_rows: vec![0],
            producer_row: None,
            rate: 2.0,
            is_fluid: false,
            tap_off_ys: vec![8],
            extra_producer_rows: vec![],
            balancer_y: None,
            family_id: None,
            fluid_port_positions: vec![],
            fluid_output_port_positions: vec![],
            family_balancer_range: None,
        };

        let row_span = make_test_row_span(
            "electronic-circuit",
            6,
            vec![ItemFlow { item: "copper-plate".to_string(), rate: 2.0, is_fluid: false }],
            vec![ItemFlow { item: "electronic-circuit".to_string(), rate: 1.0, is_fluid: false }],
            1,
            vec![8],
        );

        let mut entities: Vec<PlacedEntity> = Vec::new();
        route_lane(&mut entities, &lane, std::slice::from_ref(&lane), &[row_span], 4, None, None, &FxHashSet::default());

        // The tap-off at y=8 should be an EAST belt
        let tap_belt = entities.iter().find(|e| e.x == 2 && e.y == 8 && e.name.contains("belt") && !e.name.contains("underground"));
        assert!(tap_belt.is_some(), "Expected a belt at tap-off position (x=2, y=8)");
        assert_eq!(tap_belt.unwrap().direction, EntityDirection::East, "Tap-off belt should face EAST");
    }

    #[test]
    fn test_route_belt_lane_underground_when_crossing_another_lane() {
        // When lane.x has a west-neighbor lane with a producer output at tap_y,
        // route_belt_lane must emit underground-belt pairs to cross the conflicting y.
        // Set up two lanes: left (x=1) is west neighbor with producer output at y=5;
        // right (x=2) must cross y=5 underground.
        let west_lane = BusLane {
            item: "copper-plate".to_string(),
            x: 1,
            source_y: 3,
            consumer_rows: vec![],
            producer_row: Some(0),
            rate: 2.0,
            is_fluid: false,
            tap_off_ys: vec![],
            extra_producer_rows: vec![],
            balancer_y: None,
            family_id: None,
            fluid_port_positions: vec![],
            fluid_output_port_positions: vec![],
            family_balancer_range: None,
        };
        let east_lane = BusLane {
            item: "iron-plate".to_string(),
            x: 2,
            source_y: 0,
            consumer_rows: vec![1],
            producer_row: None,
            rate: 2.0,
            is_fluid: false,
            tap_off_ys: vec![8],
            extra_producer_rows: vec![],
            balancer_y: None,
            family_id: None,
            fluid_port_positions: vec![],
            fluid_output_port_positions: vec![],
            family_balancer_range: None,
        };

        let producer_row = make_test_row_span(
            "copper-plate",
            3,
            vec![],
            vec![ItemFlow { item: "copper-plate".to_string(), rate: 2.0, is_fluid: false }],
            1,
            vec![],
        );
        // Adjust output_belt_y to y=5 (within east_lane's trunk range 0..8)
        let mut producer_row = producer_row;
        producer_row.output_belt_y = 5;

        let consumer_row = make_test_row_span(
            "iron-gear-wheel",
            6,
            vec![ItemFlow { item: "iron-plate".to_string(), rate: 2.0, is_fluid: false }],
            vec![],
            1,
            vec![8],
        );

        let all_lanes = vec![west_lane.clone(), east_lane.clone()];
        let row_spans = vec![producer_row, consumer_row];

        let mut entities: Vec<PlacedEntity> = Vec::new();
        route_lane(&mut entities, &east_lane, &all_lanes, &row_spans, 4, None, None, &FxHashSet::default());

        // Should have underground belts at y=4 (input before y=5) and y=6 (output after y=5)
        let ug_entities: Vec<_> = entities.iter()
            .filter(|e| e.name.contains("underground-belt") && e.x == 2)
            .collect();
        assert!(
            !ug_entities.is_empty(),
            "Expected underground belt pair at x=2 to cross the foreign lane's producer output y=5; got entities: {:?}",
            entities.iter().map(|e| (&e.name, e.x, e.y, &e.direction)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_route_fluid_lane_ptg_between_source_and_consumer() {
        // A fluid lane spanning source_y=0 to tap_y=20 should have PTG pairs,
        // not just a solid pipe column (which would merge with adjacent networks).
        let lane = BusLane {
            item: "petroleum-gas".to_string(),
            x: 3,
            source_y: 0,
            consumer_rows: vec![0],
            producer_row: None,
            rate: 20.0,
            is_fluid: true,
            tap_off_ys: vec![20],
            extra_producer_rows: vec![],
            balancer_y: None,
            family_id: None,
            fluid_port_positions: vec![],
            fluid_output_port_positions: vec![],
            family_balancer_range: None,
        };

        let entities = route_fluid_lane(&lane);

        // Surface pipes only at connection points (source and tap-off)
        let surface_pipes: Vec<_> = entities.iter().filter(|e| e.name == "pipe").collect();
        assert_eq!(surface_pipes.len(), 2, "Only 2 surface pipes: source and tap-off");
        let pipe_ys: Vec<i32> = {
            let mut ys: Vec<i32> = surface_pipes.iter().map(|e| e.y).collect();
            ys.sort();
            ys
        };
        assert_eq!(pipe_ys, vec![0, 20]);

        // PTG pairs fill the gap
        let ptg: Vec<_> = entities.iter().filter(|e| e.name == "pipe-to-ground").collect();
        assert!(!ptg.is_empty(), "Expected pipe-to-ground pairs for fluid trunk isolation");

        // Every entity should carry petroleum-gas
        for e in &entities {
            assert_eq!(e.carries.as_deref(), Some("petroleum-gas"), "All fluid entities should carry petroleum-gas");
        }
    }

    #[test]
    fn test_route_fluid_lane_ptg_input_output_pairs() {
        // PTG pairs must appear as input/output pairs, not orphaned single entities.
        let lane = BusLane {
            item: "water".to_string(),
            x: 4,
            source_y: 0,
            consumer_rows: vec![0],
            producer_row: None,
            rate: 5.0,
            is_fluid: true,
            tap_off_ys: vec![12],
            extra_producer_rows: vec![],
            balancer_y: None,
            family_id: None,
            fluid_port_positions: vec![],
            fluid_output_port_positions: vec![],
            family_balancer_range: None,
        };

        let entities = route_fluid_lane(&lane);

        let ptg_inputs: Vec<_> = entities.iter()
            .filter(|e| e.name == "pipe-to-ground" && e.io_type.as_deref() == Some("input"))
            .collect();
        let ptg_outputs: Vec<_> = entities.iter()
            .filter(|e| e.name == "pipe-to-ground" && e.io_type.as_deref() == Some("output"))
            .collect();
        assert_eq!(
            ptg_inputs.len(), ptg_outputs.len(),
            "PTG inputs and outputs must be balanced; inputs={}, outputs={}",
            ptg_inputs.len(), ptg_outputs.len()
        );
        assert!(!ptg_inputs.is_empty(), "Expected at least one PTG pair for 12-tile gap");
    }

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
