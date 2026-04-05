# Port plan: Python → Rust (Track A)

Sequential unit list for porting the bus layout engine and validation into `crates/core`. Each unit has a canonical name used by the `/port` skill to delegate work to a Sonnet subagent.

For the inventory view (what's ported, what stays Python), see `docs/port-status.md`.

## Shared context (every worker needs this)

- **Workspace**: `crates/core` is pure Rust (no pyo3/wasm). Bus layout code lives in `crates/core/src/bus/`. A new `bus/` module must be added to `crates/core/src/lib.rs` for the first bus unit.
- **Error handling**: use `thiserror` (already a dep) for any fallible functions. Match error variants to the Python exceptions they replace.
- **Hashmaps**: use `rustc_hash::{FxHashMap, FxHashSet}` (already a dep), not `std::collections`.
- **Models**: shared data types live in `crates/core/src/models.rs`. `ItemFlow`, `MachineSpec`, `SolverResult`, `PlacedEntity`, `LayoutResult`, `EntityDirection` are already ported. Do not duplicate them.
- **Reference implementation**: the Python code in `src/` is the source of truth. When in doubt, match Python's behaviour exactly — this is a port, not a rewrite.
- **Test fixtures**: for each unit, write at least one unit test that matches a known Python output (check `tests/` for existing assertions to mirror).
- **Cargo.toml**: keep deps alphabetically sorted. Add new deps to `crates/core/Cargo.toml`.
- **Verification before commit**: `cargo check --workspace` must be clean; `cargo test -p fucktorio_core` must pass.

## Unit list

### Bus layout engine

#### `bus-common`
**Scope**: Port shared routing constants and helpers used by the bus engine.
**Python**: `src/routing/common.py` (123 LOC) — machine sizes, belt tier selection, direction constants, lane constants (`LANE_LEFT`/`LANE_RIGHT`), `inserter_target_lane()`.
**Rust**: `crates/core/src/common.rs` (new file). Export constants + helper fns.
**Dependencies**: none — foundation unit.
**Size**: ~150 LOC Rust.
**Done when**: `cargo test -p fucktorio_core common` passes unit tests asserting machine size for assembling-machine-3 (3×3), belt tier for rate <15 (transport-belt), etc.

#### `bus-placer`
**Scope**: Group machines by recipe into rows; split rows when throughput exceeds belt capacity.
**Python**: `src/bus/placer.py` (390 LOC).
**Rust**: `crates/core/src/bus/placer.rs` (new). Also create `crates/core/src/bus/mod.rs` with `pub mod placer;` and add `pub mod bus;` to `crates/core/src/lib.rs`.
**Dependencies**: `bus-common` must be merged first.
**Size**: ~500 LOC Rust.
**Done when**: unit test fed an electronic-circuit `SolverResult` produces the same row grouping (machines-per-row count) as the Python placer.

#### `bus-templates`
**Scope**: Belt/inserter stamp templates for bus rows — single-input, dual-input, lane-splitting.
**Python**: `src/bus/templates.py` (963 LOC). Pure data-driven entity stamping.
**Rust**: `crates/core/src/bus/templates.rs` (new).
**Dependencies**: `bus-common`.
**Size**: ~1,200 LOC Rust.
**Done when**: unit tests over a handful of row configurations produce the same `Vec<PlacedEntity>` output (by name + position + direction) as the Python templates.

#### `bus-balancer-library`
**Scope**: Pre-generated N-to-M balancer templates (SAT-solved). Mainly a data embed + stamping helper.
**Python**: `src/bus/balancer_library.py` (595 LOC). Templates are pre-generated blueprint strings stored as constants.
**Rust**: `crates/core/src/bus/balancer_library.rs` (new). Keep template blueprint strings as `&'static str` constants.
**Dependencies**: `bus-common`.
**Size**: ~700 LOC Rust.
**Done when**: template lookup for (N=2, M=1) returns the same `(entities, width, height)` tuple as Python.

#### `bus-router-trunks`
**Scope**: Trunk belt placement + EAST tap-off routing (items only, no balancers yet). First phase of `bus_router.py`.
**Python**: `src/bus/bus_router.py` lines 1-700 approximately — `plan_trunks`, tap-off underground crossings.
**Rust**: `crates/core/src/bus/bus_router.rs` (new file, append in later phases).
**Dependencies**: `bus-common`, `bus-placer`, the existing `crates/core/src/astar.rs` (already merged).
**Size**: ~900 LOC Rust.
**Done when**: iron-gear-wheel layout through this phase has the trunks positioned identically to Python's output.

#### `bus-router-balancers`
**Scope**: Balancer family stamping, producer-to-input wiring, row-reflow for tall templates. Second phase of `bus_router.py`.
**Python**: `src/bus/bus_router.py` lines ~700-1400 — `LaneFamily`, `_optimize_lane_order`, balancer feeder path rendering.
**Rust**: Append to `crates/core/src/bus/bus_router.rs`.
**Dependencies**: `bus-router-trunks`, `bus-templates`, `bus-balancer-library`.
**Size**: ~900 LOC Rust.
**Done when**: electronic-circuit layout (which uses balancer families) matches Python entity-for-entity.

#### `bus-router-mergers`
**Scope**: Output mergers (N→1 Z-wrap hand-rolled balancer), final product belt routing. Third phase of `bus_router.py`.
**Python**: `src/bus/bus_router.py` lines ~1400-1800 — `_route_intermediate_lane balance_y` code, output splitter chain.
**Rust**: Append to `crates/core/src/bus/bus_router.rs`.
**Dependencies**: `bus-router-balancers`.
**Size**: ~600 LOC Rust.
**Done when**: layouts with multiple producers per lane (N→1 merging) match Python output.

#### `bus-router-fluid`
**Scope**: Fluid pipe trunks, pipe-to-ground tap-offs, negotiated crossing map. Fourth phase of `bus_router.py`.
**Python**: `src/bus/bus_router.py` lines ~1800-2183 — fluid lane routing, `negotiate_lanes` integration.
**Rust**: Append to `crates/core/src/bus/bus_router.rs`. Call into `core::astar::negotiate_lanes`.
**Dependencies**: `bus-router-mergers`.
**Size**: ~400 LOC Rust.
**Done when**: plastic-bar layout (which uses fluid lanes) matches Python output with zero validation errors.

#### `bus-layout`
**Scope**: Top-level orchestrator for the bus engine. Consumes `SolverResult`, returns `LayoutResult`.
**Python**: `src/bus/layout.py` (182 LOC).
**Rust**: `crates/core/src/bus/layout.rs` (new). Expose `pub fn build_bus_layout(solver: &SolverResult) -> LayoutResult`.
**Dependencies**: all `bus-router-*`, `bus-placer`.
**Size**: ~220 LOC Rust.
**Done when**: calling `build_bus_layout` on an iron-gear-wheel `SolverResult` produces a `LayoutResult` with the same entity count as Python's output, and serializes into a blueprint via `core::blueprint::export`.

### Validation

#### `validate-types`
**Scope**: Foundation types + top-level `validate()` entry point. No actual checks yet.
**Python**: Extract from `src/validate.py` — `ValidationIssue` dataclass, `ValidationError` exception, `Severity` enum, top-level `validate()` dispatcher.
**Rust**: `crates/core/src/validate/mod.rs` (new). Set up `crates/core/src/validate/` module structure. Add `pub mod validate;` to `crates/core/src/lib.rs`.
**Dependencies**: `bus-common` (for layout-style enum).
**Size**: ~250 LOC Rust.
**Done when**: `validate()` returns an empty `Vec<ValidationIssue>` for any `LayoutResult` (because no checks are wired yet).

#### `validate-power`
**Scope**: `check_power_coverage` — all machines within 7×7 coverage of a medium-electric-pole.
**Python**: `src/validate.py` — `check_power_coverage` function (~100 LOC).
**Rust**: `crates/core/src/validate/power.rs` (new). Wire into `validate::validate()`.
**Dependencies**: `validate-types`.
**Size**: ~150 LOC Rust.
**Done when**: test with a layout missing power reports the expected uncovered machines; test with full coverage reports zero issues.

#### `validate-underground`
**Scope**: Underground belt checks — paired entities, correct sideloading, entry sideload rules.
**Python**: `src/validate.py` — `check_underground_belt_pairs`, `check_underground_belt_sideloading`, `check_underground_belt_entry_sideload` (~300 LOC total).
**Rust**: `crates/core/src/validate/underground.rs` (new). Wire into `validate::validate()`.
**Dependencies**: `validate-types`.
**Size**: ~400 LOC Rust.
**Done when**: test fixtures for each failure mode (unpaired, wrong sideload, entry sideload) produce expected issue lists.

#### `validate-fluids`
**Scope**: Pipe isolation and fluid port connectivity.
**Python**: `src/validate.py` — `check_pipe_isolation`, `check_fluid_port_connectivity` (~400 LOC).
**Rust**: `crates/core/src/validate/fluids.rs` (new). Fluid port positions come from `recipe_db`'s machine data (already extracted into `recipes.json`).
**Dependencies**: `validate-types`.
**Size**: ~500 LOC Rust.
**Done when**: plastic-bar layout fixture passes both checks; contrived layouts with adjacent differing-fluid pipes trigger pipe-isolation errors.

#### `validate-inserters`
**Scope**: Inserter chain validity and direction checks.
**Python**: `src/validate.py` — `check_inserter_chains`, `check_inserter_direction` (~300 LOC).
**Rust**: `crates/core/src/validate/inserters.rs` (new). Wire into `validate::validate()`.
**Dependencies**: `validate-types`.
**Size**: ~400 LOC Rust.
**Done when**: fixture with reversed inserter direction produces expected issue; valid layout produces none.

#### `validate-belt-structural`
**Scope**: Belt loops, dead-ends, item isolation, inserter conflicts, throughput, output coverage.
**Python**: `src/validate.py` — `check_belt_loops`, `check_belt_dead_ends`, `check_belt_item_isolation`, `check_belt_inserter_conflict`, `check_belt_throughput`, `check_lane_throughput`, `check_output_belt_coverage` (~700 LOC).
**Rust**: `crates/core/src/validate/belt_structural.rs` (new).
**Dependencies**: `validate-types`.
**Size**: ~900 LOC Rust.
**Done when**: iron-gear-wheel layout passes all these checks; contrived layouts with each failure mode trigger the right issue.

#### `validate-belt-flow`
**Scope**: Belt connectivity, flow paths, direction continuity, reachability, network topology, junctions.
**Python**: `src/validate.py` — `check_belt_connectivity`, `check_belt_flow_path`, `check_belt_direction_continuity`, `check_belt_flow_reachability`, `check_belt_network_topology`, `check_belt_junctions` (~800 LOC).
**Rust**: `crates/core/src/validate/belt_flow.rs` (new).
**Dependencies**: `validate-types`.
**Size**: ~1,000 LOC Rust.
**Done when**: electronic-circuit layout passes; layouts with disconnected networks or reversed flow are flagged.

## Dependency graph

```
bus-common
├── bus-placer
├── bus-templates
├── bus-balancer-library
└── validate-types ─┬── validate-power
                    ├── validate-underground
                    ├── validate-fluids
                    ├── validate-inserters
                    ├── validate-belt-structural
                    └── validate-belt-flow

bus-placer + bus-common + astar → bus-router-trunks
  → bus-router-balancers (+ bus-templates + bus-balancer-library)
    → bus-router-mergers
      → bus-router-fluid
        → bus-layout (+ bus-placer)
```

The six validation units can all run in parallel once `validate-types` lands. The bus-router phases must serialize.

## How to execute

Use the `/port <unit-name>` skill to delegate a single unit to a Sonnet subagent. The skill spawns the worker in an isolated worktree, hands it the brief from this file, and reports the PR URL when done.

Example: `/port bus-common`
