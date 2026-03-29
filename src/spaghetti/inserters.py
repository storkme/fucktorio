"""Re-export from src.routing for backward compatibility."""

from ..routing.inserters import (
    InserterAssignment as InserterAssignment,
    InsertionPlan as InsertionPlan,
    assign_inserter_positions as assign_inserter_positions,
    build_inserter_entities as build_inserter_entities,
)
