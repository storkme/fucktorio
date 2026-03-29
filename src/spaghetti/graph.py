"""Re-export from src.routing for backward compatibility."""

from ..routing.graph import (
    FlowEdge as FlowEdge,
    MachineNode as MachineNode,
    ProductionGraph as ProductionGraph,
    build_production_graph as build_production_graph,
)
