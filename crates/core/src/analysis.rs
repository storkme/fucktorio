//! Blueprint analysis: extract production statistics from a parsed layout.
//!
//! Works on `LayoutResult` (flat entity list) plus the bundled recipe DB.
//! No physical network tracing — uses recipe-level logic to infer production
//! chains and final products.

use std::collections::BTreeMap;

use rustc_hash::{FxHashMap, FxHashSet};
use serde::Serialize;

use crate::blueprint_parser;
use crate::common;
use crate::models::LayoutResult;
use crate::recipe_db;

// ---- Entity classification ----

const BELT_ENTITIES: &[&str] = &[
    "transport-belt",
    "fast-transport-belt",
    "express-transport-belt",
    "turbo-transport-belt",
];

const UNDERGROUND_BELT_ENTITIES: &[&str] = &[
    "underground-belt",
    "fast-underground-belt",
    "express-underground-belt",
    "turbo-underground-belt",
];

const SPLITTER_ENTITIES: &[&str] = &["splitter", "fast-splitter", "express-splitter", "turbo-splitter"];

const INSERTER_ENTITIES: &[&str] = &[
    "burner-inserter",
    "inserter",
    "long-handed-inserter",
    "fast-inserter",
    "bulk-inserter",
    "stack-inserter",
];

const PIPE_ENTITIES: &[&str] = &["pipe", "pipe-to-ground"];

const POLE_ENTITIES: &[&str] = &[
    "small-electric-pole",
    "medium-electric-pole",
    "big-electric-pole",
    "substation",
];

fn is_belt(name: &str) -> bool {
    BELT_ENTITIES.contains(&name)
        || UNDERGROUND_BELT_ENTITIES.contains(&name)
        || SPLITTER_ENTITIES.contains(&name)
}

fn is_inserter(name: &str) -> bool {
    INSERTER_ENTITIES.contains(&name)
}

fn is_pipe(name: &str) -> bool {
    PIPE_ENTITIES.contains(&name)
}

fn is_pole(name: &str) -> bool {
    POLE_ENTITIES.contains(&name)
}

fn is_beacon(name: &str) -> bool {
    name == "beacon"
}

fn is_furnace(name: &str) -> bool {
    matches!(name, "electric-furnace" | "steel-furnace" | "stone-furnace")
}

/// Returns true if the entity is a crafting machine (has recipes in the DB).
fn is_crafting_machine(name: &str) -> bool {
    matches!(
        name,
        "assembling-machine-1"
            | "assembling-machine-2"
            | "assembling-machine-3"
            | "chemical-plant"
            | "oil-refinery"
            | "electric-furnace"
            | "steel-furnace"
            | "stone-furnace"
            | "centrifuge"
            | "lab"
            | "rocket-silo"
            | "foundry"
            | "electromagnetic-plant"
            | "cryogenic-plant"
            | "biochamber"
            | "biolab"
            | "recycler"
            | "crusher"
    )
}

// ---- Analysis types ----

/// A group of machines all running the same recipe.
#[derive(Debug, Clone, Serialize)]
pub struct RecipeGroup {
    pub recipe: String,
    pub machine_type: String,
    pub count: usize,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    /// Estimated items/s for each output product (accounts for modules and beacons).
    pub throughput: Vec<(String, f64)>,
    /// Speed multiplier from modules + beacons (1.0 = base speed).
    pub effective_speed_multiplier: f64,
    /// Productivity bonus from modules (0.0 = no bonus).
    pub productivity_bonus: f64,
}

/// A step in the production chain.
#[derive(Debug, Clone, Serialize)]
pub struct ChainStep {
    pub recipe: String,
    pub depth: usize,
    pub machine_type: String,
    pub machine_count: usize,
}

/// Complete analysis of a parsed blueprint.
#[derive(Debug, Clone, Serialize)]
pub struct BlueprintAnalysis {
    // Identity
    pub final_products: Vec<String>,
    pub recipe_count: usize,

    // Entity counts
    pub machine_count: usize,
    pub belt_tiles: usize,
    pub pipe_tiles: usize,
    pub inserter_count: usize,
    pub beacon_count: usize,
    pub pole_count: usize,
    pub total_entities: usize,

    // Entity breakdown (name → count)
    pub entity_counts: BTreeMap<String, usize>,

    // Spatial
    pub width: i32,
    pub height: i32,
    pub area: i32,
    pub density: f64,

