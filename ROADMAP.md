# Roadmap: From Tier 1 to Tier 4

> Status as of 2026-03-31. Tier 1 (iron-gear-wheel) is solved. Everything else fails.

## Objective

Get the layout engine producing zero-error blueprints for progressively more complex recipes. Each tier in the recipe complexity ladder represents a qualitative jump in what the engine must handle. The north star is **tier 4 (advanced-circuit)** — a 5+ recipe, mixed solid/fluid chain that would prove the approach generalises.

| Tier | Recipe | What it tests | Current status |
|------|--------|--------------|----------------|
| 1 | `iron-gear-wheel` | 1 solid input, 1 machine type | **SOLVED** |
| 2 | `electronic-circuit` | 2 solid inputs, machine-to-machine routing | Failing (belt-flow-reachability) |
| 3 | `plastic-bar` | 1 fluid + 1 solid, pipe isolation | Failing (pipe isolation) |
| 4 | `advanced-circuit` | 5+ recipes, mixed solid/fluid | Failing (massive routing failures) |

## Key diagnosis from audit

**The #1 blocker is belt-flow-reachability.** The A* router finds topologically valid paths, but belt directions get corrupted by post-processing in `_fix_belt_directions()`. Specifically:

- Belt tiles adjacent to inserters are *supposed* to be dead ends (the inserter picks items off them). But `_fix_belt_directions()` treats dead-end belts as "orphans" and aggressively reorients them — breaking the directional flow chain.
- When continuation routing sideloads onto an existing network, the junction direction isn't validated — items may flow away from the destination.
- The A* paths themselves are directionally correct. The corruption happens *after* pathfinding.

This is good news: the fix is surgical, not architectural. The A* approach is sound.

---

## Phase 0: Cleanup & Foundation

**Goal:** Remove noise, re-enable feedback loops, establish a clean baseline.

**Validates:** CI stays green, no functional changes.

### 0.1 Delete dead evolution code

`src/search/layout_search.py` still contains `_mutate()` (~50 lines), `_perturb_positions()` (~26 lines), `_random_edge_order()` (~5 lines) — none of which are called. The `survivors` and `generations` parameters are accepted but ignored. The function is named `evolutionary_layout()` but does pure random search.

Delete the dead code, remove the unused parameters, rename to `random_search_layout()`.

### 0.2 Un-skip tier 2 tests and fix stale skip reasons

20+ tests in `test_spaghetti.py` are `@pytest.mark.skip(reason="Evolutionary search too slow for CI")` — but evolution was abandoned months ago. These produce zero signal.

- Change electronic-circuit tests to `@pytest.mark.xfail(reason="belt-flow-reachability")` so they run, track failures, and automatically un-xfail when fixed.
- Update all stale skip reasons that reference evolutionary search.
- Evaluate whether the 30/s iron-gear test can be re-enabled now that Rust A* makes it fast enough.

### 0.3 Fix CLAUDE.md contradictions

- "Primary remaining problem" says belt-flow-reachability "produces 2-5 errors per layout on tier 1" but the recipe ladder says tier 1 is "SOLVED". Pick one.
- Validation check count says 16, actual count is 19.

---

## Phase 1: Fix Belt-Flow-Reachability

**Goal:** Continuation routing produces direction-correct belt paths. Items flow from source to destination without direction discontinuities.

**Validates:** iron-gear-wheel stays at 0 errors. Electronic-circuit error count drops measurably.

### Root cause (detailed)

The routing pipeline works in stages:
1. `route_connections()` calls `_astar_path()` to find tile paths — these are correct.
2. `_path_to_entities()` converts paths to belt entities, setting direction based on traversal order — also correct.
3. `_fix_belt_directions()` post-processes ALL belts to fix T-junctions, underground exits, and "orphaned" dead-end stubs — **this is where directions get corrupted**.

The fixer's orphan-stub logic (router.py ~line 790-841) finds belt tiles whose forward neighbor isn't another belt and reorients them to face an adjacent belt for sideloading. But inserter pickup/drop tiles are *intentionally* dead ends. Reorienting them breaks the flow chain that `check_belt_flow_reachability` validates via directional BFS.

### 1.1 Protect inserter-adjacent belt tiles from reorientation

**File:** `src/routing/router.py`, function `_fix_belt_directions`

Pass a `protected_tiles` set (belt tiles that are inserter pickup/drop points) into `_fix_belt_directions()`. Skip reorientation for these tiles. The set comes from `edge_targets` and `edge_starts` in `route_connections()`.

This is the highest-confidence fix — it directly addresses the diagnosed corruption mechanism.

### 1.2 Validate direction at path junction points

**File:** `src/routing/orchestrate.py`

