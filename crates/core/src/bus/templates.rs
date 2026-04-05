//! Assembly row templates: patterns of belts, inserters, and machines.
//!
//! Every belt and inserter entity is tagged with `carries` so the validator
//! can trace item flow through the layout.
//!
//! Machines are packed with zero gap (3-tile pitch for 3×3 machines).
//! When lane splitting is active, machines are split into two groups with a
//! sideload bridge in between so both output belt lanes are utilised.
//!
//! Port of `src/bus/templates.py`.

use crate::models::{EntityDirection, PlacedEntity};

/// Horizontal pitch: machine width with no gap.
pub const MACHINE_PITCH: i32 = 3;

/// Gap between machine groups when lane-splitting output belts.
/// 3 tiles: 1 for sideload target filler, 1 for through-belt filler,
/// 1 for the NORTH lift from group 2.
pub const LANE_SPLIT_GAP: i32 = 3;

/// Oil refinery pitch (5×5 machine).
pub const OIL_REFINERY_PITCH: i32 = 5;

// Fluid port dx (relative to machine tile_position) for each machine type.
fn fluid_input_port_dx(machine_entity: &str) -> i32 {
    match machine_entity {
        "assembling-machine-2" | "assembling-machine-3" => 1,
        _ => 0,
    }
}

/// Map `output_east` flag to the corresponding belt direction.
fn output_dir(output_east: bool) -> EntityDirection {
    if output_east { EntityDirection::East } else { EntityDirection::West }
}

/// Return x-coordinates for each machine, accounting for lane-split gap.
fn machine_xs(x_offset: i32, machine_count: usize, lane_split: bool) -> Vec<i32> {
    if !lane_split || machine_count < 2 {
        return (0..machine_count as i32)
            .map(|i| x_offset + i * MACHINE_PITCH)
            .collect();
    }

    let g1 = machine_count / 2;
    let mut positions = Vec::with_capacity(machine_count);
    for i in 0..g1 {
        positions.push(x_offset + i as i32 * MACHINE_PITCH);
    }
    for j in 0..(machine_count - g1) {
        positions.push(x_offset + g1 as i32 * MACHINE_PITCH + LANE_SPLIT_GAP + j as i32 * MACHINE_PITCH);
    }
    positions
}

/// Generate the 6-entity sideload bridge between two machine groups.
///
/// `output_row_dy` is the output belt's offset from `y_offset`
/// (6 for `single_input_row`, 7 for `dual_input_row`).
///
/// When `output_east` is `true`, the bridge is mirrored: group 1 items
/// flow EAST across the bridge into group 2 (instead of group 2 → group 1).
fn sideload_bridge(
    gap_start_x: i32,
    y_offset: i32,
    output_row_dy: i32,
    belt: &str,
    item: &str,
    output_east: bool,
) -> Vec<PlacedEntity> {
    let bridge_y = y_offset + output_row_dy - 1;
    let output_y = y_offset + output_row_dy;

    let carries = Some(item.to_string());
    let belt = belt.to_string();

    if output_east {
        // EAST flow: group 1 → bridge EAST → group 2
        vec![
            // Bridge row
            PlacedEntity {
                name: belt.clone(),
                x: gap_start_x,
                y: bridge_y,
                direction: EntityDirection::East,
                carries: carries.clone(),
                ..Default::default()
            },
            PlacedEntity {
                name: belt.clone(),
                x: gap_start_x + 1,
                y: bridge_y,
                direction: EntityDirection::East,
                carries: carries.clone(),
                ..Default::default()
            },
            PlacedEntity {
                name: belt.clone(),
                x: gap_start_x + 2,
                y: bridge_y,
                direction: EntityDirection::South,
                carries: carries.clone(),
                ..Default::default()
            },
            // Output belt row — gap tiles
            PlacedEntity {
                name: belt.clone(),
                x: gap_start_x,
                y: output_y,
                direction: EntityDirection::North,
                carries: carries.clone(),
                ..Default::default()
            }, // lifts group1 items up to bridge
            PlacedEntity {
                name: belt.clone(),
                x: gap_start_x + 1,
                y: output_y,
                direction: EntityDirection::East,
                carries: carries.clone(),
                ..Default::default()
            }, // through-belt filler
            PlacedEntity {
                name: belt.clone(),
                x: gap_start_x + 2,
                y: output_y,
                direction: EntityDirection::East,
                carries: carries.clone(),
                ..Default::default()
            }, // sideload target (through-belt)
        ]
    } else {
        vec![
            // Bridge row (y+5 or y+6 depending on template)
            PlacedEntity {
                name: belt.clone(),
                x: gap_start_x,
                y: bridge_y,
                direction: EntityDirection::South,
                carries: carries.clone(),
                ..Default::default()
            },
            PlacedEntity {
                name: belt.clone(),
                x: gap_start_x + 1,
                y: bridge_y,
                direction: EntityDirection::West,
                carries: carries.clone(),
                ..Default::default()
            },
            PlacedEntity {
                name: belt.clone(),
                x: gap_start_x + 2,
                y: bridge_y,
                direction: EntityDirection::West,
                carries: carries.clone(),
                ..Default::default()
            },
            // Output belt row — gap tiles
            PlacedEntity {
                name: belt.clone(),
                x: gap_start_x,
                y: output_y,
                direction: EntityDirection::West,
                carries: carries.clone(),
                ..Default::default()
            }, // sideload target (through-belt)
            PlacedEntity {
                name: belt.clone(),
                x: gap_start_x + 1,
                y: output_y,
                direction: EntityDirection::West,
                carries: carries.clone(),
                ..Default::default()
            }, // through-belt filler
            PlacedEntity {
                name: belt.clone(),
                x: gap_start_x + 2,
                y: output_y,
                direction: EntityDirection::North,
                carries: carries.clone(),
                ..Default::default()
            }, // lifts group2 items up to bridge
        ]
    }
}

