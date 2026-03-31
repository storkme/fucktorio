# Roadmap: From Tier 1 to Tier 4

Status as of 2026-03-31. Tier 1 (iron-gear-wheel) is solved. Everything else fails.

## Phase 0: Cleanup & Foundation (low effort, high clarity)

**Goal:** Remove noise, re-enable feedback loops, establish a clean baseline.

### 0.1 Delete dead evolution code
- **File:** `src/search/layout_search.py`
- **What:** Remove `_mutate()` (~50 lines), `_perturb_positions()` (~26 lines), `_random_edge_order()` (~5 lines). Remove unused `survivors` and `generations` parameters from `evolutionary_layout()`. Rename function to `random_search_layout()` or similar.
- **Why:** Dead code misleads future work. The function docstring still says "Evolutionary search" but does pure random search. Tests reference "evolutionary search" in skip reasons that are now stale.
- **Complexity:** Small. Pure deletion + rename.

### 0.2 Un-skip tier 2 tests
- **File:** `tests/test_spaghetti.py`
- **What:** Change `electronic-circuit` tests from `@pytest.mark.skip` to `@pytest.mark.xfail(reason="belt-flow-reachability")`. Update all stale skip reasons that reference "Evolutionary search too slow" — this hasn't been true since the random search switch.
- **Why:** Skipped tests produce zero signal. xfail tests show *how* they fail, which guides development. We need a feedback loop on tier 2.
- **Complexity:** Small. Decorator changes only.

### 0.3 Update CLAUDE.md
- **What:** Fix the "primary remaining problem" section. Tier 1 belt-flow-reachability is solved (the text says it "produces 2-5 errors per layout on tier 1" but tier 1 status says "SOLVED"). Reconcile the contradiction. Update validation check count (says 16, actually 19).
- **Complexity:** Trivial.

---

## Phase 1: Fix Belt-Flow-Reachability (the #1 blocker)

**Goal:** Continuation routing produces direction-correct belt paths so items actually flow from source to destination.

### The Problem (diagnosed)

In `orchestrate.py` lines 656-678, input edge continuation routing uses multi-source A* starting from downstream ends of the existing network, routing toward the machine's stub tile. The A* finds a *topological* path but `_path_to_entities()` sets belt directions based on path traversal order. The issue: the path goes from network → stub, so belts point network → stub. This is correct! Items should flow toward the machine.

**But** the problem arises in specific scenarios:
1. **Stub direction mismatch**: The stub belt direction is pre-set (line 608-613) to face toward the inserter, but continuation routing may approach from a different angle, creating a direction discontinuity at the junction.
2. **Network junction direction**: Where the continuation path meets the existing network, the last tile of the existing network may point away from the new path, creating a gap in flow.
3. **Sideload assumption failures**: When continuation routing sideloads onto an existing belt, the existing belt's direction determines flow — if the sideload target belt points away from the machine, items flow the wrong way.

### 1.1 Direction-aware A* goal validation
- **Files:** `src/routing/router.py` (Python + `rust_src/lib.rs`)
- **What:** When A* reaches a goal tile, validate that the arrival direction is compatible with the goal's required flow direction. For input stubs, the belt must arrive such that flow continues toward the inserter. Add a `goal_directions` parameter to `_astar_path()` — a map from goal tile to required belt direction(s). Reject goal arrivals that produce incompatible directions.
- **Why:** This prevents the "found a path but items can't flow" class of errors at the source.
- **Complexity:** Medium. Requires changes to both Python and Rust A* implementations.

### 1.2 Stub direction reconciliation
- **File:** `src/routing/orchestrate.py`
- **What:** After continuation routing succeeds, verify the stub tile's final direction (from `_path_to_entities`) is compatible with the inserter's pickup/drop direction. If not, insert a one-tile "elbow" to redirect. Alternatively, don't pre-set stub direction — let the routing path determine it, and verify the inserter can still work.
- **Why:** Eliminates direction discontinuity at the stub-to-inserter junction.
- **Complexity:** Medium.

### 1.3 Junction flow validation
- **File:** `src/routing/orchestrate.py`
- **What:** When a continuation path connects to an existing network, verify that items can actually flow across the junction. For sideloads, this means checking the target belt's direction allows items from the sideload to reach the machine. If the junction is invalid, try routing to a different point on the network.
- **Why:** Sideloading onto a belt that flows away from your destination is topology without function.
- **Complexity:** Medium-high. Requires flow-direction-aware goal selection.

### Verification
After each sub-task:
```bash
pytest tests/test_spaghetti.py::TestSpaghettiVisualization::test_viz_iron_gear_wheel --viz -x
```
Visually inspect the viz. Count should remain at 0 errors for iron-gear-wheel. Then check electronic-circuit xfail tests — error count should decrease.

---

## Phase 2: Tier 2 — Electronic Circuit (2 solid inputs)

**Goal:** `electronic-circuit` at 10/s produces zero-error blueprints consistently.

### What makes tier 2 harder than tier 1
- **2 input items** (iron plates, copper cables) that must stay isolated on separate belt networks
- **2 machine types** (copper-cable assembler feeds electronic-circuit assembler)
- **Internal edges** — machine-to-machine routing, not just external-to-machine
- **Higher machine count** (~3-4 machines) means more placement/routing combinatorics

### 2.1 Internal edge routing reliability
- **File:** `src/routing/orchestrate.py` lines 784-811
- **What:** Internal edges use `route_connections()` which was designed for batch mode. Profile how often internal edges fail for electronic-circuit. If failure rate is high, the fix may be: try multiple inserter side combinations for internal edges (currently only tries the pre-shuffled order).
- **Complexity:** Medium. Diagnostic first, then targeted fix.

