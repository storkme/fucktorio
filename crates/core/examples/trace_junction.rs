//! CLI step-through for the junction solver.
//!
//! Runs a chosen recipe through the layout pipeline with trace
//! collection, then prints a per-cluster growth log that shows:
//!
//! - The seed tile and specs that will participate.
//! - Stamped entities near the seed (splitters, belts, UG) that could
//!   physically feed the zone.
//! - Each growth iteration: bbox, tile count, forbidden tiles,
//!   boundaries (with direction/item/external-feeder hints).
//! - Each strategy attempt per iteration, with outcome and elapsed.
//! - Each SAT invocation: the exact zone handed to the solver and the
//!   result (satisfied / UNSAT / timing / vars / clauses).
//! - Final terminal event (Solved or GrowthCapped).
//!
//! Default runs the `tier2_electronic_circuit` e2e case. Override with
//! args: `<recipe> <rate> <machine> <inputs-comma-sep>`.
//!
//! Example:
//!   cargo run --manifest-path crates/core/Cargo.toml \
//!       --example trace_junction \
//!       -- electronic-circuit 10 assembling-machine-2 iron-plate,copper-plate

use std::collections::BTreeMap;
use std::env;

use fucktorio_core::bus::layout::build_bus_layout_traced;
use fucktorio_core::solver;
use fucktorio_core::trace::{BoundarySnapshot, StampedNeighbor, TraceEvent};
use rustc_hash::FxHashSet;

fn main() {
    let args: Vec<String> = env::args().collect();
    let recipe = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "electronic-circuit".to_string());
    let rate: f64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(10.0);
    let machine = args
        .get(3)
        .cloned()
        .unwrap_or_else(|| "assembling-machine-2".to_string());
    let inputs_csv = args
        .get(4)
        .cloned()
        .unwrap_or_else(|| "iron-plate,copper-plate".to_string());
    let inputs: FxHashSet<String> = inputs_csv
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();

    eprintln!(
        "=== {} @ {:.2}/s · {} · inputs=[{}] ===\n",
        recipe,
        rate,
        machine,
        inputs_csv,
    );

    let solver_result = match solver::solve(&recipe, rate, &inputs, &machine) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("solver failed: {e}");
            std::process::exit(1);
        }
    };

    let layout = match build_bus_layout_traced(&solver_result, None) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("layout failed: {e}");
            std::process::exit(1);
        }
    };

    let events = layout.trace.as_ref().expect("traced build returns events");

    // Group junction-related events by (seed_x, seed_y). Keep events in
    // original order within each group. Terminal events (JunctionSolved
    // / JunctionGrowthCapped) close a group.
    let mut by_seed: BTreeMap<(i32, i32), Vec<&TraceEvent>> = BTreeMap::new();
    for ev in events {
        if let Some(seed) = junction_seed(ev) {
            by_seed.entry(seed).or_default().push(ev);
        }
    }

    if by_seed.is_empty() {
        eprintln!("no junction events captured — layout may have skipped the junction solver entirely");
        return;
    }

    println!("{} junction cluster(s):\n", by_seed.len());

    for ((sx, sy), cluster_events) in by_seed.iter() {
        println!("═══════════════════════════════════════════════════════════════");
        println!("  CLUSTER seeded at ({sx}, {sy})");
        println!("═══════════════════════════════════════════════════════════════");
        print_cluster(*sx, *sy, cluster_events);
        println!();
    }

    // Validation summary at the end.
    println!("═══════════════════════════════════════════════════════════════");
    println!("  VALIDATION");
    println!("═══════════════════════════════════════════════════════════════");
    if let Some(v) = layout.warnings.first() {
        println!("  ghost router warnings: {v:?}");
    }
    println!("  (validator run separately; inspect layout.entities)");
}

/// Extract the (seed_x, seed_y) key from any junction-related trace event.
fn junction_seed(ev: &TraceEvent) -> Option<(i32, i32)> {
    match ev {
        TraceEvent::JunctionGrowthStarted { seed_x, seed_y, .. }
        | TraceEvent::JunctionGrowthIteration { seed_x, seed_y, .. }
        | TraceEvent::JunctionStrategyAttempt { seed_x, seed_y, .. }
        | TraceEvent::SatInvocation { seed_x, seed_y, .. } => Some((*seed_x, *seed_y)),
        TraceEvent::JunctionSolved {
            tile_x, tile_y, ..
        }
        | TraceEvent::JunctionGrowthCapped {
            tile_x, tile_y, ..
        }
        | TraceEvent::JunctionTemplateRejected {
            tile_x, tile_y, ..
        }
        | TraceEvent::RegionWalkerVeto {
            tile_x, tile_y, ..
        } => Some((*tile_x, *tile_y)),
        _ => None,
    }
}