/// Row for a recipe with 1 solid input.
///
/// Layout per machine (3-tile horizontal pitch, no gaps):
/// ```text
///   y+0 : input belt (EAST)
///   y+1 : input inserter (SOUTH)
///   y+2..y+4 : machine (3x3)
///   y+5 : output inserter (SOUTH)
///   y+6 : output belt (WEST -- toward bus)
/// ```
///
/// When `lane_split=true`, machines are split into two groups with a
/// sideload bridge between them so the output belt uses both lanes.
///
/// Returns `(entities, row_height)`.
pub fn single_input_row(
    recipe: &str,
    machine_entity: &str,
    machine_count: usize,
    y_offset: i32,
    x_offset: i32,
    input_item: &str,
    output_item: &str,
    input_belt: &str,
    output_belt: &str,
    lane_split: bool,
    output_east: bool,
) -> (Vec<PlacedEntity>, i32) {
    const ROW_HEIGHT: i32 = 7;
    let mut entities = Vec::new();

    let lane_split = lane_split && machine_count >= 2;
    let mxs = machine_xs(x_offset, machine_count, lane_split);
    let g1 = if lane_split { machine_count / 2 } else { machine_count };

    for &mx in &mxs {
        // Input belt (3 tiles wide, continuous with adjacent machines)
        for dx in 0..3_i32 {
            entities.push(PlacedEntity {
                name: input_belt.to_string(),
                x: mx + dx,
                y: y_offset,
                direction: EntityDirection::East,
                carries: Some(input_item.to_string()),
                ..Default::default()
            });
        }

        // Input inserter
        entities.push(PlacedEntity {
            name: "inserter".to_string(),
            x: mx + 1,
            y: y_offset + 1,
            direction: EntityDirection::South,
            carries: Some(input_item.to_string()),
            ..Default::default()
        });

        // Machine (3x3)
        entities.push(PlacedEntity {
            name: machine_entity.to_string(),
            x: mx,
            y: y_offset + 2,
            direction: EntityDirection::North,
            recipe: Some(recipe.to_string()),
            ..Default::default()
        });

        // Output inserter
        entities.push(PlacedEntity {
            name: "inserter".to_string(),
            x: mx + 1,
            y: y_offset + 5,
            direction: EntityDirection::South,
            carries: Some(output_item.to_string()),
            ..Default::default()
        });

        // Output belt (3 tiles wide)
        let out_dir = output_dir(output_east);
        for dx in 0..3_i32 {
            entities.push(PlacedEntity {
                name: output_belt.to_string(),
                x: mx + dx,
                y: y_offset + 6,
                direction: out_dir,
                carries: Some(output_item.to_string()),
                ..Default::default()
            });
        }
    }

    if lane_split {
        let gap_start_x = x_offset + g1 as i32 * MACHINE_PITCH;
        // Input belt tiles through the gap (keep items flowing to group2)
        for dx in 0..LANE_SPLIT_GAP {
            entities.push(PlacedEntity {
                name: input_belt.to_string(),
                x: gap_start_x + dx,
                y: y_offset,
                direction: EntityDirection::East,
                carries: Some(input_item.to_string()),
                ..Default::default()
            });
        }
        // Sideload bridge
        entities.extend(sideload_bridge(gap_start_x, y_offset, 6, output_belt, output_item, output_east));
    }

    (entities, ROW_HEIGHT)
}

/// Row for a recipe with 2 solid inputs.
///
/// Layout per machine (3-tile horizontal pitch, no gaps):
/// ```text
///   y+0 : input belt 1 (EAST) -- far belt
///   y+1 : input belt 2 (EAST) -- close belt
///   y+2 : long-handed inserter (picks y+0) + inserter (picks y+1)
///   y+3..y+5 : machine (3x3)
///   y+6 : output inserter (SOUTH)
///   y+7 : output belt (WEST -- toward bus)
/// ```
///
/// When `lane_split=true`, machines are split into two groups with a
/// sideload bridge between them so the output belt uses both lanes.
///
/// Returns `(entities, row_height)`.
pub fn dual_input_row(
    recipe: &str,
    machine_entity: &str,
    machine_count: usize,
    y_offset: i32,
    x_offset: i32,
    input_items: (&str, &str),
    output_item: &str,
    input_belts: (&str, &str),
    output_belt: &str,
    lane_split: bool,
    output_east: bool,
) -> (Vec<PlacedEntity>, i32) {
    const ROW_HEIGHT: i32 = 8;
    let mut entities = Vec::new();

    let (input1, input2) = input_items;
    let (belt1, belt2) = input_belts;

    let lane_split = lane_split && machine_count >= 2;
    let mxs = machine_xs(x_offset, machine_count, lane_split);
    let g1 = if lane_split { machine_count / 2 } else { machine_count };

    for &mx in &mxs {
        // Input belt 1 -- far belt
        for dx in 0..3_i32 {
            entities.push(PlacedEntity {
                name: belt1.to_string(),
                x: mx + dx,
                y: y_offset,
                direction: EntityDirection::East,
                carries: Some(input1.to_string()),
                ..Default::default()
            });
        }

        // Input belt 2 -- close belt
        for dx in 0..3_i32 {
            entities.push(PlacedEntity {
                name: belt2.to_string(),
                x: mx + dx,
                y: y_offset + 1,
                direction: EntityDirection::East,
                carries: Some(input2.to_string()),
                ..Default::default()
            });
        }

        // Long-handed inserter (picks from far belt y+0, drops into machine y+3)
        entities.push(PlacedEntity {
            name: "long-handed-inserter".to_string(),
            x: mx,
            y: y_offset + 2,
            direction: EntityDirection::South,
            carries: Some(input1.to_string()),
            ..Default::default()
        });

        // Regular inserter (picks from close belt y+1, drops into machine y+3)
        entities.push(PlacedEntity {
            name: "inserter".to_string(),
            x: mx + 2,
            y: y_offset + 2,
            direction: EntityDirection::South,
            carries: Some(input2.to_string()),
            ..Default::default()
        });

        // Machine (3x3)
        entities.push(PlacedEntity {
            name: machine_entity.to_string(),
            x: mx,
            y: y_offset + 3,
            direction: EntityDirection::North,
            recipe: Some(recipe.to_string()),
            ..Default::default()
        });

        // Output inserter
        entities.push(PlacedEntity {
            name: "inserter".to_string(),
            x: mx + 1,
            y: y_offset + 6,
            direction: EntityDirection::South,
            carries: Some(output_item.to_string()),
            ..Default::default()
        });

        // Output belt
        let out_dir = output_dir(output_east);
        for dx in 0..3_i32 {
            entities.push(PlacedEntity {
                name: output_belt.to_string(),
                x: mx + dx,
                y: y_offset + 7,
                direction: out_dir,
                carries: Some(output_item.to_string()),
                ..Default::default()
            });
        }
    }

    if lane_split {
        let gap_start_x = x_offset + g1 as i32 * MACHINE_PITCH;
        // Input belt tiles through the gap for both input belts
        for dx in 0..LANE_SPLIT_GAP {
            entities.push(PlacedEntity {
                name: belt1.to_string(),
                x: gap_start_x + dx,
                y: y_offset,
                direction: EntityDirection::East,
                carries: Some(input1.to_string()),
                ..Default::default()
            });
            entities.push(PlacedEntity {
                name: belt2.to_string(),
                x: gap_start_x + dx,
                y: y_offset + 1,
                direction: EntityDirection::East,
                carries: Some(input2.to_string()),
                ..Default::default()
            });
        }
        // Sideload bridge (output belt at y+7, bridge at y+6)
        entities.extend(sideload_bridge(gap_start_x, y_offset, 7, output_belt, output_item, output_east));
    }

    (entities, ROW_HEIGHT)
}

