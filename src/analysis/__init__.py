"""Blueprint analysis: parse a Factorio blueprint string into a production graph."""

from __future__ import annotations

from .classify import classify_entities
from .graph_builder import build_edges
from .infer import infer_items
from .inserters import resolve_fluid_links, resolve_inserters
from .models import BlueprintGraph
from .trace import trace_belt_networks, trace_pipe_networks


def analyze_blueprint(bp_string: str) -> BlueprintGraph:
    """Parse a Factorio blueprint string into a rich graph representation.

    Steps:
    1. Classify entities (machines, belts, pipes, inserters)
    2. Trace belt and pipe networks (connected components)
    3. Resolve inserter connections and fluid port links
    4. Infer what item each network carries
    5. Build production edges
    """
    # Step 1: Classify
    classified = classify_entities(bp_string)

    # Step 2: Trace networks
    belt_networks = trace_belt_networks(classified.belt_segments)
    pipe_networks = trace_pipe_networks(classified.pipe_segments)

    # Assign globally unique network IDs (belt networks start at 0, pipes continue)
    offset = len(belt_networks)
    for net in pipe_networks:
        net.id += offset
    all_networks = belt_networks + pipe_networks

    # Step 3: Resolve inserters and fluid ports
    inserter_links = resolve_inserters(classified.inserters, classified.machines, all_networks)
    fluid_links = resolve_fluid_links(classified.machines, pipe_networks)

    # Step 4: Infer items
    infer_items(classified.machines, all_networks, inserter_links, fluid_links)

    # Step 5: Build edges
    edges = build_edges(all_networks, inserter_links, fluid_links)

    return BlueprintGraph(
        machines=classified.machines,
        networks=all_networks,
        inserter_links=inserter_links,
        fluid_links=fluid_links,
        edges=edges,
        unhandled=classified.unhandled,
    )