fn print_cluster(sx: i32, sy: i32, events: &[&TraceEvent]) {
    for ev in events {
        match ev {
            TraceEvent::JunctionGrowthStarted {
                participating,
                nearby_stamped,
                ..
            } => {
                println!("\n  ▶ start: {} participating spec(s)", participating.len());
                for p in participating {
                    println!(
                        "      • {:<40} item={:<15} seed=({},{}) frontier=[{}..{}] of {}",
                        p.key,
                        p.item,
                        p.initial_tile_x,
                        p.initial_tile_y,
                        p.initial_start,
                        p.initial_end,
                        p.path_len
                    );
                }
                print_nearby(nearby_stamped, (sx, sy));
            }
            TraceEvent::JunctionGrowthIteration {
                iter,
                bbox_x,
                bbox_y,
                bbox_w,
                bbox_h,
                tiles,
                forbidden_tiles,
                boundaries,
                participating,
                encountered,
                ..
            } => {
                println!(
                    "\n  ── iteration {iter} — bbox ({bbox_x},{bbox_y}) {bbox_w}×{bbox_h} · {} tiles · {} forbidden · {} boundaries",
                    tiles.len(),
                    forbidden_tiles.len(),
                    boundaries.len(),
                );
                println!("      participating: {}", summarize_keys(participating));
                if !encountered.is_empty() {
                    println!("      encountered:   {}", summarize_keys(encountered));
                }
                if !boundaries.is_empty() {
                    println!("      boundaries:");
                    for b in boundaries {
                        print_boundary(b);
                    }
                }
                if !forbidden_tiles.is_empty() {
                    println!(
                        "      forbidden: {}",
                        format_tile_list(forbidden_tiles, 12),
                    );
                }
                print_region_ascii(
                    *bbox_x,
                    *bbox_y,
                    *bbox_w,
                    *bbox_h,
                    tiles,
                    forbidden_tiles,
                    boundaries,
                );
            }
            TraceEvent::JunctionStrategyAttempt {
                iter,
                strategy,
                outcome,
                detail,
                elapsed_us,
                ..
            } => {
                let marker = match outcome.as_str() {
                    "Solved" => "✓",
                    "Vetoed" => "✗ veto",
                    "DeferredExit" => "⋯ defer",
                    "Unsatisfiable" => "✗ unsat",
                    _ => "?",
                };
                if detail.is_empty() {
                    println!(
                        "      strategy[{iter}] {strategy:<28} {marker:<10} ({elapsed_us}µs)"
                    );
                } else {
                    println!(
                        "      strategy[{iter}] {strategy:<28} {marker:<10} ({elapsed_us}µs)  {detail}"
                    );
                }
            }
            TraceEvent::SatInvocation {
                iter,
                zone_x,
                zone_y,
                zone_w,
                zone_h,
                boundaries,
                forced_empty,
                belt_tier,
                max_reach,
                satisfied,
                variables,
                clauses,
                solve_time_us,
                entities_raw,
                ..
            } => {
                println!(
                    "      SAT[{iter}] zone ({zone_x},{zone_y}) {zone_w}×{zone_h}  tier={belt_tier}  max_reach={max_reach}"
                );
                println!(
                    "           → satisfied={satisfied} vars={variables} clauses={clauses} solve={solve_time_us}µs entities={entities_raw}"
                );
                if !boundaries.is_empty() {
                    println!("           boundaries fed to SAT:");
                    for b in boundaries {
                        println!("           ");
                        print_boundary_indented(b, "             ");
                    }
                }
                if !forced_empty.is_empty() {
                    println!(
                        "           forced_empty: {}",
                        format_tile_list(forced_empty, 12)
                    );
                }
            }
            TraceEvent::JunctionSolved {
                strategy,
                growth_iter,
                region_tiles,
                ..
            } => {
                println!(
                    "\n  ✓ SOLVED by {strategy} at growth_iter={growth_iter}, region_tiles={region_tiles}"
                );
            }
            TraceEvent::JunctionGrowthCapped {
                iters,
                region_tiles,
                reason,
                ..
            } => {
                println!(
                    "\n  ✗ GROWTH CAPPED after {iters} iter(s), region_tiles={region_tiles}, reason={reason}"
                );
            }
            TraceEvent::JunctionTemplateRejected {
                bridge_dir, reason, ..
            } => {
                println!("      (template rejected: {bridge_dir} / {reason})");
            }
            TraceEvent::RegionWalkerVeto {
                strategy,
                growth_iter,
                broken_segment,
                break_tile_x,
                break_tile_y,
                break_count,
                ..
            } => {
                println!(
                    "      (walker veto: strategy={strategy} iter={growth_iter} breaks={break_count} first={broken_segment}@({break_tile_x},{break_tile_y}))"
                );
            }
            _ => {}
        }
    }
}

