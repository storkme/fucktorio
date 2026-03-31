# Reference Blueprint: Processing Unit (single cell)

Blueprint string decoded 2026-03-31. The full factory (28 machines on factorioblueprints.com) is this
cell tiled **7× vertically**.

## Summary

| Property | Value |
|----------|-------|
| Recipe | `processing-unit` |
| Machines per cell | 4 × `assembling-machine-2` |
| Cell dimensions | 13 × 14 tiles |
| Total entities | 78 |
| Machine modules | 2× `productivity-module-3` each |
| Beacons | 4× (2× `speed-module-3` each), one per machine |

## Layout (normalized, origin = top-left)

```
x:  0  1  2  3  4  5  6  7  8  9 10 11 12
y0: B  B  .  .  .  .  .  Bv .  .  .  .  .
y1: B  B  LI .  AM .  FI Bv .  BK .  .  .
y2: B  B  FI .  .  .  .  Bv .  .  .  .  PG
y3: B  B  EP .  P  PG EP Bv .  .  .  PG P
y4: B  B  LI .  .  .  .  Bv .  BK .  .  PG
y5: B  B  FI .  AM .  FI Bv .  .  .  .  .
y6: B  B  .  .  .  .  .  Bv .  .  .  .  .
y7: B  B  .  .  .  .  .  Bv .  BK .  .  .
y8: B  B  LI .  AM .  FI Bv .  .  .  .  .
y9: B  B  FI .  .  .  .  Bv .  .  .  .  PG
y10:B  B  EP .  P  PG EP Bv .  BK .  PG P
y11:B  B  LI .  .  .  .  Bv .  .  .  .  PG
y12:B  B  FI .  AM .  FI Bv .  .  .  .  .
y13:B  B  .  .  .  .  .  Bv .  .  .  .  .

B=transport-belt, Bv=transport-belt(S), AM=assembling-machine-2(3×3),
LI=long-handed-inserter(W), FI=fast-inserter(W),
P=pipe, PG=pipe-to-ground, EP=small-electric-pole, BK=beacon(3×3)
```

## Structure analysis

### Input belt bus (x=0, x=1)
Two parallel yellow belt lanes run the full height on the left, carrying solid ingredients
(electronic circuits, advanced circuits). Items flow **northward** (direction=0, default).

### Output belt (x=7)
Single yellow belt running south (direction=8). Carries finished processing units away.

### Inserter pattern (x=2, all facing West / dir=12)
Each machine gets:
- 1× `long-handed-inserter` at the machine's top row — reaches 2 tiles to grab from x=0
- 1× `fast-inserter` at the machine's middle row — grabs from x=1 (one tile away)
- 1× `fast-inserter` at the machine's bottom row — for output, depositing to belt at x=7 (via `fast-inserter` at x=6)

So inputs come from x=0 (long reach) and x=1 (short reach) — two separate item streams,
one per lane of the input bus.

### Fluid (sulfuric acid) — x=4,5 and x=11,12
Pipe network enters from the right side via `pipe-to-ground` pairs bridging under the output belt.
Each machine pair (rows 1-6 and rows 7-13) shares a pipe segment at x=4 connecting to the
machine's fluid port, with underground segments crossing the belt at x=5→x=11.

### Beacons (x=9, every 3 rows)
4 beacons, each covering 2 machines. They contain 2× speed-module-3, boosting machine crafting speed.

## What our generator needs to replicate

1. **Two-lane input bus** — left two columns, both lanes used, different items on each lane.
2. **Mixed inserter reach** — long-handed for the far lane (x=0), fast for the near lane (x=1).
3. **Dedicated output belt** — single column to the right of the inserter column, flowing away.
4. **Fluid port threading** — pipe enters from outside the cell, bridges under/over the output belt via underground pipes.
5. **Compact machine spacing** — machines at x=4 (center), gap=1 between adjacent cells (no gap between y5 and y8 machines — they share the x=4 column continuously).
6. **Tileability** — the cell boundary at y=0/y=13 is "open" belts, so cells can stack vertically without modification.

## Differences from our current generator

- Uses `assembling-machine-2` (we currently generate `assembling-machine-1`)
- Uses `long-handed-inserter` for far-lane reach (we only use regular/fast inserters)
- Input bus is a fixed 2-wide column rather than routed A* paths
- No beacons (we don't model those)
- Fluid enters from the side via underground bridges (our pipe routing is simpler)
- This is a **bus-style** layout vs our spaghetti place-and-route approach
