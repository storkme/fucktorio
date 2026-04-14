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

use fucktorio_core::bus::layout::build_bus_layout_traced;
use fucktorio_core::models::{EntityDirection, LayoutRegion, PortIo, RegionKind, RegionPort};
use fucktorio_core::solver;
use fucktorio_core::trace::TraceEvent;
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

fn port_axis(p: &RegionPort) -> &'static str {
    match p.point.direction {
        EntityDirection::East | EntityDirection::West => "h",
        EntityDirection::North | EntityDirection::South => "v",
    }
}

#[derive(Default)]
struct ItemPorts {
    inputs: Vec<RegionPort>,
    outputs: Vec<RegionPort>,
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

    // Build tile → Vec<(bridge_dir, reason)> map from JunctionTemplateRejected
    // events. A single crossing can generate two events (vertical then
    // horizontal fallback), both with the same (tile_x, tile_y). Unresolved
    // perpendicular regions are per-tile so we can look up by region (x, y).
    let mut rejections: BTreeMap<(i32, i32), Vec<(String, String)>> = BTreeMap::new();
    // Tally which strategy solved each successful crossing, plus how many
    // regions hit the growth-cap (and why). Used below to explain why SAT
    // is or isn't firing.
    let mut solved_by_strategy: BTreeMap<String, usize> = BTreeMap::new();
    let mut growth_capped: BTreeMap<String, usize> = BTreeMap::new();
    if let Some(events) = layout.trace.as_ref() {
        for ev in events {
            match ev {
                TraceEvent::JunctionTemplateRejected {
                    tile_x,
                    tile_y,
                    bridge_dir,
                    reason,
                } => {
                    rejections
                        .entry((*tile_x, *tile_y))
                        .or_default()
                        .push((bridge_dir.clone(), reason.clone()));
                }
                TraceEvent::JunctionSolved { strategy, .. } => {
                    *solved_by_strategy.entry(strategy.clone()).or_insert(0) += 1;
                }
                TraceEvent::JunctionGrowthCapped { reason, .. } => {
                    *growth_capped.entry(reason.clone()).or_insert(0) += 1;
                }
                _ => {}
            }
        }
    }

