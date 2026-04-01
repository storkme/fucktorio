"""Data models for blueprint analysis — descriptive (what was built), not prescriptive."""

from __future__ import annotations

from dataclasses import dataclass, field

from ..models import EntityDirection


@dataclass
class AnalyzedMachine:
    """A machine discovered in a parsed blueprint."""

    id: int
    name: str  # "assembling-machine-3", "chemical-plant", etc.
    recipe: str | None
    position: tuple[int, int]  # top-left tile
    size: int  # 3 or 5
    inputs: list[str] = field(default_factory=list)  # item names from recipe
    outputs: list[str] = field(default_factory=list)


@dataclass
class TransportSegment:
    """A single belt or pipe tile in a transport network."""

    position: tuple[int, int]
    name: str
    direction: EntityDirection
    is_underground: bool = False
    io_type: str | None = None  # "input"/"output" for underground


@dataclass
class TransportNetwork:
    """A connected component of belt or pipe tiles."""

    id: int
    type: str  # "belt" or "pipe"
    segments: list[TransportSegment] = field(default_factory=list)
    inferred_item: str | None = None
    # Computed metrics
    path_length: int = 0  # total tiles
    turn_count: int = 0  # direction changes (belts only)
    underground_count: int = 0  # underground segments


@dataclass
class InserterLink:
    """An inserter bridging a machine to a transport network."""

    position: tuple[int, int]
    direction: EntityDirection
    machine_id: int
    network_id: int | None = None  # None if no belt/pipe found on other side
    role: str = "input"  # "input" (belt→machine) or "output" (machine→belt)
    inferred_item: str | None = None


@dataclass
class FluidLink:
    """A direct fluid port connection (pipe network touching machine port)."""

    machine_id: int
    network_id: int
    role: str  # "input" or "output" (from machine's perspective)
    inferred_item: str | None = None


@dataclass
class ProductionEdge:
    """A logical item/fluid flow between machines via a transport network."""

    item: str
    from_machine: int | None  # None = external input
    to_machine: int | None  # None = external output
    network_id: int


@dataclass
class BlueprintGraph:
    """The full analyzed representation of a blueprint."""

    machines: list[AnalyzedMachine] = field(default_factory=list)
    networks: list[TransportNetwork] = field(default_factory=list)
    inserter_links: list[InserterLink] = field(default_factory=list)
    fluid_links: list[FluidLink] = field(default_factory=list)
    edges: list[ProductionEdge] = field(default_factory=list)
    unhandled: list[str] = field(default_factory=list)  # entity names we skipped
