"""Belt/pipe routing via A* pathfinding on the tile grid."""

from __future__ import annotations

import logging
from dataclasses import dataclass, field

from ..models import EntityDirection, PlacedEntity
from .common import (
    _UG_COST_MULTIPLIER,
    _UG_MAX_REACH,
    _UG_PIPE_REACH,
    DIR_MAP,
    DIR_VEC,
    DIRECTIONS,
    belt_entity_for_rate,
    machine_size,
    machine_tiles,
)
from .graph import FlowEdge, ProductionGraph

logger = logging.getLogger(__name__)

# Try to use Rust A* implementation for ~10-30x speedup
try:
    from fucktorio_native import astar_path as _rust_astar_path

    _USE_RUST_ASTAR = True
    logger.info("Using Rust A* implementation")
except ImportError:
    _USE_RUST_ASTAR = False
    logger.debug("Rust A* not available, using Python fallback")


@dataclass
class RoutingResult:
    """Result of routing all flow edges."""

    entities: list[PlacedEntity] = field(default_factory=list)
    occupied: set[tuple[int, int]] = field(default_factory=set)
    failed_edges: list[FlowEdge] = field(default_factory=list)
    belt_dir_map: dict[tuple[int, int], EntityDirection] = field(default_factory=dict)
    group_networks: dict[tuple[str, int], set[tuple[int, int]]] = field(default_factory=dict)


def _machine_border_tiles(x: int, y: int, size: int) -> list[tuple[int, int, int, int]]:
    """Tiles adjacent to a machine border, with direction toward the machine.

    Returns list of (tile_x, tile_y, dx_toward_machine, dy_toward_machine).
    """
    borders = []
    # Top edge (y - 1)
    for dx in range(size):
        borders.append((x + dx, y - 1, 0, 1))
    # Bottom edge (y + size)
    for dx in range(size):
        borders.append((x + dx, y + size, 0, -1))
    # Left edge (x - 1)
    for dy in range(size):
        borders.append((x - 1, y + dy, 1, 0))
    # Right edge (x + size)
    for dy in range(size):
        borders.append((x + size, y + dy, -1, 0))
    return borders


def _machine_belt_tiles(x: int, y: int, size: int) -> list[tuple[int, int]]:
    """Tiles 2 away from machine — where belts should end (inserter goes between).

    The border tile (1 away) is reserved for the inserter.
    The belt tile (2 away) is where the belt terminates.
    """
    tiles = []
    # Top (y - 2)
    for dx in range(size):
        tiles.append((x + dx, y - 2))
    # Bottom (y + size + 1)
    for dx in range(size):
        tiles.append((x + dx, y + size + 1))
    # Left (x - 2)
    for dy in range(size):
        tiles.append((x - 2, y + dy))
    # Right (x + size + 1)
    for dy in range(size):
        tiles.append((x + size + 1, y + dy))
    return tiles


