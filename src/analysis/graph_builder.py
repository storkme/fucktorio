"""Step 5: Construct production edges from resolved links."""

from __future__ import annotations

from .models import (
    FluidLink,
    InserterLink,
    ProductionEdge,
    TransportNetwork,
)


def build_edges(
    networks: list[TransportNetwork],
    inserter_links: list[InserterLink],
    fluid_links: list[FluidLink],
) -> list[ProductionEdge]:
    """Build production edges from networks with known items.

    For each labeled network, pair output links (machine→network) with
    input links (network→machine) to create edges. Networks with no
    source → external input. Networks with no destination → external output.
    """
    edges: list[ProductionEdge] = []

    for net in networks:
        if net.inferred_item is None:
            continue

        item = net.inferred_item

        # Collect source and destination machines for this network
        sources: set[int] = set()  # machine ids that output to this network
        destinations: set[int] = set()  # machine ids that receive from this network

        for link in inserter_links:
            if link.network_id != net.id:
                continue
            if link.role == "output":
                sources.add(link.machine_id)
            elif link.role == "input":
                destinations.add(link.machine_id)

        for fl in fluid_links:
            if fl.network_id != net.id:
                continue
            if fl.role == "output":
                sources.add(fl.machine_id)
            elif fl.role == "input":
                destinations.add(fl.machine_id)

        if not sources and not destinations:
            continue

        if not sources:
            # External input → all destinations
            for dest in destinations:
                edges.append(ProductionEdge(item=item, from_machine=None, to_machine=dest, network_id=net.id))
        elif not destinations:
            # All sources → external output
            for src in sources:
                edges.append(ProductionEdge(item=item, from_machine=src, to_machine=None, network_id=net.id))
        else:
            # Internal flow: each source → each destination
            for src in sources:
                for dest in destinations:
                    edges.append(ProductionEdge(item=item, from_machine=src, to_machine=dest, network_id=net.id))

    return edges
