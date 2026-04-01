"""Step 2: Trace belt and pipe networks into connected components."""

from __future__ import annotations

from collections import deque

from ..models import EntityDirection
from ..routing.common import _UG_MAX_REACH, _UG_PIPE_REACH
from .models import TransportNetwork, TransportSegment

# Underground belt entity → surface tier name for reach lookup
_UG_TO_TIER: dict[str, str] = {
    "underground-belt": "transport-belt",
    "fast-underground-belt": "fast-transport-belt",
    "express-underground-belt": "express-transport-belt",
}

_DIR_VEC: dict[EntityDirection, tuple[int, int]] = {
    EntityDirection.NORTH: (0, -1),
    EntityDirection.EAST: (1, 0),
    EntityDirection.SOUTH: (0, 1),
    EntityDirection.WEST: (-1, 0),
}


def trace_belt_networks(segments: list[TransportSegment]) -> list[TransportNetwork]:
    """Find connected components in belt segments.

    Belts connect to cardinal neighbors that are also belts.
    Underground belt pairs bridge the gap between entry and exit.
    """
    if not segments:
        return []

    tile_map: dict[tuple[int, int], TransportSegment] = {}
    for seg in segments:
        tile_map[seg.position] = seg

    # Find underground belt pairs
    ug_pairs = _find_ug_belt_pairs(segments)

    # BFS to find connected components
    visited: set[tuple[int, int]] = set()
    networks: list[TransportNetwork] = []
    net_id = 0

    for seg in segments:
        if seg.position in visited:
            continue

        component: list[TransportSegment] = []
        queue = deque([seg.position])
        visited.add(seg.position)

        while queue:
            pos = queue.popleft()
            component.append(tile_map[pos])

            # Cardinal neighbors
            x, y = pos
            for dx, dy in [(1, 0), (-1, 0), (0, 1), (0, -1)]:
                nb = (x + dx, y + dy)
                if nb in tile_map and nb not in visited:
                    visited.add(nb)
                    queue.append(nb)

            # Underground belt tunnel
            if pos in ug_pairs:
                other = ug_pairs[pos]
                if other not in visited:
                    visited.add(other)
                    queue.append(other)

        turn_count = _count_turns(component)
        ug_count = sum(1 for s in component if s.is_underground)

        networks.append(
            TransportNetwork(
                id=net_id,
                type="belt",
                segments=component,
                path_length=len(component),
                turn_count=turn_count,
                underground_count=ug_count,
            )
        )
        net_id += 1

    return networks


def trace_pipe_networks(segments: list[TransportSegment]) -> list[TransportNetwork]:
    """Find connected components in pipe segments.

    Pipes connect to all cardinal neighbors (no direction tracking).
    Pipe-to-ground pairs bridge tunnels.
    """
    if not segments:
        return []

    tile_set: set[tuple[int, int]] = {seg.position for seg in segments}
    seg_map: dict[tuple[int, int], TransportSegment] = {seg.position: seg for seg in segments}

    # Find pipe-to-ground pairs
    ptg_pairs = _find_ptg_pairs(segments)

    visited: set[tuple[int, int]] = set()
    networks: list[TransportNetwork] = []
    net_id = 0

    for seg in segments:
        if seg.position in visited:
            continue

        component: list[TransportSegment] = []
        queue = deque([seg.position])
        visited.add(seg.position)

        while queue:
            pos = queue.popleft()
            component.append(seg_map[pos])

            x, y = pos
            for dx, dy in [(1, 0), (-1, 0), (0, 1), (0, -1)]:
                nb = (x + dx, y + dy)
                if nb in tile_set and nb not in visited:
                    visited.add(nb)
                    queue.append(nb)

            # Pipe-to-ground tunnel
            if pos in ptg_pairs:
                other = ptg_pairs[pos]
                if other not in visited:
                    visited.add(other)
                    queue.append(other)

        ug_count = sum(1 for s in component if s.is_underground)
        networks.append(
            TransportNetwork(
                id=net_id,
                type="pipe",
                segments=component,
                path_length=len(component),
                turn_count=0,
                underground_count=ug_count,
            )
        )
        net_id += 1

    return networks


