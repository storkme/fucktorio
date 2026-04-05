//! Recursive recipe solver: target item/rate → machine counts & flows.

use crate::models::{ItemFlow, MachineSpec, SolverResult};
use crate::recipe_db::{find_recipe_for_item_excluding, get_crafting_speed, machine_for_recipe};
use rustc_hash::FxHashSet;

#[derive(Debug, thiserror::Error)]
pub enum SolverError {
    #[error("recipe {recipe} produces 0 of {item}")]
    ZeroProduct { recipe: String, item: String },
    #[error("no crafting speed for entity {entity}")]
    MissingCraftingSpeed { entity: String },
}

struct SolveState {
    machines: Vec<MachineSpec>,
    external_inputs: Vec<ItemFlow>, // keep insertion order, small N
    dependency_order: Vec<String>,
    resolving: FxHashSet<String>,
}

impl SolveState {
    fn add_external(&mut self, item: &str, rate: f64, is_fluid: bool) {
        if let Some(flow) = self.external_inputs.iter_mut().find(|f| f.item == item) {
            flow.rate += rate;
            flow.is_fluid = is_fluid;
        } else {
            self.external_inputs.push(ItemFlow {
                item: item.to_string(),
                rate,
                is_fluid,
            });
        }
    }
}

/// Compute machines needed to produce `target_item` at `target_rate` items/sec.
///
/// Recursively resolves intermediate recipes until hitting items in
/// `available_inputs` (which the caller must supply externally).
pub fn solve(
    target_item: &str,
    target_rate: f64,
    available_inputs: &FxHashSet<String>,
    machine_entity: &str,
) -> Result<SolverResult, SolverError> {
    solve_with_exclusions(
        target_item,
        target_rate,
        available_inputs,
        machine_entity,
        &FxHashSet::default(),
    )
}

/// Like [`solve`] but skips recipes listed in `excluded_recipes`.
///
/// Useful when several recipes produce the same item and the caller wants to
/// steer the solver away from some of them (e.g. exclude `coal-liquefaction`
/// to avoid pulling in the whole oil chain for `plastic-bar`).
pub fn solve_with_exclusions(
    target_item: &str,
    target_rate: f64,
    available_inputs: &FxHashSet<String>,
    machine_entity: &str,
    excluded_recipes: &FxHashSet<String>,
) -> Result<SolverResult, SolverError> {
    let mut state = SolveState {
        machines: Vec::new(),
        external_inputs: Vec::new(),
        dependency_order: Vec::new(),
        resolving: FxHashSet::default(),
    };

    resolve(
        target_item,
        target_rate,
        false,
        available_inputs,
        machine_entity,
        excluded_recipes,
        &mut state,
    )?;

    Ok(SolverResult {
        machines: state.machines,
        external_inputs: state.external_inputs,
        external_outputs: vec![ItemFlow {
            item: target_item.to_string(),
            rate: target_rate,
            is_fluid: false,
        }],
        dependency_order: state.dependency_order,
    })
}

fn resolve(
    item: &str,
    rate: f64,
    is_fluid: bool,
    available_inputs: &FxHashSet<String>,
    machine_entity: &str,
    excluded_recipes: &FxHashSet<String>,
    state: &mut SolveState,
) -> Result<(), SolverError> {
    if available_inputs.contains(item) {
        state.add_external(item, rate, is_fluid);
        return Ok(());
    }

    let recipe = match find_recipe_for_item_excluding(item, excluded_recipes) {
        Some(r) => r,
        None => {
            state.add_external(item, rate, is_fluid);
            return Ok(());
        }
    };

    if state.resolving.contains(item) {
        state.add_external(item, rate, is_fluid);
        return Ok(());
    }

    state.resolving.insert(item.to_string());

    let entity = machine_for_recipe(recipe, machine_entity);
    let crafting_speed = get_crafting_speed(&entity);
    if crafting_speed <= 0.0 {
        return Err(SolverError::MissingCraftingSpeed {
            entity: entity.clone(),
        });
    }

    let products_per_craft: f64 = recipe
        .products
        .iter()
        .filter(|p| p.name == item)
        .map(|p| p.amount * p.probability)
        .sum();

    if products_per_craft <= 0.0 {
        return Err(SolverError::ZeroProduct {
            recipe: recipe.name.clone(),
            item: item.to_string(),
        });
    }

    let crafts_per_sec = crafting_speed / recipe.energy;
    let items_per_sec_per_machine = crafts_per_sec * products_per_craft;
    let count = rate / items_per_sec_per_machine;

    let input_flows: Vec<ItemFlow> = recipe
        .ingredients
        .iter()
        .map(|ing| ItemFlow {
            item: ing.name.clone(),
            rate: ing.amount * crafts_per_sec,
            is_fluid: ing.type_ == "fluid",
        })
        .collect();

    let output_flows: Vec<ItemFlow> = recipe
        .products
        .iter()
        .map(|prod| ItemFlow {
            item: prod.name.clone(),
            rate: prod.amount * prod.probability * crafts_per_sec,
            is_fluid: prod.type_ == "fluid",
        })
        .collect();

    if let Some(existing) = state.machines.iter_mut().find(|m| m.recipe == recipe.name) {
        existing.count += count;
    } else {
        state.machines.push(MachineSpec {
            entity,
            recipe: recipe.name.clone(),
            count,
            inputs: input_flows,
            outputs: output_flows,
        });
        state.dependency_order.push(recipe.name.clone());
    }

    for ing in &recipe.ingredients {
        let ingredient_rate = ing.amount * crafts_per_sec * count;
        let ing_is_fluid = ing.type_ == "fluid";
        resolve(
            &ing.name,
            ingredient_rate,
            ing_is_fluid,
            available_inputs,
            machine_entity,
            excluded_recipes,
            state,
        )?;
    }

    state.resolving.remove(item);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs_of(items: &[&str]) -> FxHashSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn solves_iron_gear_wheel() {
        let available = inputs_of(&["iron-plate"]);
        let result = solve("iron-gear-wheel", 10.0, &available, "assembling-machine-3").unwrap();

        assert_eq!(result.machines.len(), 1);
        let m = &result.machines[0];
        assert_eq!(m.recipe, "iron-gear-wheel");
        // asm3 crafting_speed=1.25, recipe energy=0.5 → 2.5 crafts/s/machine
        // 10/s target ÷ 2.5 = 4.0 machines
        assert!(
            (m.count - 4.0).abs() < 0.01,
            "expected count ≈ 4.0, got {}",
            m.count
        );

        let iron = result
            .external_inputs
            .iter()
            .find(|f| f.item == "iron-plate")
            .expect("iron-plate in external inputs");
        assert!(
            (iron.rate - 20.0).abs() < 0.01,
            "expected iron-plate rate ≈ 20.0, got {}",
            iron.rate
        );

        assert_eq!(result.external_outputs.len(), 1);
        assert_eq!(result.external_outputs[0].item, "iron-gear-wheel");
        assert_eq!(result.external_outputs[0].rate, 10.0);
    }
}
