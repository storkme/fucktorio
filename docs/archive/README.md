# Archived design docs

These documents describe **historical** project state — proposals,
investigations, and refactors that have either landed, been
superseded, or were paused. They are kept as prior art and decision
context but are **not current**.

For the live pipeline reference see:

- Top-level `CLAUDE.md` for the architecture overview.
- [`../ghost-pipeline-contracts.md`](../ghost-pipeline-contracts.md)
  for the phase-by-phase contract the ghost router upholds.
- [`../junction-solver-followups.md`](../junction-solver-followups.md)
  for the live junction-solver work-in-progress notes.

## What's here

| File | Status |
|---|---|
| `rfp-band-regions.md` | Rejected — SAT-on-bands produced cycles / phantom belts. Superseded by the junction solver. |
| `rfp-belt-flow-aware-astar.md` | Aspirational — still-valid design for a future spaghetti-routing revival. |
| `rfp-ghost-cluster-routing.md` | Implemented — ghost A* + union-find + cluster SAT (the current pipeline). |
| `rfp-ghost-occupancy-refactor.md` | Implemented — typed `Occupancy` map; see `crates/core/src/bus/ghost_occupancy.rs`. |
| `rfp-junction-solver.md` | Partially implemented — T1 perpendicular template + SAT fallback wired; T2/T3 pending. |
| `rfp-region-routing.md` | Partially implemented — framework + tier 1 in place; tier 2/3 pending. |
| `sat-band-investigation.md` | Investigation notes explaining why SAT bands failed. |
| `spaghetti-roadmap.md` | Original spaghetti-router roadmap from before the pivot to bus/ghost routing. |
| `port-plan.md` | Original Python → Rust port plan. |
| `port-status.md` | Per-unit Python → Rust port status at the time the port wrapped up. |

Anything you need from these that isn't in the current docs, you can
read directly — just remember they describe a point in time, not
today's behaviour.
