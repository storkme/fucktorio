# RFP: Belt-Flow-Aware A* Pathfinding

## Summary

Encode Factorio belt direction semantics into the A* state machine so that paths are belt-flow-correct by construction, eliminating the fragile post-processing fixes (`_fix_belt_directions()`) and the `_verify_flow_continuity()` band-aid.

## Problem

The current A* pathfinder treats belt tiles as undirected graph nodes. It finds a tile-connected path from A to B, then **post-hoc** assigns belt directions based on consecutive tile deltas. This is fundamentally broken for Factorio because:

1. **Belts are directed.** A belt at (3,5) pointing South only accepts items from (3,4) and only feeds (3,6). The A* doesn't know this — it just sees "tile connected."

2. **Post-processing is heuristic.** `_fix_belt_directions()` tries to fix T-junctions, orphan stubs, and UG exit sideloading via local adjacency analysis. It's ~170 lines of pattern-matching that breaks when new edge cases appear.

3. **Flow verification is a band-aid.** `_verify_flow_continuity()` does a downstream BFS after routing to catch direction corruptions, then tries to patch them. This is O(edges × path_length) and still misses cases.

4. **Bus routing sidesteps this.** Bus lanes are mostly axis-aligned with known directions, so the post-processing works well enough. Spaghetti routing (the future goal) hits this constantly because A* freely generates turns and merges.

### Current state representation

```rust
struct State {
    x: i16,
    y: i16,
    forced: Option<Forced>,  // UG exit continuation
}
```

No direction field. Directions are computed from the path after A* completes:
```rust
fn vec_to_dir(dx: i16, dy: i16) -> u8 { ... }
// In route_astar():
let dx = (path[i + 1].0 - path[i].0).signum();
let dy = (path[i + 1].1 - path[i].1).signum();
```

## Proposed Design

