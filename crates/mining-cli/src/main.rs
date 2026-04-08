//! CLI tool for analyzing Factorio blueprint strings.
//!
//! Usage:
//!   echo "0eN..." | blueprint-analyze          # single blueprint from stdin
//!   blueprint-analyze blueprint.txt             # from file
//!   blueprint-analyze --json < blueprint.txt    # JSON output
//!   cat strings.txt | blueprint-analyze --batch # one blueprint per line
//!
//! Blueprint books are automatically expanded — each sub-blueprint is analyzed
//! individually.

use std::io::{self, BufRead, Read};
use std::process;

use fucktorio_core::analysis::{self, BlueprintAnalysis};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let json_output = args.iter().any(|a| a == "--json");
    let batch_mode = args.iter().any(|a| a == "--batch");
    let quiet = args.iter().any(|a| a == "-q" || a == "--quiet");
    let file_args: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with('-'))
        .map(|s| s.as_str())
        .collect();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        eprintln!("Usage: blueprint-analyze [OPTIONS] [FILE...]");
        eprintln!();
        eprintln!("Analyze Factorio blueprint strings (single or book).");
        eprintln!();
        eprintln!("  FILE            Read blueprint string(s) from file(s)");
        eprintln!("  (no args)       Read from stdin");
        eprintln!();
        eprintln!("Options:");
        eprintln!("  --json          Output JSON instead of human-readable text");
        eprintln!("  --batch         Batch mode: one blueprint string per line");
        eprintln!("  -q, --quiet     Suppress warnings");
        eprintln!("  -h, --help      Show this help");
        process::exit(0);
    }

    if batch_mode {
        run_batch(json_output, quiet);
    } else if file_args.is_empty() {
        let mut input = String::new();
        io::stdin()
            .read_to_string(&mut input)
            .expect("failed to read stdin");
        let input = input.trim();
        if input.is_empty() {
            eprintln!("Error: empty input");
            process::exit(1);
        }
        match analyze_and_report(input, None, json_output, quiet) {
            Ok(_) => {}
            Err(e) => {
                eprintln!("Error: {e}");
                process::exit(1);
            }
        }
    } else {
        for path in &file_args {
            let content = std::fs::read_to_string(path).unwrap_or_else(|e| {
                eprintln!("Error reading {path}: {e}");
                process::exit(1);
            });
            let bp = extract_bp_string(&content);
            if !quiet && file_args.len() > 1 {
                eprintln!("--- {} ---", path);
            }
            match analyze_and_report(&bp, Some(path), json_output, quiet) {
                Ok(_) => {}
                Err(e) => {
                    eprintln!("Error analyzing {path}: {e}");
                    process::exit(1);
                }
            }
        }
    }
}

fn run_batch(json_output: bool, quiet: bool) {
    let stdin = io::stdin();
    let mut results: Vec<serde_json::Value> = Vec::new();
    let mut ok = 0usize;
    let mut fail = 0usize;

    for (i, line) in stdin.lock().lines().enumerate() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("Error reading line {}: {e}", i + 1);
                fail += 1;
                continue;
            }
        };
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        // Support name<TAB>blueprint format
        let (name, bp) = if let Some(idx) = line.find('\t') {
            (Some(line[..idx].to_string()), line[idx + 1..].to_string())
        } else {
            (None, line)
        };

        let bp = extract_bp_string(&bp);

        match analysis::analyze_blueprint_string_any(&bp) {
            Ok(analyses) => {
                for (j, na) in analyses.iter().enumerate() {
                    // Skip empty blueprints (no entities)
                    if na.analysis.total_entities == 0 {
                        continue;
                    }
                    ok += 1;
                    let entry_label = entry_name(&name, &na.label, i, j, analyses.len());
                    if json_output {
                        let mut val = serde_json::to_value(&na.analysis).unwrap();
                        val.as_object_mut()
                            .unwrap()
                            .insert("name".into(), serde_json::Value::String(entry_label));
                        results.push(val);
                    } else {
                        println!("=== {} ===", entry_label);
                        print_analysis(&na.analysis, quiet);
                        println!();
                    }
                }
            }
            Err(e) => {
                fail += 1;
                let fallback = format!("line {}", i + 1);
                let label = name.as_deref().unwrap_or(&fallback);
                eprintln!("FAIL {}: {}", label, e);
            }
        }
    }

    if json_output {
        let output = serde_json::json!({
            "blueprints": results,
            "summary": { "ok": ok, "failed": fail }
        });
        println!("{}", serde_json::to_string_pretty(&output).unwrap());
    } else {
        eprintln!("\n{ok} analyzed, {fail} failed");
    }
}