/// Row for a recipe with 3 solid inputs.
///
/// Layout per machine (3-tile horizontal pitch, no gaps):
/// ```text
///   y+0 : input belt 1 (EAST) -- far belt (long-handed reach)
///   y+1 : input belt 2 (EAST) -- close belt (regular reach)
///   y+2 : long-handed-inserter at mx (picks y+0) + inserter at mx+2 (picks y+1)
///   y+3..y+5 : machine (3x3)
///   y+6 : output inserter at mx+1 (SOUTH) + long-handed inserter at mx+2 (NORTH, picks y+8)
///   y+7 : output belt (WEST or EAST)
///   y+8 : input belt 3 (EAST) -- delivered from south side
/// ```
///
/// Lane splitting is not supported for 3-input rows.
///
/// Returns `(entities, row_height)`.
pub fn triple_input_row(
    recipe: &str,
    machine_entity: &str,
    machine_count: usize,
    y_offset: i32,
    x_offset: i32,
    input_items: (&str, &str, &str),
    output_item: &str,
    input_belts: (&str, &str, &str),
    output_belt: &str,
    output_east: bool,
) -> (Vec<PlacedEntity>, i32) {
    const ROW_HEIGHT: i32 = 9;
    let mut entities = Vec::new();

    let (input1, input2, input3) = input_items;
    let (belt1, belt2, belt3) = input_belts;

    for i in 0..machine_count {
        let mx = x_offset + i as i32 * MACHINE_PITCH;

        // Input belt 1 -- far belt (long-handed range)
        for dx in 0..3_i32 {
            entities.push(PlacedEntity {
                name: belt1.to_string(),
                x: mx + dx,
                y: y_offset,
                direction: EntityDirection::East,
                carries: Some(input1.to_string()),
                ..Default::default()
            });
        }

        // Input belt 2 -- close belt (regular inserter range)
        for dx in 0..3_i32 {
            entities.push(PlacedEntity {
                name: belt2.to_string(),
                x: mx + dx,
                y: y_offset + 1,
                direction: EntityDirection::East,
                carries: Some(input2.to_string()),
                ..Default::default()
            });
        }

        // Long-handed inserter: picks from y+0 (input1), drops into machine at y+3
        entities.push(PlacedEntity {
            name: "long-handed-inserter".to_string(),
            x: mx,
            y: y_offset + 2,
            direction: EntityDirection::South,
            carries: Some(input1.to_string()),
            ..Default::default()
        });

        // Regular inserter: picks from y+1 (input2), drops into machine at y+3
        entities.push(PlacedEntity {
            name: "inserter".to_string(),
            x: mx + 2,
            y: y_offset + 2,
            direction: EntityDirection::South,
            carries: Some(input2.to_string()),
            ..Default::default()
        });

        // Machine (3x3)
        entities.push(PlacedEntity {
            name: machine_entity.to_string(),
            x: mx,
            y: y_offset + 3,
            direction: EntityDirection::North,
            recipe: Some(recipe.to_string()),
            ..Default::default()
        });

        // Output inserter: picks from machine south face (y+5), drops to output belt (y+7)
        entities.push(PlacedEntity {
            name: "inserter".to_string(),
            x: mx + 1,
            y: y_offset + 6,
            direction: EntityDirection::South,
            carries: Some(output_item.to_string()),
            ..Default::default()
        });

        // Input3 long-handed inserter: picks from y+8 (input belt 3), drops to machine south (y+5)
        entities.push(PlacedEntity {
            name: "long-handed-inserter".to_string(),
            x: mx + 2,
            y: y_offset + 6,
            direction: EntityDirection::North,
            carries: Some(input3.to_string()),
            ..Default::default()
        });

        // Output belt
        let out_dir = output_dir(output_east);
        for dx in 0..3_i32 {
            entities.push(PlacedEntity {
                name: output_belt.to_string(),
                x: mx + dx,
                y: y_offset + 7,
                direction: out_dir,
                carries: Some(output_item.to_string()),
                ..Default::default()
            });
        }

        // Input belt 3 -- south-side belt (long-handed range from y+6)
        for dx in 0..3_i32 {
            entities.push(PlacedEntity {
                name: belt3.to_string(),
                x: mx + dx,
                y: y_offset + 8,
                direction: EntityDirection::East,
                carries: Some(input3.to_string()),
                ..Default::default()
            });
        }
    }

    (entities, ROW_HEIGHT)
}

/// Row for a recipe with 1 solid input + 1 fluid input.
///
/// Layout per machine (3-tile pitch, no gaps):
/// ```text
///   y+0 : solid input belt (EAST)
///   y+1 : inserter (solid) + pipe (fluid port connection)
///   y+2..y+4 : machine (3x3)
///   y+5 : output inserter (SOUTH)
///   y+6 : output belt (WEST -- toward bus)
/// ```
///
/// Returns `(entities, row_height, fluid_port_pipes)` where
/// `fluid_port_pipes` is a list of `(x, y)` for each machine's fluid port pipe.
pub fn fluid_input_row(
    recipe: &str,
    machine_entity: &str,
    machine_count: usize,
    y_offset: i32,
    x_offset: i32,
    solid_item: &str,
    fluid_item: &str,
    output_item: &str,
    input_belt: &str,
    output_belt: &str,
    output_east: bool,
) -> (Vec<PlacedEntity>, i32, Vec<(i32, i32)>) {
    const ROW_HEIGHT: i32 = 7;
    let mut entities = Vec::new();
    let port_dx = fluid_input_port_dx(machine_entity);
    let mut fluid_port_pipes = Vec::new();

    for i in 0..machine_count {
        let mx = x_offset + i as i32 * MACHINE_PITCH;

        // Solid input belt (3 tiles wide)
        for dx in 0..3_i32 {
            entities.push(PlacedEntity {
                name: input_belt.to_string(),
                x: mx + dx,
                y: y_offset,
                direction: EntityDirection::East,
                carries: Some(solid_item.to_string()),
                ..Default::default()
            });
        }

        // y+1: inserter for solid
        entities.push(PlacedEntity {
            name: "inserter".to_string(),
            x: mx + 1,
            y: y_offset + 1,
            direction: EntityDirection::South,
            carries: Some(solid_item.to_string()),
            ..Default::default()
        });

        if machine_entity == "chemical-plant" {
            // Chemical-plant: pipe-to-ground bridges port (mx) past inserter
            // to port (mx+2). The ptg_exit at mx+2 connects to next machine's
            // ptg_entry at mx+3 (adjacent), forming a chain across all machines.
            entities.push(PlacedEntity {
                name: "pipe-to-ground".to_string(),
                x: mx,
                y: y_offset + 1,
                direction: EntityDirection::East,
                io_type: Some("input".to_string()),
                carries: Some(fluid_item.to_string()),
                ..Default::default()
            });
            entities.push(PlacedEntity {
                name: "pipe-to-ground".to_string(),
                x: mx + 2,
                y: y_offset + 1,
                direction: EntityDirection::East,
                io_type: Some("output".to_string()),
                carries: Some(fluid_item.to_string()),
                ..Default::default()
            });
        } else {
            // Other machines: regular pipe at the port position
            entities.push(PlacedEntity {
                name: "pipe".to_string(),
                x: mx + port_dx,
                y: y_offset + 1,
                carries: Some(fluid_item.to_string()),
                ..Default::default()
            });
        }

        if i == 0 {
            fluid_port_pipes.push((mx, y_offset + 1));
        }

        // Machine (3x3)
        entities.push(PlacedEntity {
            name: machine_entity.to_string(),
            x: mx,
            y: y_offset + 2,
            direction: EntityDirection::North,
            recipe: Some(recipe.to_string()),
            ..Default::default()
        });

        // Output inserter
        entities.push(PlacedEntity {
            name: "inserter".to_string(),
            x: mx + 1,
            y: y_offset + 5,
            direction: EntityDirection::South,
            carries: Some(output_item.to_string()),
            ..Default::default()
        });

        // Output belt
        let out_dir = output_dir(output_east);
        for dx in 0..3_i32 {
            entities.push(PlacedEntity {
                name: output_belt.to_string(),
                x: mx + dx,
                y: y_offset + 6,
                direction: out_dir,
                carries: Some(output_item.to_string()),
                ..Default::default()
            });
        }
    }

    (entities, ROW_HEIGHT, fluid_port_pipes)
}