def _astar_path(
    start: tuple[int, int] | None = None,
    goals: set[tuple[int, int]] = (),
    obstacles: set[tuple[int, int]] = (),
    max_extent: int = 200,
    allow_underground: bool = False,
    ug_max_reach: int = 4,
    start_lane: str | None = None,
    goal_lane_check: tuple[int, int] | None = None,
    belt_dir_map: dict[tuple[int, int], EntityDirection] | None = None,
    other_item_tiles: set[tuple[int, int]] | None = None,
    starts: set[tuple[int, int]] | None = None,
) -> list[tuple[int, int]] | None:
    """A* pathfinding with Manhattan heuristic, underground jumps, and lane awareness.

    Supports multi-source search: pass ``starts`` to seed the open set with
    multiple start tiles at cost 0 (single A* expansion).  Falls back to
    ``start`` for single-source calls.

    Args:
        start: Single start tile (legacy, use ``starts`` for multi-source).
        starts: Set of start tiles for multi-source A*.
        start_lane: Which lane items start on ("left"/"right"/None).
        goal_lane_check: If set, the (dx, dy) vector from belt tile toward
            the inserter at the goal. Used to dynamically compute the needed
            lane based on the A* arrival direction.
        belt_dir_map: Existing placed belt directions for sideload detection.
        other_item_tiles: Tiles belonging to belt networks carrying a different
            item. Used to prevent cross-item contamination during pathfinding.
    """
    # Normalise start(s)
    if starts is not None:
        start_set = starts - obstacles
    elif start is not None:
        start_set = {start}
    else:
        return None

    if not start_set or not goals:
        return None

    # Quick check: any start already a goal?
    overlap = start_set & set(goals)
    if overlap:
        t = next(iter(overlap))
        return [t]

    # Dispatch to Rust implementation if available
    if _USE_RUST_ASTAR:
        return _rust_astar_path(
            list(start_set),
            list(goals),
            list(obstacles),
            max_extent,
            allow_underground,
            ug_max_reach,
            start_lane,
            goal_lane_check,
            list(belt_dir_map.items()) if belt_dir_map else None,
            list(other_item_tiles) if other_item_tiles else None,
        )

    import heapq

    # Deviation penalty: compute line from start centroid to goal center
    scx = sum(x for x, _ in start_set) / len(start_set)
    scy = sum(y for _, y in start_set) / len(start_set)
    goal_cx = sum(g[0] for g in goals) / len(goals)
    goal_cy = sum(g[1] for g in goals) / len(goals)
    line_dx = goal_cx - scx
    line_dy = goal_cy - scy
    line_len = max(1.0, (line_dx**2 + line_dy**2) ** 0.5)

    def _deviation(x: int, y: int) -> float:
        """Perpendicular distance from (x,y) to the start→goal line."""
        return abs((x - scx) * line_dy - (y - scy) * line_dx) / line_len

    # Optimized heuristic: special-case single goal (common) to avoid
    # generator/min overhead; for multi-goal, use inlined abs to avoid
    # Python function call overhead (~460k calls in a typical run).
    if len(goals) == 1:
        ((gx0, gy0),) = goals

        def _h(x: int, y: int) -> int:
            dx = x - gx0
            dy = y - gy0
            return (dx if dx > 0 else -dx) + (dy if dy > 0 else -dy)
    else:
        goal_list = list(goals)

        def _h(x: int, y: int) -> int:
            best = 0x7FFFFFFF
            for gx, gy in goal_list:
                dx = x - gx
                dy = y - gy
                d = (dx if dx > 0 else -dx) + (dy if dy > 0 else -dy)
                if d < best:
                    best = d
            return best

    # State: (x, y, forced, lane) where:
    # - forced is None or (dx, dy) direction (set on UG exit tiles)
    # - lane is "left", "right", or None (which belt lane items are on)
    State = tuple[int, int, tuple[int, int] | None, str | None]

    counter = 0
    open_set: list[tuple[int, int, State]] = []
    g_score: dict[State, float] = {}
    parent: dict[State, State] = {}

    # Seed open set with all start tiles
    for sx, sy in start_set:
        initial: State = (sx, sy, None, start_lane)
        g_score[initial] = 0.0
        heapq.heappush(open_set, (_h(sx, sy), counter, initial))
        counter += 1

    while open_set:
        _, _, state = heapq.heappop(open_set)
        cx, cy, forced, lane = state

        # Only reach a goal when no forced continuation pending
        if (cx, cy) in goals and forced is None:
            # Lane check at goal: if goal_lane_check is set, verify items
            # arrive on the lane the inserter picks from (near lane).
            if goal_lane_check is not None and lane is not None and state in parent:
                prev_state = parent[state]
                pdx = cx - prev_state[0]
                pdy = cy - prev_state[1]
                if pdx != 0:
                    pdx = 1 if pdx > 0 else -1
                if pdy != 0:
                    pdy = 1 if pdy > 0 else -1
                # Belt direction at goal = arrival direction
                # Left perpendicular of belt direction
                left_dx, left_dy = -pdy, pdx
                # Dot with inserter side vector to determine needed lane
                ins_dx, ins_dy = goal_lane_check
                dot = ins_dx * left_dx + ins_dy * left_dy
                needed_lane = "left" if dot > 0 else "right"
                if lane != needed_lane:
                    # Wrong lane — don't accept this goal, keep searching
                    cur_g = g_score.get(state, 0)
                    # Continue to neighbor expansion (might find goal via different path)
                    pass
                else:
                    path: list[tuple[int, int]] = [(cx, cy)]
                    cur = state
                    while cur in parent:
                        cur = parent[cur]
                        path.append((cur[0], cur[1]))
                    path.reverse()
                    return path
            else:
                path = [(cx, cy)]
                cur = state
                while cur in parent:
                    cur = parent[cur]
                    path.append((cur[0], cur[1]))
                path.reverse()
                return path

        cur_g = g_score.get(state, 0)

        if forced is not None:
            # Prevent A* from later revisiting this position as a normal tile,
            # which would create duplicate (x, y) entries in the path.
            none_state: State = (cx, cy, None, lane)
            if none_state not in g_score or g_score[none_state] > cur_g:
                g_score[none_state] = cur_g

            # Must continue in forced direction (one straight step after UG exit)
            # Lane preserved through forced continuation.
            fdx, fdy = forced
            nx, ny = cx + fdx, cy + fdy
            if not (nx < -10 or ny < -10 or nx > max_extent or ny > max_extent) and (nx, ny) not in obstacles:
                # Item contamination check on forced continuation tile
                _forced_ok = True
                if other_item_tiles is not None:
                    if (nx + fdx, ny + fdy) in other_item_tiles:
                        _forced_ok = False
                    elif belt_dir_map is not None:
                        for cdx, cdy in DIRECTIONS:
                            adj = (nx + cdx, ny + cdy)
                            if adj in other_item_tiles:
                                adj_dir = belt_dir_map.get(adj)
                                if adj_dir is not None and DIR_VEC[adj_dir] == (-cdx, -cdy):
                                    _forced_ok = False
                                    break
                if _forced_ok:
                    new_state: State = (nx, ny, None, lane)
                    new_g = cur_g + 1.0
                    if new_state not in g_score or g_score[new_state] > new_g:
                        g_score[new_state] = new_g
                        parent[new_state] = state
                        f = new_g + _h(nx, ny)
                        heapq.heappush(open_set, (f, counter, new_state))
                        counter += 1
            continue  # No other moves when forced

        # Normal surface moves (cost 1 + turn penalty + deviation penalty)
        for dx, dy in DIRECTIONS:
            nx, ny = cx + dx, cy + dy

            if nx < -10 or ny < -10 or nx > max_extent or ny > max_extent:
                continue
            if (nx, ny) in obstacles:
                continue

            # Item contamination checks: prevent routing near foreign belt networks
            if other_item_tiles is not None:
                # Outgoing: belt at (nx,ny) would point (dx,dy); if the forward
                # tile belongs to a different item's network, we'd contaminate it
                if (nx + dx, ny + dy) in other_item_tiles:
                    continue
                # Incoming: if a foreign belt points AT (nx,ny), it would
                # push foreign items onto our belt
                _contam = False
                if belt_dir_map is not None:
                    for cdx, cdy in DIRECTIONS:
                        adj = (nx + cdx, ny + cdy)
                        if adj in other_item_tiles:
                            adj_dir = belt_dir_map.get(adj)
                            if adj_dir is not None and DIR_VEC[adj_dir] == (-cdx, -cdy):
                                _contam = True
                                break
                if _contam:
                    continue

            new_g = cur_g + 1.0

            # Proximity penalty: discourage routing adjacent to foreign networks
            if other_item_tiles is not None:
                for cdx, cdy in DIRECTIONS:
                    if (nx + cdx, ny + cdy) in other_item_tiles:
                        new_g += 3.0
                        break

            # Compute previous direction for turn detection
            is_turn = False
            prev = parent.get(state)
            if prev is not None:
                pdx = cx - prev[0]
                pdy = cy - prev[1]
                # Normalize for underground jumps (non-unit deltas)
                if pdx != 0:
                    pdx = 1 if pdx > 0 else -1
                if pdy != 0:
                    pdy = 1 if pdy > 0 else -1
                if (dx, dy) != (pdx, pdy):
                    new_g += 0.5
                    is_turn = True

            # Deviation penalty: stay near the start→goal line
            new_g += _deviation(nx, ny) * 0.1

            # Lane transition logic
            new_lane = lane
            if belt_dir_map is not None and (nx, ny) in belt_dir_map:
                # Moving onto an existing belt — check for sideload
                existing_dir = belt_dir_map[(nx, ny)]
                edx, edy = DIR_VEC[existing_dir]
                # Dot product: 0 = perpendicular (sideload), >0 = same dir
                dot = dx * edx + dy * edy
                if dot == 0:
                    # Sideload: both lanes merge onto near lane of receiver.
                    # Near lane = side the approach comes from.
                    # Left perpendicular of existing belt direction
                    left_dx, left_dy = -edy, edx
                    # Source is at (cx, cy) relative to target (nx, ny)
                    rel_x, rel_y = cx - nx, cy - ny
                    side_dot = rel_x * left_dx + rel_y * left_dy
                    new_lane = "left" if side_dot > 0 else "right"
            elif is_turn and lane is not None:
                # Turn on our own path: lanes swap (left↔right)
                # This matches validate.py:2176 convention
                new_lane = "right" if lane == "left" else "left"

            new_state: State = (nx, ny, None, new_lane)

            if new_state in g_score and g_score[new_state] <= new_g:
                continue

            g_score[new_state] = new_g
            parent[new_state] = state
            f = new_g + _h(nx, ny)
            heapq.heappush(open_set, (f, counter, new_state))
            counter += 1

        # Underground jumps (cost = dist * _UG_COST_MULTIPLIER)
        if allow_underground:
            for dx, dy in DIRECTIONS:
                for dist in range(2, ug_max_reach + 2):
                    ex, ey = cx + dx * dist, cy + dy * dist
                    if ex < -10 or ey < -10 or ex > max_extent or ey > max_extent:
                        break
                    if (ex, ey) in obstacles:
                        continue  # exit blocked, try further
                    # Don't land on a goal tile — items exit underground
                    # facing the jump direction and need a surface belt
                    # ahead to continue. If we land on a goal, the path
                    # ends here with no continuation.
                    if (ex, ey) in goals:
                        continue
                    # Tile after exit must also be free (for forced continuation)
                    if (ex + dx, ey + dy) in obstacles:
                        continue

                    # Item contamination at UG exit: the forced continuation
                    # tile (ex+dx, ey+dy) must not feed into a foreign network,
                    # and the tile after that (ex+2dx, ey+2dy) must not be foreign
                    if other_item_tiles is not None:
                        cont_tile = (ex + dx, ey + dy)
                        after_cont = (ex + 2 * dx, ey + 2 * dy)
                        if after_cont in other_item_tiles:
                            continue
                        # Incoming check at exit and continuation tiles
                        _ug_contam = False
                        if belt_dir_map is not None:
                            for _tile in ((ex, ey), cont_tile):
                                for cdx, cdy in DIRECTIONS:
                                    adj = (_tile[0] + cdx, _tile[1] + cdy)
                                    if adj in other_item_tiles:
                                        adj_dir = belt_dir_map.get(adj)
                                        if adj_dir is not None and DIR_VEC[adj_dir] == (-cdx, -cdy):
                                            _ug_contam = True
                                            break
                                if _ug_contam:
                                    break
                        if _ug_contam:
                            continue

                    # Exit state carries forced direction — next step must
                    # continue straight before turning is allowed.
                    new_g = cur_g + dist * _UG_COST_MULTIPLIER + _deviation(ex, ey) * 0.1

                    # Penalize perpendicular underground entries — approaching
                    # an underground from the side creates a sideload at the
                    # entry tile, losing one belt lane.
                    ug_lane = lane
                    prev = parent.get(state)
                    if prev is not None:
                        pdx = cx - prev[0]
                        pdy = cy - prev[1]
                        if pdx != 0:
                            pdx = 1 if pdx > 0 else -1
                        if pdy != 0:
                            pdy = 1 if pdy > 0 else -1
                        if pdx * dx + pdy * dy == 0:
                            # Perpendicular approach → sideload at UG entry
                            new_g += 10.0
                            # Apply sideload lane transition: force to near lane
                            if ug_lane is not None:
                                left_dx, left_dy = -dy, dx
                                rel_x, rel_y = prev[0] - cx, prev[1] - cy
                                side_dot = rel_x * left_dx + rel_y * left_dy
                                ug_lane = "left" if side_dot > 0 else "right"

                    new_state = (ex, ey, (dx, dy), ug_lane)
                    if new_state in g_score and g_score[new_state] <= new_g:
                        continue

                    g_score[new_state] = new_g
                    parent[new_state] = state
                    f = new_g + _h(ex, ey)
                    heapq.heappush(open_set, (f, counter, new_state))
                    counter += 1

    return None


