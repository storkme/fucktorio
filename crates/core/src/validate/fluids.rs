//! Pipe isolation and fluid port connectivity checks.
//!
//! Port of `src/validate.py` — `check_pipe_isolation` and
//! `check_fluid_port_connectivity`.

use std::collections::VecDeque;

use rustc_hash::{FxHashMap, FxHashSet};

use crate::common::DIRECTIONS;
use crate::models::{EntityDirection, LayoutResult, PlacedEntity};
use crate::recipe_db;

use super::{LayoutStyle, Severity, ValidationIssue};

// ---------------------------------------------------------------------------
// Entity-set constants (mirrors Python's module-level sets)
// ---------------------------------------------------------------------------

const PIPE_ENTITIES: &[&str] = &["pipe", "pipe-to-ground"];
const MACHINE_ENTITIES: &[&str] = &[
    "assembling-machine-1",
    "assembling-machine-2",
    "assembling-machine-3",
    "chemical-plant",
    "electric-furnace",
    "oil-refinery",
];

// ---------------------------------------------------------------------------
// Fluid port data (pre-computed from draftsman entity data)
//
// Positions are relative to the machine's top-left tile.
// Format: (rel_x, rel_y, production_type)  where production_type is "input" | "output"
// ---------------------------------------------------------------------------

/// Static fluid port data for each machine entity, mirroring `_get_fluid_ports` from Python.
///
/// These values were computed by calling `_get_fluid_ports(name)` in the
/// Python reference implementation.  Formula:
///   center = size // 2
///   port_x = rel_x + center
///   pipe_y = (rel_y + center) ± 1  (−1 for north-facing port, +1 for south)
fn fluid_ports(entity_name: &str, mirror: bool) -> &'static [(i32, i32, &'static str)] {
    // assembling-machine-2 and 3: 3x3, center=1
    //   input  pos=[0,-1] dir=0(north)  → port(1,0)  pipe y=0-1=-1  → (1,-1)
    //   output pos=[0, 1] dir=8(south)  → port(1,2)  pipe y=2+1= 3  → (1, 3)
    const AM2: &[(i32, i32, &str)] = &[(1, -1, "input"), (1, 3, "output")];

    // chemical-plant: 3x3, center=1
    //   input  pos=[-1,-1] dir=0 → port(0,0) pipe y=-1 → (0,-1)
    //   input  pos=[ 1,-1] dir=0 → port(2,0) pipe y=-1 → (2,-1)
    //   output pos=[-1, 1] dir=8 → port(0,2) pipe y= 3 → (0, 3)
    //   output pos=[ 1, 1] dir=8 → port(2,2) pipe y= 3 → (2, 3)
    const CHEM: &[(i32, i32, &str)] = &[
        (0, -1, "input"),
        (2, -1, "input"),
        (0, 3, "output"),
        (2, 3, "output"),
    ];

    // oil-refinery: 5x5, center=2
    //   input  pos=[-1, 2] dir=8(south) → port(1,4) pipe y=5 → (1, 5)
    //   input  pos=[ 1, 2] dir=8(south) → port(3,4) pipe y=5 → (3, 5)
    //   output pos=[-2,-2] dir=0(north) → port(0,0) pipe y=-1 → (0,-1)
    //   output pos=[ 0,-2] dir=0(north) → port(2,0) pipe y=-1 → (2,-1)
    //   output pos=[ 2,-2] dir=0(north) → port(4,0) pipe y=-1 → (4,-1)
    //
    // mirror=true flips inputs↔outputs and swaps their y positions:
    //   input  (1,-1), (3,-1)
    //   output (0, 5), (2, 5), (4, 5)
    const OIL: &[(i32, i32, &str)] = &[
        (1, 5, "input"),
        (3, 5, "input"),
        (0, -1, "output"),
        (2, -1, "output"),
        (4, -1, "output"),
    ];
    const OIL_MIRROR: &[(i32, i32, &str)] = &[
        (1, -1, "input"),
        (3, -1, "input"),
        (0, 5, "output"),
        (2, 5, "output"),
        (4, 5, "output"),
    ];

    match entity_name {
        "assembling-machine-2" | "assembling-machine-3" => AM2,
        "chemical-plant" => CHEM,
        // oil-refinery is the only entity where mirror flips port y-positions
        "oil-refinery" => {
            if mirror { OIL_MIRROR } else { OIL }
        }
        _ => &[],
    }
}

