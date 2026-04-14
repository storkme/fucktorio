//! Belt path rendering helpers used by `ghost_router` to turn A*
//! routed paths into entity lists, plus a few small utility predicates
//! shared between rendering and planning.
//!
//! - `render_path` walks a path of tiles and emits surface belts /
//!   underground pairs, inheriting a segment id.
//! - `trunk_segments` slices a `[start_y, end_y]` trunk range into
//!   contiguous sub-ranges minus a skip set (tap-off rows, balancer
//!   blocks).
//! - `is_intermediate` — a small predicate used by `ghost_router`'s
//!   spec generator to classify lanes.

use rustc_hash::FxHashSet;

use crate::models::{EntityDirection, PlacedEntity};
use crate::bus::balancer::underground_for_belt;
use crate::bus::lane_planner::BusLane;

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
            segment_id: segment_id.clone(),
            rate,
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
/// An "intermediate" lane has both producers and consumers — its
/// items are produced by one or more rows and consumed by others,
/// so it needs both return paths (producer → trunk) and tap-offs
/// (trunk → consumer). Returns `false` for collector lanes (no
/// consumers) and external-input lanes (no producers).
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