def _path_to_entities(
    path: list[tuple[int, int]],
    entity_name: str,
    item: str,
    is_fluid: bool,
) -> list[PlacedEntity]:
    """Convert a tile path to placed belt or pipe entities.

    Underground jumps appear as non-adjacent consecutive tiles in the path
    (Manhattan distance > 1). These are converted to underground-belt
    input/output pairs.
    """
    entities: list[PlacedEntity] = []
    ug_name = "pipe-to-ground" if is_fluid else "underground-belt"

    for i, (x, y) in enumerate(path):
        # Detect underground jumps: non-adjacent consecutive tiles
        prev_dist = abs(x - path[i - 1][0]) + abs(y - path[i - 1][1]) if i > 0 else 1
        next_dist = abs(path[i + 1][0] - x) + abs(path[i + 1][1] - y) if i + 1 < len(path) else 1

        if next_dist > 1:
            # Underground entry
            dx = (path[i + 1][0] - x) // next_dist
            dy = (path[i + 1][1] - y) // next_dist
            direction = DIR_MAP.get((dx, dy), EntityDirection.SOUTH)
            entities.append(
                PlacedEntity(
                    name=ug_name,
                    x=x,
                    y=y,
                    direction=direction,
                    io_type="input",
                    carries=item,
                )
            )
        elif prev_dist > 1:
            # Underground exit
            dx = (x - path[i - 1][0]) // prev_dist
            dy = (y - path[i - 1][1]) // prev_dist
            direction = DIR_MAP.get((dx, dy), EntityDirection.SOUTH)
            entities.append(
                PlacedEntity(
                    name=ug_name,
                    x=x,
                    y=y,
                    direction=direction,
                    io_type="output",
                    carries=item,
                )
            )
        elif is_fluid:
            entities.append(PlacedEntity(name="pipe", x=x, y=y, carries=item))
        else:
            # Normal surface belt
            if i + 1 < len(path):
                dx = path[i + 1][0] - x
                dy = path[i + 1][1] - y
            elif i > 0:
                dx = x - path[i - 1][0]
                dy = y - path[i - 1][1]
            else:
                dx, dy = 0, 1

            direction = DIR_MAP.get((dx, dy), EntityDirection.SOUTH)
            entities.append(
                PlacedEntity(
                    name=entity_name,
                    x=x,
                    y=y,
                    direction=direction,
                    carries=item,
                )
            )

    return entities


def _network_downstream_ends(
    network: set[tuple[int, int]],
    belt_dir_map: dict[tuple[int, int], EntityDirection],
) -> set[tuple[int, int]]:
    """Find tiles at the downstream end of a belt network.

    A downstream end is a network tile whose belt direction points to a tile
    NOT in the network — the tip where items would exit.
    """
    ends = set()
    for tile in network:
        d = belt_dir_map.get(tile)
        if d is None:
            continue
        dx, dy = DIR_VEC[d]
        forward = (tile[0] + dx, tile[1] + dy)
        if forward not in network:
            ends.add(tile)
    return ends


def _perpendicular_approach_tiles(
    network: set[tuple[int, int]],
    belt_dir_map: dict[tuple[int, int], EntityDirection],
    occupied: set[tuple[int, int]],
) -> set[tuple[int, int]]:
    """Find unoccupied tiles perpendicular to existing network belt directions.

    These are valid sideload connection points — approaching a belt from
    the side rather than head-on or from behind.
    """
    approach = set()
    for nx, ny in network:
        d = belt_dir_map.get((nx, ny))
        if d is None:
            continue
        dx, dy = DIR_VEC[d]
        # Perpendicular directions: rotate 90deg both ways
        for pdx, pdy in [(-dy, dx), (dy, -dx)]:
            tile = (nx + pdx, ny + pdy)
            if tile not in occupied and tile not in network:
                approach.add(tile)
    return approach