### 2.2 Multi-item network isolation under placement pressure
- **File:** `src/routing/router.py`
- **What:** With 2 items, the `other_item_tiles` hard-blocking works but may make routing infeasible (too many blocked tiles). Diagnose whether routing failures in tier 2 are from contamination avoidance making paths impossible. If so, consider: (a) increasing search candidate count for multi-item recipes, (b) placement strategies that naturally separate item flows.
- **Complexity:** Medium. Requires diagnostic data collection.

### 2.3 Search parameter tuning for multi-machine layouts
- **File:** `src/search/layout_search.py`
- **What:** 60 candidates may not be enough for tier 2. Consider: scaling candidate count with machine count, or adding a lightweight local search (try 2-3 variations of the best candidate's inserter sides).
- **Complexity:** Low-medium.

### Verification
The target is: `electronic-circuit` at 10/s produces 0 validation errors in at least 3/5 search retries. Track error distribution across retries.

---

## Phase 3: Tier 3 — Plastic Bar (fluids)

**Goal:** `plastic-bar` (1 fluid + 1 solid input) produces zero-error blueprints.

### What makes tier 3 harder
- **Fluids** — pipes connect to ALL adjacent pipes (unlike belts which are directional). Separate fluid networks MUST be physically isolated (no adjacent pipe tiles carrying different fluids).
- **Fluid ports** — machines have specific tile positions for fluid connections, queryable from draftsman data
- **Mixed routing** — some edges need belts (solid items), others need pipes (fluids), on the same layout

### 3.1 Pipe isolation in routing
- **File:** `src/routing/router.py`
- **What:** The A* router needs pipe-specific logic: when routing a pipe, any adjacent tile (not just same-direction tiles) carrying a different fluid is an obstacle. This is analogous to `other_item_tiles` for belts but with 4-adjacency instead of direction-based adjacency.
- **Why:** Pipe merge contamination (failure mode #7) is the tier 3 blocker.
- **Complexity:** Medium. The contamination avoidance pattern exists for belts; extend it to pipes with adjacency-based blocking.

### 3.2 Fluid port routing
- **File:** `src/routing/orchestrate.py`
- **What:** Fluid edges connect to machine fluid ports (specific tiles on the machine border), not via inserters. The inserter assignment logic needs a fluid-port path: skip inserter, route pipe directly to the port tile.
- **Complexity:** Medium-high. May require refactoring inserter assignment to handle fluid ports as a separate case.

### 3.3 Pipe isolation validation
- **File:** `src/validate.py`
- **What:** `check_pipe_isolation()` already exists. Verify it catches all adjacency violations including diagonal-adjacent (Factorio pipes only connect orthogonally, so diagonal is fine — confirm the validator agrees).
- **Complexity:** Low. Mostly verification.

---

## Phase 4: Tier 4 — Advanced Circuit (scale)

**Goal:** `advanced-circuit` (5+ recipes, mixed solid/fluid) produces layouts with <5 errors.

This phase is more speculative — the specific blockers will emerge from tiers 2-3. Likely work:

### 4.1 Search strategy rethink
- 60 random candidates with no refinement won't work for 8+ machine layouts. Options:
  - **Local search**: Take top-3 candidates, try ~10 variations of each (different inserter sides, position jitter)
  - **Constraint propagation**: Before routing, identify forced inserter sides (e.g., if a machine has only one neighbor on its east side, that side must be the input)
  - **Hierarchical placement**: Place connected subgraphs as units, not individual machines

### 4.2 Routing scalability
- With 5+ recipes and 10+ machines, `other_item_tiles` blocks most of the grid. The router may need: wider grid (more spacing between machines), or smarter obstacle avoidance (block only *adjacent* tiles to foreign networks, not the network tiles themselves).

### 4.3 Trunk planning generalization
- The current trunk planning (`plan_trunks()`) hardcodes vertical layout. Generalize to support horizontal trunks or L-shaped trunks based on machine placement geometry.

---

## Cross-cutting concerns

### Furnace/smelting support (parallel stream)
Currently being worked on separately. This is effectively tier 0 — raw ore → plates. The solver needs to handle furnace recipes (different crafting speed, different machine type). This work should integrate cleanly since the solver/graph/placement pipeline is machine-type-agnostic. If furnaces are hitting routing problems, they're likely the same belt-flow-reachability issues from Phase 1.

### Performance budget
One search attempt (60 candidates) should complete in <2s with Rust A*. Monitor this as recipe complexity grows. If tier 4 pushes above 10s, the bottleneck is likely placement scoring (O(n^2) in placer.py) not A*.

### Magic number audit
The router has 7+ untuned heuristic weights (deviation: 0.1, turn: 0.5, underground: 5x, sideload: 10.0, proximity: 3.0). These should be documented with rationale. Consider: are any of these actively harmful for higher tiers? The proximity penalty (3.0) in particular may block valid paths in dense multi-item layouts.

---

## Success criteria

| Phase | Recipe | Target | Measured by |
|-------|--------|--------|------------|
| 0 | — | Clean codebase, enabled tests | CI green, no dead code |
| 1 | iron-gear-wheel | Maintain 0 errors | Existing tests pass |
| 2 | electronic-circuit | 0 errors, 3/5 retries | xfail tests start passing |
| 3 | plastic-bar | 0 errors, 3/5 retries | New tests passing |
| 4 | advanced-circuit | <5 errors | Error count tracking |
