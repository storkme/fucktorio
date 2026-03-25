"""Shared data structures for the solver → layout → blueprint pipeline."""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum

# ---------------------------------------------------------------------------
# Solver-level models (no positions)
# ---------------------------------------------------------------------------


@dataclass
class ItemFlow:
    """An item flowing at a certain rate."""

    item: str  # e.g. "iron-plate"
    rate: float  # items / second
    is_fluid: bool = False


@dataclass
class MachineSpec:
    """One production step: which machine, which recipe, how many."""

    entity: str  # e.g. "assembling-machine-3"
    recipe: str  # e.g. "electronic-circuit"
    count: float  # fractional – caller decides rounding
    inputs: list[ItemFlow] = field(default_factory=list)  # per-machine rates
    outputs: list[ItemFlow] = field(default_factory=list)  # per-machine rates


@dataclass
class SolverResult:
    """Everything the solver produces – no positional data."""

    machines: list[MachineSpec]
    external_inputs: list[ItemFlow]  # items the user must supply
    external_outputs: list[ItemFlow]  # items that leave the factory
    # recipe → list of ingredient recipes (topological order helpers)
    dependency_order: list[str] = field(default_factory=list)


# ---------------------------------------------------------------------------
# Layout-level models (positions, no rates)
# ---------------------------------------------------------------------------


class EntityDirection(int, Enum):
    """Matches draftsman Direction constants (16-way, we only use 4)."""

    NORTH = 0
    EAST = 4
    SOUTH = 8
    WEST = 12


@dataclass
class PlacedEntity:
    """A single entity placed in the blueprint grid."""

    name: str  # e.g. "assembling-machine-3"
    x: int = 0
    y: int = 0
    direction: EntityDirection = EntityDirection.NORTH
    recipe: str | None = None  # only for crafting machines
    io_type: str | None = None  # "input" or "output" for underground belts
    entity_number: int | None = None  # assigned during blueprint export


@dataclass
class Connection:
    """A wire / circuit / power connection between two entities."""

    type: str  # "power" | "red" | "green"
    from_index: int  # index into PlacedEntity list
    to_index: int


@dataclass
class LayoutResult:
    """Everything the layout engine produces – no rate data."""

    entities: list[PlacedEntity]
    connections: list[Connection] = field(default_factory=list)
    width: int = 0
    height: int = 0