def _classify_approach_by_lane(
    network: set[tuple[int, int]],
    belt_dir_map: dict[tuple[int, int], EntityDirection],
    occupied: set[tuple[int, int]],
) -> tuple[set[tuple[int, int]], set[tuple[int, int]]]:
    """Split perpendicular approach tiles into left-lane and right-lane sets."""
    left_goals: set[tuple[int, int]] = set()
    right_goals: set[tuple[int, int]] = set()
    for nx, ny in network:
        d = belt_dir_map.get((nx, ny))
        if d is None:
            continue
        dx, dy = DIR_VEC[d]
        left_tile = (nx - dy, ny + dx)
        if left_tile not in occupied and left_tile not in network:
            left_goals.add(left_tile)
        right_tile = (nx + dy, ny - dx)
        if right_tile not in occupied and right_tile not in network:
            right_goals.add(right_tile)
    return left_goals, right_goals


def _detect_sideload_lane(
    path: list[tuple[int, int]],
    network: set[tuple[int, int]],
    belt_dir_map: dict[tuple[int, int], EntityDirection],
) -> str | None:
    """Determine which trunk lane items enter via this path's sideload point."""
    for idx in range(len(path) - 1, -1, -1):
        px, py = path[idx]
        if (px, py) in network:
            continue
        if idx + 1 < len(path) and path[idx + 1] in network:
            net_x, net_y = path[idx + 1]
            d = belt_dir_map.get((net_x, net_y))
            if d is None:
                return None
            dx, dy = DIR_VEC[d]
            left_dx, left_dy = -dy, dx
            rel_x, rel_y = px - net_x, py - net_y
            dot = rel_x * left_dx + rel_y * left_dy
            if dot > 0:
                return "left"
            elif dot < 0:
                return "right"
            return None
    return None


def _network_upstream_ends(
    network: set[tuple[int, int]],
    belt_dir_map: dict[tuple[int, int], EntityDirection],
) -> set[tuple[int, int]]:
    """Find tiles at the upstream end of a belt network.

    An upstream end is a network tile whose BACKWARD neighbor (opposite of
    belt direction) is NOT in the network — the tip where items enter.
    """
    ends = set()
    for tile in network:
        d = belt_dir_map.get(tile)
        if d is None:
            continue
        dx, dy = DIR_VEC[d]
        backward = (tile[0] - dx, tile[1] - dy)
        if backward not in network:
            ends.add(tile)
    return ends


def _compute_io_y_slots(
    graph: ProductionGraph,
    positions: dict[int, tuple[int, int]],
) -> tuple[dict[str, int], dict[str, int]]:
    """Compute y-slot assignments for external input and output items.

    Returns (input_slots, output_slots) where each maps item name -> y coordinate.
    Items are stacked vertically starting from the minimum grid y.
    """
    if not positions:
        return {}, {}

    min_gy = min(y for _, y in positions.values()) - 3

    input_items: list[str] = []
    output_items: list[str] = []
    seen_input: set[str] = set()
    seen_output: set[str] = set()

    for edge in graph.edges:
        if edge.from_node is None and edge.item not in seen_input:
            input_items.append(edge.item)
            seen_input.add(edge.item)
        if edge.to_node is None and edge.item not in seen_output:
            output_items.append(edge.item)
            seen_output.add(edge.item)

    input_slots = {item: min_gy + i for i, item in enumerate(input_items)}
    output_slots = {item: min_gy + i for i, item in enumerate(output_items)}
    return input_slots, output_slots


def _fix_belt_directions(
    entities: list[PlacedEntity],
    belt_dir_map: dict[tuple[int, int], EntityDirection],
    protected_tiles: set[tuple[int, int]] | None = None,
) -> None:
    """Post-process belt directions to fix T-junctions, underground exits, and orphans.

    Modifies entities in-place and updates belt_dir_map.

    Args:
        protected_tiles: Belt tiles adjacent to inserter pickup/drop points.
            These are NOT reoriented by the orphan stub fixer, since their
            direction was set intentionally by the routing pipeline.
    """
    if protected_tiles is None:
        protected_tiles = set()
    # Build position -> entity index mapping (belts and underground belts only)
    belt_names = {
        "transport-belt",
        "fast-transport-belt",
        "express-transport-belt",
        "underground-belt",
    }
    pos_to_idx: dict[tuple[int, int], int] = {}
    for i, e in enumerate(entities):
        if e.name in belt_names:
            pos_to_idx[(e.x, e.y)] = i

    if not pos_to_idx:
        return

    belt_positions = set(pos_to_idx.keys())

    # Build carries map for contamination guard during direction changes
    carries_map: dict[tuple[int, int], str | None] = {}
    for e in entities:
        if e.name in belt_names and e.carries:
            carries_map[(e.x, e.y)] = e.carries

    def _would_contaminate(pos: tuple[int, int], new_dir: EntityDirection) -> bool:
        """Check if changing belt at pos to new_dir would create cross-item contamination."""
        our_carry = carries_map.get(pos)
        if not our_carry:
            return False
        dvec = DIR_VEC[new_dir]
        fwd = (pos[0] + dvec[0], pos[1] + dvec[1])
        fwd_carry = carries_map.get(fwd)
        return fwd_carry is not None and fwd_carry != our_carry

    # 1. Build adjacency map
    adj: dict[tuple[int, int], list[tuple[int, int]]] = {pos: [] for pos in belt_positions}
    for pos in belt_positions:
        x, y = pos
        for dx, dy in DIRECTIONS:
            neighbor = (x + dx, y + dy)
            if neighbor in belt_positions:
                adj[pos].append(neighbor)

    # 2. Log head-on belt collisions at T-junctions
    for pos in belt_positions:
        ent = entities[pos_to_idx[pos]]
        if ent.name == "underground-belt":
            continue
        cur_dir = belt_dir_map.get(pos)
        if cur_dir is None:
            continue

        cur_dvec = DIR_VEC[cur_dir]
        feeders = []
        for nx, ny in adj[pos]:
            n_dir = belt_dir_map.get((nx, ny))
            if n_dir is None:
                continue
            n_dvec = DIR_VEC[n_dir]
            if (nx + n_dvec[0], ny + n_dvec[1]) == pos:
                feeders.append((nx, ny))

        if len(feeders) < 2:
            continue

        # Count inline feeders (same axis as belt direction)
        inline_count = sum(
            1 for fx, fy in feeders if (pos[0] - fx, pos[1] - fy) == cur_dvec or (fx - pos[0], fy - pos[1]) == cur_dvec
        )
        if inline_count >= 2:
            logger.warning(
                "Head-on belt collision at (%d, %d): feeders from opposite sides",
                pos[0],
                pos[1],
            )

    # 3. Fix underground exit directions for sideloading
    for pos in belt_positions:
        idx = pos_to_idx[pos]
        ent = entities[idx]
        if ent.name != "underground-belt" or ent.io_type != "output":
            continue
        cur_dir = belt_dir_map.get(pos)
        if cur_dir is None:
            continue
        cur_dvec = DIR_VEC[cur_dir]
        exit_target = (pos[0] + cur_dvec[0], pos[1] + cur_dvec[1])

        # If exit target has a perpendicular belt, ensure exit points at the trunk
        if exit_target in belt_dir_map:
            target_dir = belt_dir_map[exit_target]
            target_dvec = DIR_VEC[target_dir]
            if cur_dvec[0] * target_dvec[0] + cur_dvec[1] * target_dvec[1] == 0:
                expected_dvec = (exit_target[0] - pos[0], exit_target[1] - pos[1])
                expected_dir = DIR_MAP.get(expected_dvec)
                if expected_dir is not None and expected_dir != cur_dir and not _would_contaminate(pos, expected_dir):
                    ent.direction = expected_dir
                    belt_dir_map[pos] = expected_dir

    # 4. Fix orphaned belt stubs (dead ends pointing into nothing)
    for pos in belt_positions:
        idx = pos_to_idx[pos]
        ent = entities[idx]
        if ent.name == "underground-belt":
            continue

        # Never reorient inserter-adjacent belt tiles — their direction
        # was set intentionally by the routing pipeline.
        if pos in protected_tiles:
            continue

        cur_dir = belt_dir_map.get(pos)
        if cur_dir is None:
            continue
        cur_dvec = DIR_VEC[cur_dir]
        forward = (pos[0] + cur_dvec[0], pos[1] + cur_dvec[1])

        # If forward tile has no belt, this is a dead end
        if forward in belt_positions:
            continue

        # Check if any upstream neighbor exists (points at us)
        upstream = None
        for nx, ny in adj[pos]:
            n_dir = belt_dir_map.get((nx, ny))
            if n_dir is None:
                continue
            n_dvec = DIR_VEC[n_dir]
            if (nx + n_dvec[0], ny + n_dvec[1]) == pos:
                upstream = (nx, ny)
                break

        if upstream is not None:
            # Orient this belt to continue in the upstream direction
            up_dir = belt_dir_map.get(upstream)
            if up_dir is not None and up_dir != cur_dir and not _would_contaminate(pos, up_dir):
                ent.direction = up_dir
                belt_dir_map[pos] = up_dir

        # If still a dead end, try to face an adjacent belt to create
        # a sideload connection (works even without an upstream belt,
        # e.g. belts fed directly by inserters)
        cur_dvec2 = DIR_VEC[belt_dir_map[pos]]
        forward2 = (pos[0] + cur_dvec2[0], pos[1] + cur_dvec2[1])
        if forward2 not in belt_positions:
            for nx, ny in adj[pos]:
                if (nx, ny) == upstream:
                    continue
                n_dir = belt_dir_map.get((nx, ny))
                if n_dir is None:
                    continue
                face_vec = (nx - pos[0], ny - pos[1])
                face_dir = DIR_MAP.get(face_vec)
                if face_dir is not None and not _would_contaminate(pos, face_dir):
                    ent.direction = face_dir
                    belt_dir_map[pos] = face_dir
                    break


