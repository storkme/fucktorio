"""Re-export from src.routing for backward compatibility."""

from ..routing.common import (
    DIR_MAP as _DIR_MAP,
    belt_entity_for_rate as _belt_entity_for_rate,
    machine_tiles as _machine_tiles,
)
from ..routing.router import (
    RoutingResult as RoutingResult,
    _astar_path as _astar_path,
    route_connections as route_connections,
)
