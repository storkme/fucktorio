# SAT encoder: can we make it work for band-sized zones?

**Status**: Investigation closed 2026-04-13. Answer: **No, not at useful performance**.
**Related**: [`docs/rfp-band-regions.md`](rfp-band-regions.md), [`docs/rfp-junction-solver.md`](rfp-junction-solver.md)

## What we were trying to do

The band-regions RFP proposed replacing the three-phase ghost-crossing pipeline (corridor templates + per-tile templates + SAT clusters) with a single pass: one SAT region per horizontal band. The motivation was structural: every crossing in tier2/tier4 sits on a row's horizontal belt line, so bands partition crossings cleanly by geometry and eliminate the 22 overlap pairs the current pipeline produces.

Before committing to the refactor we ran three gate checks. Gates 1 and 2 passed. Gate 3 — SAT performance — passed with a synthetic benchmark showing ~14 ms per tier2 band and ~40 ms per tier4 band. Looked great.

Then we actually checked whether the SAT solutions were *correct*.

## The finding that killed it

The existing SAT encoder in `sat.rs` enforces only **local** constraints:

- Type/direction mutual exclusion per tile
- Adjacency (belt outputs `d` → neighbor not empty, has compatible type/direction)
- Item propagation along flow edges (belt-out-d + item → neighbor item matches)
- Underground pairing and propagation
- Boundary ports fix item bits at port tiles

There is no constraint that items must actually flow from an input port to a matching output port. The encoder allows:

- **Phantom belt chains**: a belt carrying item X that is not reached from any input port.
- **Cycles**: a length-4+ belt cycle where each tile has item X and its neighbor has item X, locally consistent but globally disconnected from any source.

### The existing encoder also had a bug

`encode_item_transport` had clauses for `belt → surface neighbor`, `ug_in → underground channel`, `underground → underground`, and `underground → ug_out surface (same tile)`, but **no clause for `ug_out → surface neighbor`**. A UG-out's own surface item was constrained by the underground channel, but the downstream belt's item was unconstrained. This bug is present in the main-branch encoder as of commit `5936e6e`.

Fix at `encode_item_transport`: add the same two clauses as the `is_belt` case but with `is_ug_out`. Three lines each direction. **This fix lands regardless of the rest of the investigation.**

Impact on production: tier2 ghost scoreboard drops from 39 errors to 38 errors. Tier4 unchanged. No regressions.

### Why production gets away with it today

All production SAT calls in ghost cluster resolution are 5×5 single-tile-crossing clusters with heavy `forced_empty` constraints from `Occupancy`. Most interior tiles are forced to be empty because of machines, poles, or pre-existing belts. The solver can't construct cyclic phantom structures because the cells required for cycles are forced empty. The encoder's weakness is hidden by geometric constraint density, not by any property of the encoder itself.

Band regions are the opposite: 90×5 with only a few forced_empty tiles. Plenty of room for cycles. That's where the weakness surfaces.

## What we tried

### Experiment 1 — predecessor existence + 4-cycle ban

Added a `pred[t]` aux var per tile, plus per-direction `inflow[t][d]` aux vars wiring `pred[t] ↔ ∃d: (neighbor at −dir_delta(d)).is_belt ∨ is_ug_out) ∧ out_dir[d]`. Required `is_belt ∨ is_ug_in → pred[t]` for all non-input-port tiles. Added explicit ban on clockwise/counter-clockwise 2×2 four-belt cycles.

Cost: ~450 new vars + ~4000 new clauses on a tier2 band. 5×5 synthetic test went from 350 vars / 2444 clauses / 297 µs to 475 vars / 3127 clauses / 355 µs (+20%).

Result on synthetic benchmark:

| Case | Before fixes | With pred + 4-cycle + ug_out |
|---|---|---|
| 5×5 | INVALID (cycle via UG) | **VALID** |
| 9×5 | INVALID | **VALID** |
| 7×5 | INVALID | INVALID (longer cycle using UG teleport) |
| 11×5 | INVALID | INVALID |
| 15×5 | INVALID | INVALID |
| 21×5 | INVALID | INVALID |

Result on band-sized zones (the actual target):

| Scenario | Before | With constraints | Issues remaining |
|---|---|---|---|
| 30×5 | INVALID (6) @ 4 ms | INVALID (2) @ 9 ms | 2 cycles |
| 90×5 tier2 | INVALID (10) @ 14 ms | INVALID (5) @ **159 ms** | 5 cycles |
| 90×9 tier4 merged | INVALID (14) @ 41 ms | INVALID (12) @ **8.5 s** | 12 cycles |
| 124×9 stress | INVALID (23) @ 86 ms | INVALID (10) @ **48.9 s** | 10 cycles |

**Two independent failures of the approach**:

1. **Correctness**: constraints catch phantom belts and length-4 cycles, but cycles of length 6+ (especially those using UG-in/UG-out teleport pairs) slip through. Real bands still produce invalid solutions.
2. **Performance**: adding partial reachability constraints makes the SAT search much harder without narrowing it enough. Solve time scales catastrophically — the stress scenario went from 86 ms to 48.9 seconds. This is unusable regardless of correctness.

### Why it scales this badly

CDCL SAT solvers are good at problems where unit propagation quickly narrows the search space. Adding aux-var reachability constraints (`pred` and `inflow`) creates long propagation chains without pruning early — the solver explores many more backtracks per conflict. For small zones the added variables are absorbed into the existing constraint tightness. For large zones with few `forced_empty` tiles, the solver has room to thrash.

Unary-depth encoding (the only thing that would actually prevent all cycles) would add O(width × height × max_depth) variables, making an already-hard problem significantly harder. We didn't implement it, but based on the scaling observed with partial constraints, it would likely time out or take minutes per band.

## Conclusion

The SAT encoder was designed for small, heavily-constrained crossing zones, and the existing production use relies on `forced_empty` density for correctness rather than on the encoder itself being sound. For band-sized zones the encoder is not viable, and strengthening it within the SAT framework is either insufficient (partial constraints) or too expensive (unary depth).

**Verdict**: SAT is not the right tool for band-sized crossing resolution. The right direction is the junction solver (`docs/rfp-junction-solver.md`): deterministic templates for common patterns, with SAT reserved as a last-resort fallback for genuinely complex clusters — the cases where today's small-zone reliance on `forced_empty` is already enough.

## What lands from this investigation

1. **The `ug_out → surface neighbor` item propagation fix in `encode_item_transport`** — real bug, strict correctness improvement, tier2 ghost improved by 1 error, no regressions. Lands.
2. **Two ignored test cases in `sat::tests`**: `band_regions_sat_bench` and `validate_existing_small_tests`. These carry the trace-validator helpers (`validate_band_solution`, `trace_flow`, `render_band_solution`, `render_band_with_items`). They stay in the codebase as regression guards and as a way to re-run the investigation if future encoder changes need validation. Both are `#[ignore]` so they don't run in CI by default.
3. **This document** — closes the investigation and points at the junction solver RFP.

## What does not land

- `pred[t]` / `inflow[t][d]` aux vars and the `encode_flow_constraints` method.
- Per-tile predecessor constraint.
- Explicit 4-cycle ban.
- The band-regions RFP's "SAT is fast enough" conclusion (see doc update).

The investigation code is reverted in the encoder. The `band_regions_sat_bench` test will fail (as invalid) if re-run against a band-sized zone — that's the intended state. The test exists to *prove* that SAT alone isn't sufficient for bands, not to pass.