/// Row for a recipe with 2 solid inputs + 1 fluid input.
///
/// Fluid is delivered via a horizontal pipe header ABOVE the machine row,
/// with vertical pipe-to-ground tunnels per machine dropping fluid down to
/// the machine's fluid input port. This frees y+4 for two inserters.
///
/// Layout per machine (3-tile horizontal pitch, no gaps):
/// ```text
///   y+0 : horizontal fluid header (pipes carrying fluid_item)
///   y+1 : pipe-to-ground input at mx+port_dx (direction SOUTH)
///   y+2 : solid input belt 1 (EAST) -- far belt
///   y+3 : solid input belt 2 (EAST) -- close belt
///   y+4 : long-handed-inserter at mx+1 + inserter at mx+2 +
///           pipe-to-ground output at mx+port_dx (direction SOUTH)
///   y+5..y+7 : machine (3x3)
///   y+8 : fluid output pipes (if output_is_fluid) OR output inserter
///   y+9 : output belt (solid output only)
/// ```
///
/// Returns `(entities, row_height, fluid_input_port_pipes, fluid_output_port_pipes)`.
pub fn fluid_dual_input_row(
    recipe: &str,
    machine_entity: &str,
    machine_count: usize,
    y_offset: i32,
    x_offset: i32,
    solid_items: (&str, &str),
    fluid_item: &str,
    output_item: &str,
    output_is_fluid: bool,
    input_belts: (&str, &str),
    output_belt: &str,
    output_east: bool,
) -> (Vec<PlacedEntity>, i32, Vec<(i32, i32)>, Vec<(i32, i32)>) {
    // Fluid output occupies y+8; add a trailing empty row so sub-row
    // stacking doesn't put output pipes adjacent to the next sub-row's
    // fluid header row (which would trip pipe-isolation).
    const ROW_HEIGHT: i32 = 10;
    let mut entities = Vec::new();

    let (input1, input2) = solid_items;
    let (belt1, belt2) = input_belts;
    let port_dx = fluid_input_port_dx(machine_entity);

    let header_y = y_offset;
    let ptg_in_y = y_offset + 1;
    let belt1_y = y_offset + 2;
    let belt2_y = y_offset + 3;
    let inserter_y = y_offset + 4;
    let machine_y = y_offset + 5;
    let output_y = y_offset + 8;

    // Horizontal fluid header chain: spans x_offset .. last machine's mx+2
    let last_mx = x_offset + (machine_count as i32 - 1) * MACHINE_PITCH;
    let header_end_x = last_mx + 2;
    for x in x_offset..=header_end_x {
        entities.push(PlacedEntity {
            name: "pipe".to_string(),
            x,
            y: header_y,
            carries: Some(fluid_item.to_string()),
            ..Default::default()
        });
    }

    let mut fluid_output_port_pipes = Vec::new();

    for i in 0..machine_count {
        let mx = x_offset + i as i32 * MACHINE_PITCH;

        // Vertical PTG pair: input at y+1 tunnels SOUTH to output at y+4
        entities.push(PlacedEntity {
            name: "pipe-to-ground".to_string(),
            x: mx + port_dx,
            y: ptg_in_y,
            direction: EntityDirection::South,
            io_type: Some("input".to_string()),
            carries: Some(fluid_item.to_string()),
            ..Default::default()
        });
        entities.push(PlacedEntity {
            name: "pipe-to-ground".to_string(),
            x: mx + port_dx,
            y: inserter_y,
            direction: EntityDirection::South,
            io_type: Some("output".to_string()),
            carries: Some(fluid_item.to_string()),
            ..Default::default()
        });

        // Solid input belts (3 tiles wide each)
        for dx in 0..3_i32 {
            entities.push(PlacedEntity {
                name: belt1.to_string(),
                x: mx + dx,
                y: belt1_y,
                direction: EntityDirection::East,
                carries: Some(input1.to_string()),
                ..Default::default()
            });
            entities.push(PlacedEntity {
                name: belt2.to_string(),
                x: mx + dx,
                y: belt2_y,
                direction: EntityDirection::East,
                carries: Some(input2.to_string()),
                ..Default::default()
            });
        }

        // Inserter placement depends on which column the fluid PTG occupies.
        // port_dx == 0 (chemical-plant): PTG at mx+0, inserters at mx+1 (long) and mx+2 (regular).
        // port_dx == 1 (assembling-machine-2/3): PTG at mx+1, so move the
        //   long-handed inserter to mx+2 and the regular inserter to mx+0.
        let (long_x, reg_x) = if port_dx == 1 {
            (mx + 2, mx)
        } else {
            (mx + 1, mx + 2)
        };

        entities.push(PlacedEntity {
            name: "long-handed-inserter".to_string(),
            x: long_x,
            y: inserter_y,
            direction: EntityDirection::South,
            carries: Some(input1.to_string()),
            ..Default::default()
        });
        entities.push(PlacedEntity {
            name: "inserter".to_string(),
            x: reg_x,
            y: inserter_y,
            direction: EntityDirection::South,
            carries: Some(input2.to_string()),
            ..Default::default()
        });

        // Machine (3x3)
        entities.push(PlacedEntity {
            name: machine_entity.to_string(),
            x: mx,
            y: machine_y,
            direction: EntityDirection::North,
            recipe: Some(recipe.to_string()),
            ..Default::default()
        });

        // Output row
        if output_is_fluid {
            // Chemical-plant fluid output ports at (0,2) and (2,2) south ->
            // pipes one tile south of the machine (y=output_y) at mx+0, mx+2.
            entities.push(PlacedEntity {
                name: "pipe".to_string(),
                x: mx,
                y: output_y,
                carries: Some(output_item.to_string()),
                ..Default::default()
            });
            entities.push(PlacedEntity {
                name: "pipe".to_string(),
                x: mx + 2,
                y: output_y,
                carries: Some(output_item.to_string()),
                ..Default::default()
            });
            fluid_output_port_pipes.push((mx, output_y));
            fluid_output_port_pipes.push((mx + 2, output_y));
        } else {
            // Solid output: inserter at y+8, belt at y+9
            entities.push(PlacedEntity {
                name: "inserter".to_string(),
                x: mx + 1,
                y: output_y,
                direction: EntityDirection::South,
                carries: Some(output_item.to_string()),
                ..Default::default()
            });
            let out_dir = output_dir(output_east);
            for dx in 0..3_i32 {
                entities.push(PlacedEntity {
                    name: output_belt.to_string(),
                    x: mx + dx,
                    y: output_y + 1,
                    direction: out_dir,
                    carries: Some(output_item.to_string()),
                    ..Default::default()
                });
            }
        }
    }

    let fluid_input_port_pipes = vec![(x_offset, header_y)];

    (entities, ROW_HEIGHT, fluid_input_port_pipes, fluid_output_port_pipes)
}

