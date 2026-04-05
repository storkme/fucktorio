//! Recipe and entity lookups backed by a bundled `recipes.json`.

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

/// Raw JSON payload bundled at compile time.
static RAW: &str = include_str!("../data/recipes.json");

/// Default crafting time when not specified in data.
const DEFAULT_ENERGY: f64 = 0.5;

/// Recipe categories that should never be used for production chains.
const EXCLUDED_CATEGORIES: &[&str] = &["recycling", "crushing", "recycling-or-hand-crafting"];

fn default_ingredient_type() -> String {
    "item".to_string()
}

fn default_probability() -> f64 {
    1.0
}

fn default_category() -> String {
    "crafting".to_string()
}

fn default_energy() -> f64 {
    DEFAULT_ENERGY
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ingredient {
    pub name: String,
    pub amount: f64,
    #[serde(default = "default_ingredient_type", rename = "type")]
    pub type_: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Product {
    pub name: String,
    pub amount: f64,
    #[serde(default = "default_ingredient_type", rename = "type")]
    pub type_: String,
    #[serde(default = "default_probability")]
    pub probability: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recipe {
    pub name: String,
    #[serde(default = "default_category")]
    pub category: String,
    #[serde(default = "default_energy", alias = "energy_required")]
    pub energy: f64,
    #[serde(default)]
    pub ingredients: Vec<Ingredient>,
    #[serde(default, alias = "results")]
    pub products: Vec<Product>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineData {
    pub crafting_speed: f64,
}

#[derive(Debug, Deserialize)]
struct RawRoot {
    recipes: FxHashMap<String, Recipe>,
    #[serde(default)]
    machines: FxHashMap<String, MachineData>,
}

#[derive(Debug)]
pub struct RecipeDb {
    pub recipes: FxHashMap<String, Recipe>,
    pub machines: FxHashMap<String, MachineData>,
}

static DB: LazyLock<RecipeDb> = LazyLock::new(|| {
    let root: RawRoot = serde_json::from_str(RAW).expect("recipes.json is malformed");
    RecipeDb {
        recipes: root.recipes,
        machines: root.machines,
    }
});

/// Global recipe database (lazily parsed from the bundled JSON).
pub fn db() -> &'static RecipeDb {
    &DB
}

/// Find the first recipe whose products include *item*.
///
/// Skips recycling/crushing recipes. Iteration order is unspecified but stable
/// per run (FxHashMap ordering). Returns `None` if no recipe produces this item.
pub fn find_recipe_for_item(item: &str) -> Option<&'static Recipe> {
    for recipe in db().recipes.values() {
        if EXCLUDED_CATEGORIES.contains(&recipe.category.as_str()) {
            continue;
        }
        if recipe.products.iter().any(|p| p.name == item) {
            return Some(recipe);
        }
    }
    None
}

/// Return the crafting_speed of an entity, defaulting to 1.0 if unknown.
pub fn get_crafting_speed(entity: &str) -> f64 {
    db()
        .machines
        .get(entity)
        .map(|m| m.crafting_speed)
        .unwrap_or(1.0)
}

/// Choose the right machine entity for a recipe based on its category.
pub fn machine_for_recipe<'a>(recipe: &Recipe, default: &'a str) -> &'a str {
    match recipe.category.as_str() {
        "chemistry" | "chemistry-or-cryogenics" | "organic-or-chemistry" => "chemical-plant",
        "oil-processing" => "oil-refinery",
        "smelting" => "electric-furnace",
        _ => default,
    }
}

/// List all items that at least one non-excluded recipe produces.
pub fn all_producible_items() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut seen: rustc_hash::FxHashSet<&str> = rustc_hash::FxHashSet::default();
    for recipe in db().recipes.values() {
        if EXCLUDED_CATEGORIES.contains(&recipe.category.as_str()) {
            continue;
        }
        for p in &recipe.products {
            if seen.insert(p.name.as_str()) {
                out.push(p.name.clone());
            }
        }
    }
    out.sort();
    out
}

/// List all machine entity names known to the database.
pub fn all_producer_machines() -> Vec<String> {
    let mut out: Vec<String> = db().machines.keys().cloned().collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_iron_gear_wheel_recipe() {
        let recipe = find_recipe_for_item("iron-gear-wheel").expect("recipe exists");
        assert_eq!(recipe.name, "iron-gear-wheel");
        let iron_plate_ings: Vec<&Ingredient> = recipe
            .ingredients
            .iter()
            .filter(|i| i.name == "iron-plate")
            .collect();
        assert_eq!(iron_plate_ings.len(), 1);
        assert_eq!(iron_plate_ings[0].amount, 2.0);
    }

    #[test]
    fn crafting_speed_defaults_to_one() {
        assert_eq!(get_crafting_speed("nonexistent-machine"), 1.0);
        assert!(get_crafting_speed("assembling-machine-3") > 0.0);
    }
}
