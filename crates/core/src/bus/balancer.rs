//! Balancer block stamping for `LaneFamily` blocks.
//!
//! Given a planned `LaneFamily` (an N→M producer-to-trunk balancer
//! requirement), pick the right template from `balancer_library` and
//! stamp it into a `Vec<PlacedEntity>`. Falls back to template
//! decomposition if no direct (N, M) template exists. Returns an empty
//! vec if neither path finds a template — `layout.rs` then surfaces
//! a missing-balancer warning.
//!
//! Also exports the small `splitter_for_belt` / `underground_for_belt`
//! lookup helpers since the balancer needs them and so does `render_path`
//! in `bus_router.rs`.

use crate::models::PlacedEntity;
use crate::bus::bus_router::LaneFamily;

/// Splitter name mapping by belt tier.
const SPLITTER_MAP: &[(&str, &str)] = &[
    ("transport-belt", "splitter"),
    ("fast-transport-belt", "fast-splitter"),
    ("express-transport-belt", "express-splitter"),
];

/// Underground belt name mapping by belt tier.
const UNDERGROUND_MAP: &[(&str, &str)] = &[
    ("transport-belt", "underground-belt"),
    ("fast-transport-belt", "fast-underground-belt"),
    ("express-transport-belt", "express-underground-belt"),
];

pub(crate) fn splitter_for_belt(belt: &str) -> &'static str {
    SPLITTER_MAP.iter()
        .find(|(b, _)| *b == belt)
        .map(|(_, s)| *s)
        .unwrap_or("splitter")
}

pub(crate) fn underground_for_belt(belt: &str) -> &'static str {
    UNDERGROUND_MAP.iter()
        .find(|(b, _)| *b == belt)
        .map(|(_, u)| *u)
        .unwrap_or("underground-belt")
}

/// Stamp a balancer template at the family's origin position.
///
/// Template entity tiles are offset by the family's stamp origin
/// (x = min(lane_xs), y = balancer_y_start). The item each entity
/// carries is set to the family's item. Belt and splitter tiers are
/// chosen from the family's total rate so the balancer matches its
/// sibling trunks.
pub(crate) fn stamp_family_balancer(
    family: &LaneFamily,
    max_belt_tier: Option<&str>,
) -> Result<Vec<PlacedEntity>, String> {
    use crate::bus::balancer_library::balancer_templates;
    use crate::common::belt_entity_for_rate;

    let templates = balancer_templates();
    let (n, m) = (family.shape.0 as u32, family.shape.1 as u32);
    let template_key = (n, m);

    if family.lane_xs.is_empty() {
        return Err(format!("LaneFamily for item {} has no lane_xs assigned", family.item));
    }

    let belt_tier = belt_entity_for_rate(family.total_rate, max_belt_tier);
    let splitter_name = splitter_for_belt(belt_tier);
    let ug_name = underground_for_belt(belt_tier);
    let balancer_seg_id = Some(format!("balancer:{}", family.item));

    if let Some(template) = templates.get(&template_key) {
        // Direct template match.
        let origin_x = *family.lane_xs.iter().min().unwrap();
        let origin_y = family.balancer_y_start;

        let mut entities = template.stamp(
            origin_x, origin_y, belt_tier, splitter_name, ug_name,
            Some(&family.item),
        );
        for ent in &mut entities {
            ent.segment_id = balancer_seg_id.clone();
        }
        return Ok(entities);
    }

    // Decomposition fallback: try to split (N, M) into groups that have
    // templates. Search for a divisor g of N where (N/g, M/g) has a
    // template. E.g., (6,8) → g=2 → 2 copies of (3,4). (5,10) → g=5 →
    // 5 copies of (1,2).
    for g in (1..=n).rev() {
        if n % g != 0 || m % g != 0 {
            continue;
        }
        let sub_n = n / g;
        let sub_m = m / g;
        if let Some(sub_template) = templates.get(&(sub_n, sub_m)) {
            let mut all_entities = Vec::new();
            let lanes_per_group = sub_m as usize;

            for gi in 0..(g as usize) {
                let lane_start = gi * lanes_per_group;
                let lane_end = (lane_start + lanes_per_group).min(family.lane_xs.len());
                let lane_chunk = &family.lane_xs[lane_start..lane_end];
                if lane_chunk.is_empty() {
                    continue;
                }
                let sub_origin_x = *lane_chunk.iter().min().unwrap();
                let sub_origin_y = family.balancer_y_start;

                let mut ents = sub_template.stamp(
                    sub_origin_x, sub_origin_y, belt_tier, splitter_name, ug_name,
                    Some(&family.item),
                );
                for ent in &mut ents {
                    ent.segment_id = Some(format!("balancer:{}:{}", family.item, gi));
                }
                all_entities.extend(ents);
            }
            return Ok(all_entities);
        }
    }

    // No template and no decomposition possible — skip.
    Ok(Vec::new())
}