// ---------------------------------------------------------------------------
// check_pipe_isolation
// ---------------------------------------------------------------------------

/// For a pipe-to-ground entity, return the single surface-side neighbour tile.
///
/// Input PTGs expose their surface on the side *opposite* their flow direction
/// (fluid enters from behind).  Output PTGs expose it on the *same* side as
/// their flow direction (fluid exits ahead).
fn ptg_surface_neighbour(
    x: i32,
    y: i32,
    direction: EntityDirection,
    io_type: Option<&str>,
) -> (i32, i32) {
    let (dx, dy) = match direction {
        EntityDirection::North => (0i32, -1i32),
        EntityDirection::East => (1, 0),
        EntityDirection::South => (0, 1),
        EntityDirection::West => (-1, 0),
    };
    // For inputs, the surface side is *behind* the flow → negate delta
    let (dx, dy) = if io_type == Some("input") {
        (-dx, -dy)
    } else {
        (dx, dy)
    };
    (x + dx, y + dy)
}

/// Check that adjacent pipes don't carry different fluids.
///
/// In Factorio, adjacent pipes automatically connect and merge their fluid
/// networks.  Two pipes carrying different fluids must not be connected on
/// the surface.
pub fn check_pipe_isolation(layout_result: &LayoutResult) -> Vec<ValidationIssue> {
    type PipeEntry<'a> = (Option<&'a str>, &'a str, EntityDirection, Option<&'a str>);
    let mut pipe_map: FxHashMap<(i32, i32), PipeEntry<'_>> = FxHashMap::default();

    for e in &layout_result.entities {
        if PIPE_ENTITIES.contains(&e.name.as_str()) {
            pipe_map.insert(
                (e.x, e.y),
                (
                    e.carries.as_deref(),
                    e.name.as_str(),
                    e.direction,
                    e.io_type.as_deref(),
                ),
            );
        }
    }

    let mut issues = Vec::new();
    // Canonical pairs prevent double-reporting the same edge.
    let mut checked: FxHashSet<((i32, i32), (i32, i32))> = FxHashSet::default();

    for (&(px, py), &(carries, name, direction, io_type)) in &pipe_map {
        let carries = match carries {
            Some(c) => c,
            None => continue,
        };

        // Determine which neighbours to check: PTGs expose only one surface
        // side; regular pipes connect on all four sides.
        let ptg_nb;
        let neighbours: &[(i32, i32)] = if name == "pipe-to-ground" {
            ptg_nb = [ptg_surface_neighbour(px, py, direction, io_type)];
            &ptg_nb
        } else {
            &[(px + 1, py), (px - 1, py), (px, py + 1), (px, py - 1)]
        };

        for &nb in neighbours {
            let Some(&(nb_carries, nb_name, nb_direction, nb_io)) = pipe_map.get(&nb) else {
                continue;
            };
            let nb_carries = match nb_carries {
                Some(c) => c,
                None => continue,
            };

            // If neighbour is a PTG, its surface side must face back at us
            if nb_name == "pipe-to-ground" {
                let nb_surface = ptg_surface_neighbour(nb.0, nb.1, nb_direction, nb_io);
                if nb_surface != (px, py) {
                    continue;
                }
            }

            // Canonical pair to avoid double-reporting
            let pair = if (px, py) <= nb { ((px, py), nb) } else { (nb, (px, py)) };
            if !checked.insert(pair) {
                continue;
            }

            if nb_carries != carries {
                issues.push(ValidationIssue::with_pos(
                    Severity::Error,
                    "pipe-isolation",
                    format!(
                        "Adjacent pipes carry different fluids: ({px},{py}) carries {carries}, \
                         ({},{}) carries {nb_carries}",
                        nb.0, nb.1
                    ),
                    px,
                    py,
                ));
            }
        }
    }

    issues
}