### Core idea: add `incoming_dir` to State

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct State {
    x: i16,
    y: i16,
    incoming_dir: u8,  // DIR_NORTH/EAST/SOUTH/WEST — direction we arrived from
    forced: Option<Forced>,
}
```

The `incoming_dir` encodes the direction a belt at this tile is **facing** (items flow in this direction). This is the direction the predecessor tile was outputting toward.

### Factorio belt rules to enforce

From `docs/factorio-mechanics.md` and the game rules:

| Rule | Encoding |
|------|----------|
| **B1**: Belt outputs in one direction | Successor must align with `incoming_dir` or be a valid turn |
| **B2**: 90° turn preserves both lanes | Allow turns; items transition correctly |
| **B3**: Sideloading fills near lane only | If entering a tile from the side (perpendicular to its `incoming_dir`), mark as sideload |
| **U1**: UG pair = input + output, same direction | Already handled by UG jump + Forced |
| **U4**: UG exit emits in facing direction | Forced continuation already handles this |
| **U5**: Sideloading onto UG input fills far lane only | Block perpendicular entry onto UG input (already done via `hard_block_perp_ug`) |

### Successor generation

For a state at `(x, y)` with `incoming_dir = d`:

1. **Straight continuation**: move in direction `d` to `(x+dx, y+dy)`.
   - Belt at neighbor faces `d`.
   - Always valid if neighbor is unoccupied.

2. **90° turn (CW/CCW)**: turn left or right relative to `d`.
   - Belt at `(x, y)` faces `d`, items flow to turn tile.
   - Turn tile faces the new direction.
   - Valid Factorio behavior (B2): both lanes preserved through turns.
   - Cost: add turn penalty (currently 0.5).

3. **180° reversal**: NOT allowed.
   - U-turns don't exist on belts (no B7 rule).
   - Eliminated by construction.

4. **Underground jump**: as current, in any of the 4 directions.
   - Perpendicular entry penalty/checks unchanged.
   - Forced continuation unchanged.

### Starting states

When seeding the open set, `incoming_dir` is determined by:
- `flow_dir` from `LaneSpec` (the lane's primary flow direction)
- If no `flow_dir`, try all 4 directions as separate starting states

### State space impact

**Before**: `State` = (x, y, forced) → ~O(W × H × 2) states
**After**: `State` = (x, y, incoming_dir, forced) → ~O(W × H × 4 × 2) states

This is a **~4x expansion** of the state space. In practice:

- **Straight paths** (most of bus routing) explore exactly 1 direction per tile — no expansion.
- **Turns** explore 3 successors instead of 4 (no reversal) — minimal overhead.
- **g_score hashmap** grows ~4x but still fits in memory for realistic grids (200×200 → 320K states).
- **Wall-clock** impact should be <2x because the direction pruning eliminates dead-end successors early.

### Direction propagation through UG jumps

UG jumps currently produce a `Forced` state at the exit tile. With belt-flow-aware A*, the exit tile's `incoming_dir` is the jump direction (items emerge facing that direction). The forced continuation tile then inherits this direction. No change to UG logic needed — the `incoming_dir` is just set to the jump direction on the landing state.

### Integration with contamination checks

Current contamination checks (`incoming_contamination`, `proximity_check`) operate on tile positions and belt direction maps. With belt-flow-aware A*, the contamination logic becomes simpler and more correct:

- **Instead of checking if a foreign belt "points at" our tile**, we know our belt's direction. A foreign belt at an adjacent tile only contaminates if it points INTO our tile AND carries a different item.
- The `belt_dir_map` becomes implicit in the A* state — no need to pass it separately.

## Implementation Plan

### Phase 1: Add `incoming_dir` to State (non-breaking)

1. Extend `State` with `incoming_dir: u8` field
2. Update `Hash`/`Eq` to include the new field
3. Update `g_score`/`parent` maps
4. Seed starting states with `incoming_dir` from `flow_dir` or try all 4
5. Update `reconstruct()` to include direction info in the path output

### Phase 2: Direction-aware successor generation

1. Replace the current "try all 4 directions" loop with direction-aware successors:
   - Straight: `(dx, dy) = dir_vec(incoming_dir)`
   - CW turn: rotate `incoming_dir` 90° clockwise
   - CCW turn: rotate `incoming_dir` 90° counter-clockwise
   - No reversal
2. UG jumps: unchanged, but landing state gets `incoming_dir = jump_direction`
3. Forced continuation: inherits `forced` direction as `incoming_dir`

### Phase 3: Extend path output to include directions

1. Change `astar_inner` return type from `Vec<(i16, i16)>` to `Vec<(i16, i16, u8)>` (tile + direction)
2. Update `route_astar()` to use the embedded directions directly instead of computing from path deltas
3. Update `negotiate_lanes()` to propagate directions
4. Update PyO3 bindings to expose the new return type
5. Update WASM bindings similarly

### Phase 4: Remove post-processing

1. Remove `_fix_belt_directions()` calls from spaghetti routing (Python)
2. Remove `_verify_flow_continuity()` calls
3. Run validation suite — all belt-flow checks should pass without post-processing
4. Verify no regression on bus layout (tier 1-3 recipes)

### Phase 5: Direction-aware contamination

1. Replace `belt_dir_map` parameter with direct state-based contamination checks
2. Remove `incoming_contamination()` helper (now implicit in state transitions)
3. Simplify `route_astar()` — no need to pass separate belt direction data

## Risks and Mitigations

| Risk | Mitigation |
|------|-----------|
| 4x state space → slower routing | Direction pruning offsets; profile before/after. Bus lanes (constrained) are unaffected. |
| Starting direction unknown for spaghetti | Try all 4 starts; overhead is 4 × start_count which is small |
| UG continuation semantics change | Forced continuation already encodes direction; `incoming_dir` is redundant with `Forced.dx/dy` — unify them |
| Breaks existing Python `router.py` | Phase 1-3 are additive; Phase 4 only removes Python post-processing after validation passes |
| Balancer template compatibility | Balancers are stamped independently and don't use A* — no impact |

## Scope

- **In scope**: `crates/core/src/astar.rs`, PyO3 bindings, WASM bindings, spaghetti routing post-processing removal
- **Out of scope**: Bus routing (already correct via axis-aligned templates), SAT crossing solver, balancer library
- **Dependencies**: None (can be done independently of the general SAT crossing solver)

## Success Criteria

1. A* paths have correct belt directions without post-processing
2. All existing tests pass
3. Spaghetti routing (tier 1 `iron-gear-wheel`) achieves lower error counts
4. No regression on bus layout (tiers 1-3)
5. Wall-clock routing time increases <2x on benchmark workloads
