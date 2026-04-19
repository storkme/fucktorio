<!--
Keep this short. The goal is to capture intent and surface deviations,
not to ceremony-ise small changes. Trivial PRs (typo, one-liner, obvious
bug from a stack trace) can delete sections that don't apply — say so
explicitly rather than leaving them blank.
-->

## Intent

What problem this solves and why now. 1–3 sentences. If this implements an
RFP or closes an issue, link it.

## Scope

- **In:** what's included
- **Out:** what was deliberately not done (and why, if non-obvious)

## Verification

What was actually run — not "tests pass," but *which* tests, *which* URL in
the web app, *which* snapshot. For layout-engine changes follow the
verification protocol in [`CLAUDE.md`](../CLAUDE.md#verification-protocol-for-layout-engine-changes).

- [ ] `cargo test --manifest-path crates/core/Cargo.toml`
- [ ] `cargo clippy -- -D warnings`
- [ ] WASM rebuild + browser eyeball (URL: …)
- [ ] Snapshot inspected for the specific bug being fixed (if applicable)

## Deviations from intent

Anything that differs from what was discussed or RFP'd, judgement calls
made mid-flight, things intentionally deferred. **"None" is a valid answer
— write it explicitly so it's clear nothing was glossed over.**

## Models / contracts touched

Which abstraction this affects (e.g. `ghost-pipeline-contracts.md`,
`factorio-mechanics.md`, the tier ladder in `CLAUDE.md`). If a contract
changed, the doc update is part of this PR. "None" is fine for chores.
