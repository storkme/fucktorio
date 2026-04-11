# RFP: General-Purpose SAT Crossing Solver for Bus Trunks

## Summary

Generalize the existing SAT crossing zone solver (`crates/core/src/sat.rs`) to handle arbitrary crossing configurations — multiple tap-offs crossing multiple trunk groups at varying heights, overlapping zones, and non-contiguous crossed columns. Replace the hand-rolled UG bridge patterns in `bus_router.rs` with SAT-solved entity placement.

## Problem

### Current approach

`extract_and_solve_crossings()` in `bus_router.rs` (line 2433) identifies crossing zones where horizontal tap-offs cross vertical trunk belts. For each zone:

1. It collects the trunk columns a tap-off crosses on the surface
2. Creates a 3-tile-high `CrossingZone` with trunk I/O boundaries
3. Calls `sat::solve_crossing_zone()` which finds a valid entity arrangement using Varisat (CDCL SAT solver)

The SAT solver itself (`crates/core/src/sat.rs`, ~835 lines) is already general-purpose — it handles:
- Surface belts, UG input/output entities
- 4-direction underground passage with max-reach constraints
- Item transport consistency across surface and underground channels
- Boundary port specifications (direction, item, input/output)
- Forced-empty tiles (no surface entities)

### Current limitations

1. **Fixed 3-tile height** (line 2517): `let zone_height: u32 = 3;` — the zone is always 3 tiles tall. This assumes one trunk row crossing one tap-off row. When multiple tap-offs cross at different y-positions, or when a trunk group spans multiple adjacent columns, the 3-tile window is too small.

2. **Per-tap-off zones, no merging**: Each tap-off at each y-position creates an independent zone. If two tap-offs at y=5 and y=7 both cross trunks at x=10 and x=12, we get two separate 3-tile zones instead of one merged zone covering y=4..=8. This means the SAT solver can't find solutions that coordinate between the two crossings.

3. **No overlapping zone handling**: Adjacent zones can't share tiles. When zones overlap, the second zone's `forced_empty` constraints conflict with the first zone's placed entities. The comment on line 2506 acknowledges this: `"For now, just solve each independently."`

4. **Contiguous columns only**: Zones span from `x_min` to `x_max` of crossed trunks, assuming all columns in between are part of the zone. If a trunk at x=15 is crossed but x=14 isn't a trunk, the zone still includes x=14 — wasting tiles and potentially conflicting with surface entities there.

5. **No multi-row crossings**: Current bus layout places tap-offs on single y-rows. As the bus scales (more items, wider trunk groups), tap-offs may need to route across trunk groups that span 3-4 columns, requiring wider zones with more complex internal routing.

6. **Silent fallback**: When SAT fails (`sat-unsolved`), the tap-off is silently skipped. The downstream `route_belt_lane()` falls back to hand-rolled UG bridges which don't account for the crossing entities placed by adjacent zones.

### Scalability impact

The bus width scales linearly with distinct item count. Each trunk lane occupies one column. Current layout:
- Tier 1 (`iron-gear-wheel`): 1 trunk → trivial
- Tier 3 (`plastic-bar`): 3-4 trunks → manageable
- Tier 5 (`processing-unit`): 15+ trunks → tap-offs cross many trunks, zones overlap

The 3-tile fixed height and per-tap-off isolation break down around 8-10 trunks because:
- Trunk groups (lanes for the same item split across multiple columns) can be 2-3 columns wide
- Tap-offs from different rows cross different subsets of trunks, creating overlapping zones
- The hand-rolled UG bridge fallback doesn't handle 4+ consecutive trunk crossings correctly

## Proposed Design

### 1. Variable-height zones

Replace the fixed 3-tile height with dynamic sizing:

```rust
/// Compute zone height from the tap-off positions and trunk crossings.
/// Minimum height: max_reach + 2 (input row, underground span, output row).
/// For grouped crossings: span from topmost tap-off - 1 to bottommost tap-off + max_reach.
fn compute_zone_height(
    tap_ys: &[i32],
    max_ug_reach: u32,
) -> u32 {
    if tap_ys.is_empty() {
        return 3;
    }
    let y_min = *tap_ys.iter().min().unwrap() - 1;
    let y_max = *tap_ys.iter().max().unwrap() + 1 + max_ug_reach as i32;
    (y_max - y_min + 1).max(3) as u32
}
```

### 2. Zone merging

Group overlapping or adjacent zones into merged mega-zones before solving:

```rust
struct CrossingGroup {
    /// All tap-offs in this group.
    taps: Vec<TapInfo>,
    /// All crossed trunk columns.
    trunks: Vec<TrunkInfo>,
    /// Bounding box of the merged zone.
    x_min: i32,
    x_max: i32,
    y_min: i32,
    y_max: i32,
}

fn merge_zones(zone_specs: &[ZoneSpec]) -> Vec<CrossingGroup> {
    // 1. Sort zones by y-position
    // 2. Greedily merge zones whose y-ranges overlap or are adjacent (±max_reach)
    // 3. For each merged group, collect all unique trunk columns and tap rows
    // 4. Compute the bounding box
}
```

Two zones should be merged when:
- They share a trunk column (both cross x=10), OR
- Their y-ranges overlap (zone A covers y=4..6, zone B covers y=6..8), OR
- They are within `max_ug_reach` tiles vertically (underground spans from one zone could interact with the other)

### 3. Multiple boundary ports per zone

The current `CrossingZone` already supports multiple `ZoneBoundary` entries. The solver handles them correctly — each boundary is a fixed I/O port. For merged zones, we simply add more boundaries:

