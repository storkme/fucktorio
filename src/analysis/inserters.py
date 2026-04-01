"""Step 3: Resolve inserter connections and fluid port links."""

from __future__ import annotations

from ..routing.common import DIR_VEC, machine_tiles
from .classify import _RawInserter
from .models import (
    AnalyzedMachine,
    FluidLink,
    InserterLink,
    TransportNetwork,
)

# Inserter reach by entity name
_INSERTER_REACH: dict[str, int] = {
    "inserter": 1,
    "fast-inserter": 1,
    "stack-inserter": 1,
    "long-handed-inserter": 2,
}


def resolve_inserters(
    raw_inserters: list[_RawInserter],
    machines: list[AnalyzedMachine],
    networks: list[TransportNetwork],
) -> list[InserterLink]:
    """Resolve each inserter's pickup/drop to a machine and network.

    An inserter at (ix, iy) facing direction d with reach r:
    - drops at (ix + dx*r, iy + dy*r)
    - picks up from (ix - dx*r, iy - dy*r)

    If the drop position is inside a machine → role="input" (belt feeds machine).
    If the pickup position is inside a machine → role="output" (machine feeds belt).
    """
    # Build lookup structures
    machine_tile_map = _build_machine_tile_map(machines)
    network_tile_map = _build_network_tile_map(networks)

    links: list[InserterLink] = []

    for ins in raw_inserters:
        reach = _INSERTER_REACH.get(ins.name, 1)
        dx, dy = DIR_VEC[ins.direction]
        ix, iy = ins.position

        drop_pos = (ix + dx * reach, iy + dy * reach)
        pickup_pos = (ix - dx * reach, iy - dy * reach)

        drop_machine = machine_tile_map.get(drop_pos)
        pickup_machine = machine_tile_map.get(pickup_pos)
        drop_network = network_tile_map.get(drop_pos)
        pickup_network = network_tile_map.get(pickup_pos)

        if drop_machine is not None:
            # Inserter drops into machine → role=input, belt on pickup side
            links.append(
                InserterLink(
                    position=ins.position,
                    direction=ins.direction,
                    machine_id=drop_machine,
                    network_id=pickup_network,
                    role="input",
                )
            )
        elif pickup_machine is not None:
            # Inserter picks from machine → role=output, belt on drop side
            links.append(
                InserterLink(
                    position=ins.position,
                    direction=ins.direction,
                    machine_id=pickup_machine,
                    network_id=drop_network,
                    role="output",
                )
            )
        # else: inserter between two belts or two machines — skip

    return links


def resolve_fluid_links(
    machines: list[AnalyzedMachine],
    pipe_networks: list[TransportNetwork],
) -> list[FluidLink]:
    """Find direct fluid port connections (pipe network touching machine port).

    Uses draftsman entity data to find fluid port positions, then checks
    if any pipe network tile overlaps.
    """
    from ..validate import _get_fluid_ports

    if not pipe_networks:
        return []

    # Build pipe tile → network id map
    pipe_tile_map: dict[tuple[int, int], int] = {}
    for net in pipe_networks:
        for seg in net.segments:
            pipe_tile_map[seg.position] = net.id

    links: list[FluidLink] = []

    for machine in machines:
        if machine.recipe is None:
            continue

        ports = _get_fluid_ports(machine.name)
        if not ports:
            continue

        mx, my = machine.position
        for rel_x, rel_y, prod_type in ports:
            abs_pos = (mx + rel_x, my + rel_y)
            net_id = pipe_tile_map.get(abs_pos)
            if net_id is not None:
                links.append(
                    FluidLink(
                        machine_id=machine.id,
                        network_id=net_id,
                        role=prod_type,  # "input" or "output"
                    )
                )

    return links


def _build_machine_tile_map(machines: list[AnalyzedMachine]) -> dict[tuple[int, int], int]:
    """Map every tile occupied by a machine to its machine id."""
    tile_map: dict[tuple[int, int], int] = {}
    for m in machines:
        for tile in machine_tiles(m.position[0], m.position[1], m.size):
            tile_map[tile] = m.id
    return tile_map


def _build_network_tile_map(networks: list[TransportNetwork]) -> dict[tuple[int, int], int]:
    """Map every tile in a network to its network id."""
    tile_map: dict[tuple[int, int], int] = {}
    for net in networks:
        for seg in net.segments:
            tile_map[seg.position] = net.id
    return tile_map