/// Build a display name for an analyzed entry.
fn entry_name(
    file_name: &Option<String>,
    bp_label: &Option<String>,
    line_idx: usize,
    entry_idx: usize,
    total_entries: usize,
) -> String {
    let fallback = format!("line {}", line_idx + 1);
    let base = file_name.as_deref().unwrap_or(&fallback);
    if total_entries == 1 {
        bp_label
            .as_deref()
            .map(|l| format!("{} ({})", base, l))
            .unwrap_or_else(|| base.to_string())
    } else {
        let suffix = bp_label
            .as_deref()
            .map(|l| format!(" ({})", l))
            .unwrap_or_default();
        format!("{}[{}]{}", base, entry_idx, suffix)
    }
}

fn analyze_and_report(
    bp: &str,
    file_name: Option<&str>,
    json_output: bool,
    quiet: bool,
) -> Result<(), String> {
    let analyses = analysis::analyze_blueprint_string_any(bp)?;

    if json_output {
        if analyses.len() == 1 {
            println!(
                "{}",
                serde_json::to_string_pretty(&analyses[0].analysis)
                    .map_err(|e| e.to_string())?
            );
        } else {
            let arr: Vec<_> = analyses.iter().map(|na| &na.analysis).collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&arr).map_err(|e| e.to_string())?
            );
        }
    } else {
        for (j, na) in analyses.iter().enumerate() {
            if na.analysis.total_entities == 0 {
                continue;
            }
            if analyses.len() > 1 {
                let label = na
                    .label
                    .as_deref()
                    .map(|l| format!("[{}] {}", j, l))
                    .unwrap_or_else(|| format!("[{}]", j));
                println!("=== {} ===", label);
            } else if let Some(ref label) = na.label {
                println!("=== {} ===", label);
            } else if let Some(f) = file_name {
                println!("=== {} ===", f);
            }
            print_analysis(&na.analysis, quiet);
            if analyses.len() > 1 {
                println!();
            }
        }
    }
    Ok(())
}

fn print_analysis(a: &BlueprintAnalysis, quiet: bool) {
    // Final products
    if a.final_products.is_empty() {
        println!("  Final product: (unknown)");
    } else {
        println!("  Final product: {}", a.final_products.join(", "));
    }

    // Size
    println!(
        "  Size: {}x{} ({} entities, density {:.2})",
        a.width, a.height, a.total_entities, a.density
    );

    // Entity summary
    println!(
        "  Machines: {}  Belts: {}  Pipes: {}  Inserters: {}  Beacons: {}  Poles: {}",
        a.machine_count,
        a.belt_tiles,
        a.pipe_tiles,
        a.inserter_count,
        a.beacon_count,
        a.pole_count
    );

    // Recipes
    if !a.recipe_groups.is_empty() {
        println!("  Recipes ({}):", a.recipe_count);
        for g in &a.recipe_groups {
            let throughput_str: String = g
                .throughput
                .iter()
                .map(|(item, rate)| format!("{:.1}/s {}", rate, item))
                .collect::<Vec<_>>()
                .join(", ");
            let module_str = if (g.effective_speed_multiplier - 1.0).abs() > 0.01
                || g.productivity_bonus > 0.001
            {
                let mut parts = Vec::new();
                if (g.effective_speed_multiplier - 1.0).abs() > 0.01 {
                    parts.push(format!(
                        "speed {:.0}%",
                        (g.effective_speed_multiplier - 1.0) * 100.0
                    ));
                }
                if g.productivity_bonus > 0.001 {
                    parts.push(format!("prod +{:.0}%", g.productivity_bonus * 100.0));
                }
                format!(" [{}]", parts.join(", "))
            } else {
                String::new()
            };
            println!(
                "    {}x {} ({}) -> {}{}",
                g.count, g.recipe, g.machine_type, throughput_str, module_str
            );
        }
    }

    // Production chain
    if !a.production_chain.is_empty() {
        println!("  Production chain (depth {}):", a.chain_depth);
        for step in &a.production_chain {
            let indent = "    ".to_string() + &"  ".repeat(step.depth);
            println!(
                "{}{}x {} ({})",
                indent, step.machine_count, step.recipe, step.machine_type
            );
        }
    }

    // Unknown recipes
    if !quiet && !a.unknown_recipes.is_empty() {
        println!("  Unknown recipes: {}", a.unknown_recipes.join(", "));
    }
}

/// Try to extract a blueprint string from a JSON wrapper, or return as-is.
fn extract_bp_string(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.starts_with('{') {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
            for key in &[
                "blueprintString",
                "blueprint_string",
                "blueprint-string",
                "string",
                "data",
            ] {
                if let Some(s) = val.get(key).and_then(|v| v.as_str()) {
                    return s.to_string();
                }
            }
        }
    }
    trimmed.to_string()
}