    // Production
    pub recipe_groups: Vec<RecipeGroup>,
    pub production_chain: Vec<ChainStep>,
    pub chain_depth: usize,
    pub throughput_estimates: BTreeMap<String, f64>,

    // Machines without known recipes (entity had recipe field but we can't look it up)
    pub unknown_recipes: Vec<String>,
}

/// Collect all smelting-category recipes from the DB.
fn smelting_recipes() -> Vec<&'static recipe_db::Recipe> {
    recipe_db::db()
        .recipes
        .values()
        .filter(|r| r.category == "smelting")
        .collect()
}

/// Infer which smelting recipes the recipe-less furnaces are running.
///
/// Strategy:
/// 1. If other machines in the blueprint consume smelting products, assign
///    furnaces to produce those products (proportional to consumption).
/// 2. If no other machines exist (pure smelting array), try all smelting recipes
///    and pick the most likely based on common patterns (steel-plate if many
///    furnaces, iron-plate as default).
///
/// Returns a list of (recipe_name, furnace_type, count) to merge into recipe_machines.
fn infer_furnace_recipes(
    recipeless_furnaces: &FxHashMap<String, usize>,
    existing_recipes: &FxHashMap<String, (String, usize)>,
) -> Vec<(String, String, usize)> {
    let mut result: Vec<(String, String, usize)> = Vec::new();
    let smelting = smelting_recipes();

    // smelting product name → smelting recipe name
    let smelting_products: FxHashMap<&str, &str> = smelting
        .iter()
        .flat_map(|r| {
            r.products
                .iter()
                .map(move |p| (p.name.as_str(), r.name.as_str()))
        })
        .collect();

    let total_furnaces: usize = recipeless_furnaces.values().sum();
    let furnace_type = recipeless_furnaces.keys().next().unwrap().clone();

    // Find which smelting products are consumed by other recipes in this blueprint
    let mut needed_smelting: Vec<String> = Vec::new();
    for recipe_name in existing_recipes.keys() {
        if let Some(recipe) = recipe_db::db().recipes.get(recipe_name) {
            for ing in &recipe.ingredients {
                if smelting_products.contains_key(ing.name.as_str())
                    && !needed_smelting.contains(&ing.name)
                {
                    needed_smelting.push(ing.name.clone());
                }
            }
        }
    }

    if !needed_smelting.is_empty() {
        if needed_smelting.len() == 1 {
            // All furnaces produce the one needed smelting product
            if let Some(&recipe_name) = smelting_products.get(needed_smelting[0].as_str()) {
                result.push((recipe_name.to_string(), furnace_type, total_furnaces));
            }
        } else {
            // Multiple smelting products needed — split evenly
            let per_product = total_furnaces / needed_smelting.len();
            let mut remaining = total_furnaces;
            for (i, product) in needed_smelting.iter().enumerate() {
                let count = if i == needed_smelting.len() - 1 {
                    remaining
                } else {
                    per_product
                };
                remaining -= count;
                if let Some(&recipe_name) = smelting_products.get(product.as_str()) {
                    result.push((recipe_name.to_string(), furnace_type.clone(), count));
                }
            }
        }
    } else {
        // Pure smelting array — no consumers to infer from.
        // Check if some furnaces already have a smelting recipe set.
        let existing_smelting: Vec<&String> = existing_recipes
            .keys()
            .filter(|r| smelting.iter().any(|s| s.name == **r))
            .collect();

        if existing_smelting.len() == 1 {
            result.push((
                existing_smelting[0].clone(),
                furnace_type,
                total_furnaces,
            ));
        } else {
            // No hints at all. Default to iron-plate (most common smelting recipe).
            result.push(("iron-plate".to_string(), furnace_type, total_furnaces));
        }
    }

    result
}