// ---------------------------------------------------------------------------
// Helpers for check_fluid_port_connectivity
// ---------------------------------------------------------------------------

/// Find pipe-to-ground pairs: returns a bidirectional map `pos_a ↔ pos_b`.
///
/// PTGs travelling in the same direction along the same axis are paired:
/// each input is matched with the nearest downstream output.
fn find_ptg_pairs(layout_result: &LayoutResult) -> FxHashMap<(i32, i32), (i32, i32)> {
    // Group PTGs by (direction, axis_value).
    // EAST/WEST flow → axis is y; NORTH/SOUTH flow → axis is x.
    type GroupKey = (u8, i32); // (direction as u8, axis)
    let mut groups: FxHashMap<GroupKey, Vec<&PlacedEntity>> = FxHashMap::default();

    for e in &layout_result.entities {
        if e.name != "pipe-to-ground" {
            continue;
        }
        let axis = match e.direction {
            EntityDirection::East | EntityDirection::West => e.y,
            EntityDirection::North | EntityDirection::South => e.x,
        };
        groups
            .entry((e.direction as u8, axis))
            .or_default()
            .push(e);
    }

    let mut pairs: FxHashMap<(i32, i32), (i32, i32)> = FxHashMap::default();

    for group in groups.values() {
        let mut inputs: Vec<&PlacedEntity> = group
            .iter()
            .copied()
            .filter(|e| e.io_type.as_deref() == Some("input"))
            .collect();
        let mut outputs: Vec<&PlacedEntity> = group
            .iter()
            .copied()
            .filter(|e| e.io_type.as_deref() == Some("output"))
            .collect();

        inputs.sort_by_key(|e| (e.x, e.y));
        outputs.sort_by_key(|e| (e.x, e.y));

        let mut remaining = outputs;

        for inp in &inputs {
            let dir = inp.direction;
            let matched_idx = remaining.iter().position(|out| {
                match dir {
                    EntityDirection::East => out.x > inp.x,
                    EntityDirection::West => out.x < inp.x,
                    EntityDirection::South => out.y > inp.y,
                    EntityDirection::North => out.y < inp.y,
                }
            });
            if let Some(idx) = matched_idx {
                let out = remaining.remove(idx);
                let a = (inp.x, inp.y);
                let b = (out.x, out.y);
                pairs.insert(a, b);
                pairs.insert(b, a);
            }
        }
    }

    pairs
}

/// BFS flood-fill through adjacent pipe tiles from `start`.
///
/// Also traverses pipe-to-ground tunnel connections via `ptg_pairs`.
fn bfs_pipe_reach(
    start: (i32, i32),
    pipe_tiles: &FxHashSet<(i32, i32)>,
    ptg_pairs: &FxHashMap<(i32, i32), (i32, i32)>,
) -> FxHashSet<(i32, i32)> {
    let mut visited: FxHashSet<(i32, i32)> = FxHashSet::default();
    let mut queue: VecDeque<(i32, i32)> = VecDeque::new();
    visited.insert(start);
    queue.push_back(start);

    while let Some((x, y)) = queue.pop_front() {
        // Adjacent tile connections
        for (dx, dy) in DIRECTIONS {
            let nb = (x + dx, y + dy);
            if pipe_tiles.contains(&nb) && visited.insert(nb) {
                queue.push_back(nb);
            }
        }
        // Pipe-to-ground tunnel jump
        if let Some(&other) = ptg_pairs.get(&(x, y)) {
            if visited.insert(other) {
                queue.push_back(other);
            }
        }
    }

    visited
}

/// Return `true` if `recipe_name` produces at least one fluid product.
fn recipe_has_fluid_output(recipe_name: &str) -> bool {
    if let Some(recipe) = recipe_db::db().recipes.get(recipe_name) {
        recipe.products.iter().any(|p| p.type_ == "fluid")
    } else {
        false
    }
}

