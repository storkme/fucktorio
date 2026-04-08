# Verifactory Comparison

**Date**: 2026-04-08
**Repo**: https://github.com/alegnani/verifactory

## What verifactory does

Formal verification of belt balancer properties using Z3 SAT solving. It encodes belt networks as graphs (Input, Output, Connector, Splitter, Merger nodes with capacity-annotated edges) and proves properties:

- **Belt Balancing**: All output belts receive equal flow rates
- **Equal Drain**: Any subset of outputs can draw items without affecting others
- **Throughput Unlimited**: Full input throughput reaches any output subset
- **Universal Balancer**: All three combined

Performance: <1s for 64×64 balancers.

## What we already validate

Our 21 checks (`crates/core/src/validate/`) cover structural and connectivity properties:

- Pipe isolation and fluid port connectivity
- Inserter chains, direction, conflicts
- Belt connectivity, flow paths, reachability, direction continuity
- Belt loops, dead ends, item isolation, junctions
- Underground belt pairs and sideloading
- Lane throughput (rate-based warnings)
- Power pole coverage

## Overlap

**Minimal.** Our suite checks structural correctness (are things connected?). Verifactory checks functional correctness (does this balancer mathematically balance?). They're complementary.

## Gaps we could fill

Verifactory's approach is narrowly scoped to balancers. But the underlying technique — flow-based analysis on a belt network graph — could be adapted for broader checks:

1. **Balancer template verification**: Prove our SAT-generated balancer templates are correct (they likely are by construction, but extra assurance)
2. **Input distribution**: Verify that when N producer belts feed into M consumer taps, items are evenly distributed
3. **Throughput saturation**: Prove that no belt segment is overloaded given the solver's rate calculations

Items 2 and 3 don't require Z3 — they can be done with flow analysis on the belt network graph we already build during validation.

## Integration assessment

- **Full integration** (Z3 dependency, blueprint→IR conversion): Not worth it. Heavy dependency for narrow use case.
- **Borrowing the approach** (flow-based belt network analysis): Worth doing. Our existing `belt_dir_map` and belt BFS infrastructure could support rate-propagation checks without SAT solving.
- **Status**: Investigated, no immediate action. Balancer verification added as potential future validation category.