def _find_ug_belt_pairs(
    segments: list[TransportSegment],
) -> dict[tuple[int, int], tuple[int, int]]:
    """Find underground belt entry/exit pairs.

    Underground belts face the same direction; entry has io_type="input",
    exit has io_type="output". Pair each input with nearest output ahead
    on the same axis within reach.
    """
    pairs: dict[tuple[int, int], tuple[int, int]] = {}

    # Group by (direction, perpendicular axis)
    groups: dict[tuple[EntityDirection, int], list[TransportSegment]] = {}
    for seg in segments:
        if not seg.is_underground:
            continue
        d = seg.direction
        x, y = seg.position
        if d in (EntityDirection.EAST, EntityDirection.WEST):
            key = (d, y)
        else:
            key = (d, x)
        groups.setdefault(key, []).append(seg)

    for (d, _axis), group in groups.items():
        inputs = sorted(
            [s for s in group if s.io_type == "input"],
            key=lambda s: (s.position[0], s.position[1]),
        )
        outputs = sorted(
            [s for s in group if s.io_type == "output"],
            key=lambda s: (s.position[0], s.position[1]),
        )

        remaining = list(outputs)
        for inp in inputs:
            tier = _UG_TO_TIER.get(inp.name, "transport-belt")
            max_reach = _UG_MAX_REACH.get(tier, 4)
            ix, iy = inp.position

            for out in remaining:
                ox, oy = out.position
                ahead = (
                    (d == EntityDirection.EAST and ox > ix)
                    or (d == EntityDirection.WEST and ox < ix)
                    or (d == EntityDirection.SOUTH and oy > iy)
                    or (d == EntityDirection.NORTH and oy < iy)
                )
                if not ahead:
                    continue
                dist = abs(ox - ix) + abs(oy - iy) - 1  # tiles between
                if dist > max_reach:
                    continue

                pairs[inp.position] = out.position
                pairs[out.position] = inp.position
                remaining.remove(out)
                break

    return pairs


def _find_ptg_pairs(
    segments: list[TransportSegment],
) -> dict[tuple[int, int], tuple[int, int]]:
    """Find pipe-to-ground entry/exit pairs.

    Same logic as underground belts but with pipe-specific reach.
    """
    pairs: dict[tuple[int, int], tuple[int, int]] = {}

    ptg = [s for s in segments if s.is_underground]

    groups: dict[tuple[EntityDirection, int], list[TransportSegment]] = {}
    for seg in ptg:
        d = seg.direction
        x, y = seg.position
        if d in (EntityDirection.EAST, EntityDirection.WEST):
            key = (d, y)
        else:
            key = (d, x)
        groups.setdefault(key, []).append(seg)

    for (d, _axis), group in groups.items():
        inputs = sorted(
            [s for s in group if s.io_type == "input"],
            key=lambda s: (s.position[0], s.position[1]),
        )
        outputs = sorted(
            [s for s in group if s.io_type == "output"],
            key=lambda s: (s.position[0], s.position[1]),
        )

        remaining = list(outputs)
        for inp in inputs:
            ix, iy = inp.position
            for out in remaining:
                ox, oy = out.position
                ahead = (
                    (d == EntityDirection.EAST and ox > ix)
                    or (d == EntityDirection.WEST and ox < ix)
                    or (d == EntityDirection.SOUTH and oy > iy)
                    or (d == EntityDirection.NORTH and oy < iy)
                )
                if not ahead:
                    continue
                dist = abs(ox - ix) + abs(oy - iy) - 1
                if dist > _UG_PIPE_REACH:
                    continue

                pairs[inp.position] = out.position
                pairs[out.position] = inp.position
                remaining.remove(out)
                break

    return pairs


def _count_turns(segments: list[TransportSegment]) -> int:
    """Count direction changes among adjacent belt segments."""
    if len(segments) < 2:
        return 0

    pos_to_dir: dict[tuple[int, int], EntityDirection] = {s.position: s.direction for s in segments}
    turns = 0
    seen: set[tuple[tuple[int, int], tuple[int, int]]] = set()

    for seg in segments:
        x, y = seg.position
        for dx, dy in [(1, 0), (-1, 0), (0, 1), (0, -1)]:
            nb = (x + dx, y + dy)
            if nb not in pos_to_dir:
                continue
            pair = (min(seg.position, nb), max(seg.position, nb))
            if pair in seen:
                continue
            seen.add(pair)
            if pos_to_dir[nb] != seg.direction:
                turns += 1

    return turns