/// Analyze a `LayoutResult` and extract production statistics.
pub fn analyze(layout: &LayoutResult) -> BlueprintAnalysis {
    let mut entity_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut machine_count = 0usize;
    let mut belt_tiles = 0usize;
    let mut pipe_tiles = 0usize;
    let mut inserter_count = 0usize;
    let mut beacon_count = 0usize;
    let mut pole_count = 0usize;

    // recipe_name → (machine_type, count)
    let mut recipe_machines: FxHashMap<String, (String, usize)> = FxHashMap::default();
    let mut unknown_recipes: Vec<String> = Vec::new();

    // furnace_type → count of recipe-less furnaces
    let mut recipeless_furnaces: FxHashMap<String, usize> = FxHashMap::default();

    // Collect beacons with their positions and module effects
    struct BeaconInfo {
        cx: i32,
        cy: i32,
        speed_bonus: f64,
        productivity_bonus: f64,
    }
    let mut beacons: Vec<BeaconInfo> = Vec::new();

    // First pass: count entities and collect beacon info
    for e in &layout.entities {
        *entity_counts.entry(e.name.clone()).or_default() += 1;

        if is_crafting_machine(&e.name) {
            machine_count += 1;
            if let Some(ref recipe) = e.recipe {
                let entry = recipe_machines
                    .entry(recipe.clone())
                    .or_insert_with(|| (e.name.clone(), 0));
                entry.1 += 1;
            } else if is_furnace(&e.name) {
                *recipeless_furnaces.entry(e.name.clone()).or_default() += 1;
            }
        } else if is_belt(&e.name) {
            belt_tiles += 1;
        } else if is_pipe(&e.name) {
            pipe_tiles += 1;
        } else if is_inserter(&e.name) {
            inserter_count += 1;
        } else if is_beacon(&e.name) {
            beacon_count += 1;
            // Beacon center (3×3 entity, top-left stored)
            let cx = e.x + 1;
            let cy = e.y + 1;
            let mut speed = 0.0;
            let mut prod = 0.0;
            for mi in &e.items {
                let eff = common::module_effect(&mi.item);
                speed += eff.speed * mi.count as f64;
                prod += eff.productivity * mi.count as f64;
            }
            beacons.push(BeaconInfo {
                cx,
                cy,
                speed_bonus: speed,
                productivity_bonus: prod,
            });
        } else if is_pole(&e.name) {
            pole_count += 1;
        }
    }

    // Infer recipes for furnaces that don't store them in the blueprint
    if !recipeless_furnaces.is_empty() {
        for (recipe, furnace_type, count) in
            infer_furnace_recipes(&recipeless_furnaces, &recipe_machines)
        {
            let entry = recipe_machines
                .entry(recipe)
                .or_insert_with(|| (furnace_type, 0));
            entry.1 += count;
        }
    }

    // Second pass: for each machine with a recipe, compute module+beacon bonuses.
    // We accumulate per-recipe-group totals.
    // Key: recipe_name → (total_speed_multiplier_sum, total_productivity_sum, machine_count_with_data)
    let mut recipe_bonuses: FxHashMap<String, (f64, f64, usize)> = FxHashMap::default();
    let beacon_dist = common::BEACON_SUPPLY_DISTANCE;
    let beacon_eff = common::BEACON_DISTRIBUTION_EFFECTIVITY;

    for e in &layout.entities {
        if !is_crafting_machine(&e.name) {
            continue;
        }
        let recipe_name = match e.recipe.as_ref() {
            Some(r) => r,
            None => continue,
        };

        // Internal module bonuses
        let mut speed_bonus = 0.0f64;
        let mut prod_bonus = 0.0f64;
        for mi in &e.items {
            let eff = common::module_effect(&mi.item);
            speed_bonus += eff.speed * mi.count as f64;
            prod_bonus += eff.productivity * mi.count as f64;
        }

        // Beacon bonuses: check which beacons are in range
        let machine_size = common::machine_size(&e.name) as i32;

        for b in &beacons {
            // Check if any tile of the machine overlaps the beacon's supply area.
            // Beacon supply area: center ± (1 + supply_distance) on each axis.
            // Machine occupies [e.x .. e.x+size-1] × [e.y .. e.y+size-1].
            let reach = 1 + beacon_dist; // beacon half-size (1) + supply distance
            let in_x = e.x <= b.cx + reach && e.x + machine_size >= b.cx - reach + 1;
            let in_y = e.y <= b.cy + reach && e.y + machine_size >= b.cy - reach + 1;
            if in_x && in_y {
                speed_bonus += b.speed_bonus * beacon_eff;
                prod_bonus += b.productivity_bonus * beacon_eff;
            }
        }

        // speed_multiplier = 1 + total_speed_bonus (clamped to minimum 0.2)
        let speed_mult = (1.0 + speed_bonus).max(0.2);
        let prod_mult = 1.0 + prod_bonus;

        let entry = recipe_bonuses
            .entry(recipe_name.clone())
            .or_insert((0.0, 0.0, 0));
        entry.0 += speed_mult;
        entry.1 += prod_mult;
        entry.2 += 1;
    }

    // Build recipe groups with module-aware throughput estimates
    let mut recipe_groups: Vec<RecipeGroup> = Vec::new();
    let mut all_produced: FxHashMap<String, f64> = FxHashMap::default();
    let mut all_consumed: FxHashSet<String> = FxHashSet::default();
    let mut unknown_set: FxHashSet<String> = FxHashSet::default();

    for (recipe_name, (machine_type, count)) in &recipe_machines {
        match recipe_db::db().recipes.get(recipe_name) {
            Some(recipe) => {
                let base_speed = recipe_db::get_crafting_speed(machine_type);
                let energy = if recipe.energy > 0.0 {
                    recipe.energy
                } else {
                    0.5
                };

                // Get average speed/productivity multipliers from module data
                let (avg_speed_mult, avg_prod_mult) =
                    if let Some(&(speed_sum, prod_sum, n)) = recipe_bonuses.get(recipe_name) {
                        if n > 0 {
                            (speed_sum / n as f64, prod_sum / n as f64)
                        } else {
                            (1.0, 1.0)
                        }
                    } else {
                        // Furnaces with inferred recipes won't have per-entity data
                        (1.0, 1.0)
                    };

                let inputs: Vec<String> =
                    recipe.ingredients.iter().map(|i| i.name.clone()).collect();
                let outputs: Vec<String> =
                    recipe.products.iter().map(|p| p.name.clone()).collect();

                let mut throughput = Vec::new();
                for prod in &recipe.products {
                    let rate = *count as f64
                        * base_speed
                        * avg_speed_mult
                        * prod.amount
                        * prod.probability
                        * avg_prod_mult
                        / energy;
                    throughput.push((prod.name.clone(), rate));
                    *all_produced.entry(prod.name.clone()).or_default() += rate;
                }

                for ing in &recipe.ingredients {
                    all_consumed.insert(ing.name.clone());
                }

                recipe_groups.push(RecipeGroup {
                    recipe: recipe_name.clone(),
                    machine_type: machine_type.clone(),
                    count: *count,
                    inputs,
                    outputs,
                    throughput,
                    effective_speed_multiplier: avg_speed_mult,
                    productivity_bonus: avg_prod_mult - 1.0,
                });
            }
            None => {
                if unknown_set.insert(recipe_name.clone()) {
                    unknown_recipes.push(recipe_name.clone());
                }
            }
        }
    }

    // Sort recipe groups by count descending
    recipe_groups.sort_by(|a, b| b.count.cmp(&a.count));

    // Determine final products: items produced but not consumed by other recipes
    let final_products: Vec<String> = {
        let mut candidates: Vec<(String, f64)> = all_produced
            .iter()
            .filter(|(item, _)| !all_consumed.contains(item.as_str()))
            .map(|(item, rate)| (item.clone(), *rate))
            .collect();
        candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        candidates.into_iter().map(|(item, _)| item).collect()
    };

    // Build production chain via recipe dependency walking
    let (chain, chain_depth) = build_production_chain(&recipe_machines);

    // Throughput estimates (only for final/intermediate products)
    let throughput_estimates: BTreeMap<String, f64> = all_produced
        .into_iter()
        .collect();

    let area = layout.width * layout.height;
    let total_entities = layout.entities.len();
    let density = if area > 0 {
        total_entities as f64 / area as f64
    } else {
        0.0
    };

    BlueprintAnalysis {
        final_products,
        recipe_count: recipe_machines.len(),
        machine_count,
        belt_tiles,
        pipe_tiles,
        inserter_count,
        beacon_count,
        pole_count,
        total_entities,
        entity_counts,
        width: layout.width,
        height: layout.height,
        area,
        density,
        recipe_groups,
        production_chain: chain,
        chain_depth,
        throughput_estimates,
        unknown_recipes,
    }
}

