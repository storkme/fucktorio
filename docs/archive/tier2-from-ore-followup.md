# Tier 2 `from_ore` follow-up work

Session handoff. Picks up from commits `655eb6c` (belt-flow topo sort fix) and
`3013ca1` (A\* UG-exit on `goal_on_obstacle`). Both fixes landed on main.

## What's already green

- `tier2_electronic_circuit` (plates variant) — un-ignored, passes.
- `tier2_electronic_circuit_20s_from_ore` — `belt-dead-end` error at (13,52) is
  gone. Test still `#[ignore]`'d because it has 31 other warnings.
- All 358 unit tests + 8 default e2e tests green. Clippy clean, WASM clean.

## What still needs fixing

Two `#[ignore]`'d tier2 tests, both blocked by three independent bugs:

- `tier2_electronic_circuit_from_ore` (10/s): 4× belt-direction, 35×
  input-rate-delivery (copper-plate at y=29 area), 1× power (27 poles).
- `tier2_electronic_circuit_20s_from_ore` (20/s): 2× belt-direction, 28×
  input-rate-delivery (mixed items), 1× power (25 poles).

Tackle in this order — the balancer bug is highest leverage.

### 1. Balancer template overlaps UG tunnel tiles (HIGH PRIORITY)

**Strongest hypothesis for the input-rate-delivery starvation.** The
copper-plate balancer stamped for the 10/s from_ore layout has this on
column x=12, y=15–28 (from the snapshot):

```
(12, 20) underground-belt  South  input   balancer:copper-plate
(12, 21) transport-belt    South          balancer:copper-plate
(12, 22) transport-belt    South          balancer:copper-plate
(12, 23) transport-belt    South          balancer:copper-plate
(12, 24) transport-belt    West           balancer:copper-plate
(12, 25) underground-belt  South  output  balancer:copper-plate
```

(12, 20) UG-in and (12, 25) UG-out are paired (5-tile span for yellow
UG, right at max reach). The tiles (12, 21)–(12, 23) are same-direction
surface belts *on top of the UG tunnel*, which is physically invalid in
Factorio — either the UG pair won't form (Factorio's auto-pair finds
nothing) or the blueprint fails to import.

The balancer that's producing this is stamped by `stamp_family_balancer`
from a pre-baked template in `crates/core/src/bus/balancer_library.rs`.
The template itself may be wrong (SAT-solved with an inconsistent model),
or the stamping logic is overlaying two separate sub-templates onto the
same column without noticing.

**Investigation steps:**

1. Dump the layout snapshot:
   ```
   FUCKTORIO_DUMP_SNAPSHOTS=1 cargo test --manifest-path crates/core/Cargo.toml \
       --test e2e -- --ignored tier2_electronic_circuit_from_ore --exact
   ```
2. Extract the copper-plate balancer's shape (N→M) from the trace:
   ```python
   # in the snapshot's trace.events, look for BalancerStamped with item=copper-plate
   ```
3. Inspect the template in `balancer_library.rs` — does the raw template
   already contain the conflicting (12, 21)–(12, 23) tiles? If yes, the
   template is broken. If no, the conflict is introduced by
   `stamp_family_balancer` (likely a decomposed-template overlap).
4. Check whether the overlap is specific to this layout's N/M or whether
   every tier2 from-ore variant hits it (the 20/s variant has a different
   balancer shape because it has different machine counts).

**Expected fix:** either regenerate the broken template via
`scripts/generate_balancer_library.py` (see memory
`project_sat_crossing_solver.md`) with the UG span correctly modelled, or
make `stamp_family_balancer` reject templates whose entity list conflicts
with their own UG pairs.

**Related diagnostic tip:** the `RouteFailure` trace is a reliable signal
(we used it to confirm hypothesis today). Check for it in the snapshot
before assuming a feeder bug:

```python
import json
d = json.load(open('snapshot-...fls')); ...  # see below for full decoder
failures = [e for e in d['trace']['events'] if 'RouteFailure' in str(e)]
```

### 2. Belt-direction dead spots

2–4 per variant, adjacent belts pointing at each other:

- 10/s: (17,60)/(17,59), (13,62)/(13,63), (10,25)/(10,24), +1 more
- 20/s: (9,41)/(9,42), (6,57)/(6,56)

The agent's earlier hypothesis was the sideload-bridge pattern in
`crates/core/src/bus/templates.rs:289-304` colliding with merger south
columns in `bus_router.rs:1158-1197`. Verify by dumping entities at
those exact coordinates and tracing which segment_id each belongs to.
If both belts are in the same `row:*:belt-out` segment, it's a template
bug; if one is merger and one is row, it's the stamping overlap.

### 3. Pole cluster bridging

1× `power` warning with 25–27 disconnected poles. Commit `084322b`
rewrote pole placement ("Rewrite bus pole placement: regular row
lines"), and `a364071` added `repair_pole_connectivity`. The repair
pass does a ring search at radius ≤ 6 (roughly `layout.rs:895-995` — was
true pre-rewrite, verify after). For 80-wide layouts with sparse row
gaps, two clusters may sit further than 6 tiles apart, so the repair
never bridges them.

Check:

1. Is `repair_pole_connectivity` still being called after the `084322b`
   rewrite? (It was before; the rewrite may have removed it.)
2. If yes, what radius does it use, and is it dynamic vs constant?
3. Dump the pole entities from the snapshot and plot them to see where
   the disconnected clusters actually are.

## Useful snippets

### Decode a `.fls` snapshot

```bash
tail -c +5 crates/core/target/tmp/snapshot-<test>.fls \
  | base64 -d | gunzip | python3 -c "
import json, sys
d = json.load(sys.stdin)
print('Top keys:', list(d.keys()))
# d['layout']['entities'], d['trace']['events'], d['solver']['machines']
"
```

### Inspect entities in a coordinate range

```bash
tail -c +5 <snapshot.fls> | base64 -d | gunzip | python3 -c "
import json, sys
d = json.load(sys.stdin)
ents = d['layout']['entities']
for e in ents:
    if 8 <= e.get('x',0) <= 14 and 50 <= e.get('y',0) <= 54:
        print(e)
"
```

### Run a single ignored test with output

```bash
cargo test --manifest-path crates/core/Cargo.toml --test e2e -- \
    --ignored <test_name> --exact --nocapture
```

### Dump snapshots for all tests (including passing)

```bash
FUCKTORIO_DUMP_SNAPSHOTS=1 cargo test --manifest-path crates/core/Cargo.toml --test e2e
```

## Files most likely to be touched

- `crates/core/src/bus/balancer_library.rs` — pre-baked SAT templates.
  Also `scripts/generate_balancer_library.py` for regeneration.
- `crates/core/src/bus/bus_router.rs` — `stamp_family_balancer` (~687),
  `render_family_input_paths` (~891), `merge_output_rows` (~1125).
- `crates/core/src/bus/templates.rs` — row output templates including
  the sideload-bridge lane-split pattern.
- `crates/core/src/bus/layout.rs` — pole placement (`place_poles`,
  `repair_pole_connectivity`) around lines 800–1000.
- `crates/core/src/validate/belt_flow.rs` — we already know this is
  solid for the splitter/UG case; if validator flags new cases check
  whether it's a propagation bug or a real layout bug (it's usually
  the latter now).
