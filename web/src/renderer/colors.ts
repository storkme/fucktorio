// Entity color palette — ported from src/visualize.py.
// Colors are expressed as 0xRRGGBB hex numbers for PixiJS.

export const ENTITY_COLORS: Record<string, number> = {
  // Assemblers — blue-grey family
  "assembling-machine-1": 0x5a6e82,
  "assembling-machine-2": 0x4a6278,
  "assembling-machine-3": 0x3a526a,

  // Furnaces
  "stone-furnace": 0x8a6040,
  "steel-furnace": 0x7a5030,
  "electric-furnace": 0x6a5a80,

  // Specialised machines
  "chemical-plant": 0x3a7a50,
  "oil-refinery": 0x5a3a8a,
  centrifuge: 0x3a7a80,
  lab: 0x4a6a50,
  "rocket-silo": 0x4a4a6a,

  // Infrastructure — belts
  "transport-belt": 0xc8b560,
  "fast-transport-belt": 0xe05050,
  "express-transport-belt": 0x50a0e0,
  "underground-belt": 0xa89040,
  "fast-underground-belt": 0xe05050,
  "express-underground-belt": 0x50a0e0,
  splitter: 0xc8b560,
  "fast-splitter": 0xe05050,
  "express-splitter": 0x50a0e0,

  // Inserters
  inserter: 0x6a8e3e,
  "fast-inserter": 0x4a90d0,
  "long-handed-inserter": 0xd04040,

  // Pipes / power
  pipe: 0x4a7ab5,
  "pipe-to-ground": 0x3a6090,
  pump: 0x4a7a6a,
  "medium-electric-pole": 0x8b6914,
  "small-electric-pole": 0xa67c20,
};

export const DEFAULT_COLOR = 0x888888;