fn print_nearby(nearby: &[StampedNeighbor], _seed: (i32, i32)) {
    if nearby.is_empty() {
        return;
    }
    println!("    nearby stamped entities (±2 of seed):");
    for n in nearby {
        let marker = if n.feeds_seed_area { "→" } else { " " };
        let carries = n.carries.as_deref().unwrap_or("-");
        let seg = n.segment_id.as_deref().unwrap_or("-");
        println!(
            "      {marker} ({},{}) {:<28} {:<6} carries={:<15} seg={}",
            n.x, n.y, n.name, n.direction, carries, seg,
        );
    }
}

fn print_boundary(b: &BoundarySnapshot) {
    let io = if b.is_input { "IN " } else { "OUT" };
    let feeder_str = if let Some(f) = &b.external_feeder {
        format!(
            "  ← fed by {} at ({},{}) dir={}",
            f.entity_name, f.entity_x, f.entity_y, f.direction
        )
    } else {
        String::new()
    };
    println!(
        "        {io} ({:>3},{:>3}) dir={:<5} item={:<15} spec={}{}",
        b.x, b.y, b.direction, b.item, b.spec_key, feeder_str
    );
}

fn print_boundary_indented(b: &BoundarySnapshot, indent: &str) {
    let io = if b.is_input { "IN " } else { "OUT" };
    let feeder_str = if let Some(f) = &b.external_feeder {
        format!(
            "  ← fed by {} at ({},{}) dir={}",
            f.entity_name, f.entity_x, f.entity_y, f.direction
        )
    } else {
        String::new()
    };
    println!(
        "{indent}{io} ({:>3},{:>3}) dir={:<5} item={}{}",
        b.x, b.y, b.direction, b.item, feeder_str
    );
}

fn summarize_keys(keys: &[String]) -> String {
    if keys.len() <= 3 {
        keys.join(", ")
    } else {
        format!("{}, {}, +{} more", keys[0], keys[1], keys.len() - 2)
    }
}

fn format_tile_list(tiles: &[(i32, i32)], limit: usize) -> String {
    if tiles.len() <= limit {
        tiles
            .iter()
            .map(|(x, y)| format!("({x},{y})"))
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        let head: Vec<String> = tiles
            .iter()
            .take(limit)
            .map(|(x, y)| format!("({x},{y})"))
            .collect();
        format!("{} +{} more", head.join(" "), tiles.len() - limit)
    }
}

/// ASCII render of the current region: '·' = in region, '#' = forbidden,
/// 'I'/'O' = input/output boundary, '.' = outside region.
fn print_region_ascii(
    bx: i32,
    by: i32,
    bw: u32,
    bh: u32,
    tiles: &[(i32, i32)],
    forbidden: &[(i32, i32)],
    boundaries: &[BoundarySnapshot],
) {
    if bw == 0 || bh == 0 || bw > 40 || bh > 40 {
        return;
    }
    use std::collections::HashSet;
    let tile_set: HashSet<(i32, i32)> = tiles.iter().copied().collect();
    let forbid_set: HashSet<(i32, i32)> = forbidden.iter().copied().collect();
    let input_set: HashSet<(i32, i32)> = boundaries
        .iter()
        .filter(|b| b.is_input)
        .map(|b| (b.x, b.y))
        .collect();
    let output_set: HashSet<(i32, i32)> = boundaries
        .iter()
        .filter(|b| !b.is_input)
        .map(|b| (b.x, b.y))
        .collect();

    println!();
    // Column header.
    print!("        ");
    for dx in 0..bw as i32 {
        let x = bx + dx;
        print!("{}", (x.rem_euclid(10)).to_string().chars().next().unwrap());
    }
    println!();
    for dy in 0..bh as i32 {
        let y = by + dy;
        print!("      {:>2} ", y);
        for dx in 0..bw as i32 {
            let x = bx + dx;
            let ch = if input_set.contains(&(x, y)) {
                'I'
            } else if output_set.contains(&(x, y)) {
                'O'
            } else if forbid_set.contains(&(x, y)) {
                '#'
            } else if tile_set.contains(&(x, y)) {
                '·'
            } else {
                '.'
            };
            print!("{ch}");
        }
        println!();
    }
    println!();
}
