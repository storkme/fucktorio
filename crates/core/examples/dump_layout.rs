//! Debug tool: dump a bus layout as entity counts + ASCII map.
//!
//! Usage: cargo run -p fucktorio_core --example dump_layout [recipe] [rate]
//! Default: iron-gear-wheel 10.0

use fucktorio_core::bus::layout::build_bus_layout;
use fucktorio_core::models::LayoutResult;
use fucktorio_core::solver;
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::BTreeMap;
use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    let recipe = args.get(1).cloned().unwrap_or_else(|| "iron-gear-wheel".to_string());
    let rate: f64 = args
        .get(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(10.0);

    println!("=== {} @ {:.2}/s ===\n", recipe, rate);

    let solver_result = match solver::solve(&recipe, rate, &FxHashSet::default(), "assembling-machine-3") {
        Ok(r) => r,
        Err(e) => {
            eprintln!("solver failed: {}", e);
            std::process::exit(1);
        }
    };

    println!("Solver: {} machines, {} external inputs", solver_result.machines.len(), solver_result.external_inputs.len());
    for m in &solver_result.machines {
        println!("  {} × {} ({})", m.recipe, m.count.ceil() as i32, m.entity);
    }
    println!();

    let layout = match build_bus_layout(&solver_result, None) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("layout failed: {}", e);
            std::process::exit(1);
        }
    };

    println!("Layout: {}×{} ({} entities)\n", layout.width, layout.height, layout.entities.len());

    // Count entities by name
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for e in &layout.entities {
        *counts.entry(e.name.clone()).or_insert(0) += 1;
    }
    println!("Entity counts:");
    for (name, count) in &counts {
        println!("  {:<28} {}", name, count);
    }
    println!();

    // Direction distribution for belts (helps spot missing tap-offs)
    let mut belt_dirs: FxHashMap<String, usize> = FxHashMap::default();
    for e in &layout.entities {
        if e.name.contains("transport-belt") {
            let d = format!("{:?}", e.direction);
            *belt_dirs.entry(d).or_insert(0) += 1;
        }
    }
    if !belt_dirs.is_empty() {
        println!("Transport-belt directions:");
        for (d, c) in &belt_dirs {
            println!("  {:<6} {}", d, c);
        }
        println!();
    }

    println!("ASCII map:");
    print_ascii_map(&layout);
}

fn print_ascii_map(layout: &LayoutResult) {
    let machines_3x3 = ["assembling-machine-1", "assembling-machine-2", "assembling-machine-3", "chemical-plant", "electric-furnace"];
    let machines_5x5 = ["oil-refinery"];

    let base_sym: FxHashMap<&str, char> = [
        ("transport-belt", '='),
        ("fast-transport-belt", '='),
        ("express-transport-belt", '='),
        ("underground-belt", 'U'),
        ("fast-underground-belt", 'U'),
        ("express-underground-belt", 'U'),
        ("inserter", 'i'),
        ("fast-inserter", 'i'),
        ("long-handed-inserter", 'L'),
        ("pipe", '|'),
        ("pipe-to-ground", 'P'),
        ("medium-electric-pole", '+'),
        ("splitter", 'S'),
        ("fast-splitter", 'S'),
        ("express-splitter", 'S'),
    ]
    .iter()
    .copied()
    .collect();

    // Assign each recipe a digit/letter
    let digits: Vec<char> = "123456789abcdefghijklmnop".chars().collect();
    let mut recipe_to_sym: FxHashMap<String, char> = FxHashMap::default();
    let is_machine = |name: &str| machines_3x3.contains(&name) || machines_5x5.contains(&name);

    for e in &layout.entities {
        if is_machine(&e.name) {
            if let Some(r) = &e.recipe {
                if !recipe_to_sym.contains_key(r) {
                    let idx = recipe_to_sym.len();
                    recipe_to_sym.insert(r.clone(), digits.get(idx).copied().unwrap_or('?'));
                }
            }
        }
    }

    let mut grid: FxHashMap<(i32, i32), char> = FxHashMap::default();
    for e in &layout.entities {
        let x = e.x;
        let y = e.y;
        if is_machine(&e.name) {
            let sym = e.recipe.as_ref().and_then(|r| recipe_to_sym.get(r)).copied().unwrap_or('?');
            let size = if machines_5x5.contains(&e.name.as_str()) { 5 } else { 3 };
            for dx in 0..size {
                for dy in 0..size {
                    grid.insert((x + dx, y + dy), sym);
                }
            }
        } else {
            let sym = base_sym.get(e.name.as_str()).copied().unwrap_or('?');
            grid.insert((x, y), sym);
        }
    }

    if grid.is_empty() {
        println!("   (empty)");
        return;
    }

    let min_x = grid.keys().map(|(x, _)| *x).min().unwrap();
    let max_x = grid.keys().map(|(x, _)| *x).max().unwrap();
    let min_y = grid.keys().map(|(_, y)| *y).min().unwrap();
    let max_y = grid.keys().map(|(_, y)| *y).max().unwrap();

    println!("   x: {} → {}  y: {} → {}", min_x, max_x, min_y, max_y);
    for y in min_y..=max_y {
        print!("{:>4} ", y);
        for x in min_x..=max_x {
            print!("{}", grid.get(&(x, y)).copied().unwrap_or(' '));
        }
        println!();
    }
    println!();

    println!("Legend:");
    let mut recipes: Vec<_> = recipe_to_sym.iter().collect();
    recipes.sort_by_key(|(_, c)| *c);
    for (r, c) in recipes {
        println!("  {} = {}", c, r);
    }
}