After `_path_to_entities()` generates entities for a continuation path segment, validate that:
- The first tile of the new path has a direction compatible with flow from the existing network (for inputs: trunk end → first tile → ... → machine)
- The last tile creates valid flow into the target (for sideloads: perpendicular to trunk direction)

If validation fails, try rotating the connecting tile or re-routing to a different network attachment point.

### 1.3 Add post-routing flow-reachability check

**File:** `src/routing/router.py`, after `_fix_belt_directions()` in `route_connections()`

After `_fix_belt_directions()` runs, do a lightweight directional BFS on each network to verify flow continuity. If any direction was corrupted, either revert the corruption or flag the edge as failed (so the search can try a different candidate).

This is a safety net — it catches anything 1.1 and 1.2 miss.

### 1.4 (Fallback) Direction-aware A* goal validation

**Files:** `src/routing/router.py`, `rust_src/lib.rs`

Only if 1.1-1.3 are insufficient: add a `goal_directions` parameter to `_astar_path()` — a map from goal tile to acceptable arrival direction(s). Reject goal arrivals that would produce incompatible belt directions. Requires changes to both Python and Rust A* implementations.

This is more invasive (increases A* state space) so we try the surgical fixes first.

### How to test

After each sub-task:
```bash
# Must stay at 0 errors:
pytest tests/test_spaghetti.py::TestSpaghettiVisualization::test_viz_iron_gear_wheel --viz -x

# Should show decreasing error counts:
pytest tests/test_spaghetti.py -k "electronic" -v --tb=short
```

Visual inspection of the viz is mandatory — zero errors means nothing if the layout looks wrong. Check that belt arrows form continuous flow paths from boundary to machines and back.

---

## Phase 2: Tier 2 — Electronic Circuit

**Goal:** `electronic-circuit` at 10/s produces 0 validation errors in at least 3 of 5 search retries.

**Validates:** xfail tests start passing. Viz shows two separate belt networks (iron, copper) feeding into correct machines.

### What makes tier 2 harder

- **2 input items** (iron plates, copper cables) that must stay on isolated belt networks
- **Internal edges** — copper-cable assembler feeds electronic-circuit assembler (machine-to-machine)
- **~3-4 machines** — more placement/routing combinatorics than tier 1's 1-2 machines
- **Item contamination pressure** — `other_item_tiles` hard-blocking can make paths infeasible in tight layouts

### 2.1 Diagnose tier 2 failure modes

