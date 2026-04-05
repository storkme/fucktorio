//! Bus layout routing: trunk belt placement and tap-off coordination.
//!
//! Each item that flows between rows gets a dedicated vertical bus lane.
//! Lanes run SOUTH (top to bottom). At the consuming row, the lane turns
//! EAST into the row's input belt (tap-off). When a tap-off crosses another
//! lane's vertical segment, the tap-off goes underground (EAST) past it.
//!
//! Port of `src/bus/bus_router.py` lines 1-700 (trunk placement + tap-off infrastructure).

use std::cmp::Ordering;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::models::SolverResult;
use crate::bus::placer::RowSpan;

/// Per-lane capacity for each belt tier (half of total throughput).
const LANE_CAPACITY_TABLE: &[(&str, f64)] = &[
    ("transport-belt", 7.5),
    ("fast-transport-belt", 15.0),
    ("express-transport-belt", 22.5),
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
            item_to_consumers.entry(inp.item.clone()).or_insert_with(Vec::new).push(idx);
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
            item_to_producers.entry(out.item.clone()).or_insert_with(Vec::new).push(idx);
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
        }
    }
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
    let n = ordered.len();
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

    for pos in 0..n {
        let lane = &ordered[pos];
        // EAST tap-off crossings
        for &tap_y in &lane.tap_off_ys {
            for rpos in (pos + 1)..n {
                let (rs, re) = ranges[rpos];
                if rs <= tap_y && tap_y <= re {
                    score += 1;
                }
            }
        }

        // WEST output return crossings
        let all_producers = lane_all_producers(lane);
        for &pri in &all_producers {
            let ret_y = row_spans[pri].output_belt_y;
            for rpos in (pos + 1)..n {
                let (rs, re) = ranges[rpos];
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
                -(*ln.tap_off_ys.iter().min().unwrap() as i32)
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
                producer_rows: all_producer_rows.iter()
                    .copied()
                    .collect::<Vec<_>>(),
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
}
