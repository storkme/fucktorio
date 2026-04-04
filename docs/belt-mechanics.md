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

### Sideloading onto a UG input

**CRITICAL QUIRK: sideloading onto a UG input fills only the FAR lane, not the near lane.**

This is the opposite of regular belt sideloading. On a regular belt, sideloading fills the near lane. On a UG input, sideloading fills the far lane.

```
    v   (feeder belt, facing SOUTH)
  >>V   (UG input, facing EAST -- feeder hits north side)
```

The feeder approaches from the north side. For an EAST-facing belt the north side is the left (near) lane. But because this is a UG input, items go to the FAR lane (right/south) instead.

### Consequence

**You MUST feed UG inputs straight (same direction) to load both lanes.** If you need to change direction before a UG input, place a turn belt first, then feed the UG input straight from behind. Never rely on sideloading a UG input to fill both lanes.

## Underground belt exits

### Items emerging from a UG exit

Items emerge on both lanes in the UG's facing direction, same as any belt. A UG exit behaves like a normal belt for most purposes.

### Sideloading FROM a UG exit

When a UG exit belt is adjacent to another belt and forms a T-junction, it acts as a sideload feeder. The receiving belt's **near lane** (closest to the UG exit) gets the items -- standard sideload rules apply.

### Sideloading ONTO a UG exit

Feeding into the side of a UG exit tile from a perpendicular belt: this is blocked on the lane where items are emerging from underground. The emerging items have priority. In practice, avoid sideloading onto UG exits -- the behavior is unreliable and layout-dependent.

## Splitters

Splitters occupy 2 tiles (1x2 perpendicular to facing direction). They take one input belt and produce two output belts.

### Default behavior

- Items are distributed **50/50** between the two output belts.
- **Lane assignment is preserved**: left-lane items stay on the left lane of whichever output belt they go to; same for right lane.
- If one output is blocked/full, all items go to the other output.

### Priority and filtering

- **Input priority** (left/right): preferentially pull from one input belt.
- **Output priority** (left/right): preferentially send to one output belt.
- **Filter**: send a specific item to one side, everything else to the other.

These settings are available in Factorio but the layout engine currently uses only default (unfiltered, no priority) splitters for lane balancing.

## Implications for bus routing

These rules constrain how the bus layout engine (`src/bus/bus_router.py`) builds trunk-to-row connections.

### Trunk-to-tapoff turns

When a vertical trunk (SOUTH) turns EAST into a row's input belt, the turn tile **must be a surface belt**, not a UG input. A surface belt turn preserves both lanes. If the turn were a UG input receiving a sideload, only the far lane would get items.

### Underground trunk crossings

When a tap-off needs to cross another trunk lane, the tap-off goes underground (EAST). The UG entry must be fed **straight** (from a SOUTH-to-EAST turn belt behind it), not sideloaded from the trunk above. This ensures both lanes enter the underground segment.

### Output returns via sideload

When a row's output belt returns items to a trunk, it sideloads from one side. This fills only the **near lane** of the trunk. To fill both trunk lanes, the bus router uses splitter-based lane balancing: a second producer's output sideloads from the **opposite side** of the trunk.

### Merger underground crossings

When output mergers route WEST toward the final output columns, underground crossings are used to hop over other trunk lanes. The same straight-feed rule applies -- UG entries must be approached straight, never sideloaded.

## Quick reference

| Scenario | Lanes filled | Safe? |
|----------|-------------|-------|
| Straight feed into belt | Both | Yes |
| Sideload onto belt | Near only | Yes (one lane) |
| Straight feed into UG input | Both | Yes |
| Sideload onto UG input | **Far only** | AVOID -- use straight feed |
| Items from UG exit | Both | Yes |
| Sideload from UG exit onto belt | Near lane of receiver | Yes |
| Sideload onto UG exit | Blocked/unreliable | AVOID |
| Splitter output | Both, 50/50 split | Yes |
| Belt turn (90 deg) | Both (inner/outer preserved) | Yes |