Before writing fixes, collect data. Run electronic-circuit 20 times and categorise failures:
- How many are belt-flow-reachability? (should drop after Phase 1)
- How many are failed routing edges? (A* can't find any path)
- How many are belt-item-contamination? (items on wrong belt)
- How many are internal edge failures? (machine-to-machine routing fails)

This determines whether the problem is routing quality, search coverage, or placement geometry.

### 2.2 Internal edge routing improvements

**File:** `src/routing/orchestrate.py` lines 784-811

Internal edges use `route_connections()` which was designed for batch mode. If internal edge failures are common:
- Try multiple inserter side combinations (currently only tries the pre-shuffled order)
- Reserve corridors for internal edges before routing external ones

### 2.3 Search coverage for multi-item recipes

**File:** `src/search/layout_search.py`

60 random candidates may not be enough. Options (in order of effort):
- Scale candidate count with edge count: `population = max(60, len(graph.edges) * 20)`
- Add lightweight local search: take top-3 candidates, try ~10 variations each (different inserter sides)
- Better placement: bias machine positions to naturally separate item flows (e.g., iron consumers on one side, copper on the other)

### How to test

```bash
# Target: 0 errors, 3/5 retries
pytest tests/test_spaghetti.py -k "electronic_circuit" -v

# Track error distribution:
# Run 5x, record: [errors_attempt1, errors_attempt2, ...]
```

---

## Phase 3: Tier 3 — Plastic Bar (fluids)

**Goal:** `plastic-bar` (petroleum gas + coal → plastic) produces 0 validation errors.

**Validates:** New test for plastic-bar passes. Viz shows pipes connecting to chemical plant fluid ports, isolated from any other pipe network.

### What makes tier 3 harder

- **Fluids connect omnidirectionally** — any adjacent pipe merges into the same network (unlike belts which are directional). Separate fluid networks must be physically isolated by at least 1 tile gap.
- **Fluid ports** — chemical plants have specific tile positions for fluid I/O, not general borders
- **Mixed routing** — coal arrives by belt, petroleum gas arrives by pipe, on the same layout

### 3.1 Pipe contamination avoidance in A*

**File:** `src/routing/router.py`

Currently `other_item_tiles` is only populated for non-fluid edges (orchestrate.py ~line 647: `if not edge.is_fluid`). For fluid edges, extend contamination avoidance: when routing a pipe, block all tiles adjacent to pipes carrying a different fluid (1-tile buffer). This prevents accidental merging.

### 3.2 Fluid port routing

**File:** `src/routing/orchestrate.py`

Fluid edges should route pipes directly to machine fluid port tiles, bypassing the inserter assignment logic entirely. This may require:
- Detecting which edges are fluid vs. solid during inserter assignment
- Routing fluid edges as pipe paths from the fluid port tile to the pipe network
- Using `draftsman.data.entities` to query fluid port positions for chemical plants

### 3.3 Validate existing pipe isolation checks

**File:** `src/validate.py`

`check_pipe_isolation()` and `check_fluid_port_connectivity()` already exist. Verify they catch all the failure modes that tier 3 introduces. Confirm that diagonal-adjacent pipes don't trigger false positives (Factorio pipes only connect orthogonally).

### How to test

```bash
# New test:
pytest tests/test_spaghetti.py -k "plastic_bar" -v

# Viz inspection: verify pipe network is isolated, fluid ports connected
pytest tests/test_spaghetti.py::TestSpaghettiVisualization::test_viz_plastic_bar --viz -x
```

---

## Phase 4: Tier 4 — Advanced Circuit (scale)

**Goal:** `advanced-circuit` (5+ recipes, mixed solid/fluid) produces layouts with fewer than 5 errors.

**Validates:** Error count tracking shows consistent improvement. This phase is exploratory — specific blockers will emerge from tiers 2-3.

### Likely work

**Search strategy**: 60 random candidates with no refinement won't work for 8+ machine layouts. Options:
- **Local search**: Top-3 candidates get ~10 variations each
- **Constraint propagation**: Pre-compute forced inserter sides (e.g., machine with one neighbor on east → east must be input side)
- **Hierarchical placement**: Place connected subgraphs as units

**Routing scalability**: With 5+ items and 10+ machines, `other_item_tiles` blocks most of the grid. May need: wider spacing, or smarter blocking (only block tiles *adjacent* to foreign networks, not the networks themselves).

**Trunk generalisation**: Current `plan_trunks()` hardcodes vertical (SOUTH) layout. Generalise based on placement geometry.

---

## Cross-cutting concerns

### Furnace/smelting (parallel stream)

Being worked on separately. This is tier 0 (raw ore → plates). The solver needs furnace recipe support (different crafting speed, different machine type). If furnaces are hitting routing failures, they're likely the same belt-flow-reachability issues from Phase 1 — worth comparing notes.

### Performance budget

One search attempt (60 candidates) should complete in <2s with Rust A*. Monitor as complexity grows. If tier 4 pushes above 10s, the bottleneck is likely placement scoring (O(n^2) in `placer.py`), not A*.

### Router heuristic constants

7+ untuned magic numbers in the A* cost function:

| Constant | Value | Purpose | Risk at higher tiers |
|----------|-------|---------|---------------------|
| Turn penalty | 0.5 | Prefer straight paths | Low |
| Deviation weight | 0.1 | Stay near start→goal line | Low |
| Proximity penalty | 3.0 | Avoid foreign item networks | **High** — may block valid paths in dense layouts |
| UG cost multiplier | 5x | Prefer surface belts | Low |
| Perpendicular UG entry | 10.0 | Discourage odd underground entries | Medium |
| Sideload penalty | 10.0 | Discourage unnecessary sideloads | Medium |
| Contamination retries | 5 | Retry on contamination detection | Low |

The proximity penalty (3.0) is the most likely to cause problems at scale. It should be reduced or made adaptive as item count grows.

---

## Success criteria

| Phase | Target | How we measure | Exit condition |
|-------|--------|---------------|----------------|
| 0 | Clean codebase, active feedback loops | CI green, tier 2 tests running as xfail | No dead code, no stale skip reasons |
| 1 | Belt directions correct after routing | iron-gear stays 0 errors; e-circuit errors drop | Viz shows continuous flow paths |
| 2 | electronic-circuit works | 0 errors in 3/5 retries | xfail tests start passing |
| 3 | plastic-bar works | 0 errors in 3/5 retries | New fluid tests passing |
| 4 | advanced-circuit approachable | <5 errors consistently | Error count tracked per run |

## Sequencing

Phases are sequential — each builds on the last. Phase 0 can start immediately. Phase 1 is the highest-leverage work and should begin as soon as Phase 0 is done (or in parallel). Phases 2-3 may overlap if the Phase 1 fixes are sufficient to unblock tier 2 without additional work.

Estimated order of magnitude:
- Phase 0: hours
- Phase 1: days
- Phase 2: days (possibly free if Phase 1 is sufficient)
- Phase 3: days
- Phase 4: weeks (exploratory)