    // Group regions by (kind, class)
    let mut by_kc: BTreeMap<(RegionKind, Class), Vec<&LayoutRegion>> = BTreeMap::new();
    for r in &layout.regions {
        let c = classify(r);
        by_kc.entry((r.kind, c)).or_default().push(r);
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
    {
        let mut by_kind: BTreeMap<String, usize> = BTreeMap::new();
        for issue in &issues {
            *by_kind.entry(issue.category.to_string()).or_insert(0) += 1;
        }
        if !by_kind.is_empty() {
            println!("  errors by code: {:?}", by_kind);
        }
        for issue in issues.iter().take(40) {
            println!(
                "    err {} @ ({:?},{:?}): {}",
                issue.category, issue.x, issue.y, issue.message
            );
            if issue.category == "entity-overlap" {
                let (Some(ex), Some(ey)) = (issue.x, issue.y) else { continue; };
                let here: Vec<_> = layout
                    .entities
                    .iter()
                    .filter(|e| e.x == ex && e.y == ey)
                    .map(|e| {
                        format!(
                            "{}/{:?}/seg={}",
                            e.name,
                            e.direction,
                            e.segment_id.as_deref().unwrap_or("-")
                        )
                    })
                    .collect();
                println!("       entities: {:?}", here);
            }
        }
    }
    if !layout.warnings.is_empty() {
        println!("  warnings: {:?}", layout.warnings);
    }

    if !solved_by_strategy.is_empty() {
        println!(
            "  junction-solver success by strategy: {:?}",
            solved_by_strategy
        );
    }
    if !growth_capped.is_empty() {
        println!("  growth-capped by reason: {:?}", growth_capped);
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
            "    {:20?}  {:20}  ×{:3}  {} err-touching regions ({} total errors)",
            kind, class.label(), regs.len(), err_touch, err_total
        );
        // Drill into the specific anomaly: unresolved regions that the
        // classifier calls perpendicular. Print the tile-level rejection
        // reasons so we can categorise why the template matcher bailed.
        if *kind == RegionKind::Unresolved && *class == Class::Perpendicular {
            for r in regs {
                // Unresolved regions are 1×1 per-tile in today's pipeline;
                // scan each contained tile and print reasons if present.
                let mut any_shown = false;
                for ty in r.y..r.y + r.height {
                    for tx in r.x..r.x + r.width {
                        if let Some(reasons) = rejections.get(&(tx, ty)) {
                            let joined: Vec<String> = reasons
                                .iter()
                                .map(|(bd, rs)| format!("{}={}", bd, rs))
                                .collect();
                            println!(
                                "      ({},{}) reasons: {}",
                                tx,
                                ty,
                                joined.join(", ")
                            );
                            any_shown = true;
                        }
                    }
                }
                if !any_shown {
                    println!(
                        "      ({},{}) {}×{}  (no trace events — investigate)",
                        r.x, r.y, r.width, r.height
                    );
                }
            }
        }
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
    if label.contains("user URL") {
        println!("  entities near (4, 35):");
        let mut around: Vec<_> = layout
            .entities
            .iter()
            .filter(|e| e.x >= 0 && e.x <= 10 && e.y >= 30 && e.y <= 40)
            .collect();
        around.sort_by_key(|e| (e.y, e.x));
        for e in around {
            println!(
                "    ({},{}) {} {:?} carries={:?} seg={}",
                e.x,
                e.y,
                e.name,
                e.direction,
                e.carries,
                e.segment_id.as_deref().unwrap_or("-")
            );
        }
    }

    // Dump row y-positions per item: helps spot whether place_rows is
    // leaving room for the balancer block.
    {
        let mut rows_by_item: BTreeMap<String, Vec<i32>> = BTreeMap::new();
        for e in &layout.entities {
            let Some(seg) = e.segment_id.as_deref() else { continue };
            if let Some(rest) = seg.strip_prefix("row:") {
                if let Some(item) = rest.split(':').next() {
                    rows_by_item.entry(item.to_string()).or_default().push(e.y);
                }
            }
        }
        for (item, ys) in &mut rows_by_item {
            let mut unique: Vec<i32> = ys.iter().copied().collect();
            unique.sort_unstable();
            unique.dedup();
            println!("  row:{} ys: {:?}", item, unique);
        }
    }

    if let Some(events) = layout.trace.as_ref() {
        let phases: Vec<_> = events
            .iter()
            .filter_map(|ev| {
                if let TraceEvent::PhaseTime { phase, .. } = ev {
                    Some(phase.clone())
                } else {
                    None
                }
            })
            .collect();
        let retry_count = phases
            .iter()
            .filter(|p| p.starts_with("place_rows_2_attempt_"))
            .count();
        println!("  phases: place_rows_1={} place_rows_2_attempts={}",
            phases.iter().filter(|p| p.as_str() == "place_rows_1").count(),
            retry_count
        );
    }

    if let Some(events) = layout.trace.as_ref() {
        let balancers: Vec<_> = events
            .iter()
            .filter_map(|ev| {
                if let TraceEvent::BalancerStamped {
                    item,
                    shape,
                    y_start,
                    y_end,
                    template_found,
                } = ev
                {
                    Some((item.clone(), *shape, *y_start, *y_end, *template_found))
                } else {
                    None
                }
            })
            .collect();
        if !balancers.is_empty() {
            println!("  balancers: {:?}", balancers);
        }
    }

    println!("  SAT clusters touching validator errors:");
    let mut any = false;
    for (i, r) in layout.regions.iter().enumerate() {
        if r.kind == RegionKind::CrossingZone && region_err_counts[i] > 0 {
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

fn group_by_item(r: &LayoutRegion) -> BTreeMap<String, Vec<&RegionPort>> {
    let mut m: BTreeMap<String, Vec<&RegionPort>> = BTreeMap::new();
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
    run_case_with_belt(
        "tier4 advanced-circuit 5/s AM1 (user URL, no belt cap)",
        "advanced-circuit",
        5.0,
        "assembling-machine-1",
        &[
            "steel-plate", "stone", "coal", "water", "crude-oil",
            "iron-ore", "copper-ore",
        ],
        None,
    );
}

fn run_case_with_belt(
    label: &str,
    recipe: &str,
    rate: f64,
    machine: &str,
    inputs: &[&str],
    max_belt_tier: Option<&str>,
) {
    let input_set: FxHashSet<String> = inputs.iter().map(|s| s.to_string()).collect();
    let solver_result = solver::solve(recipe, rate, &input_set, machine).expect("solve");
    let layout = build_bus_layout_traced(&solver_result, max_belt_tier).expect("layout");
    let issues = match validate::validate(&layout, Some(&solver_result), LayoutStyle::Bus) {
        Ok(issues) => issues,
        Err(e) => e.issues,
    };
    let issues: Vec<_> = issues
        .into_iter()
        .filter(|i| matches!(i.severity, validate::Severity::Error))
        .collect();

    println!("\n=== {} ===", label);
    println!(
        "  layout: {} entities, {} regions, {} validator errors",
        layout.entities.len(),
        layout.regions.len(),
        issues.len()
    );
    if !layout.warnings.is_empty() {
        println!("  warnings: {:?}", layout.warnings);
    }
    let mut by_kind: BTreeMap<String, usize> = BTreeMap::new();
    for issue in &issues {
        *by_kind.entry(issue.category.to_string()).or_insert(0) += 1;
    }
    println!("  errors by code: {:?}", by_kind);
    for issue in &issues {
        println!(
            "    err {} @ ({:?},{:?}): {}",
            issue.category, issue.x, issue.y, issue.message
        );
    }

}
