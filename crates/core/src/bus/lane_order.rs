//! Lane left-to-right ordering optimiser.
//!
//! Given the planner's `BusLane` list, picks the column order that
//! minimises tap-off / return-path crossings while keeping balancer
//! family lanes contiguous. Exact search for small lane sets, hill
//! climbing above the cutoff. Called once per `plan_bus_lanes` pass.

use rustc_hash::FxHashMap;

use crate::bus::lane_planner::BusLane;
use crate::bus::placer::RowSpan;

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
        }
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
