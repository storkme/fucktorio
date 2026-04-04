# Factorio Belt Lane Mechanics

Reference for the lane-level physics rules that the layout engine must respect. Every routing decision that touches belts should be checked against this document.

## Belt basics

- Each belt occupies one tile and has a **facing direction** (N/S/E/W).
- Each belt has **two lanes**: left and right, relative to the belt's facing direction.
- Both lanes carry items in the facing direction at the belt's tier speed (yellow 15/s, red 30/s, blue 45/s).
- Each lane carries half the belt's total throughput (e.g. yellow = 7.5/s per lane).
- Adjacent belts facing the same direction connect naturally -- items flow from one to the next.
- Placing a belt facing into the side of another belt is a **sideload** (see below).

### Lane orientation

Standing behind the belt (looking in its facing direction):

| Facing | Left lane | Right lane |
|--------|-----------|------------|
| NORTH  | west side | east side  |
| SOUTH  | east side | west side  |
| EAST   | north side| south side |
| WEST   | south side| north side |

## Belt turns

A 90-degree turn preserves both lanes. Items on the inner lane stay on the inner lane; items on the outer lane stay on the outer lane.

- **Clockwise turn** (e.g. NORTH to EAST): left lane becomes near lane, right lane becomes far lane.
- **Counter-clockwise turn** (e.g. NORTH to WEST): right lane becomes near lane, left lane becomes far lane.

Both lanes are preserved -- no items are lost or merged. This makes turns safe for lane-specific routing.

## Sideloading

Feeding a belt **perpendicular** into the side of another belt (a T-junction):

```
    v  (feeder belt, facing SOUTH)
  >>>>>>  (target belt, facing EAST)
```

**Rule: sideloading only fills the NEAR lane** -- the lane of the target belt closest to the feeder.

In the example above, the feeder approaches from the north side of the EAST-facing target belt. The north side is the LEFT lane of an EAST belt. So only the left lane gets items.

### Sideloading implications

- To fill BOTH lanes of a target belt, you need either a straight feed (same direction) or two sideloads from opposite sides.
- A sideload can fill at most one lane's worth of throughput (half the belt's capacity).
- If the target lane is already full, sideloaded items back up on the feeder belt.

## Underground belt inputs

Underground belts consist of an **input** (entrance) and **output** (exit) placed some tiles apart. Items travel underground between them.

### Straight feed into UG input

Feeding a UG input from behind (same direction as the UG belt) loads **both lanes** normally. This is the standard, safe approach.

```
  v   (feeder belt, facing SOUTH)
  V   (UG input, facing SOUTH)
  ~   (underground)
  ^   (UG output, facing SOUTH)
```

### Sideloading onto a UG input/output

**CRITICAL QUIRK: sideloading onto a UG input or output only allows items to flow from the lane which is closer to the belt side of the underground entrance or exit.**

// todo expand this

### Consequence

**You MUST feed UG inputs straight (same direction) to load both lanes.** If you need to change direction before a UG input, place a turn belt first, then feed the UG input straight from behind. Never rely on sideloading a UG input to fill both lanes.

## Underground belt exits

### Items emerging from a UG exit

Items emerge on both lanes in the UG's facing direction, same as any belt. A UG exit behaves like a normal belt for most purposes.

### Sideloading FROM a UG exit

When a UG exit belt is adjacent to another belt and forms a T-junction, it acts as a sideload feeder. The receiving belt's **near lane** (closest to the UG exit) gets the items -- standard sideload rules apply.

## Splitters

Splitters occupy 2 tiles (1x2 perpendicular to facing direction). They take one or two input belts and produce one or two output belts. 

### Default behavior

- Items are distributed **50/50** between the two output belts.
- **Lane assignment is preserved**: left-lane items stay on the left lane of whichever output belt they go to; same for right lane.
- If one output LANE is blocked/full, all items go to the other output LANE.

### Priority and filtering

- **Input priority** (left/right): preferentially pull from one input belt.
- **Output priority** (left/right): preferentially send to one output belt.
- **Filter**: send a specific item to one side, everything else to the other.

These settings are available in Factorio but the layout engine currently uses only default (unfiltered, no priority) splitters for lane balancing.

