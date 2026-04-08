# Factorio Game Mechanics Reference

Formal rules the layout engine must satisfy. Statements are numbered per section and build on each other. Later statements may reference earlier ones.

---

## Entities & Grid

- **G1.** The world is a 2D grid of 1x1 tiles, addressed by integer (x, y) coordinates.
- **G2.** Every entity occupies a rectangular footprint of WxH tiles, centered on an anchor tile.
- **G3.** No two entities may occupy the same tile (no overlap). *(Core constraint for all placement.)*
- **G4.** Each entity has a **direction**: one of NORTH, SOUTH, EAST, WEST (encoded 0, 4, 8, 12 in Factorio's coordinate system).
- **G5.** Rotation changes the direction but does not change footprint dimensions for square entities (3x3, 5x5). Non-square entities (e.g. splitters, 2x1) swap W and H on rotation.
- **G6.** Common entity sizes:
  - 1x1: belt, underground belt, splitter half-tile, pipe, pipe-to-ground, inserter, electric pole (small)
  - 2x1: splitter (perpendicular to facing direction)
  - 2x2: medium electric pole
  - 3x3: assembling machine, chemical plant, electric mining drill
  - 5x5: oil refinery

---

## Belts

- **B1.** A belt tile occupies 1x1 and has a facing direction (G4).
- **B2.** Each belt has exactly two **lanes**: left and right, defined relative to the belt's facing direction (standing behind the belt, looking forward).
- **B3.** Lane orientation by direction:

  | Facing | Left lane side | Right lane side |
  |--------|---------------|-----------------|
  | NORTH  | west          | east            |
  | SOUTH  | east          | west            |
  | EAST   | north         | south           |
  | WEST   | south         | north           |

- **B4.** Both lanes move items in the facing direction at the belt's tier speed.
- **B5.** Throughput per tier (total, both lanes): yellow 15/s, red 30/s, blue 45/s. Each lane carries exactly half (7.5/s, 15/s, 22.5/s per lane).
- **B6.** Two adjacent belts facing the same direction, placed end-to-end, connect automatically: items flow from the upstream belt to the downstream belt.
- **B7.** **Straight merge**: a belt facing into the back of another same-direction belt feeds both lanes normally.
- **B8.** **Sideload**: a belt feeding perpendicular into the side of another belt fills **only the near lane** (the lane of the target belt closest to the feeder). *(Critical for lane-specific routing.)*
- **B9.** If the target lane (B8) is full, items back up on the feeder belt.
- **B10.** To fill both lanes of a belt, use either a straight feed (B7) or two sideloads from opposite sides.
- **B11.** A 90-degree **turn** (belt A facing into the side of belt B, where B continues in the perpendicular direction) preserves both lanes. Inner-lane items stay on the inner lane, outer on outer. No items are lost or merged.
- **B12.** **Belt weaving / underground crossing**: to cross one belt line over another without mixing items, use an underground belt pair. Place a UG input before the crossing belt, and a UG output after it. Items travel underground past the crossing belt (U4). This is the standard technique for bus tap-offs crossing adjacent trunk belts.

---

## Underground Belts

- **U1.** An underground belt pair consists of an **input** (entrance) and **output** (exit), both occupying 1x1, on the same axis, facing the same direction.
- **U2.** The input faces the travel direction; the output faces the same direction and is placed downstream.
- **U3.** Maximum underground distance per tier: yellow 4 tiles, red 6 tiles, blue 8 tiles (gap between input and output, exclusive).
- **U4.** Items travel underground between input and output, passing under any entities on tiles between them. *(Enables crossing other belt lines and pipes.)*
- **U5.** Underground belt pairs must be the **same tier** and on the **same axis**. An input pairs with the nearest unpaired output of matching tier on the same axis. *(Mismatched pairing is a common layout bug.)*
- **U6.** **Straight feed into UG input** (same direction as the UG): loads both lanes normally. This is the safe default.
- **U7.** **Sideloading onto a UG input fills only the FAR lane** (the lane of the UG farthest from the feeder), not the near lane. This is the opposite of normal belt sideloading (B8). *(Critical quirk -- the generator must never rely on sideloading a UG input to fill both lanes.)*
- **U8.** To load both lanes of a UG input, always feed it straight from behind (U6). If a direction change is needed, place a turn belt first, then feed the UG input straight.
- **U9.** Items emerge from a UG output on both lanes, same as a normal belt.
- **U10.** A UG output sideloading into another belt follows normal sideload rules (B8): fills the near lane of the receiving belt.

---

## Splitters

- **S1.** A splitter occupies 2x1 tiles (2 wide perpendicular to facing direction, 1 deep along it).
- **S2.** It accepts up to 2 input belts (one per tile on the input side) and produces up to 2 output belts (one per tile on the output side).
- **S3.** Default behavior: items are distributed 50/50 between the two output belts.
- **S4.** Lane assignment is preserved: left-lane items remain on the left lane of whichever output belt they reach; same for right lane.
- **S5.** If one output belt is blocked/full, all items route to the other output belt.
- **S6.** **Input priority** (left/right): preferentially pull from one input belt first.
- **S7.** **Output priority** (left/right): preferentially send to one output belt first.
- **S8.** **Filter mode**: route a specific item type to one output, everything else to the other.
- **S9.** **Tap-off from a running trunk**: to branch items off a belt that must continue flowing, a splitter is required. A belt turn or sideload redirects or copies items but does NOT split the flow — the trunk belt would terminate or starve downstream. The standard bus tap-off pattern: place a South-facing splitter on the trunk; one output continues South (trunk), the other output feeds East (tap-off to consumer row). The splitter occupies the trunk column and one adjacent column.

---

## Inserters

- **I1.** An inserter occupies 1x1 and has a facing direction (G4).
- **I2.** An inserter picks items from the tile **behind** it (opposite to facing direction) and drops them on the tile **ahead** (in the facing direction).
- **I3.** **Regular inserter**: pickup and drop tiles are each 1 tile away from the inserter (reach = 1).
- **I4.** **Long-handed inserter**: pickup and drop tiles are each 2 tiles away from the inserter (reach = 2). *(Allows feeding across a belt line or gap.)*
- **I5.** Inserters interact with belt lanes: an inserter dropping onto a belt places items on the **near lane** (the lane closest to the inserter). Geometrically, "near lane" is determined by the dot product of the inserter's approach vector and the belt's perpendicular: positive → right lane, negative → left lane.
- **I6.** **Pickup from belts**: an inserter picking from a belt grabs items from **both lanes**, not just the near lane. It alternates between lanes based on item availability. The effective pickup rate is limited by the total belt throughput (both lanes combined), not a single lane. *(This means a fully loaded belt delivers its full throughput to the inserter, regardless of lane distribution.)*
- **I7.** Inserters can pick from / drop into machines, belts, chests, and other entities that have item slots.
- **I8.** Inserter throughput (approximate, chest-to-chest at default stack size):

  | Type | Items/swing | Swings/s | Throughput |
  |------|------------|----------|------------|
  | Regular inserter | 1 | ~0.83 | ~0.83/s |
  | Long-handed inserter | 1 | ~1.2 | ~1.2/s |
  | Fast inserter | 1 | ~2.31 | ~2.31/s |
  | Stack inserter (stack 12) | 12 | ~2.31 | ~27.7/s |

  Actual throughput varies with pickup/drop distance and belt speed. Multiple inserters may be needed to saturate a belt lane (7.5/s yellow → need ~9 regular inserters or ~4 fast inserters per lane).

- **I9.** **Stack inserter**: picks/drops multiple items per swing (stack size depends on research, max 12). Higher throughput than regular inserters. *(Relevant for high-throughput designs.)*
- **I10.** An inserter dropping into a machine will only insert items that the machine's current recipe accepts. *(No explicit filter needed for recipe-locked machines.)*

---

## Machines

- **M1.** A machine (assembler, chemical plant, refinery) occupies a fixed footprint (G6) and is assigned exactly one recipe.
- **M2.** Each recipe specifies a set of ingredient item/fluid types with quantities, a set of product item/fluid types with quantities, and a crafting time.
- **M3.** **Crafting speed** is a multiplier on recipe time. Assembling machine 1: 0.5, AM2: 0.75, AM3: 1.25. Chemical plant: 1.0. Oil refinery: 1.0.
- **M4.** Solid ingredients are delivered by inserters (I1-I9); solid products are extracted by inserters.
- **M5.** Fluid ingredients/products are transferred through **fluid ports** at specific tile positions on the machine's boundary. *(Port positions are fixed per entity type and direction -- must be queried from game data.)*
- **M6.** Fluid ports connect to adjacent pipes; a pipe or pipe-to-ground must be placed on the adjacent tile to transfer fluid. Known fluid input port dx offsets from machine left edge (facing North): assembling-machine-2/3 = 1, others = 0. *(Full port positions should be queried from draftsman for non-standard machines.)*
- **M7.** Machine sizes relevant to the generator:
  - Assembling machine (1/2/3): 3x3, 4 potential inserter sides
  - Chemical plant: 3x3, has fluid ports (positions depend on direction)
  - Electric furnace: 3x3
  - Oil refinery: 5x5, has multiple fluid ports (3 inputs, 2 outputs in vanilla)
  - Electromagnetic plant (Space Age): 4x4
  - Cryogenic plant (Space Age): 5x5
  - Foundry (Space Age): 5x5
  - Biochamber (Space Age): 3x3

---

## Fluids & Pipes

- **F1.** A pipe occupies 1x1 and connects to **all four** adjacent pipes automatically. *(Unlike belts, pipes have no direction -- fluid flows wherever there is a connection.)*
- **F2.** A pipe network is the connected component of all mutually adjacent pipes and fluid ports.
- **F3.** A pipe network may carry only **one fluid type**. Mixing fluids in one network is an error. *(Different fluid networks MUST be physically isolated -- no shared tiles, no adjacency.)*
- **F4.** **Pipe-to-ground** occupies 1x1 and has a facing direction. It connects underground to the nearest pipe-to-ground of opposite facing on the same axis, within a max distance (vanilla: 10 tiles).
- **F5.** Pipe-to-ground has a surface connection on the side opposite to its facing direction, and an underground connection in the facing direction. The surface side connects to adjacent pipes normally.
- **F6.** Pipes and belts on the same tile are **not** possible (both occupy the full tile), but pipes and belts on adjacent tiles do not interfere. *(Pipes and belts can run in parallel on neighboring columns.)*
- **F7.** Pipe-to-ground pairs allow fluid lines to cross under belt lines without interference (analogous to U4 for belts).

---

## Power

- **P1.** All machines, inserters, and other active entities require electricity to operate. An unpowered entity does nothing.
- **P2.** Electricity is delivered via **electric poles** that define a supply area.
- **P3.** **Small electric pole**: 1x1 footprint, 5x5 supply area (centered), wire reach 7.5 tiles.
- **P4.** **Medium electric pole**: 2x2 footprint, 7x7 supply area (centered), wire reach 9 tiles.
- **P5.** An entity is powered if any tile of its footprint falls within at least one pole's supply area.
- **P6.** Poles connect to each other via copper wire if within wire reach, forming the electric network. At least one pole must connect (directly or transitively) to a power source.
- **P7.** For the generator, pole placement must ensure every machine and inserter is within a pole's supply area (P5), and all poles form a connected network (P6). *(Medium electric poles are the standard choice -- good coverage-to-footprint ratio.)*

---

## Modules & Beacons

- **MB1.** **Modules** can be inserted into machine module slots to modify crafting speed, productivity, energy consumption, and quality (Space Age).
- **MB2.** Module slot counts per machine:

  | Machine | Slots |
  |---------|-------|
  | Assembling machine 1 | 0 |
  | Assembling machine 2 | 2 |
  | Assembling machine 3 | 4 |
  | Chemical plant | 3 |
  | Oil refinery | 3 |
  | Electric furnace | 2 |
  | Electromagnetic plant | 5 |
  | Cryogenic plant | 8 |
  | Foundry | 4 |
  | Biochamber | 4 |

- **MB3.** Module effects (speed bonus / productivity bonus per module):

  | Module | Speed | Productivity |
  |--------|-------|-------------|
  | Speed module 1 | +20% | — |
  | Speed module 2 | +30% | — |
  | Speed module 3 | +50% | — |
  | Productivity module 1 | -5% | +4% |
  | Productivity module 2 | -10% | +6% |
  | Productivity module 3 | -15% | +10% |

  Effects are additive across all modules in the machine. Productivity modules can only be placed in machines that craft intermediate products (not raw resources).

- **MB4.** **Beacons** broadcast module effects to nearby machines. A beacon occupies 3x3 tiles, has 2 module slots, and a supply distance of 3 tiles (measured from the beacon's edge to the machine's edge).
- **MB5.** Beacon **distribution effectivity** is 50% — each module in a beacon applies half its effect to every machine in range. Multiple beacons stack additively.
- **MB6.** For the layout generator, beacon placement affects machine throughput calculations: a machine surrounded by speed beacons crafts faster, requiring higher belt throughput for inputs/outputs.

---

## Space Age (Factorio 2.0+)

- **SA1.** Entities with fluid boxes (oil refinery, chemical plant, etc.) support a **`mirror: true`** blueprint attribute that flips fluid port positions along the entity's primary axis.
- **SA2.** Combined with direction (G4), mirroring gives up to 8 orientations (4 rotations x 2 mirror states).
- **SA3.** For oil refinery: `mirror=true` flips inputs-south/outputs-north to inputs-north/outputs-south, enabling the same header-above-machine layout pattern used for chemical plants.
- **SA4.** The `mirror` attribute is only effective in Factorio 2.0+ (Space Age). It is ignored by Factorio 1.1.