```rust
// For each tap-off in the group:
for tap in &group.taps {
    boundaries.push(ZoneBoundary {
        x: tap.entry_x,
        y: tap.entry_y,
        direction: EntityDirection::East,
        item: tap.item.clone(),
        is_input: true,
    });
    boundaries.push(ZoneBoundary {
        x: tap.exit_x,
        y: tap.exit_y,
        direction: EntityDirection::East,
        item: tap.item.clone(),
        is_input: false,
    });
}

// For each crossed trunk column:
for trunk in &group.trunks {
    boundaries.push(ZoneBoundary {
        x: trunk.x,
        y: group.y_min,
        direction: EntityDirection::South,
        item: trunk.item.clone(),
        is_input: true,
    });
    boundaries.push(ZoneBoundary {
        x: trunk.x,
        y: group.y_max,
        direction: EntityDirection::South,
        item: trunk.item.clone(),
        is_input: false,
    });
}
```

### 4. Sparse column support

Instead of assuming all columns between x_min and x_max are part of the zone, mark non-trunk, non-tap columns as `forced_empty`:

```rust
for x in group.x_min..=group.x_max {
    if !group.trunks.iter().any(|t| t.x == x) {
        // This column is not a trunk — mark all its tiles as forced-empty
        // unless they're part of a tap-off path.
        for y in group.y_min..=group.y_max {
            if !group.taps.iter().any(|t| t.path_contains(x, y)) {
                forced_empty.push((x, y));
            }
        }
    }
}
```

### 5. Solver scaling analysis

The SAT solver's complexity scales with zone area and item count:

| Zone size | Items | Variables | Clauses | Solve time (est.) |
|-----------|-------|-----------|---------|-------------------|
| 3×3 | 2 | ~200 | ~2000 | <1ms |
| 5×3 | 2 | ~350 | ~3500 | <5ms |
| 10×5 | 4 | ~2000 | ~20000 | <50ms |
| 20×8 | 8 | ~8000 | ~80000 | <500ms |
| 40×10 | 15 | ~30000 | ~300000 | 1-5s |

For realistic bus layouts (15 items, zones up to 20×8), the solver should finish in under 1 second. Varisat is a CDCL solver with clause learning — it handles these sizes well.

If solver time becomes a bottleneck for very large zones:
- **Decompose**: Split mega-zones into sub-zones with shared boundaries (solve independently, then stitch)
- **Incremental solving**: Use Varisat's `solve_with_assumptions()` to warm-start from previous solutions
- **Timeout**: Set a solve budget (e.g., 2s per zone) and fall back to greedy UG placement

### 6. Replace hand-rolled UG bridges

The `route_belt_lane()` function (line 1693) currently uses `_blocked_xs_at()` to find trunk positions and places hand-rolled UG bridge patterns. After generalizing the SAT solver:

1. `extract_and_solve_crossings()` produces merged zones with SAT solutions
2. `route_belt_lane()` queries the SAT solution for entity placement in its crossing region
3. If no SAT solution exists for a crossing, use the greedy UG bridge as fallback
4. The greedy bridge is only for single-tap, single-trunk crossings (trivial cases)

## Implementation Plan

### Phase 1: Zone merging and variable height (non-breaking)

1. Add `CrossingGroup` struct and `merge_zones()` function
2. Modify `extract_and_solve_crossings()` to group zones before solving
3. Compute dynamic zone height from tap/trunk geometry
4. Pass merged zones to the existing SAT solver (already supports multiple boundaries)
5. Test: existing tier 1-3 recipes should produce identical layouts

### Phase 2: Sparse columns and forced-empty

1. Add column-sparseness logic to mark non-trunk/non-tap tiles as `forced_empty`
2. Verify SAT solver handles forced-empty correctly (it already does — just adding more entries)
3. Test: zones with gaps between trunk columns should solve correctly

### Phase 3: Replace hand-rolled UG bridges

1. Add a lookup from tap-off position to SAT solution entities
2. In `route_belt_lane()`, check for SAT solution before placing UG bridges
3. If SAT solution exists, use its entity placements instead of hand-rolled pattern
4. Keep hand-rolled bridge as fallback for unsolved zones
5. Test: all tier 1-3 recipes pass validation

### Phase 4: Scale testing

1. Generate bus layouts for tier 4+ recipes (advanced-circuit, processing-unit)
2. Measure SAT solver time per zone
3. Verify zero validation errors on SAT-solved crossings
4. Profile and optimize if needed (decomposition, timeout)

## Risks and Mitigations

| Risk | Mitigation |
|------|-----------|
| Large zones are slow to solve | Timeout + greedy fallback; decompose mega-zones |
| SAT solver returns UNSAT for valid crossings | Debug encoding; add missing constraints; fall back to hand-rolled |
| Merged zones over-constrain the solver | Allow flexible zone boundaries; don't force adjacent zones to merge if they don't share trunks |
| Solver results differ between runs | Varisat is deterministic for a given CNF; non-determinism only from zone construction order |
| WASM binary size increase from Varisat | Varisat is already included (used by balancer library); no new dependency |
| Regression on existing tier 1-3 layouts | Phase 1 is non-breaking; existing zone specs remain valid |

## Scope

- **In scope**: `crates/core/src/sat.rs` (minor changes), `crates/core/src/bus/bus_router.rs` (zone construction + merging), `route_belt_lane()` UG bridge replacement
- **Out of scope**: A* changes (separate RFP), balancer library, Python routing code
- **Dependencies**: None (can be done independently of the belt-flow-aware A* RFP)

## Success Criteria

1. SAT solver handles zones up to 20×8 with ≤8 items in <1 second
2. Merged zones produce correct entity placements for overlapping tap-offs
3. All tier 1-3 recipes pass validation with SAT-solved crossings
4. Tier 4 recipe (advanced-circuit from plates) has fewer lane-throughput warnings
5. No regression in bus layout quality for existing recipes
