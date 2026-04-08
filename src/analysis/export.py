"""Convert a BlueprintGraph to LayoutResult-compatible JSON for the web renderer."""

from __future__ import annotations

from ..models import EntityDirection
from .models import BlueprintGraph

_DIR_TO_STR: dict[EntityDirection, str] = {
    EntityDirection.NORTH: "North",
    EntityDirection.EAST: "East",
    EntityDirection.SOUTH: "South",
    EntityDirection.WEST: "West",
}


def to_layout_result(graph: BlueprintGraph) -> dict:
    """Convert a BlueprintGraph to LayoutResult-compatible JSON.

    Produces the same shape as the Rust LayoutResult / PlacedEntity types
    so the web app renderer can display community blueprints directly.

    Fields mapped per entity type:
    - Machines: name, x, y, direction, recipe
    - Belts/pipes: name, x, y, direction, carries (from inferred_item), io_type
    - Inserters: name, x, y, direction, carries (from inferred_item)
    """
    entities: list[dict] = []

    # Build network-id → inferred_item lookup
    _net_item: dict[int, str | None] = {n.id: n.inferred_item for n in graph.networks}

    # Inserter-position → inferred_item lookup (from inserter_links)
    _inserter_item: dict[tuple[int, int], str | None] = {lk.position: lk.inferred_item for lk in graph.inserter_links}

    # Machines
    for m in graph.machines:
        e: dict = {
            "name": m.name,
            "x": m.position[0],
            "y": m.position[1],
            "direction": "North",
        }
        if m.recipe is not None:
            e["recipe"] = m.recipe
        entities.append(e)

    # Belt and pipe segments (from all networks)
    for net in graph.networks:
        for seg in net.segments:
            e = {
                "name": seg.name,
                "x": seg.position[0],
                "y": seg.position[1],
                "direction": _DIR_TO_STR.get(seg.direction, "North"),
            }
            if net.inferred_item is not None:
                e["carries"] = net.inferred_item
            if seg.io_type is not None:
                e["io_type"] = seg.io_type
            entities.append(e)

    # Inserters (from inserter_links — position/direction/name are stored there)
    for lk in graph.inserter_links:
        e = {
            "name": lk.name,
            "x": lk.position[0],
            "y": lk.position[1],
            "direction": _DIR_TO_STR.get(lk.direction, "North"),
        }
        if lk.inferred_item is not None:
            e["carries"] = lk.inferred_item
        entities.append(e)

    # Compute bounding box
    width, height = 0, 0
    if entities:
        xs = [e["x"] for e in entities]
        ys = [e["y"] for e in entities]
        width = max(xs) - min(xs) + 1
        height = max(ys) - min(ys) + 1

    return {
        "entities": entities,
        "width": width,
        "height": height,
        "warnings": [],
    }