// ---------------------------------------------------------------------------
// check_fluid_port_connectivity
// ---------------------------------------------------------------------------

/// Check that every machine's fluid ports have connected pipes.
///
/// For each machine with fluid ports, verifies:
/// 1. At least one input port has an adjacent pipe.
/// 2. (`Bus` style only) At least one input pipe is reachable from the bus
///    via BFS.
/// 3. At least one output port has an adjacent pipe (only if the recipe
///    actually produces a fluid).
pub fn check_fluid_port_connectivity(
    layout_result: &LayoutResult,
    layout_style: LayoutStyle,
) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    // Build pipe tile set
    let pipe_tiles: FxHashSet<(i32, i32)> = layout_result
        .entities
        .iter()
        .filter(|e| PIPE_ENTITIES.contains(&e.name.as_str()))
        .map(|e| (e.x, e.y))
        .collect();

    // Build PTG pair map for tunnel traversal
    let ptg_pairs = find_ptg_pairs(layout_result);

    // Find bus pipe positions (pipes west of the leftmost machine).
    // Only needed for Bus-mode connectivity checks.
    let bus_pipes: FxHashSet<(i32, i32)> = if layout_style == LayoutStyle::Bus && !pipe_tiles.is_empty() {
        let leftmost_machine_x = layout_result
            .entities
            .iter()
            .filter(|e| MACHINE_ENTITIES.contains(&e.name.as_str()))
            .map(|e| e.x)
            .min();

        if let Some(leftmost) = leftmost_machine_x {
            let west_pipes: FxHashSet<_> =
                pipe_tiles.iter().copied().filter(|(x, _)| *x < leftmost).collect();
            if !west_pipes.is_empty() {
                west_pipes
            } else {
                // Fallback: leftmost pipe column
                let min_x = pipe_tiles.iter().map(|(x, _)| *x).min().unwrap();
                pipe_tiles.iter().copied().filter(|(x, _)| *x == min_x).collect()
            }
        } else {
            // No machines — fallback to leftmost column
            let min_x = pipe_tiles.iter().map(|(x, _)| *x).min().unwrap();
            pipe_tiles.iter().copied().filter(|(x, _)| *x == min_x).collect()
        }
    } else {
        FxHashSet::default()
    };

    for e in &layout_result.entities {
        if !MACHINE_ENTITIES.contains(&e.name.as_str()) {
            continue;
        }
        let recipe = match &e.recipe {
            Some(r) => r.as_str(),
            None => continue,
        };

        let ports = fluid_ports(e.name.as_str(), e.mirror);
        if ports.is_empty() {
            continue;
        }

        // assembling-machine-{2,3}: fluid boxes are disabled when no fluid
        // recipe is assigned — skip if no pipes adjacent to any port.
        if e.name == "assembling-machine-2" || e.name == "assembling-machine-3" {
            let has_any_pipe = ports
                .iter()
                .any(|(rx, ry, _)| pipe_tiles.contains(&(e.x + rx, e.y + ry)));
            if !has_any_pipe {
                continue;
            }
        }

        let input_ports: Vec<(i32, i32)> = ports
            .iter()
            .filter(|(_, _, pt)| *pt == "input")
            .map(|(rx, ry, _)| (e.x + rx, e.y + ry))
            .collect();
        let output_ports: Vec<(i32, i32)> = ports
            .iter()
            .filter(|(_, _, pt)| *pt == "output")
            .map(|(rx, ry, _)| (e.x + rx, e.y + ry))
            .collect();

        // --- Input port checks ---
        if !input_ports.is_empty() {
            let input_pipe_positions: Vec<(i32, i32)> = input_ports
                .iter()
                .copied()
                .filter(|pos| pipe_tiles.contains(pos))
                .collect();

            if input_pipe_positions.is_empty() {
                issues.push(ValidationIssue::with_pos(
                    Severity::Error,
                    "fluid-connectivity",
                    format!(
                        "{} at ({},{}): no input port has an adjacent pipe",
                        e.name, e.x, e.y
                    ),
                    e.x,
                    e.y,
                ));
            } else if layout_style == LayoutStyle::Bus && !bus_pipes.is_empty() {
                // Check at least one input pipe connects to the bus via BFS
                let any_connected = input_pipe_positions.iter().any(|&pos| {
                    !bfs_pipe_reach(pos, &pipe_tiles, &ptg_pairs)
                        .is_disjoint(&bus_pipes)
                });
                if !any_connected {
                    issues.push(ValidationIssue::with_pos(
                        Severity::Error,
                        "fluid-connectivity",
                        format!(
                            "{} at ({},{}): input pipes not connected to bus",
                            e.name, e.x, e.y
                        ),
                        e.x,
                        e.y,
                    ));
                }
            }
        }

        // --- Output port checks ---
        if !output_ports.is_empty() && recipe_has_fluid_output(recipe) {
            let has_output_pipe = output_ports
                .iter()
                .any(|pos| pipe_tiles.contains(pos));
            if !has_output_pipe {
                issues.push(ValidationIssue::with_pos(
                    Severity::Error,
                    "fluid-connectivity",
                    format!(
                        "{} at ({},{}): no output port has an adjacent pipe",
                        e.name, e.x, e.y
                    ),
                    e.x,
                    e.y,
                ));
            }
        }
    }

    issues
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{EntityDirection, LayoutResult, PlacedEntity};

    fn pipe(x: i32, y: i32, carries: Option<&str>) -> PlacedEntity {
        PlacedEntity {
            name: "pipe".to_string(),
            x,
            y,
            direction: EntityDirection::North,
            carries: carries.map(|s| s.to_string()),
            ..Default::default()
        }
    }

    fn ptg(
        x: i32,
        y: i32,
        dir: EntityDirection,
        io_type: &str,
        carries: Option<&str>,
    ) -> PlacedEntity {
        PlacedEntity {
            name: "pipe-to-ground".to_string(),
            x,
            y,
            direction: dir,
            io_type: Some(io_type.to_string()),
            carries: carries.map(|s| s.to_string()),
            ..Default::default()
        }
    }

    fn machine(name: &str, x: i32, y: i32, recipe: &str, mirror: bool) -> PlacedEntity {
        PlacedEntity {
            name: name.to_string(),
            x,
            y,
            recipe: Some(recipe.to_string()),
            mirror,
            ..Default::default()
        }
    }

    fn layout(entities: Vec<PlacedEntity>) -> LayoutResult {
        LayoutResult { entities, width: 20, height: 20 }
    }

    // === check_pipe_isolation ===

    #[test]
    fn same_fluid_adjacent_ok() {
        let lr = layout(vec![
            pipe(0, 0, Some("water")),
            pipe(1, 0, Some("water")),
            pipe(2, 0, Some("water")),
        ]);
        assert!(check_pipe_isolation(&lr).is_empty());
    }

    #[test]
    fn different_fluid_adjacent_error() {
        let lr = layout(vec![
            pipe(0, 0, Some("water")),
            pipe(1, 0, Some("crude-oil")),
        ]);
        let issues = check_pipe_isolation(&lr);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].severity, Severity::Error);
        assert_eq!(issues[0].category, "pipe-isolation");
    }

    #[test]
    fn diagonal_pipes_ok() {
        let lr = layout(vec![
            pipe(0, 0, Some("water")),
            pipe(1, 1, Some("crude-oil")),
        ]);
        assert!(check_pipe_isolation(&lr).is_empty());
    }

    #[test]
    fn untagged_pipes_ignored() {
        let lr = layout(vec![
            pipe(0, 0, Some("water")),
            pipe(1, 0, None),
        ]);
        assert!(check_pipe_isolation(&lr).is_empty());
    }

    #[test]
    fn separated_pipes_ok() {
        let lr = layout(vec![
            pipe(0, 0, Some("water")),
            pipe(2, 0, Some("crude-oil")),
        ]);
        assert!(check_pipe_isolation(&lr).is_empty());
    }

    #[test]
    fn different_fluid_reported_once_not_twice() {
        let lr = layout(vec![
            pipe(0, 0, Some("water")),
            pipe(1, 0, Some("petroleum-gas")),
        ]);
        assert_eq!(check_pipe_isolation(&lr).len(), 1);
    }

    #[test]
    fn ptg_input_surface_neighbour_check() {
        // PTG input facing EAST: surface side is WEST (behind direction)
        // So ptg at (3,0) facing EAST io=input → surface neighbour is (2,0)
        // pipe at (2,0) carries water, ptg carries crude-oil → isolation error
        let lr = layout(vec![
            pipe(2, 0, Some("water")),
            ptg(3, 0, EntityDirection::East, "input", Some("crude-oil")),
        ]);
        let issues = check_pipe_isolation(&lr);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].category, "pipe-isolation");
    }

    #[test]
    fn ptg_wrong_side_not_checked() {
        // PTG input at (3,0) facing EAST: only connects to (2,0)
        // pipe at (4,0) is on the wrong side → not connected → no error
        let lr = layout(vec![
            pipe(4, 0, Some("crude-oil")),
            ptg(3, 0, EntityDirection::East, "input", Some("water")),
        ]);
        assert!(check_pipe_isolation(&lr).is_empty());
    }

    // === check_fluid_port_connectivity ===

    #[test]
    fn no_fluid_machines_no_issues() {
        let lr = layout(vec![
            machine("assembling-machine-1", 0, 0, "iron-gear-wheel", false),
        ]);
        let issues = check_fluid_port_connectivity(&lr, LayoutStyle::Spaghetti);
        assert!(issues.is_empty());
    }

    #[test]
    fn assembling_machine_no_pipes_skipped() {
        // assembling-machine-2 without adjacent pipes → skipped (fluid_boxes_off)
        let lr = layout(vec![
            machine("assembling-machine-2", 0, 0, "iron-gear-wheel", false),
        ]);
        let issues = check_fluid_port_connectivity(&lr, LayoutStyle::Spaghetti);
        assert!(issues.is_empty());
    }

    #[test]
    fn chemical_plant_no_input_pipe_error() {
        // chemical-plant at (0,0): input ports at (0,-1) and (2,-1)
        // No pipes placed → should error
        let lr = layout(vec![
            machine("chemical-plant", 0, 0, "plastic-bar", false),
        ]);
        let issues = check_fluid_port_connectivity(&lr, LayoutStyle::Spaghetti);
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        assert!(!errors.is_empty(), "expected fluid-connectivity error");
        assert!(errors.iter().all(|i| i.category == "fluid-connectivity"));
    }

    #[test]
    fn chemical_plant_with_input_pipe_ok_spaghetti() {
        // plastic-bar has no fluid output so only input check applies
        // chemical-plant at (0,0): input port at (0,-1)
        let lr = layout(vec![
            machine("chemical-plant", 0, 0, "plastic-bar", false),
            pipe(0, -1, Some("petroleum-gas")),
        ]);
        let issues = check_fluid_port_connectivity(&lr, LayoutStyle::Spaghetti);
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        assert!(errors.is_empty(), "unexpected errors: {:?}", errors);
    }

    #[test]
    fn oil_refinery_fluid_output_needs_pipe() {
        // basic-oil-processing produces fluid outputs
        // oil-refinery at (0,0): output ports at (0,-1),(2,-1),(4,-1)
        // Place input pipes but no output pipe → should error on output
        let lr = layout(vec![
            machine("oil-refinery", 0, 0, "basic-oil-processing", false),
            pipe(1, 5, Some("crude-oil")),  // input port 1
        ]);
        let issues = check_fluid_port_connectivity(&lr, LayoutStyle::Spaghetti);
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        // Should have output-pipe-missing error
        assert!(!errors.is_empty());
        assert!(errors.iter().any(|i| i.message.contains("output port")));
    }

    #[test]
    fn oil_refinery_mirror_ports_flipped() {
        // mirror=true: input ports move to (1,-1),(3,-1); outputs to (0,5),(2,5),(4,5)
        let lr = layout(vec![
            machine("oil-refinery", 0, 0, "basic-oil-processing", true),
            pipe(1, -1, Some("crude-oil")), // input port with mirror
        ]);
        let issues = check_fluid_port_connectivity(&lr, LayoutStyle::Spaghetti);
        // With mirror, input at (1,-1) should be adjacent → only output error remains
        let input_errors: Vec<_> = issues
            .iter()
            .filter(|i| i.message.contains("input") && i.severity == Severity::Error)
            .collect();
        assert!(input_errors.is_empty(), "unexpected input errors with mirror: {:?}", input_errors);
    }

    #[test]
    fn bus_mode_input_pipe_not_connected_to_bus_error() {
        // Bus mode: machine at x=5, bus pipe at x=0
        // Machine's input pipe at (5, 3) but not connected to bus
        let lr = layout(vec![
            machine("chemical-plant", 5, 4, "plastic-bar", false),
            // Input port at (5+0, 4-1) = (5,3)
            pipe(5, 3, Some("petroleum-gas")), // adjacent but not connected to bus
            // Bus pipe far to the left
            pipe(0, 3, Some("petroleum-gas")),
        ]);
        let issues = check_fluid_port_connectivity(&lr, LayoutStyle::Bus);
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        assert!(!errors.is_empty(), "expected bus connectivity error");
        assert!(errors.iter().any(|i| i.message.contains("not connected to bus")));
    }

    #[test]
    fn bus_mode_input_pipe_connected_via_ptg_to_bus_ok() {
        // Bus mode: machine at x=5, bus pipe at x=0
        // PTG tunnel bridges the gap
        let lr = layout(vec![
            machine("chemical-plant", 5, 4, "plastic-bar", false),
            // Input port at (5,3)
            pipe(5, 3, Some("petroleum-gas")),
            // PTG tunnel from x=4 to x=1 (WEST direction)
            ptg(4, 3, EntityDirection::West, "input", Some("petroleum-gas")),
            ptg(1, 3, EntityDirection::West, "output", Some("petroleum-gas")),
            // Bus pipe
            pipe(0, 3, Some("petroleum-gas")),
        ]);
        // Connect the chain: (5,3)-(4,3) adjacent, ptg tunnel (4,3)-(1,3), (1,3)-(0,3) adjacent
        let issues = check_fluid_port_connectivity(&lr, LayoutStyle::Bus);
        let errors: Vec<_> = issues.iter().filter(|i| i.severity == Severity::Error).collect();
        assert!(errors.is_empty(), "unexpected errors: {:?}", errors);
    }

    // === find_ptg_pairs helper ===

    #[test]
    fn ptg_pairs_east_direction() {
        let lr = layout(vec![
            ptg(0, 0, EntityDirection::East, "input", None),
            ptg(3, 0, EntityDirection::East, "output", None),
        ]);
        let pairs = find_ptg_pairs(&lr);
        assert_eq!(pairs.get(&(0, 0)), Some(&(3, 0)));
        assert_eq!(pairs.get(&(3, 0)), Some(&(0, 0)));
    }

    #[test]
    fn ptg_pairs_north_direction() {
        let lr = layout(vec![
            ptg(0, 3, EntityDirection::North, "input", None),
            ptg(0, 0, EntityDirection::North, "output", None),
        ]);
        let pairs = find_ptg_pairs(&lr);
        assert_eq!(pairs.get(&(0, 3)), Some(&(0, 0)));
        assert_eq!(pairs.get(&(0, 0)), Some(&(0, 3)));
    }

    #[test]
    fn ptg_pairs_wrong_direction_not_paired() {
        // Output is behind the input (EAST flow) → no pairing
        let lr = layout(vec![
            ptg(3, 0, EntityDirection::East, "input", None),
            ptg(0, 0, EntityDirection::East, "output", None),
        ]);
        let pairs = find_ptg_pairs(&lr);
        assert!(pairs.is_empty());
    }

    // === bfs_pipe_reach ===

    #[test]
    fn bfs_reaches_adjacent_tiles() {
        let tiles: FxHashSet<(i32, i32)> =
            [(0, 0), (1, 0), (2, 0)].iter().copied().collect();
        let ptg: FxHashMap<(i32, i32), (i32, i32)> = FxHashMap::default();
        let reached = bfs_pipe_reach((0, 0), &tiles, &ptg);
        assert!(reached.contains(&(0, 0)));
        assert!(reached.contains(&(2, 0)));
    }

    #[test]
    fn bfs_traverses_ptg_tunnel() {
        let tiles: FxHashSet<(i32, i32)> =
            [(0, 0), (1, 0), (5, 0), (6, 0)].iter().copied().collect();
        let mut ptg: FxHashMap<(i32, i32), (i32, i32)> = FxHashMap::default();
        ptg.insert((1, 0), (5, 0));
        ptg.insert((5, 0), (1, 0));
        let reached = bfs_pipe_reach((0, 0), &tiles, &ptg);
        assert!(reached.contains(&(6, 0)));
    }

    // === recipe_has_fluid_output ===

    #[test]
    fn plastic_bar_has_no_fluid_output() {
        assert!(!recipe_has_fluid_output("plastic-bar"));
    }

    #[test]
    fn basic_oil_processing_has_fluid_output() {
        assert!(recipe_has_fluid_output("basic-oil-processing"));
    }

    #[test]
    fn unknown_recipe_has_no_fluid_output() {
        assert!(!recipe_has_fluid_output("nonexistent-recipe"));
    }

    // === fluid_ports static data ===

    #[test]
    fn fluid_ports_assembling_machine_2() {
        let ports = fluid_ports("assembling-machine-2", false);
        assert_eq!(ports.len(), 2);
        assert!(ports.iter().any(|&(x, y, t)| x == 1 && y == -1 && t == "input"));
        assert!(ports.iter().any(|&(x, y, t)| x == 1 && y == 3 && t == "output"));
    }

    #[test]
    fn fluid_ports_chemical_plant() {
        let ports = fluid_ports("chemical-plant", false);
        assert_eq!(ports.len(), 4);
        let inputs: Vec<_> = ports.iter().filter(|(_, _, t)| *t == "input").collect();
        let outputs: Vec<_> = ports.iter().filter(|(_, _, t)| *t == "output").collect();
        assert_eq!(inputs.len(), 2);
        assert_eq!(outputs.len(), 2);
    }

    #[test]
    fn fluid_ports_oil_refinery_normal() {
        let ports = fluid_ports("oil-refinery", false);
        assert_eq!(ports.len(), 5);
        let inputs: Vec<_> = ports.iter().filter(|(_, _, t)| *t == "input").collect();
        let outputs: Vec<_> = ports.iter().filter(|(_, _, t)| *t == "output").collect();
        assert_eq!(inputs.len(), 2);
        assert_eq!(outputs.len(), 3);
        // Inputs are at y=5 (south side)
        assert!(inputs.iter().all(|(_, y, _)| *y == 5));
        // Outputs are at y=-1 (north side)
        assert!(outputs.iter().all(|(_, y, _)| *y == -1));
    }

    #[test]
    fn fluid_ports_oil_refinery_mirror() {
        let ports = fluid_ports("oil-refinery", true);
        assert_eq!(ports.len(), 5);
        let inputs: Vec<_> = ports.iter().filter(|(_, _, t)| *t == "input").collect();
        let outputs: Vec<_> = ports.iter().filter(|(_, _, t)| *t == "output").collect();
        // With mirror: inputs flip to y=-1, outputs to y=5
        assert!(inputs.iter().all(|(_, y, _)| *y == -1));
        assert!(outputs.iter().all(|(_, y, _)| *y == 5));
    }

    #[test]
    fn fluid_ports_assembling_machine_1_empty() {
        // am-1 has no fluid ports
        assert!(fluid_ports("assembling-machine-1", false).is_empty());
    }
}