def _verify_flow_continuity(
    entities: list[PlacedEntity],
    belt_dir_map: dict[tuple[int, int], EntityDirection],
    edge_starts: dict[int, tuple[int, int]] | None,
    edge_targets: dict[int, tuple[int, int]] | None,
) -> None:
    """Lightweight post-routing check: verify items can flow from start to target.

    For each edge with both a start (output inserter drop) and target (input
    inserter pickup), do a directional BFS downstream from the start tile. If
    the target tile is unreachable, scan the path for the first direction
    discontinuity and attempt to fix it.

    This catches direction corruptions that _fix_belt_directions() may introduce
    on non-protected tiles (e.g., mid-path tiles).
    """
    if not edge_starts or not edge_targets:
        return

    # Build position -> entity index for fixing directions
    pos_to_ent: dict[tuple[int, int], int] = {}
    belt_names = {"transport-belt", "fast-transport-belt", "express-transport-belt"}
    for idx, e in enumerate(entities):
        if e.name in belt_names:
            pos_to_ent[(e.x, e.y)] = idx

    # Find edges that have both start and target
    common_edges = set(edge_starts.keys()) & set(edge_targets.keys())
    if not common_edges:
        return

    for edge_idx in common_edges:
        start = edge_starts[edge_idx]
        target = edge_targets[edge_idx]

        if start not in belt_dir_map or target not in belt_dir_map:
            continue

        # BFS downstream from start
        visited: set[tuple[int, int]] = set()
        queue = [start]
        visited.add(start)
        while queue:
            pos = queue.pop(0)
            d = belt_dir_map.get(pos)
            if d is None:
                continue
            dx, dy = DIR_VEC[d]
            nb = (pos[0] + dx, pos[1] + dy)
            if nb in belt_dir_map and nb not in visited:
                visited.add(nb)
                queue.append(nb)

        if target in visited:
            continue  # Flow is valid

        # Target not reachable — try to find and fix the break.
        # Walk backward from target to find the last reachable tile on the path.
        # Then check if the unreachable tile's direction can be fixed.
        for ddx, ddy in DIRECTIONS:
            neighbor = (target[0] + ddx, target[1] + ddy)
            if neighbor in visited and neighbor in belt_dir_map:
                n_dir = belt_dir_map[neighbor]
                n_vec = DIR_VEC[n_dir]
                # If neighbor doesn't point at target, try to fix it
                if (neighbor[0] + n_vec[0], neighbor[1] + n_vec[1]) != target:
                    fix_vec = (target[0] - neighbor[0], target[1] - neighbor[1])
                    fix_dir = DIR_MAP.get(fix_vec)
                    if fix_dir is not None:
                        logger.debug(
                            "Flow fix at (%d,%d): %s→%s (target unreachable)",
                            neighbor[0],
                            neighbor[1],
                            n_dir,
                            fix_dir,
                        )
                        belt_dir_map[neighbor] = fix_dir
                        if neighbor in pos_to_ent:
                            entities[pos_to_ent[neighbor]].direction = fix_dir
                        break  # Fixed one break, stop searching


