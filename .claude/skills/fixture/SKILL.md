---
name: fixture
description: Turn a web-app URL into a reproducible test fixture. Use when the user says "/fixture <url>" or asks to add a fixture/test case/showcase entry for a specific recipe+rate+inputs. Parses params, adds an e2e regression test, adds a landing showcase card, reproduces the failure (if any), and opens a task to fix it when broken.
allowed-tools: Read, Edit, Write, Bash, Grep, Glob, TaskCreate
---

# Fixture — promote a web-app URL to a tracked test case

The user hands you a URL like
`http://localhost:5173/?item=advanced-circuit&rate=5&machine=assembling-machine-1&in=iron-plate,copper-plate,coal,water,crude-oil,iron-ore,copper-ore&belt=transport-belt`
and wants it promoted to a first-class fixture: an e2e regression test, a
landing-page showcase card, and — if it's currently broken — a tracked task
to fix it.

## Steps

### 1. Parse the URL

Extract these query params (URL-decode as needed):
- `item` — recipe name (required)
- `rate` — target rate per second, as a float (required)
- `machine` — machine type (required, e.g. `assembling-machine-1`)
- `in` — comma-separated input list (required)
- `belt` — belt-tier override, optional (e.g. `transport-belt`, `fast-transport-belt`). If absent, the fixture uses `None`/`undefined`.

If any required param is missing, ask the user.

Pick a **fixture slug** from the params. Convention:
`tier<N>_<item_snake>_<variant>` where N is the complexity tier (see
`CLAUDE.md`'s ladder) and the variant encodes what's distinctive —
`_from_ore_am1`, `_20s_from_ore`, etc. Match the naming style of existing
tests in `crates/core/tests/e2e.rs`. If unsure, grep for sibling tests and
follow their convention.

### 2. Add the e2e test

Edit `crates/core/tests/e2e.rs`. Insert a new test alongside its tier's
siblings. Template:

```rust
/// <one-line description of what makes this case distinctive>
#[test]
#[ignore] // Goal: make this green.
#[ntest::timeout(30000)]
fn <fixture_slug>() {
    let inputs: FxHashSet<String> = [<inputs>]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let result = run_e2e(
        "<fixture_slug>",
        "<item>",
        <rate>,
        "<machine>",
        <belt_tier option: Some("transport-belt") or None>,
        &inputs,
    )
    .unwrap_or_else(|e| panic!("<fixture_slug>: {e}"));

    assert_no_errors(&result);
    assert_no_warnings(&result);
    assert_produces(&result, "<item>", <rate>);
    assert_round_trip(&result);
}
```

Start with `#[ignore]` — you'll remove it only if step 4 shows the fixture
is already green.

### 3. Add the landing showcase card

Edit `web/src/ui/landing.ts`. Append a new `ShowcaseEntry` to the
`SHOWCASE` array. `ShowcaseEntry` already supports an optional `beltTier`
field — if the URL had `belt=`, pass it here; otherwise omit.

```ts
{
  label: "<human label>",
  item: "<item>",
  rate: <rate>,
  inputs: [<inputs>],
  machine: "<machine>",
  beltTier: "<belt>", // omit if not set
  tier: <N>,
  status: "partial", // or "solved" if step 4 passes
  desc: "<one-line desc>",
},
```

**Status rule**: use `"solved"` only if the fixture passes with zero errors
**and** zero warnings in step 4. Otherwise `"partial"` (keeps the card
clickable so users can see the current broken state). Don't use `"wip"` —
that disables the click handler.

### 4. Reproduce

Run the new test to capture its current state:

```
cargo test --manifest-path crates/core/Cargo.toml --test e2e -- \
    --ignored <fixture_slug> --exact --nocapture
```

Record:
- Whether the pipeline completes (solver → layout → validate)
- Error count by category
- Warning count by category
- Snapshot path (`crates/core/target/tmp/snapshot-<slug>.fls`) if it
  dumped one

Also run `cd web && npx tsc --noEmit` so the landing edit is clean.

### 5. Decide pass/fail and open a task

**If 0 errors and 0 warnings**: remove the `#[ignore]` from the e2e test,
flip the showcase status to `"solved"`, and report it as already-working.
Do not open a fix task.

**Otherwise**: leave the `#[ignore]` in place, keep status `"partial"`, and
open a task with `TaskCreate`:

- subject: `Fix <fixture_slug>`
- description: one-paragraph summary of the failure mode (error count by
  category, notable coordinates, snapshot path). Mention any prior related
  issues if obvious (search `CLAUDE.md`'s tier ladder for linked GH issues
  on the same recipe).

### 6. Report back

Short summary to the user:
- fixture slug
- e2e test location (`crates/core/tests/e2e.rs:<line>`)
- landing card added
- pass/fail status + top error/warning categories
- snapshot path if dumped
- task ID if a fix task was opened

Keep it tight — this is a setup skill, not a debugging session. The user
will dig into the fix separately.