/// Build a production chain by walking recipe dependencies depth-first.
///
/// Returns (chain steps in dependency order, max depth).
fn build_production_chain(
    recipe_machines: &FxHashMap<String, (String, usize)>,
) -> (Vec<ChainStep>, usize) {
    // Build item→recipe lookup: for each item produced in this blueprint,
    // which recipe produces it?
    let mut item_recipe: FxHashMap<String, String> = FxHashMap::default();
    for recipe_name in recipe_machines.keys() {
        if let Some(recipe) = recipe_db::db().recipes.get(recipe_name) {
            for prod in &recipe.products {
                item_recipe.entry(prod.name.clone()).or_insert_with(|| recipe_name.clone());
            }
        }
    }

    // Find root recipes: recipes whose outputs aren't consumed by any other
    // recipe in this blueprint
    let consumed_items: FxHashSet<String> = recipe_machines
        .keys()
        .flat_map(|rn| {
            recipe_db::db()
                .recipes
                .get(rn)
                .map(|r| r.ingredients.iter().map(|i| i.name.clone()).collect::<Vec<_>>())
                .unwrap_or_default()
        })
        .collect();

    let mut root_recipes: Vec<String> = Vec::new();
    for recipe_name in recipe_machines.keys() {
        if let Some(recipe) = recipe_db::db().recipes.get(recipe_name) {
            let is_root = recipe
                .products
                .iter()
                .any(|p| !consumed_items.contains(&p.name));
            if is_root {
                root_recipes.push(recipe_name.clone());
            }
        }
    }

    if root_recipes.is_empty() {
        // Fallback: all recipes are roots
        root_recipes = recipe_machines.keys().cloned().collect();
    }

    // DFS from roots to compute depths
    let mut visited: FxHashSet<String> = FxHashSet::default();
    let mut steps: Vec<ChainStep> = Vec::new();
    let mut max_depth = 0usize;

    fn dfs(
        recipe_name: &str,
        depth: usize,
        recipe_machines: &FxHashMap<String, (String, usize)>,
        item_recipe: &FxHashMap<String, String>,
        visited: &mut FxHashSet<String>,
        steps: &mut Vec<ChainStep>,
        max_depth: &mut usize,
    ) {
        if !visited.insert(recipe_name.to_string()) {
            return;
        }
        *max_depth = (*max_depth).max(depth);

        // Recurse into ingredients first (dependencies before dependents)
        if let Some(recipe) = recipe_db::db().recipes.get(recipe_name) {
            for ing in &recipe.ingredients {
                if let Some(dep_recipe) = item_recipe.get(&ing.name) {
                    if recipe_machines.contains_key(dep_recipe.as_str()) {
                        dfs(
                            dep_recipe,
                            depth + 1,
                            recipe_machines,
                            item_recipe,
                            visited,
                            steps,
                            max_depth,
                        );
                    }
                }
            }
        }

        if let Some((machine_type, count)) = recipe_machines.get(recipe_name) {
            steps.push(ChainStep {
                recipe: recipe_name.to_string(),
                depth,
                machine_type: machine_type.clone(),
                machine_count: *count,
            });
        }
    }

    for root in &root_recipes {
        dfs(
            root,
            0,
            recipe_machines,
            &item_recipe,
            &mut visited,
            &mut steps,
            &mut max_depth,
        );
    }

    (steps, max_depth)
}

