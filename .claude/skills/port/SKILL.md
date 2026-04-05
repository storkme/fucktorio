---
name: port
description: Delegate a Python-to-Rust port unit to a Sonnet subagent. Use when the user says "/port <unit-name>", e.g. "/port bus-common", "/port validate-power". The skill reads the unit brief from docs/port-plan.md and spawns one foreground Sonnet worker in an isolated worktree.
allowed-tools: Read, Bash, Agent
---

# Port — Delegate a single port unit

The user is asking you to delegate one port unit from `docs/port-plan.md` to a Sonnet subagent.

## Steps

### 1. Parse the unit name

The user said something like `/port bus-common` or `/port validate-power`. Extract the canonical unit name from their message.

If no unit name was given, list the available units from `docs/port-plan.md` (search for `^#### ` headings under the "Unit list" section) and ask the user which one.

### 2. Look up the unit brief

Read `docs/port-plan.md` and find the section whose heading matches the unit name (format: `#### \`unit-name\``).

Extract:
- **Scope** line
- **Python** reference file and line range
- **Rust** target file(s)
- **Dependencies** — list of prerequisite unit names
- **Size** estimate
- **Done when** criterion

Also read the "Shared context" section near the top of `docs/port-plan.md` — the worker needs this verbatim.

### 3. Verify dependencies are merged

For each listed dependency (e.g. `bus-common`, `astar`), check that the corresponding Rust code exists in `crates/core/src/`. Quick checks:
- `bus-common` → `crates/core/src/common.rs` exists
- `bus-placer` → `crates/core/src/bus/placer.rs` exists
- `astar` → `crates/core/src/astar.rs` exists (already merged)
- `validate-types` → `crates/core/src/validate/mod.rs` exists

If a dependency is missing, tell the user and stop. Do not spawn the worker — they'd be blocked.

### 4. Spawn the Sonnet worker

Use the `Agent` tool with:
- `subagent_type: "general-purpose"`
- `model: "sonnet"`
- `isolation: "worktree"`
- `run_in_background: false` (foreground — user wants to wait for this one)
- `description`: short phrase like "Port: bus-common"
- `prompt`: the full brief (see template below)

### 5. Report the result

When the agent completes, parse the `PR: <url>` line from its output and tell the user:
```
<unit-name> ported → <PR URL>
```

If the agent failed, summarise why in one line and suggest a next step.

## Worker prompt template

Assemble this prompt for the Agent tool. Replace `{UNIT_NAME}`, `{SCOPE}`, `{PYTHON_REF}`, `{RUST_TARGET}`, `{DEPS}`, `{SIZE}`, `{DONE_WHEN}` with values from the port-plan.md unit section. Copy `{SHARED_CONTEXT}` verbatim from the "Shared context" section of port-plan.md.

```
## Overall goal
Port a Python module to Rust in crates/core for the Fucktorio web app. This is one unit in a larger porting effort — see docs/port-plan.md for the full plan.

## Your unit: {UNIT_NAME}

**Scope**: {SCOPE}

**Python reference**: {PYTHON_REF}

**Rust target**: {RUST_TARGET}

**Dependencies (already merged)**: {DEPS}

**Size estimate**: {SIZE}

**Done when**: {DONE_WHEN}

## Approach

1. Read the Python reference file carefully. Understand the algorithm before translating.
2. Check what's already in `crates/core/src/` — do NOT duplicate types or helpers that already exist.
3. Create the target Rust file(s). Add module declarations to `crates/core/src/lib.rs` or the relevant `mod.rs`.
4. Port the Python logic as faithfully as possible. This is a translation, not a rewrite.
5. Write unit tests that mirror the Done-When criterion. Where possible, compare against Python output for the same fixture.
6. Verify: `cargo check --workspace` is clean, `cargo test -p fucktorio_core` passes.

## Shared context (project conventions)

{SHARED_CONTEXT}

## Worker instructions

After you finish implementing the change:
1. **Simplify** — Invoke the `Skill` tool with `skill: "simplify"` to review and clean up your changes.
2. **Run tests** — Run `cargo test -p fucktorio_core` and `cargo check --workspace`. Fix any failures.
3. **Commit and push** — Commit with a clear message, push the branch, and create a PR with `gh pr create`. Use a descriptive title like "Port {UNIT_NAME} to Rust".
4. **Report** — End with a single line: `PR: <url>`. If no PR was created, end with `PR: none — <reason>`.
```

## Notes

- **Foreground mode**: this skill spawns the agent in the foreground and waits. The user is okay being blocked — they'll typically queue one unit at a time.
- **If the user wants multiple units in parallel**, tell them the standard `/batch` workflow is better for that — this skill is for single-unit delegation.
- **If Cargo.toml conflicts with another open PR**, the worker will handle it via rebase later when the user merges. Don't block on that here.
- **Do not run the worker on a unit whose dependencies aren't merged yet** — the code won't compile.