def route_connections(
    graph: ProductionGraph,
    positions: dict[int, tuple[int, int]],
    edge_targets: dict[int, tuple[int, int]] | None = None,
    edge_starts: dict[int, tuple[int, int]] | None = None,
    reserved_tiles: set[tuple[int, int]] | None = None,
    edge_exclusions: dict[int, set[tuple[int, int]]] | None = None,
    edge_subgroups: dict[str, list[list[int]]] | None = None,
    edge_order: list[int] | None = None,
    edge_lane_info: dict[int, tuple[str | None, tuple[int, int] | None]] | None = None,
    skip_edges: set[int] | None = None,
    existing_belt_dir_map: dict[tuple[int, int], EntityDirection] | None = None,
    existing_group_networks: dict[tuple[str, int], set[tuple[int, int]]] | None = None,
    io_y_slots: tuple[dict[str, int] | None, dict[str, int] | None] | None = None,
) -> RoutingResult:
    """Route all flow edges as belts/pipes using A* pathfinding.

    External edges sharing the same item are grouped and routed consecutively.
    After the first edge in a group is routed, subsequent edges branch from
    the existing network rather than routing independently from the boundary.

    Args:
        edge_targets: Mapping from edge index to a specific belt tile target.
        edge_starts: Mapping from edge index to a specific belt tile start.
        reserved_tiles: Pre-occupied tiles the router must avoid.
        edge_exclusions: Per-edge tiles to temporarily unblock from obstacles.
            Maps edge index -> set of tiles that only this edge may use.
        edge_subgroups: Per-item sub-groups for capacity splitting.
            Maps item -> list of sub-groups (each a list of edge indices).
            Sub-groups route independently with separate trunk networks.
        edge_order: Optional custom edge routing order. When provided,
            a list of edge indices into graph.edges that overrides
            the default internal-then-input-then-output ordering.
        edge_lane_info: Lane info for A* pathfinding. Maps edge index to
            (start_lane, goal_inserter_side_vec). start_lane is "left"/"right"
            for output inserters. goal_inserter_side_vec is (dx, dy) from belt
            tile toward the input inserter, for dynamic goal lane checking.
    """
    if edge_targets is None:
        edge_targets = {}
    if edge_starts is None:
        edge_starts = {}
    if edge_exclusions is None:
        edge_exclusions = {}
    if edge_subgroups is None:
        edge_subgroups = {}
    if edge_lane_info is None:
        edge_lane_info = {}
    if skip_edges is None:
        skip_edges = set()
    entities: list[PlacedEntity] = []
    failed_edges: list[FlowEdge] = []

    # Pre-compute y-slot assignments for external I/O boundary convention
    if io_y_slots is not None:
        input_y_slots, output_y_slots = io_y_slots
    else:
        input_y_slots, output_y_slots = _compute_io_y_slots(graph, positions)

    # Build initial obstacle set from machine footprints + reserved tiles
    # Only include machines that have positions (supports incremental building)
    occupied: set[tuple[int, int]] = set(reserved_tiles) if reserved_tiles else set()
    node_map = {n.id: n for n in graph.nodes}
    for node_id, (x, y) in positions.items():
        node = node_map.get(node_id)
        if node is None:
            continue
        size = machine_size(node.spec.entity)
        occupied |= machine_tiles(x, y, size)

    # Compute max grid extent for A* bounds
    if positions:
        max_x = max(x for x, y in positions.values()) + 10
        max_y = max(y for x, y in positions.values()) + 10
        max_extent = max(max_x, max_y, 50)
    else:
        max_extent = 50

    # Track belt networks per sub-group for network-aware routing
    # Key: (item, subgroup_idx) — each sub-group routes independently
    # Initialize from existing state if provided (for incremental calls)
    group_networks: dict[tuple[str, int], set[tuple[int, int]]] = (
        dict(existing_group_networks) if existing_group_networks else {}
    )
    # Track belt directions for junction-aware routing
    belt_dir_map: dict[tuple[int, int], EntityDirection] = dict(existing_belt_dir_map) if existing_belt_dir_map else {}

    # --- Map each edge index to its sub-group key ---
    edge_group_key: dict[int, tuple[str, int]] = {}
    for item, groups in edge_subgroups.items():
        for g_idx, edge_indices in enumerate(groups):
            for ei in edge_indices:
                edge_group_key[ei] = (item, g_idx)

    # --- Group edges by type ---
    internal_indices = []
    # Routing groups: each is (group_key, [edge_indices])
    # Input and output sub-groups route as separate groups
    input_routing_groups: list[tuple[tuple[str, int], list[int]]] = []
    output_routing_groups: list[tuple[tuple[str, int], list[int]]] = []
    # Track edges not in any sub-group (fallback to item-based grouping)
    ungrouped_inputs: dict[str, list[int]] = {}
    ungrouped_outputs: dict[str, list[int]] = {}

    for i, edge in enumerate(graph.edges):
        if edge.from_node is not None and edge.to_node is not None:
            internal_indices.append(i)
        elif edge.from_node is None:
            if i in edge_group_key:
                # Will be added via sub-groups below
                pass
            else:
                ungrouped_inputs.setdefault(edge.item, []).append(i)
        elif i in edge_group_key:
            pass  # handled via sub-groups
        else:
            ungrouped_outputs.setdefault(edge.item, []).append(i)

    # Build sub-group routing groups from edge_subgroups
    for item, groups in edge_subgroups.items():
        for g_idx, edge_indices in enumerate(groups):
            key = (item, g_idx)
            inputs = [i for i in edge_indices if graph.edges[i].from_node is None]
            outputs = [i for i in edge_indices if graph.edges[i].to_node is None]
            if inputs:
                input_routing_groups.append((key, inputs))
            if outputs:
                output_routing_groups.append((key, outputs))

    # Add ungrouped edges as their own groups
    for item, indices in ungrouped_inputs.items():
        input_routing_groups.append(((item, 0), indices))
    for item, indices in ungrouped_outputs.items():
        output_routing_groups.append(((item, 0), indices))

    # Sort internal edges by distance (shorter first)
    def _distance_key(idx: int) -> float:
        e = graph.edges[idx]
        if e.from_node is None or e.to_node is None:
            return 0
        if e.from_node not in positions or e.to_node not in positions:
            return 0
        fx, fy = positions[e.from_node]
        tx, ty = positions[e.to_node]
        return abs(fx - tx) + abs(fy - ty)

    internal_indices.sort(key=_distance_key)

    # Sort edges within each input group by spatial proximity
    for _key, indices in input_routing_groups:
        if len(indices) <= 1:
            continue

        def _input_sort_key(idx: int) -> float:
            e = graph.edges[idx]
            if e.to_node is None or e.to_node not in positions:
                return 0
            tx, ty = positions[e.to_node]
            return tx + ty

        indices.sort(key=_input_sort_key)

    # Sort edges within each output group by spatial proximity
    for _key, indices in output_routing_groups:
        if len(indices) <= 1:
            continue

        def _output_sort_key(idx: int) -> float:
            e = graph.edges[idx]
            if e.from_node is None or e.from_node not in positions:
                return 0
            fx, fy = positions[e.from_node]
            return fx + fy

        indices.sort(key=_output_sort_key)

    # Build the routing order: internal edges, then input groups, then output groups
    routing_order: list[tuple[int, bool, tuple[str, int]]] = []
    for idx in internal_indices:
        edge = graph.edges[idx]
        key = edge_group_key.get(idx, (edge.item, 0))
        routing_order.append((idx, False, key))
    for key, indices in input_routing_groups:
        for rank, idx in enumerate(indices):
            routing_order.append((idx, rank > 0, key))
    for key, indices in output_routing_groups:
        for rank, idx in enumerate(indices):
            routing_order.append((idx, rank > 0, key))

    # Compute total rate per routing group (for belt tier selection)
    group_total_rate: dict[tuple[str, int], float] = {}
    for key, indices in input_routing_groups:
        group_total_rate[key] = sum(graph.edges[i].rate for i in indices)
    for key, indices in output_routing_groups:
        group_total_rate.setdefault(key, 0)
        group_total_rate[key] += sum(graph.edges[i].rate for i in indices)

    # Override routing order if custom edge_order is provided
    if edge_order is not None:
        # Build a set of all edge indices present in the default routing_order
        default_indices = {idx for idx, _, _ in routing_order}
        # Build a lookup from edge index to its group key
        idx_to_key: dict[int, tuple[str, int]] = {}
        for idx, _, key in routing_order:
            idx_to_key[idx] = key
        # Rebuild routing_order using the custom edge_order, keeping
        # is_continuation logic: first occurrence per group_key is not
        # a continuation, subsequent ones are.
        seen_groups: set[tuple[str, int]] = set()
        new_routing_order: list[tuple[int, bool, tuple[str, int]]] = []
        for idx in edge_order:
            if idx not in default_indices:
                continue
            key = idx_to_key.get(idx, (graph.edges[idx].item, 0))
            is_cont = key in seen_groups
            seen_groups.add(key)
            new_routing_order.append((idx, is_cont, key))
        # Append any edges not in edge_order (safety net)
        covered = {i for i, _, _ in new_routing_order}
        for idx, is_cont, key in routing_order:
            if idx not in covered:
                new_routing_order.append((idx, is_cont, key))
        routing_order = new_routing_order

    # --- Route each edge ---
    for edge_idx, is_continuation, group_key in routing_order:
        if edge_idx in skip_edges:
            continue

        edge = graph.edges[edge_idx]

        # Temporarily unblock tiles reserved for this specific edge
        exclusions = edge_exclusions.get(edge_idx, set())
        if exclusions:
            occupied -= exclusions

        # Determine belt tier early (needed for underground reach)
        if edge.is_fluid:
            belt_name = "pipe"
        elif group_key in group_total_rate:
            belt_name = belt_entity_for_rate(group_total_rate[group_key])
        else:
            belt_name = belt_entity_for_rate(edge.rate)
        ug_reach = _UG_MAX_REACH.get(belt_name, 4)

        # Determine start and goal tiles
        has_start = edge_idx in edge_starts
        has_target = edge_idx in edge_targets
        network = group_networks.get(group_key, set())

        if is_continuation and network:
            # Network-aware routing for continuations.
            # Input vs output use different strategies:
            # - Inputs: extend trunk from downstream ends (items flow forward)
            # - Outputs: sideload into trunk via perpendicular approach tiles
            approach = _perpendicular_approach_tiles(network, belt_dir_map, occupied)
            junction_tiles = approach if approach else set(network)

            if edge.from_node is None and not edge.is_fluid:
                # External input continuation (belts): extend the trunk from
                # its downstream end toward this machine's input belt tile.
                # Items flow forward through the trunk extension.
                downstream_ends = _network_downstream_ends(network, belt_dir_map)
                forward_tiles = set()
                for tile in downstream_ends:
                    d = belt_dir_map.get(tile)
                    if d is not None:
                        dx, dy = DIR_VEC[d]
                        forward_tiles.add((tile[0] + dx, tile[1] + dy))
                start_tiles = forward_tiles if forward_tiles else junction_tiles
                if has_target:
                    goal_tiles = {edge_targets[edge_idx]}
                else:
                    _, goal_tiles = _edge_endpoints(edge, graph, positions, occupied, input_y_slots, output_y_slots)
                # Forward tiles are outside the network — no obstacle changes needed
            elif edge.from_node is None:
                # External input continuation (fluids): pipes connect
                # omnidirectionally, perpendicular approach works fine
                start_tiles = junction_tiles
                if has_target:
                    goal_tiles = {edge_targets[edge_idx]}
                else:
                    _, goal_tiles = _edge_endpoints(edge, graph, positions, occupied, input_y_slots, output_y_slots)
            else:
                # External output continuation: route to nearest trunk
                # connection point (sideload or upstream approach)
                if has_start:
                    start_tiles = {edge_starts[edge_idx]}
                else:
                    start_tiles, _ = _edge_endpoints(edge, graph, positions, occupied, input_y_slots, output_y_slots)
                upstream_ends = _network_upstream_ends(network, belt_dir_map)
                upstream_approach = set()
                for tile in upstream_ends:
                    d = belt_dir_map.get(tile)
                    if d is not None:
                        dx, dy = DIR_VEC[d]
                        back = (tile[0] - dx, tile[1] - dy)
                        if back not in occupied and back not in network:
                            upstream_approach.add(back)
                goal_tiles = junction_tiles | upstream_approach

            # Never remove network from obstacles — routing through existing
            # network tiles creates belt loops. If no approach/forward tiles
            # are available, the edge will fail routing (better than a loop).
        elif has_start or has_target:
            if has_start and has_target:
                start_tiles = {edge_starts[edge_idx]}
                goal_tiles = {edge_targets[edge_idx]}
            elif has_target:
                start_tiles, _ = _edge_endpoints(edge, graph, positions, occupied, input_y_slots, output_y_slots)
                goal_tiles = {edge_targets[edge_idx]}
            else:  # has_start only
                _, goal_tiles = _edge_endpoints(edge, graph, positions, occupied, input_y_slots, output_y_slots)
                start_tiles = {edge_starts[edge_idx]}
        else:
            start_tiles, goal_tiles = _edge_endpoints(edge, graph, positions, occupied, input_y_slots, output_y_slots)

        if not start_tiles or not goal_tiles:
            if exclusions:
                occupied |= exclusions
            continue

        # Single-pass A* with underground always enabled.
        # The UG cost multiplier (3x) ensures surface is preferred when free.
        # For fluid edges, use pipe-to-ground reach instead of belt-tier reach.
        effective_ug_reach = _UG_PIPE_REACH if edge.is_fluid else ug_reach

        # Lane info for this edge
        s_lane, g_lane_check = edge_lane_info.get(edge_idx, (None, None))

        # Precompute tiles belonging to other items' networks (for contamination avoidance)
        other_item_tiles: set[tuple[int, int]] | None = None
        if not edge.is_fluid:
            other_item_tiles = set()
            for (other_item, _sg), tiles in group_networks.items():
                if other_item != edge.item:
                    other_item_tiles |= tiles
            if not other_item_tiles:
                other_item_tiles = None  # no foreign networks yet

        best_path = None
        for start in start_tiles:
            if start in occupied:
                continue
            path = _astar_path(
                start,
                goal_tiles - occupied,
                occupied,
                max_extent,
                allow_underground=True,
                ug_max_reach=effective_ug_reach,
                start_lane=s_lane,
                goal_lane_check=g_lane_check,
                belt_dir_map=belt_dir_map,
                other_item_tiles=other_item_tiles,
            )
            if path and (best_path is None or len(path) < len(best_path)):
                best_path = path

        # Safety net: check for cross-item contamination that slipped past
        # the in-A* filter (e.g. due to direction post-processing).
        # Checks both outgoing (our belt → their network) and incoming
        # (their belt → our tile). Retry up to 5 times.
        if best_path and not edge.is_fluid:
            all_blocked: set[tuple[int, int]] = set()
            for _retry in range(5):
                contaminating: set[tuple[int, int]] = set()
                for i, (px, py) in enumerate(best_path):
                    if (px, py) in network:
                        continue
                    # Outgoing check: infer belt direction from path neighbors
                    if i + 1 < len(best_path):
                        ddx = best_path[i + 1][0] - px
                        ddy = best_path[i + 1][1] - py
                    elif i > 0:
                        ddx = px - best_path[i - 1][0]
                        ddy = py - best_path[i - 1][1]
                    else:
                        continue
                    target = (px + ddx, py + ddy)
                    for (oi, _sg), other_tiles in group_networks.items():
                        if oi != edge.item and target in other_tiles:
                            contaminating.add((px, py))
                            break
                    # Incoming check: foreign belt pointing at our tile
                    if (px, py) not in contaminating:
                        for cdx, cdy in DIRECTIONS:
                            adj = (px + cdx, py + cdy)
                            adj_dir = belt_dir_map.get(adj)
                            if adj_dir is not None and DIR_VEC[adj_dir] == (-cdx, -cdy):
                                for (oi, _sg), other_tiles in group_networks.items():
                                    if oi != edge.item and adj in other_tiles:
                                        contaminating.add((px, py))
                                        break
                                if (px, py) in contaminating:
                                    break

                if not contaminating:
                    break  # path is clean

                all_blocked |= contaminating
                occupied |= contaminating
                best_path = None
                for start in start_tiles:
                    if start in occupied:
                        continue
                    path = _astar_path(
                        start,
                        goal_tiles - occupied,
                        occupied,
                        max_extent,
                        allow_underground=True,
                        ug_max_reach=effective_ug_reach,
                        start_lane=s_lane,
                        goal_lane_check=g_lane_check,
                        belt_dir_map=belt_dir_map,
                        other_item_tiles=other_item_tiles,
                    )
                    if path and (best_path is None or len(path) < len(best_path)):
                        best_path = path
                if best_path is None:
                    break  # no alternative found

            occupied -= all_blocked  # restore

        # Re-block the exclusion tiles
        if exclusions:
            occupied |= exclusions

        if best_path is None:
            failed_edges.append(edge)
            continue

        # Post-process: if path ends with an underground jump, extend by one
        # surface tile in the jump direction so items have somewhere to go
        if len(best_path) >= 2:
            last = best_path[-1]
            prev = best_path[-2]
            dist = abs(last[0] - prev[0]) + abs(last[1] - prev[1])
            if dist > 1:
                dx = (last[0] - prev[0]) // dist
                dy = (last[1] - prev[1]) // dist
                ext = (last[0] + dx, last[1] + dy)
                if ext not in occupied:
                    best_path.append(ext)

        # Place entities along path, skipping tiles already on the network
        new_tiles = [t for t in best_path if t not in network]
        if new_tiles:
            path_entities = _path_to_entities(new_tiles, belt_name, edge.item, edge.is_fluid)
            entities.extend(path_entities)
            # Track belt directions for junction-aware routing
            for pe in path_entities:
                belt_dir_map[(pe.x, pe.y)] = pe.direction

        # Update network and occupied tiles
        path_set = set(best_path)
        group_networks.setdefault(group_key, set()).update(path_set)
        occupied |= path_set

    # Collect inserter-adjacent belt tiles that must not be reoriented
    _protected: set[tuple[int, int]] = set()
    if edge_targets:
        _protected.update(edge_targets.values())
    if edge_starts:
        _protected.update(edge_starts.values())

    # Post-process belt directions to fix T-junctions, underground exits, orphans
    _fix_belt_directions(entities, belt_dir_map, protected_tiles=_protected)

    # Safety net: verify flow continuity after direction post-processing (Phase 1.3)
    _verify_flow_continuity(entities, belt_dir_map, edge_starts, edge_targets)

    return RoutingResult(
        entities=entities,
        occupied=occupied,
        failed_edges=failed_edges,
        belt_dir_map=belt_dir_map,
        group_networks=group_networks,
    )


