//! Final-product output merger.
//!
//! After ghost routing places all trunk / tap / return belts, this
//! module merges the east-flowing output belts of rows producing the
//! same final product into a single south-facing splitter chain at
//! the bottom-right of the layout. Called once per final product at
//! the end of `route_bus_ghost` (Step 7).

use crate::models::{EntityDirection, PlacedEntity};
use crate::bus::balancer::splitter_for_belt;
use crate::bus::placer::RowSpan;

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
}
