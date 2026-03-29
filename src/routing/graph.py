"""Production graph: directed graph of machines and item/fluid flows."""

from __future__ import annotations

import math
from dataclasses import dataclass, field

from ..models import MachineSpec, SolverResult


@dataclass
class MachineNode:
    """A single machine instance in the production graph."""

    id: int
    spec: MachineSpec  # entity type, recipe, per-machine inputs/outputs
    instance: int  # which instance of this recipe (0..ceil(count)-1)


@dataclass
class FlowEdge:
    """A directed item/fluid flow between machines (or from/to external)."""

    item: str
    rate: float  # items per second through this edge
    is_fluid: bool
    from_node: int | None  # None = external input
    to_node: int | None  # None = external output


@dataclass
class ProductionGraph:
    """Directed graph of machines and the flows connecting them."""

    nodes: list[MachineNode] = field(default_factory=list)
    edges: list[FlowEdge] = field(default_factory=list)

    def nodes_for_recipe(self, recipe: str) -> list[MachineNode]:
        return [n for n in self.nodes if n.spec.recipe == recipe]

    def inputs_for(self, node_id: int) -> list[FlowEdge]:
        return [e for e in self.edges if e.to_node == node_id]

    def outputs_for(self, node_id: int) -> list[FlowEdge]:
        return [e for e in self.edges if e.from_node == node_id]

    def external_inputs(self) -> list[FlowEdge]:
        return [e for e in self.edges if e.from_node is None]

    def external_outputs(self) -> list[FlowEdge]:
        return [e for e in self.edges if e.to_node is None]


def build_production_graph(solver_result: SolverResult) -> ProductionGraph:
    """Build a production graph from solver output.

    Creates individual machine nodes (one per physical machine instance)
    and flow edges connecting producers to consumers. External inputs/outputs
    are represented as edges with None endpoints.
    """
    graph = ProductionGraph()

    # Create machine nodes — one per physical instance
    recipe_to_nodes: dict[str, list[MachineNode]] = {}
    node_id = 0
    for spec in solver_result.machines:
        count = max(1, math.ceil(spec.count))
        nodes = []
        for i in range(count):
            node = MachineNode(id=node_id, spec=spec, instance=i)
            graph.nodes.append(node)
            nodes.append(node)
            node_id += 1
        recipe_to_nodes[spec.recipe] = nodes

    # Build a map: item → which recipe produces it
    item_to_producer_recipe: dict[str, str] = {}
    for spec in solver_result.machines:
        for out in spec.outputs:
            item_to_producer_recipe[out.item] = spec.recipe

    # Create flow edges
    for spec in solver_result.machines:
        consumer_nodes = recipe_to_nodes[spec.recipe]

        for inp in spec.inputs:
            producer_recipe = item_to_producer_recipe.get(inp.item)

            if producer_recipe is None or producer_recipe not in recipe_to_nodes:
                # External input — distribute across consumer instances
                per_instance_rate = inp.rate
                for consumer in consumer_nodes:
                    graph.edges.append(
                        FlowEdge(
                            item=inp.item,
                            rate=per_instance_rate,
                            is_fluid=inp.is_fluid,
                            from_node=None,
                            to_node=consumer.id,
                        )
                    )
            else:
                # Internal flow — connect producer instances to consumer instances
                producer_nodes = recipe_to_nodes[producer_recipe]
                _connect_producers_to_consumers(graph, inp.item, inp.rate, inp.is_fluid, producer_nodes, consumer_nodes)

    # External output edges
    for out_flow in solver_result.external_outputs:
        producer_recipe = item_to_producer_recipe.get(out_flow.item)
        if producer_recipe and producer_recipe in recipe_to_nodes:
            for producer in recipe_to_nodes[producer_recipe]:
                per_instance_rate = out_flow.rate / len(recipe_to_nodes[producer_recipe])
                graph.edges.append(
                    FlowEdge(
                        item=out_flow.item,
                        rate=per_instance_rate,
                        is_fluid=out_flow.is_fluid,
                        from_node=producer.id,
                        to_node=None,
                    )
                )

    return graph


def _connect_producers_to_consumers(
    graph: ProductionGraph,
    item: str,
    per_consumer_rate: float,
    is_fluid: bool,
    producers: list[MachineNode],
    consumers: list[MachineNode],
) -> None:
    """Distribute flow from producers to consumers.

    Uses a capacity-aware greedy fill: each producer has a limited output
    rate, and each consumer has a demand. When one producer can't satisfy
    a consumer's full demand, the remainder spills to the next producer.
    """
    if not producers:
        return

    # Per-producer capacity for this item
    per_producer_rate = 0.0
    for out in producers[0].spec.outputs:
        if out.item == item:
            per_producer_rate = out.rate
            break

    if per_producer_rate <= 0:
        return

    # Track remaining capacity per producer
    remaining = [per_producer_rate] * len(producers)
    p = 0  # current producer index

    for consumer in consumers:
        demand = per_consumer_rate
        while demand > 1e-9 and p < len(producers):
            alloc = min(demand, remaining[p])
            if alloc > 1e-9:
                graph.edges.append(
                    FlowEdge(
                        item=item,
                        rate=alloc,
                        is_fluid=is_fluid,
                        from_node=producers[p].id,
                        to_node=consumer.id,
                    )
                )
                remaining[p] -= alloc
                demand -= alloc
            if remaining[p] < 1e-9:
                p += 1