/// A named analysis result (from a single blueprint or one entry in a book).
#[derive(Debug, Clone, Serialize)]
pub struct NamedAnalysis {
    pub label: Option<String>,
    pub layout: LayoutResult,
    pub analysis: BlueprintAnalysis,
}

/// Parse a blueprint string and analyze it in one step.
pub fn analyze_blueprint_string(bp: &str) -> Result<(LayoutResult, BlueprintAnalysis), String> {
    let layout = blueprint_parser::parse_blueprint_string(bp)?;
    let analysis = analyze(&layout);
    Ok((layout, analysis))
}

/// Parse a blueprint string (single or book) and analyze all blueprints.
pub fn analyze_blueprint_string_any(bp: &str) -> Result<Vec<NamedAnalysis>, String> {
    let parsed = blueprint_parser::parse_blueprint_string_any(bp)?;
    Ok(parsed
        .into_iter()
        .map(|p| {
            let analysis = analyze(&p.layout);
            NamedAnalysis {
                label: p.label,
                layout: p.layout,
                analysis,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blueprint;
    use crate::models::{EntityDirection, PlacedEntity};

    fn make_test_layout() -> LayoutResult {
        LayoutResult {
            entities: vec![
                PlacedEntity {
                    name: "assembling-machine-2".into(),
                    x: 0,
                    y: 0,
                    direction: EntityDirection::North,
                    recipe: Some("iron-gear-wheel".into()),
                    ..Default::default()
                },
                PlacedEntity {
                    name: "assembling-machine-2".into(),
                    x: 4,
                    y: 0,
                    direction: EntityDirection::North,
                    recipe: Some("iron-gear-wheel".into()),
                    ..Default::default()
                },
                PlacedEntity {
                    name: "transport-belt".into(),
                    x: 3,
                    y: 0,
                    direction: EntityDirection::South,
                    ..Default::default()
                },
                PlacedEntity {
                    name: "transport-belt".into(),
                    x: 3,
                    y: 1,
                    direction: EntityDirection::South,
                    ..Default::default()
                },
                PlacedEntity {
                    name: "inserter".into(),
                    x: 3,
                    y: 2,
                    direction: EntityDirection::East,
                    ..Default::default()
                },
            ],
            width: 7,
            height: 3,
            ..Default::default()
        }
    }

    #[test]
    fn basic_entity_counts() {
        let analysis = analyze(&make_test_layout());
        assert_eq!(analysis.machine_count, 2);
        assert_eq!(analysis.belt_tiles, 2);
        assert_eq!(analysis.inserter_count, 1);
        assert_eq!(analysis.recipe_count, 1);
    }

    #[test]
    fn detects_final_product() {
        let analysis = analyze(&make_test_layout());
        assert!(analysis.final_products.contains(&"iron-gear-wheel".to_string()));
    }

    #[test]
    fn throughput_estimate() {
        let analysis = analyze(&make_test_layout());
        let gear_rate = analysis.throughput_estimates.get("iron-gear-wheel");
        assert!(gear_rate.is_some());
        assert!(*gear_rate.unwrap() > 0.0);
    }

    #[test]
    fn round_trip_analysis() {
        let layout = make_test_layout();
        let bp = blueprint::export(&layout, "test");
        let (_, analysis) = analyze_blueprint_string(&bp).unwrap();
        assert_eq!(analysis.machine_count, 2);
        assert!(analysis.final_products.contains(&"iron-gear-wheel".to_string()));
    }

    #[test]
    fn module_and_beacon_throughput() {
        use crate::models::ModuleItem;
        // 1 assembling-machine-3 with 4x productivity-module-3
        // 1 beacon with 2x speed-module-3, adjacent to the machine
        let layout = LayoutResult {
            entities: vec![
                PlacedEntity {
                    name: "assembling-machine-3".into(),
                    x: 0,
                    y: 0,
                    direction: EntityDirection::North,
                    recipe: Some("iron-gear-wheel".into()),
                    items: vec![ModuleItem {
                        item: "productivity-module-3".into(),
                        count: 4,
                    }],
                    ..Default::default()
                },
                PlacedEntity {
                    name: "beacon".into(),
                    x: 3,
                    y: 0,
                    direction: EntityDirection::North,
                    items: vec![ModuleItem {
                        item: "speed-module-3".into(),
                        count: 2,
                    }],
                    ..Default::default()
                },
            ],
            width: 6,
            height: 3,
            ..Default::default()
        };

        let analysis = analyze(&layout);
        assert_eq!(analysis.machine_count, 1);
        assert_eq!(analysis.beacon_count, 1);
        assert_eq!(analysis.recipe_count, 1);

        let group = &analysis.recipe_groups[0];
        assert_eq!(group.recipe, "iron-gear-wheel");

        // Internal: 4x prod-module-3 = speed: -0.60, prod: +0.40
        // Beacon: 2x speed-module-3 = speed: +1.0, × 0.5 effectivity = +0.50
        // Total speed mult = 1.0 - 0.60 + 0.50 = 0.90
        // Total prod mult = 1.0 + 0.40 = 1.40
        assert!(
            (group.effective_speed_multiplier - 0.90).abs() < 0.01,
            "speed mult should be ~0.90, got {}",
            group.effective_speed_multiplier
        );
        assert!(
            (group.productivity_bonus - 0.40).abs() < 0.01,
            "prod bonus should be ~0.40, got {}",
            group.productivity_bonus
        );

        // Base: 1 machine × 1.25 speed × 1 gear / 0.5s = 2.5/s
        // With modules: 2.5 × 0.90 × 1.40 = 3.15/s
        let gear_rate = analysis.throughput_estimates.get("iron-gear-wheel").unwrap();
        assert!(
            (*gear_rate - 3.15).abs() < 0.1,
            "expected ~3.15/s, got {}",
            gear_rate
        );
    }
}
