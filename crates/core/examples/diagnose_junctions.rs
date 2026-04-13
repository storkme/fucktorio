//! Diagnostic dump of ghost-routed regions for tier2/3/4, classifying each
//! region by shape (T1/T2/T3/T4/...) and cross-referencing with validator
//! errors. This is the "Step 0" diagnosis for the junction solver work —
//! it tells us which region classes currently exist, which are solved
//! correctly, and which need new templates.
//!
//! Run with:
//!   cargo run --manifest-path crates/core/Cargo.toml \
//!       --example diagnose_junctions --release

use std::collections::BTreeMap;

use fucktorio_core::bus::layout::{build_bus_layout_traced, GhostModeGuard};
use fucktorio_core::models::{LayoutRegion, PortEdge, PortIo, PortSpec};
use fucktorio_core::solver;
use fucktorio_core::validate::{self, LayoutStyle};
use rustc_hash::FxHashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Class {
    Perpendicular,   // T1
    Corridor,        // T2
    SameDirection,   // T3
    Complex,         // T4
    SingleItem,
    Unbalanced,
    NoPorts,
}

impl Class {
    fn label(&self) -> &'static str {
        match self {
            Class::Perpendicular => "T1 perpendicular",
            Class::Corridor => "T2 corridor",
            Class::SameDirection => "T3 same-direction",
            Class::Complex => "T4 complex",
            Class::SingleItem => "single-item",
            Class::Unbalanced => "unbalanced",
            Class::NoPorts => "no-ports",
        }
    }
}

fn port_axis(p: &PortSpec) -> &'static str {
    match p.edge {
        PortEdge::E | PortEdge::W => "h",
        PortEdge::N | PortEdge::S => "v",
    }
}

#[derive(Default)]
struct ItemPorts {
    inputs: Vec<PortSpec>,
    outputs: Vec<PortSpec>,
    axes: FxHashSet<&'static str>,
}

fn classify(region: &LayoutRegion) -> Class {
    if region.ports.is_empty() {
        return Class::NoPorts;
    }
    // Group by item
    let mut items: BTreeMap<String, ItemPorts> = BTreeMap::new();
    for p in &region.ports {
        let entry = items.entry(p.item.clone().unwrap_or_else(|| "?".to_string())).or_default();
        if p.io == PortIo::Input {
            entry.inputs.push(p.clone());
        } else {
            entry.outputs.push(p.clone());
        }
        entry.axes.insert(port_axis(p));
    }
    // Unbalanced check
    for ip in items.values() {
        if ip.inputs.is_empty() || ip.outputs.is_empty() {
            return Class::Unbalanced;
        }
    }
    let item_count = items.len();
    let item_list: Vec<&ItemPorts> = items.values().collect();

    if item_count == 1 {
        let ip = item_list[0];
        if ip.inputs.len() == 1 && ip.outputs.len() == 1 {
            return Class::SingleItem;
        }
        return Class::SameDirection;
    }

    if item_count == 2 {
        let a = item_list[0];
        let b = item_list[1];
        let a_axes: Vec<_> = a.axes.iter().copied().collect();
        let b_axes: Vec<_> = b.axes.iter().copied().collect();
        let a_single = a.inputs.len() == 1 && a.outputs.len() == 1;
        let b_single = b.inputs.len() == 1 && b.outputs.len() == 1;
        let perpendicular = a_axes.len() == 1 && b_axes.len() == 1 && a_axes[0] != b_axes[0];
        if a_single && b_single && perpendicular {
            return Class::Perpendicular;
        }
        if a_axes == b_axes {
            return Class::SameDirection;
        }
    }

    // 3+ items — corridor (one horizontal + N vertical, all single-port) or complex
    let horiz: Vec<&ItemPorts> = item_list
        .iter()
        .copied()
        .filter(|ip| ip.axes.len() == 1 && ip.axes.contains("h"))
        .collect();
    let vert: Vec<&ItemPorts> = item_list
        .iter()
        .copied()
        .filter(|ip| ip.axes.len() == 1 && ip.axes.contains("v"))
        .collect();
    if horiz.len() == 1 && vert.len() == item_count - 1 {
        let h = horiz[0];
        let all_single_v = vert.iter().all(|v| v.inputs.len() == 1 && v.outputs.len() == 1);
        if all_single_v && h.inputs.len() == 1 && h.outputs.len() == 1 {
            return Class::Corridor;
        }
    }
    if vert.len() == 1 && horiz.len() == item_count - 1 {
        return Class::Corridor;
    }
    Class::Complex
}

struct Dims {
    w: i32,
    h: i32,
}

fn bucket_dims(d: &Dims) -> String {
    format!("{}×{}", d.w, d.h)
}

