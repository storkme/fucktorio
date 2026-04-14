//! Lane planning: deciding how items flow between rows on the main bus.
//!
//! Takes the solver's machine list + the placer's row spans and
//! produces one `BusLane` per item that needs trunk routing, plus
//! zero or more `LaneFamily` balancer blocks. Lane column order is
//! chosen via `lane_order::optimize_lane_order` to minimise tap-off
//! crossings. See `docs/ghost-pipeline-contracts.md` for the phase
//! contract this module upholds.

use std::cmp::Ordering;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::models::SolverResult;
use crate::bus::lane_order::optimize_lane_order;
use crate::bus::placer::RowSpan;

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


/// Build the full `(BusLane, LaneFamily)` set from a solver result and
/// placed row spans. Lanes are ordered left-to-right to minimise
/// tap-off crossings; families are created for any item whose rate
/// exceeds per-lane capacity OR whose producer/lane shape needs a
/// balancer block. Each lane's `family_id` indexes into the returned
/// family vector.
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


#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ItemFlow, MachineSpec};

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
            output_east: true,
            output_belt_x_min: 0,
            output_belt_x_max: 9,
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
                output_east: true,
                output_belt_x_min: 0,
                output_belt_x_max: 9,
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