/// Row for basic-oil-processing (1 fluid in, 1 fluid out, 5×5 refinery).
///
/// Refineries are placed at `direction=NORTH` with `mirror=true` so
/// crude-oil inputs sit at the NORTH edge (matching the bus trunk-above
/// pattern) and petroleum-gas outputs sit at the SOUTH edge.
///
/// ```text
///   y+0 : crude-oil input pipe (at mx+1, one per refinery)
///   y+1..y+5 : oil-refinery entity (5x5)
///   y+6 : petroleum-gas output pipe (at mx+0, one per refinery)
/// ```
///
/// Returns `(entities, row_height, fluid_input_port_pipes, fluid_output_port_pipes)`.
pub fn oil_refinery_row(
    recipe: &str,
    machine_count: usize,
    y_offset: i32,
    x_offset: i32,
    fluid_input_item: &str,
    fluid_output_item: &str,
) -> (Vec<PlacedEntity>, i32, Vec<(i32, i32)>, Vec<(i32, i32)>) {
    const ROW_HEIGHT: i32 = 7;
    let mut entities = Vec::new();
    let mut fluid_input_port_pipes = Vec::new();
    let mut fluid_output_port_pipes = Vec::new();

    for i in 0..machine_count {
        let mx = x_offset + i as i32 * OIL_REFINERY_PITCH;

        // Input port pipe (crude-oil), 1 tile north of the refinery footprint
        let input_x = mx + 1;
        let input_y = y_offset;
        entities.push(PlacedEntity {
            name: "pipe".to_string(),
            x: input_x,
            y: input_y,
            carries: Some(fluid_input_item.to_string()),
            ..Default::default()
        });
        fluid_input_port_pipes.push((input_x, input_y));

        // Refinery (5x5), mirrored so inputs face north, outputs face south
        entities.push(PlacedEntity {
            name: "oil-refinery".to_string(),
            x: mx,
            y: y_offset + 1,
            direction: EntityDirection::North,
            recipe: Some(recipe.to_string()),
            mirror: true,
            ..Default::default()
        });

        // Output port pipe (petroleum-gas), 1 tile south of the refinery footprint
        let output_y = y_offset + 6;
        entities.push(PlacedEntity {
            name: "pipe".to_string(),
            x: mx,
            y: output_y,
            carries: Some(fluid_output_item.to_string()),
            ..Default::default()
        });
        fluid_output_port_pipes.push((mx, output_y));
    }

    (entities, ROW_HEIGHT, fluid_input_port_pipes, fluid_output_port_pipes)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper: assert at least one entity at (x, y) with given name; returns the first match.
    fn assert_entity<'a>(entities: &'a [PlacedEntity], x: i32, y: i32, name: &str) -> &'a PlacedEntity {
        let found: Vec<_> = entities.iter().filter(|e| e.x == x && e.y == y).collect();
        assert!(!found.is_empty(), "No entity at ({x}, {y}), expected '{name}'");
        assert_eq!(found[0].name, name, "Wrong entity at ({x}, {y}): got '{}', expected '{name}'", found[0].name);
        found[0]
    }

    // ---- single_input_row ----

    #[test]
    fn single_input_row_basic_entity_count() {
        // 2 machines: 2*(3+1+1+1+3) = 2*9 = 18 entities (no lane split).
        let (entities, height) = single_input_row(
            "iron-gear-wheel",
            "assembling-machine-3",
            2,
            0,
            0,
            "iron-plate",
            "iron-gear-wheel",
            "transport-belt",
            "transport-belt",
            false,
            false,
        );
        assert_eq!(height, 7);
        // 2 machines × (3 input belts + 1 inserter + 1 machine + 1 output inserter + 3 output belts)
        assert_eq!(entities.len(), 2 * 9);
    }

    #[test]
    fn single_input_row_one_machine_positions() {
        let (entities, _) = single_input_row(
            "iron-gear-wheel",
            "assembling-machine-3",
            1,
            0, // y_offset
            0, // x_offset
            "iron-plate",
            "iron-gear-wheel",
            "transport-belt",
            "transport-belt",
            false,
            false,
        );

        // Input belts at y=0, x=0,1,2 facing EAST
        for dx in 0..3_i32 {
            let e = assert_entity(&entities, dx, 0, "transport-belt");
            assert_eq!(e.direction, EntityDirection::East);
            assert_eq!(e.carries.as_deref(), Some("iron-plate"));
        }

        // Inserter at (1, 1) facing SOUTH
        let ins = assert_entity(&entities, 1, 1, "inserter");
        assert_eq!(ins.direction, EntityDirection::South);
        assert_eq!(ins.carries.as_deref(), Some("iron-plate"));

        // Machine at (0, 2) facing NORTH
        let machine = assert_entity(&entities, 0, 2, "assembling-machine-3");
        assert_eq!(machine.direction, EntityDirection::North);
        assert_eq!(machine.recipe.as_deref(), Some("iron-gear-wheel"));

        // Output inserter at (1, 5) facing SOUTH
        let out_ins = assert_entity(&entities, 1, 5, "inserter");
        assert_eq!(out_ins.direction, EntityDirection::South);
        assert_eq!(out_ins.carries.as_deref(), Some("iron-gear-wheel"));

        // Output belts at y=6, x=0,1,2 facing WEST
        for dx in 0..3_i32 {
            let e = assert_entity(&entities, dx, 6, "transport-belt");
            assert_eq!(e.direction, EntityDirection::West);
            assert_eq!(e.carries.as_deref(), Some("iron-gear-wheel"));
        }
    }

    #[test]
    fn single_input_row_x_y_offset() {
        // With x_offset=6, y_offset=10, first machine should be at (6, 12).
        let (entities, _) = single_input_row(
            "iron-gear-wheel",
            "assembling-machine-3",
            1,
            10,
            6,
            "iron-plate",
            "iron-gear-wheel",
            "transport-belt",
            "transport-belt",
            false,
            false,
        );
        assert_entity(&entities, 6, 12, "assembling-machine-3");
    }

    #[test]
    fn single_input_row_output_east() {
        let (entities, _) = single_input_row(
            "iron-gear-wheel",
            "assembling-machine-3",
            1,
            0,
            0,
            "iron-plate",
            "iron-gear-wheel",
            "transport-belt",
            "transport-belt",
            false,
            true, // output_east
        );
        // Output belts at y=6 should face EAST
        for dx in 0..3_i32 {
            let e = assert_entity(&entities, dx, 6, "transport-belt");
            assert_eq!(e.direction, EntityDirection::East);
        }
    }

    #[test]
    fn single_input_row_lane_split_two_machines() {
        // 2 machines with lane_split: machines at x=0 and x=3+3=6 (g1=1, gap_start=3)
        let (entities, height) = single_input_row(
            "iron-gear-wheel",
            "assembling-machine-3",
            2,
            0,
            0,
            "iron-plate",
            "iron-gear-wheel",
            "transport-belt",
            "transport-belt",
            true, // lane_split
            false,
        );
        assert_eq!(height, 7);

        // Machine 1 at x=0
        assert_entity(&entities, 0, 2, "assembling-machine-3");
        // Machine 2 at x=6 (g1=1, gap_start=3, gap=3, so g2_start = 3+3=6)
        assert_entity(&entities, 6, 2, "assembling-machine-3");

        // Sideload bridge: 3 input belt tiles through gap at x=3,4,5 y=0
        for dx in 3..6_i32 {
            let e = assert_entity(&entities, dx, 0, "transport-belt");
            assert_eq!(e.direction, EntityDirection::East);
        }

        // Bridge entities: 6 total at gap_start_x=3
        // bridge_y = y_offset + 6 - 1 = 5
        // output_y = y_offset + 6 = 6
        // West-flowing bridge (not output_east):
        // (3, 5) SOUTH, (4, 5) WEST, (5, 5) WEST
        let b0 = assert_entity(&entities, 3, 5, "transport-belt");
        assert_eq!(b0.direction, EntityDirection::South);
        let b1 = assert_entity(&entities, 4, 5, "transport-belt");
        assert_eq!(b1.direction, EntityDirection::West);
        let b2 = assert_entity(&entities, 5, 5, "transport-belt");
        assert_eq!(b2.direction, EntityDirection::West);
        // (3, 6) WEST, (4, 6) WEST, (5, 6) NORTH
        let b3 = assert_entity(&entities, 3, 6, "transport-belt");
        assert_eq!(b3.direction, EntityDirection::West);
        let b4 = assert_entity(&entities, 4, 6, "transport-belt");
        assert_eq!(b4.direction, EntityDirection::West);
        let b5 = assert_entity(&entities, 5, 6, "transport-belt");
        assert_eq!(b5.direction, EntityDirection::North);
    }

    #[test]
    fn single_input_row_lane_split_ignored_for_one_machine() {
        // lane_split with only 1 machine should be a no-op
        let (entities_split, _) = single_input_row(
            "iron-gear-wheel",
            "assembling-machine-3",
            1,
            0,
            0,
            "iron-plate",
            "iron-gear-wheel",
            "transport-belt",
            "transport-belt",
            true,
            false,
        );
        let (entities_no_split, _) = single_input_row(
            "iron-gear-wheel",
            "assembling-machine-3",
            1,
            0,
            0,
            "iron-plate",
            "iron-gear-wheel",
            "transport-belt",
            "transport-belt",
            false,
            false,
        );
        assert_eq!(entities_split.len(), entities_no_split.len());
    }

    // ---- dual_input_row ----

    #[test]
    fn dual_input_row_basic() {
        let (entities, height) = dual_input_row(
            "electronic-circuit",
            "assembling-machine-3",
            1,
            0,
            0,
            ("copper-cable", "iron-plate"),
            "electronic-circuit",
            ("transport-belt", "transport-belt"),
            "transport-belt",
            false,
            false,
        );
        assert_eq!(height, 8);

        // Input belt 1 (far, y=0): copper-cable, EAST
        for dx in 0..3_i32 {
            let e = assert_entity(&entities, dx, 0, "transport-belt");
            assert_eq!(e.direction, EntityDirection::East);
            assert_eq!(e.carries.as_deref(), Some("copper-cable"));
        }

        // Input belt 2 (close, y=1): iron-plate, EAST
        for dx in 0..3_i32 {
            let e = assert_entity(&entities, dx, 1, "transport-belt");
            assert_eq!(e.direction, EntityDirection::East);
            assert_eq!(e.carries.as_deref(), Some("iron-plate"));
        }

        // Long-handed inserter at (0, 2) SOUTH, carries copper-cable
        let lh = assert_entity(&entities, 0, 2, "long-handed-inserter");
        assert_eq!(lh.direction, EntityDirection::South);
        assert_eq!(lh.carries.as_deref(), Some("copper-cable"));

        // Regular inserter at (2, 2) SOUTH, carries iron-plate
        let ri = assert_entity(&entities, 2, 2, "inserter");
        assert_eq!(ri.direction, EntityDirection::South);
        assert_eq!(ri.carries.as_deref(), Some("iron-plate"));

        // Machine at (0, 3)
        assert_entity(&entities, 0, 3, "assembling-machine-3");

        // Output inserter at (1, 6) SOUTH
        let oi = assert_entity(&entities, 1, 6, "inserter");
        assert_eq!(oi.direction, EntityDirection::South);

        // Output belts at y=7
        for dx in 0..3_i32 {
            let e = assert_entity(&entities, dx, 7, "transport-belt");
            assert_eq!(e.direction, EntityDirection::West);
        }
    }

    #[test]
    fn dual_input_row_lane_split_four_machines() {
        // 4 machines with lane_split: g1=2 at x=0,3; g2=2 at x=6+3=9, 9+3=12
        // gap_start_x = 0 + 2*3 = 6
        let (entities, _) = dual_input_row(
            "electronic-circuit",
            "assembling-machine-3",
            4,
            0,
            0,
            ("copper-cable", "iron-plate"),
            "electronic-circuit",
            ("transport-belt", "transport-belt"),
            "transport-belt",
            true,
            false,
        );

        // Machines in group 1: x=0, x=3
        assert_entity(&entities, 0, 3, "assembling-machine-3");
        assert_entity(&entities, 3, 3, "assembling-machine-3");
        // Machines in group 2: x=9, x=12
        assert_entity(&entities, 9, 3, "assembling-machine-3");
        assert_entity(&entities, 12, 3, "assembling-machine-3");

        // Both input belts span the gap (x=6,7,8 for y=0 and y=1)
        for dx in 6..9_i32 {
            assert_entity(&entities, dx, 0, "transport-belt");
            assert_entity(&entities, dx, 1, "transport-belt");
        }

        // Bridge at gap_start_x=6, output_row_dy=7:
        // bridge_y = 0 + 7 - 1 = 6
        // output_y = 0 + 7 = 7
        // (6, 6) SOUTH, (7, 6) WEST, (8, 6) WEST
        let b0 = assert_entity(&entities, 6, 6, "transport-belt");
        assert_eq!(b0.direction, EntityDirection::South);
        let b3 = assert_entity(&entities, 6, 7, "transport-belt");
        assert_eq!(b3.direction, EntityDirection::West);
    }

    // ---- triple_input_row ----

    #[test]
    fn triple_input_row_basic() {
        let (entities, height) = triple_input_row(
            "advanced-circuit",
            "assembling-machine-3",
            1,
            0,
            0,
            ("copper-cable", "plastic-bar", "iron-plate"),
            "advanced-circuit",
            ("transport-belt", "transport-belt", "transport-belt"),
            "transport-belt",
            false,
        );
        assert_eq!(height, 9);

        // Input belt 1 at y=0 (copper-cable)
        for dx in 0..3_i32 {
            let e = assert_entity(&entities, dx, 0, "transport-belt");
            assert_eq!(e.carries.as_deref(), Some("copper-cable"));
        }
        // Input belt 2 at y=1 (plastic-bar)
        for dx in 0..3_i32 {
            let e = assert_entity(&entities, dx, 1, "transport-belt");
            assert_eq!(e.carries.as_deref(), Some("plastic-bar"));
        }
        // Long-handed inserter at (0, 2) SOUTH
        let lh = assert_entity(&entities, 0, 2, "long-handed-inserter");
        assert_eq!(lh.direction, EntityDirection::South);
        // Regular inserter at (2, 2) SOUTH
        let ri = assert_entity(&entities, 2, 2, "inserter");
        assert_eq!(ri.direction, EntityDirection::South);
        // Machine at (0, 3)
        assert_entity(&entities, 0, 3, "assembling-machine-3");
        // Output inserter at (1, 6) SOUTH
        let oi = assert_entity(&entities, 1, 6, "inserter");
        assert_eq!(oi.direction, EntityDirection::South);
        // Long-handed inserter at (2, 6) NORTH (picks iron-plate from y+8)
        let lh3 = assert_entity(&entities, 2, 6, "long-handed-inserter");
        assert_eq!(lh3.direction, EntityDirection::North);
        assert_eq!(lh3.carries.as_deref(), Some("iron-plate"));
        // Output belt at y=7
        for dx in 0..3_i32 {
            let e = assert_entity(&entities, dx, 7, "transport-belt");
            assert_eq!(e.direction, EntityDirection::West);
        }
        // Input belt 3 at y=8 (iron-plate)
        for dx in 0..3_i32 {
            let e = assert_entity(&entities, dx, 8, "transport-belt");
            assert_eq!(e.carries.as_deref(), Some("iron-plate"));
        }
    }

    // ---- fluid_input_row ----

    #[test]
    fn fluid_input_row_chemical_plant() {
        let (entities, height, fluid_port_pipes) = fluid_input_row(
            "plastic-bar",
            "chemical-plant",
            2,
            0,
            0,
            "coal",
            "petroleum-gas",
            "plastic-bar",
            "transport-belt",
            "transport-belt",
            false,
        );
        assert_eq!(height, 7);
        assert_eq!(fluid_port_pipes, vec![(0, 1)]);

        // chemical-plant uses pipe-to-ground at x=0 and x=2 for first machine
        let ptg_in = assert_entity(&entities, 0, 1, "pipe-to-ground");
        assert_eq!(ptg_in.direction, EntityDirection::East);
        assert_eq!(ptg_in.io_type.as_deref(), Some("input"));
        let ptg_out = assert_entity(&entities, 2, 1, "pipe-to-ground");
        assert_eq!(ptg_out.direction, EntityDirection::East);
        assert_eq!(ptg_out.io_type.as_deref(), Some("output"));

        // Second machine: pipe-to-ground at x=3 (input) and x=5 (output)
        let ptg2_in = assert_entity(&entities, 3, 1, "pipe-to-ground");
        assert_eq!(ptg2_in.io_type.as_deref(), Some("input"));
        let ptg2_out = assert_entity(&entities, 5, 1, "pipe-to-ground");
        assert_eq!(ptg2_out.io_type.as_deref(), Some("output"));

        // Machines at y=2
        assert_entity(&entities, 0, 2, "chemical-plant");
        assert_entity(&entities, 3, 2, "chemical-plant");
    }

    #[test]
    fn fluid_input_row_assembling_machine() {
        // assembling-machine-2 has port_dx=1, so pipe at mx+1 (same x as inserter)
        let (entities, _, fluid_port_pipes) = fluid_input_row(
            "some-recipe",
            "assembling-machine-2",
            1,
            0,
            0,
            "solid-item",
            "fluid-item",
            "output-item",
            "transport-belt",
            "transport-belt",
            false,
        );
        assert_eq!(fluid_port_pipes, vec![(0, 1)]);

        // Pipe at (0+1, 1) = (1, 1) — note: inserter is also at (1, 1)
        let pipes: Vec<_> = entities
            .iter()
            .filter(|e| e.x == 1 && e.y == 1 && e.name == "pipe")
            .collect();
        assert_eq!(pipes.len(), 1, "Expected a pipe at (1, 1)");
        assert_eq!(pipes[0].carries.as_deref(), Some("fluid-item"));
    }

    // ---- fluid_dual_input_row ----

    #[test]
    fn fluid_dual_input_row_solid_output() {
        let (entities, height, fluid_in_ports, fluid_out_ports) = fluid_dual_input_row(
            "some-solid-recipe",
            "chemical-plant",
            2,
            0,
            0,
            ("input1", "input2"),
            "fluid",
            "output",
            false, // output_is_fluid = false
            ("transport-belt", "transport-belt"),
            "transport-belt",
            false,
        );
        assert_eq!(height, 10);
        assert_eq!(fluid_in_ports, vec![(0, 0)]);
        assert!(fluid_out_ports.is_empty());

        // Fluid header at y=0, x=0..=last_mx+2 = 0..=3+2=5
        for x in 0..=5_i32 {
            let pipe = assert_entity(&entities, x, 0, "pipe");
            assert_eq!(pipe.carries.as_deref(), Some("fluid"));
        }

        // PTG input at (0+0, 1) = (0, 1) direction SOUTH for chemical-plant (port_dx=0)
        let ptg_in = assert_entity(&entities, 0, 1, "pipe-to-ground");
        assert_eq!(ptg_in.direction, EntityDirection::South);
        assert_eq!(ptg_in.io_type.as_deref(), Some("input"));

        // PTG output at (0+0, 4) = (0, 4) direction SOUTH
        let ptg_out = assert_entity(&entities, 0, 4, "pipe-to-ground");
        assert_eq!(ptg_out.direction, EntityDirection::South);
        assert_eq!(ptg_out.io_type.as_deref(), Some("output"));

        // Solid input belt 1 at y=2
        for dx in 0..3_i32 {
            assert_entity(&entities, dx, 2, "transport-belt");
        }
        // Solid input belt 2 at y=3
        for dx in 0..3_i32 {
            assert_entity(&entities, dx, 3, "transport-belt");
        }

        // Long-handed inserter at (1, 4) for chemical-plant (port_dx=0, long_x=mx+1=1)
        let lh = assert_entity(&entities, 1, 4, "long-handed-inserter");
        assert_eq!(lh.direction, EntityDirection::South);
        // Regular inserter at (2, 4)
        assert_entity(&entities, 2, 4, "inserter");

        // Machine at (0, 5)
        assert_entity(&entities, 0, 5, "chemical-plant");

        // Solid output: inserter at (1, 8), output belt at y=9
        assert_entity(&entities, 1, 8, "inserter");
        for dx in 0..3_i32 {
            let e = assert_entity(&entities, dx, 9, "transport-belt");
            assert_eq!(e.direction, EntityDirection::West);
        }
    }

    #[test]
    fn fluid_dual_input_row_fluid_output() {
        let (entities, height, fluid_in_ports, fluid_out_ports) = fluid_dual_input_row(
            "sulfuric-acid",
            "chemical-plant",
            1,
            0,
            0,
            ("iron-plate", "sulfur"),
            "water",
            "sulfuric-acid",
            true, // output_is_fluid = true
            ("transport-belt", "transport-belt"),
            "transport-belt",
            false,
        );
        assert_eq!(height, 10);
        assert_eq!(fluid_in_ports, vec![(0, 0)]);
        // 2 output port pipes per machine
        assert_eq!(fluid_out_ports.len(), 2);
        assert!(fluid_out_ports.contains(&(0, 8)));
        assert!(fluid_out_ports.contains(&(2, 8)));

        // Output pipes at y=8, x=0 and x=2
        assert_entity(&entities, 0, 8, "pipe");
        assert_entity(&entities, 2, 8, "pipe");
    }

    #[test]
    fn fluid_dual_input_row_assembling_machine_inserter_positions() {
        // assembling-machine-2 has port_dx=1, so long_x=mx+2, reg_x=mx+0
        let (entities, _, _, _) = fluid_dual_input_row(
            "some-recipe",
            "assembling-machine-2",
            1,
            0,
            0,
            ("input1", "input2"),
            "fluid",
            "output",
            false,
            ("transport-belt", "transport-belt"),
            "transport-belt",
            false,
        );
        // long-handed inserter at (2, 4), regular at (0, 4)
        let lh = assert_entity(&entities, 2, 4, "long-handed-inserter");
        assert_eq!(lh.direction, EntityDirection::South);
        let ri = assert_entity(&entities, 0, 4, "inserter");
        assert_eq!(ri.direction, EntityDirection::South);
    }

    // ---- oil_refinery_row ----

    #[test]
    fn oil_refinery_row_one_refinery() {
        let (entities, height, fluid_in, fluid_out) = oil_refinery_row(
            "basic-oil-processing",
            1,
            0,
            0,
            "crude-oil",
            "petroleum-gas",
        );
        assert_eq!(height, 7);
        assert_eq!(fluid_in.len(), 1);
        assert_eq!(fluid_out.len(), 1);
        assert_eq!(fluid_in[0], (1, 0));
        assert_eq!(fluid_out[0], (0, 6));

        // Input pipe at (1, 0)
        let in_pipe = assert_entity(&entities, 1, 0, "pipe");
        assert_eq!(in_pipe.carries.as_deref(), Some("crude-oil"));

        // Refinery at (0, 1) NORTH mirrored
        let refinery = assert_entity(&entities, 0, 1, "oil-refinery");
        assert_eq!(refinery.direction, EntityDirection::North);
        assert!(refinery.mirror);
        assert_eq!(refinery.recipe.as_deref(), Some("basic-oil-processing"));

        // Output pipe at (0, 6)
        let out_pipe = assert_entity(&entities, 0, 6, "pipe");
        assert_eq!(out_pipe.carries.as_deref(), Some("petroleum-gas"));
    }

    #[test]
    fn oil_refinery_row_two_refineries() {
        let (entities, _, fluid_in, fluid_out) = oil_refinery_row(
            "basic-oil-processing",
            2,
            0,
            0,
            "crude-oil",
            "petroleum-gas",
        );
        assert_eq!(fluid_in.len(), 2);
        assert_eq!(fluid_out.len(), 2);

        // Second refinery at x=5 (OIL_REFINERY_PITCH=5)
        assert_entity(&entities, 5, 1, "oil-refinery");
        // Its input pipe at (6, 0)
        assert_eq!(fluid_in[1], (6, 0));
        // Its output pipe at (5, 6)
        assert_eq!(fluid_out[1], (5, 6));
    }

    // ---- machine_xs ----

    #[test]
    fn machine_xs_no_split() {
        let xs = machine_xs(0, 3, false);
        assert_eq!(xs, vec![0, 3, 6]);
    }

    #[test]
    fn machine_xs_split_four() {
        // 4 machines, lane_split: g1=2 at 0,3; g2=2 at 6+3=9, 12
        let xs = machine_xs(0, 4, true);
        assert_eq!(xs, vec![0, 3, 9, 12]);
    }

    #[test]
    fn machine_xs_split_two() {
        // 2 machines, lane_split: g1=1 at 0; g2=1 at 3+3=6
        let xs = machine_xs(0, 2, true);
        assert_eq!(xs, vec![0, 6]);
    }

    #[test]
    fn machine_xs_split_ignored_for_one() {
        let xs_split = machine_xs(0, 1, true);
        let xs_no_split = machine_xs(0, 1, false);
        assert_eq!(xs_split, xs_no_split);
    }
}
