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

## Related

- [`docs/rfp-ghost-cluster-routing.md`](rfp-ghost-cluster-routing.md) —
  the ghost-cluster routing rewrite that this builds on.
- [`crates/core/src/sat.rs`](../crates/core/src/sat.rs) — the SAT
  crossing solver that becomes the fallback strategy.
- [`crates/core/src/bus/ghost_router.rs`](../crates/core/src/bus/ghost_router.rs) —
  `resolve_clusters` is where the junction solver plugs in.
- [#138](https://github.com/storkme/fucktorio/issues/138) �� SAT solver
  optimisation backlog (memoisation, bifurcation, warm-start).
