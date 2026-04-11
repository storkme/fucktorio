# Fluid Row Pattern

The layout engine's convention for placing pipes on rows that feed fluid ingredients to a machine. Not a Factorio rule — just a specific pattern that is simple to reason about, easy to stack multiple fluids, and guarantees per-fluid isolation.

## Pattern for one fluid input

For each fluid consumed by the row's recipe, emit a **vertical column** that carries that fluid from the east–west fluid trunk down through the solid belt layer and directly into the machine's fluid port.

ASCII for a chemical plant at `(mx, my)` with a single fluid port at `(mx + port_dx, my)` (so the port-adjacent tile is `(mx + port_dx, my - 1)`):

```
y = my - 4   ...── UG ── pipe ── UG ──...    ← east-west fluid trunk
                         │                     (pipe is ONLY at T-intersection;
                         │                      flanking tiles are UG pipes)
y = my - 3           [UG pipe IN]             ← facing south, enters tunnel
y = my - 2         [solid belt row]           ← ingredient belt (tunnel crosses under)
y = my - 1           [UG pipe OUT]            ← facing south, adjacent to port
y = my             ▓▓▓ machine top (port) ▓▓▓
y = my + 1         ▓▓▓
y = my + 2         ▓▓▓
```

Key rules:

1. **UG pipe OUT is directly adjacent to the machine's fluid port** — no regular pipe between them. One tile of overhead.
2. **The solid belt passes between the UG input and UG output** — fluid tunnels under the belt. UG reach is 10 tiles so the 2-tile span is trivial.
3. **At the trunk row (y = my - 4), the T-intersection uses `UG — pipe — UG`** — the vertical branch dropping into the UG pipe IN is a single regular pipe. The tiles to its left and right on the trunk are pipe-to-ground (pipe underground) inputs/outputs, not regular pipes. This prevents a pipe at an adjacent trunk row from accidentally merging into this fluid network via adjacency.
4. **The inserter(s) for solid inputs go on the same row as the UG pipe OUT**, at different x columns (typically the center column of the machine or adjacent to the port). The UG pipe OUT and the inserter share the "interface row" between the belt and the machine.

## Pattern for multiple fluid inputs

Each additional fluid input gets its own vertical column:

- One extra trunk row above the solid belt for the new fluid (isolated via the `UG–pipe–UG` trick)
- Extra columns of (UG in → tunnel under belt → UG out → port) at the machine's other fluid-port x positions

For a recipe with N fluid inputs, the row vertical budget grows by roughly `N + 1` tiles above the solid belt (1 for each fluid trunk row, plus 1 for the UG-in row above the belt). Every fluid stays in its own isolated network.

## Why UG pipes on the trunk row flanks

Regular pipes connect to ALL adjacent pipes (rule F2 in the mechanics doc). If two different fluid trunk rows are stacked vertically (petroleum-gas at `y = -4`, sulfuric-acid at `y = -5`), and both use regular pipes along the trunk, the two networks would merge wherever they're adjacent — one tile of contact is enough to mix fluids (rule F3).

Using `UG–pipe–UG` means the ONLY regular-pipe tile on a given trunk row is at each machine's T-intersection. If T-intersections for different fluids don't line up on the same x column (and they generally won't, because different recipes have different x positions), the trunks never touch.

## Why it scales

Adding a new fluid is:
- One extra row at the top of the layout (new trunk)
- One extra column per machine that needs it (new T)

Both are linear in the number of fluids, not quadratic. Every fluid's plumbing is locally isolated, so placement doesn't have to reason about global fluid connectivity.

## Limitations

- **Not always the most compact pattern.** Some recipes could pack fluids more tightly by reusing trunks or sharing columns — we're explicitly choosing simplicity over density.
- **Requires the row pitch to accommodate the extra trunk rows above the machine.** The placer's row-height calculation has to account for fluid count, not just "has fluid: yes/no".
- **Doesn't handle multi-fluid-output recipes directly.** Oil refinery (3 outputs) needs a different pattern — it already has `RowKind::OilRefinery`. This pattern is for *consuming* fluids, not producing them.

## Related mechanics

- Rule **F3** (different fluids must be physically isolated) is the reason we need the UG-flank isolation trick
- Rule **F4** (pipe-to-ground max reach 10 tiles) gives us plenty of headroom for the short tunnel under the solid belt
- Rule **U7/U8** (UG input sideload restrictions) doesn't apply to pipe-to-ground — it's belt-specific
