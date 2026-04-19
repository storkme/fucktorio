# RFP template

Copy this file to `docs/rfp-<short-name>.md` for any non-trivial design
work. The goal of an RFP is **shared understanding before code** and
**a way to know when to stop** if the approach turns out to be wrong.

The dominant rework shape on this project is *exploration rework* — we
build something, discover it doesn't pan out, and pivot. The
**kill criteria** section below is the most important part of this
template. Without explicit kill criteria, exploration drags on past the
point where the evidence already says "this approach is wrong."

Existing examples live in [`docs/archive/`](archive/) — read one or two
before writing your own. Note that the archived ones predate this
template, so they don't have kill criteria; the new ones must.

---

# RFP: <Title>

## Summary

One paragraph. What is being proposed and why. Someone who has never
seen this should be able to read the summary and know what they'd be
agreeing to.

## Motivation

The concrete problem this solves. Prefer a real failing case (a test
that fails, a layout that produces N validator errors, a screenshot of
the bug) over abstract reasoning. If the problem isn't reproducible
*today*, that's a yellow flag — say so.

## Design

The proposed approach. Doesn't need to be exhaustive — enough that a
reader can predict what the diff will look like and spot disagreements
early. Include:

- The shape of the new code (types, functions, modules touched)
- How it integrates with existing pipeline phases
- Anything load-bearing about correctness or performance
- Trade-offs against alternatives you considered (and rejected)

## Kill criteria

**Required.** Explicit, observable conditions under which this RFP
should be abandoned or rethought. Phrase them so the answer is
unambiguous after you run the experiment.

Good examples:

- *"After implementing strategy 1 + 2, if the
  `tier4_advanced_circuit_from_ore_am1` snapshot still has phantom
  belts in any junction zone, this approach is wrong — the SAT zones
  are not the root cause."*
- *"If the new junction solver requires more than ~500 LOC of strategy
  code per template, the strategy abstraction is too granular and we
  should reconsider."*
- *"If end-to-end runtime regresses by >2x on the existing test corpus,
  we drop this even if correctness improves."*

Bad examples (vague, unfalsifiable, post-hoc):

- *"If it doesn't work, we'll stop."*
- *"If users don't like it."*
- *"If performance is bad."*

If you can't write a kill criterion you'd actually act on, the RFP
isn't ready — the success/failure shape is too fuzzy and you'll know
even less after building it.

## Verification plan

How you'll know this is done and correct, before writing the PR.
Reference the relevant verification protocol in
[`CLAUDE.md`](../CLAUDE.md#verification-protocol-for-layout-engine-changes)
where applicable. Include the specific tests, URLs, snapshots, or
trace-event signals you'll check.

## Phasing (optional)

If the work splits into landable chunks, list them here so partial
landings are obviously partial.

## Decision log

Append entries here as the RFP progresses. Date them.

- *2026-MM-DD — accepted, work tracked in #NNN.*
- *2026-MM-DD — phase 1 landed in <commit>; phase 2 deferred pending …*
- *2026-MM-DD — abandoned; kill criterion <X> tripped on <test>. See
  follow-up notes in `docs/<...>.md`. Move this file to `docs/archive/`
  with a status of "Rejected" in the archive README.*

The decision log is the part that prevents "why did we drop this?"
amnesia six months later. Don't skip it.