fn run_case(label: &str, recipe: &str, rate: f64, machine: &str, inputs: &[&str]) {
    let input_set: FxHashSet<String> = inputs.iter().map(|s| s.to_string()).collect();
    let solver_result = solver::solve(recipe, rate, &input_set, machine)
        .expect("solve");

    let _g = GhostModeGuard::new();
    let layout = build_bus_layout_traced(&solver_result, Some("transport-belt"))
        .expect("layout");

    // validate returns Err when errors are found; the issues live inside
    // the error variant too. Drain both branches.
    let issues = match validate::validate(&layout, Some(&solver_result), LayoutStyle::Bus) {
        Ok(issues) => issues,
        Err(e) => e.issues,
    };
    // Keep only actual errors (validate returns warnings via the same Vec).
    let issues: Vec<_> = issues
        .into_iter()
        .filter(|i| matches!(i.severity, validate::Severity::Error))
        .collect();

    // Group regions by (kind, class)
    let mut by_kc: BTreeMap<(String, Class), Vec<&LayoutRegion>> = BTreeMap::new();
    for r in &layout.regions {
        let c = classify(r);
        by_kc.entry((r.kind.clone(), c)).or_default().push(r);
    }

    // Also map region → contained validator errors
    let region_contains = |r: &LayoutRegion, (x, y): (i32, i32)| -> bool {
        x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height
    };
    let mut region_err_counts: Vec<usize> = vec![0; layout.regions.len()];
    let mut global_err_count = 0usize;
    for issue in &issues {
        global_err_count += 1;
        let Some(x) = issue.x else { continue };
        let Some(y) = issue.y else { continue };
        let tile = (x, y);
        for (i, r) in layout.regions.iter().enumerate() {
            if region_contains(r, tile) {
                region_err_counts[i] += 1;
            }
        }
    }

    println!("\n=== {} ===", label);
    println!("  layout: {} entities, {} regions, {} validator errors",
             layout.entities.len(), layout.regions.len(), global_err_count);
    if !layout.warnings.is_empty() {
        println!("  warnings: {:?}", layout.warnings);
    }

    println!("  regions by (kind × class):");
    for ((kind, class), regs) in &by_kc {
        let err_touch: usize = regs.iter().filter_map(|r| {
            let idx = layout.regions.iter().position(|x| std::ptr::eq(*r, x))?;
            Some(region_err_counts[idx])
        }).filter(|&c| c > 0).count();
        let err_total: usize = regs.iter().filter_map(|r| {
            let idx = layout.regions.iter().position(|x| std::ptr::eq(*r, x))?;
            Some(region_err_counts[idx])
        }).sum();
        println!(
            "    {:20}  {:20}  ×{:3}  {} err-touching regions ({} total errors)",
            kind, class.label(), regs.len(), err_touch, err_total
        );
    }

    // Dimension histogram for T4 Complex + SAT clusters (the hard cases)
    let mut complex_dims: BTreeMap<String, usize> = BTreeMap::new();
    for r in &layout.regions {
        if classify(r) == Class::Complex {
            let key = bucket_dims(&Dims { w: r.width, h: r.height });
            *complex_dims.entry(key).or_insert(0) += 1;
        }
    }
    if !complex_dims.is_empty() {
        println!("  T4 complex regions by dimension:");
        for (dims, count) in &complex_dims {
            println!("    {:8}  ×{}", dims, count);
        }
    }

    // Specific problem regions: ghost_cluster SAT regions that overlap errors
    println!("  SAT clusters touching validator errors:");
    let mut any = false;
    for (i, r) in layout.regions.iter().enumerate() {
        if r.kind == "ghost_cluster" && region_err_counts[i] > 0 {
            any = true;
            let c = classify(r);
            println!(
                "    ({},{}) {}×{}  {}  ports={}  errs={}",
                r.x, r.y, r.width, r.height, c.label(),
                r.ports.len(),
                region_err_counts[i]
            );
            for (item, _) in group_by_item(r) {
                println!("      item: {}", item);
            }
        }
    }
    if !any {
        println!("    (none)");
    }
}

fn group_by_item(r: &LayoutRegion) -> BTreeMap<String, Vec<&PortSpec>> {
    let mut m: BTreeMap<String, Vec<&PortSpec>> = BTreeMap::new();
    for p in &r.ports {
        m.entry(p.item.clone().unwrap_or_else(|| "?".to_string())).or_default().push(p);
    }
    m
}

fn main() {
    println!("junction-solver diagnosis: region class breakdown across tier2/3/4 ghost layouts");

    run_case(
        "tier2 electronic-circuit from ore, 30/s yellow AM1",
        "electronic-circuit",
        30.0,
        "assembling-machine-1",
        &["iron-ore", "copper-ore"],
    );
    run_case(
        "tier3 plastic-bar, 30/s yellow chemical-plant",
        "plastic-bar",
        30.0,
        "chemical-plant",
        &["petroleum-gas", "coal"],
    );
    run_case(
        "tier4 advanced-circuit from ore, 5/s yellow AM1",
        "advanced-circuit",
        5.0,
        "assembling-machine-1",
        &["iron-ore", "copper-ore", "coal", "water", "crude-oil"],
    );
}