def _edge_endpoints(
    edge: FlowEdge,
    graph: ProductionGraph,
    positions: dict[int, tuple[int, int]],
    occupied: set[tuple[int, int]],
    input_y_slots: dict[str, int] | None = None,
    output_y_slots: dict[str, int] | None = None,
) -> tuple[set[tuple[int, int]], set[tuple[int, int]]]:
    """Determine start and goal tile sets for routing an edge.

    For internal edges: start/goal = belt tiles (2 tiles from machine),
    leaving the border tile free for an inserter.
    For external inputs: start = LEFT edge at assigned y-slot.
    For external outputs: goal = RIGHT edge at assigned y-slot.
    """
    start_tiles: set[tuple[int, int]] = set()
    goal_tiles: set[tuple[int, int]] = set()

    if edge.from_node is not None:
        fx, fy = positions[edge.from_node]
        src_node = next(n for n in graph.nodes if n.id == edge.from_node)
        size = machine_size(src_node.spec.entity)
        for bx, by in _machine_belt_tiles(fx, fy, size):
            start_tiles.add((bx, by))
    else:
        # External input — start from the LEFT edge at this item's y-slot
        if positions:
            all_x = [x for x, _ in positions.values()]
            left_x = min(all_x) - 3
            if input_y_slots and edge.item in input_y_slots:
                y_slot = input_y_slots[edge.item]
                start_tiles.add((left_x, y_slot))
            else:
                # Fallback: spread along left edge
                all_y = [y for _, y in positions.values()]
                min_gy = min(all_y) - 3
                max_gy = max(all_y) + 8
                for y in range(min_gy, max_gy + 1):
                    start_tiles.add((left_x, y))

    if edge.to_node is not None:
        tx, ty = positions[edge.to_node]
        dst_node = next(n for n in graph.nodes if n.id == edge.to_node)
        size = machine_size(dst_node.spec.entity)
        for bx, by in _machine_belt_tiles(tx, ty, size):
            goal_tiles.add((bx, by))
    else:
        # External output — route to the RIGHT edge at this item's y-slot
        if positions:
            all_x = [x for x, _ in positions.values()]
            # Find max machine size for right boundary
            max_size = max(
                (machine_size(n.spec.entity) for n in graph.nodes),
                default=3,
            )
            right_x = max(all_x) + max_size + 3
            if output_y_slots and edge.item in output_y_slots:
                y_slot = output_y_slots[edge.item]
                goal_tiles.add((right_x, y_slot))
            else:
                # Fallback: spread along right edge
                all_y = [y for _, y in positions.values()]
                min_gy = min(all_y) - 3
                max_gy = max(all_y) + 8
                for y in range(min_gy, max_gy + 1):
                    goal_tiles.add((right_x, y))

    return start_tiles, goal_tiles
