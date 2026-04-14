# RFP: Junction Solver

## Summary

Replace the current "throw everything at SAT" approach in ghost cluster
resolution with a **strategy-based junction solver** that uses
deterministic templates for common crossing patterns and reserves SAT for
genuinely complex multi-path clusters.

Phase 3 of ghost-cluster routing proved that cluster detection, boundary
extraction, and SAT solving work end-to-end. But the SAT solver produces
poor-quality output (belt loops, item isolation, dead ends) when zones
straddle the bus/machine-row boundary. The root cause is that the
5x5 zones give the solver too much freedom — it fills unconstrained tiles
with locally-valid but globally-harmful entities.

The insight: most crossings have a trivially simple shape. One horizontal
path crosses one vertical path. You UG-bridge one of them. You don't
need a general-purpose SAT solver for that.

## Motivation

From the Phase 3 investigation on `tier4_advanced_circuit_from_ore_am1`:

- 18 ghost clusters detected, all SAT-solved
- 93 validator errors remain (78 belt-related, 15 fluid)
- ASCII visualisation of zone (29,73):

```
     28 29 30 31 32 33 34
 73:  .  · ↓C U↓ →C ↓C  .     SAT fills a 5x5 zone
 74:  ▾  · →C →C ↑C ←C  .     2x2 loop at (31-32,73-74)
 75:  → →C ↑C U↑  ▸  ▸  ▸     row template belts at x=32+
 76:  .  ·  · →C ↑C  ⊥  .     inserter
 77:  .  ·  · ↓P ███ ·  .     machine
```

The crossing is one tile where copper-plate (East) meets plastic-bar
(South). The correct solution is a 3-tile UG bridge:

```
 74:  →C →C →C      copper-plate continues East on surface
 75:  .  U↓  .      plastic-bar goes underground
```

Instead, the SAT solver gets a 5x5 zone, the right edge hits a machine
row, the East boundary port gets filtered, and the solver creates a loop
to park the copper-plate items that have nowhere to go.

## Design

### Junction solver interface

```rust
pub struct Junction {
    /// Bounding box in world coordinates.
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    /// Boundary ports (same as current ZoneBoundary).
    pub boundaries: Vec<ZoneBoundary>,
    /// Tiles inside the junction that must stay empty.
    pub forced_empty: Vec<(i32, i32)>,
}

pub struct JunctionSolution {
    pub entities: Vec<PlacedEntity>,
    pub strategy: &'static str,  // which strategy solved it
}

pub fn solve_junction(
    junction: &Junction,
    max_ug_reach: u32,
    belt_tier: &str,
) -> Option<JunctionSolution>;
```

### Strategy pipeline

`solve_junction` tries strategies in order, returning the first success:

1. **Template match** — check if the crossing pattern matches a known
   shape. If so, stamp a pre-computed entity list. O(1), always correct.

2. **Inline UG bridge** — for simple perpendicular crossings (one path
   crosses another at one tile), compute the UG entry/exit positions
   deterministically. O(1), always correct.

3. **SAT fallback** — for complex multi-path clusters that don't match
   any template. Uses the existing `sat::solve_crossing_zone`. O(SAT),
   may produce suboptimal output.

### Template catalogue

Based on the crossing patterns observed in the Phase 3 data:

#### T1: Perpendicular crossing (horizontal meets vertical)

The most common case. One path goes East/West, another goes
North/South. They share exactly one tile.

```
Zone: 3 tiles in the bridged direction, 1 tile in the other.

Before:            After (bridge vertical):
  . ↓ .              . U↓ .
  → X →              → → →
  . ↓ .              . U↑ .
```

The bridged path gets UG-in above the crossing and UG-out below (or
vice versa for bridging the horizontal). Choice of which path to bridge
can prefer the one with more room (further from hard obstacles).

Boundary ports: 4 (one entry + exit per path).
Zone size: 3x1 or 1x3 depending on bridge orientation.

#### T2: Same-direction different-item overlap

Two paths go East at the same y, carrying different items. One needs
to go underground past the other.

```
Before:            After (bridge item B):
  →A →A →A           →A  →A  →A
  →B →B →B           U↓B  .  U↑B
```

Zone size: 1×N where N = overlap length. UG bridge at the start and
end of the overlap.

#### T3: Multi-path cluster

Three or more paths cross in the same area. No simple template.
Fall back to SAT.

### Junction sizing

The key improvement over the current approach: **junction zones are
sized to the strategy, not padded uniformly.**

- Template strategies produce their own bbox (tight, no padding).
- Inline UG bridge computes the exact tiles it needs.
- SAT fallback uses the current padding logic, but only for the rare
  cases that actually need it.

This eliminates the "5x5 zone straddling the machine row" problem
because most crossings get a 3x1 or 1x3 zone that stays well within
the bus.

### Choosing which path to bridge

When two paths cross, one goes on the surface and the other goes
underground. The choice matters:

- **Prefer bridging the path that has more room.** If one direction has
  hard obstacles 2 tiles away, bridge the other direction.
- **Prefer bridging the path with fewer crossings.** If path A crosses
  3 trunks and path B crosses 1, bridge path B (fewer UG pairs).
- **Prefer bridging the vertical path** when both have equal room,
  since horizontal paths connect to row inputs and must stay on the
  surface for inserter access.

### Integration with ghost router

`resolve_clusters` currently builds a `CrossingZone` and calls
`sat::solve_crossing_zone` for every cluster. The change:

1. Classify the cluster by its crossing pattern (how many paths, their
   directions, the overlap shape).
2. Try template match and inline UG bridge first.
3. Fall back to SAT for complex clusters.

The entity filtering and replacement logic stays the same — the
junction solver returns `Vec<PlacedEntity>` just like the SAT solver.

## Optimisation: exhaustive SAT search

The current SAT solver finds *a* satisfying assignment, not the best
one. For small zones, we could find *optimal* solutions:

- **Minimize entity count**: add a cardinality constraint, solve, if
  SAT, tighten, repeat. Binary search on entity count.
- **Minimize UG pairs**: same approach on UG-in count.
- **Penalise direction changes**: add soft clauses (MaxSAT) to prefer
  straight runs.

This matters more for the SAT fallback path (complex clusters) than
for templates (which are already optimal by construction). Worth doing
after correctness is solid.

An alternative to iterative tightening: dump the solution, extract its
structure, and check whether a simpler template applies post-hoc. If
the SAT solution is a simple UG bridge, record that pattern and add it
to the template catalogue.

## Phasing

### Phase A — Template solver for perpendicular crossings

Implement T1 (perpendicular crossing template). Classify each ghost
cluster: if it has exactly 2 paths crossing at a single tile in
perpendicular directions, use the template. Everything else falls
through to the existing SAT solver.

Expected impact: ~12 of 18 clusters solved by template (the x=29 zones
are all perpendicular crossings), eliminating their loops and item
isolation errors. Remaining ~6 clusters (the larger multi-path ones)
stay on SAT.

### Phase B — Same-direction overlap template

Implement T2 for same-direction different-item overlaps. These are less
common but appear in the feeder/return paths.

### Phase C — SAT quality improvements

With most crossings handled by templates, the SAT fallback only handles
genuinely complex cases. At that point, adding anti-loop constraints
and exhaustive search is more tractable (fewer zones, each one
actually needs SAT).

### Phase D — Template learning

Run the SAT solver on small zones, inspect the output, and
automatically extract recurring patterns into new templates. Grows the
catalogue without manual effort.

## Investigation findings (2026-04-12)

### What the remaining crossing tiles actually are

After the perpendicular template handles the simple cases (15 zones,
all 1×3), **141 crossing tiles** remain. Diagnostic classification:

| Specs at tile | Count | Example location |
|---|---|---|
| 3 | 43 | (6,120) — mid-bus dense area |
| 4 | 6 | (4,196) — output merger |
| 5 | 47 | (31,184) — output merger |
| 6 | 4 | (31,168) |
| 7 | 15 | (11,196) — output merger |
| 8 | 20 | (30,196) — output merger |
| 2 (same-direction) | 12 | (11,119) — anti-parallel |

These are NOT point crossings. They are tiles where 3–8 ghost-routed
specs share the same position. They cluster in two areas:

1. **Output merger zone** (y≈178–196): many product belts converge
   toward the south of the layout. Every tile in this area is shared
   by many specs.

2. **Mid-bus dense area** (x≈3–11, y≈108–120): return paths from
   multiple machine rows cross through each other.

### Why these create giant SAT zones

The crossing tiles form long connected chains (same x column, many y
values) because the overlap extends for the entire length of a spec's
path. For example:

- `ret:plastic-bar:2:18`: path length 233, **100 crossings**
- `ret:electronic-circuit:4:144`: path length 95, **93 crossings**

These paths cross through every other spec at the same y-row for their
entire horizontal run. Every tile of overlap is a "crossing," so the
crossing set forms continuous ribbons. With merge distance
ug\_max\_reach+1, these ribbons chain into zones like the 5×21
(29,135) and the 33×21 (1,178).

### The SAT zones' boundary problem

The resulting SAT zones span 20+ rows and their edges inevitably hit
machine rows. At the zone boundary:

- Input ports get created where ghost paths enter the zone
- Output ports get **skipped** where paths exit toward machine rows
  (the `occupied_by_existing` filter)
- The SAT solver receives 4 inputs and 1 output → unbalanced flow
- It creates loops and dead-ends to absorb the excess items

This is not a SAT solver quality issue. The solver is producing the
best possible output given fundamentally broken boundary conditions.
No amount of SAT constraint tuning can fix missing output ports.

### The real shape of the problem

The remaining crossings are not "two paths cross at a point." They are
**long parallel or anti-parallel overlaps** where many specs share the
same tiles for extended runs. The crossing set treats each shared tile
as independent, but the correct resolution operates at the spec-run
level: bridge one spec underground for an entire run, not per-tile.

The per-tile template approach (T1 perpendicular) works beautifully
for actual point crossings. But the 141 remaining tiles need a
different decomposition — one that reasons about spec runs, not
individual tiles.

## Related

- [`docs/rfp-ghost-cluster-routing.md`](rfp-ghost-cluster-routing.md) —
  the ghost-cluster routing rewrite that this builds on.
- [`crates/core/src/sat.rs`](../crates/core/src/sat.rs) — the SAT
  crossing solver that becomes the fallback strategy.
- [`crates/core/src/bus/ghost_router.rs`](../crates/core/src/bus/ghost_router.rs) —
  `resolve_clusters` is where the junction solver plugs in.
- [#138](https://github.com/storkme/fucktorio/issues/138) �� SAT solver
  optimisation backlog (memoisation, bifurcation, warm-start).
