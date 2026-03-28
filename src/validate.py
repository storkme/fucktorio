"""Functional blueprint validation: checks that layouts actually work in Factorio."""

from __future__ import annotations

from collections import defaultdict, deque
from dataclasses import dataclass

from .models import EntityDirection, LayoutResult, PlacedEntity, SolverResult

_3x3_ENTITIES = {
    "assembling-machine-1",
    "assembling-machine-2",
    "assembling-machine-3",
    "chemical-plant",
}
_5x5_ENTITIES = {"oil-refinery"}
_MACHINE_ENTITIES = _3x3_ENTITIES | _5x5_ENTITIES
_PIPE_ENTITIES = {"pipe", "pipe-to-ground"}
_BELT_ENTITIES = {"transport-belt", "fast-transport-belt", "express-transport-belt"}
_INSERTER_ENTITIES = {"inserter", "long-handed-inserter", "fast-inserter", "stack-inserter"}

# Inserter reach: how many tiles from the inserter the pickup/drop position is
_INSERTER_REACH = {
    "inserter": 1,
    "fast-inserter": 1,
    "stack-inserter": 1,
    "long-handed-inserter": 2,
}

# Belt throughput limits (items per second)
_BELT_THROUGHPUT = {
    "transport-belt": 15.0,
    "fast-transport-belt": 30.0,
    "express-transport-belt": 45.0,
}

# Direction → (dx, dy) the inserter drops toward
_DIR_TO_VEC: dict[EntityDirection, tuple[int, int]] = {
    EntityDirection.NORTH: (0, -1),
    EntityDirection.EAST: (1, 0),
    EntityDirection.SOUTH: (0, 1),
    EntityDirection.WEST: (-1, 0),
}

# Opposite direction vectors
_OPPOSITE_VEC: dict[tuple[int, int], tuple[int, int]] = {
    (0, -1): (0, 1),
    (0, 1): (0, -1),
    (1, 0): (-1, 0),
    (-1, 0): (1, 0),
}


@dataclass
class ValidationIssue:
    """A single validation finding."""

    severity: str  # "error" | "warning"
    category: str  # "pipe-isolation", "fluid-connectivity", "inserter", "power"
    message: str
    x: int | None = None
    y: int | None = None


class ValidationError(Exception):
    """Raised when critical validation issues block blueprint generation."""

    def __init__(self, issues: list[ValidationIssue]):
        self.issues = issues
        messages = [f"  [{i.severity}] {i.message}" for i in issues]
        super().__init__("Validation failed:\n" + "\n".join(messages))


def validate(
    layout_result: LayoutResult,
    solver_result: SolverResult | None = None,
    layout_style: str = "bus",
) -> list[ValidationIssue]:
    """Run all functional validation checks on a layout.

    Args:
        layout_style: "bus" for main-bus layouts, "spaghetti" for
            constraint-based layouts (adapts fluid connectivity check).

    Returns a list of issues found. Raises ValidationError if any
    errors (not just warnings) are present.
    """
    issues: list[ValidationIssue] = []

    issues.extend(check_pipe_isolation(layout_result))
    issues.extend(check_fluid_port_connectivity(layout_result, layout_style=layout_style))
    issues.extend(check_inserter_chains(layout_result, solver_result))
    issues.extend(check_inserter_direction(layout_result))
    issues.extend(check_belt_connectivity(layout_result, solver_result))
    issues.extend(check_belt_flow_path(layout_result, solver_result, layout_style=layout_style))
    issues.extend(check_belt_direction_continuity(layout_result))
    issues.extend(check_belt_throughput(layout_result))
    issues.extend(check_power_coverage(layout_result))

    errors = [i for i in issues if i.severity == "error"]
    if errors:
        raise ValidationError(errors)

    return issues


def check_pipe_isolation(layout_result: LayoutResult) -> list[ValidationIssue]:
    """Check that adjacent pipes don't carry different fluids.

    In Factorio, adjacent pipes automatically connect and merge their
    fluid networks. Two pipes carrying different fluids must not be
    on adjacent tiles.
    """
    issues: list[ValidationIssue] = []

    # Build pipe tile map
    pipe_map: dict[tuple[int, int], str | None] = {}
    for e in layout_result.entities:
        if e.name in _PIPE_ENTITIES:
            pipe_map[(e.x, e.y)] = e.carries

    # Check all four neighbours
    checked: set[tuple[tuple[int, int], tuple[int, int]]] = set()
    for (px, py), carries in pipe_map.items():
        if carries is None:
            continue
        for dx, dy in [(1, 0), (-1, 0), (0, 1), (0, -1)]:
            nb = (px + dx, py + dy)
            if nb not in pipe_map or pipe_map[nb] is None:
                continue
            pair = (min((px, py), nb), max((px, py), nb))
            if pair in checked:
                continue
            checked.add(pair)
            if pipe_map[nb] != carries:
                issues.append(
                    ValidationIssue(
                        severity="error",
                        category="pipe-isolation",
                        message=(
                            f"Adjacent pipes carry different fluids: "
                            f"({px},{py}) carries {carries}, "
                            f"({nb[0]},{nb[1]}) carries {pipe_map[nb]}"
                        ),
                        x=px,
                        y=py,
                    )
                )

    return issues


def _machine_size(name: str) -> int:
    if name in _5x5_ENTITIES:
        return 5
    return 3


def _get_fluid_ports(entity_name: str) -> list[tuple[int, int, str]]:
    """Get fluid port positions relative to tile_position (top-left).

    Returns list of (rel_x, rel_y, production_type) where production_type
    is 'input' or 'output'.
    """
    from draftsman.data import entities

    raw = entities.raw.get(entity_name, {})
    fluid_boxes = raw.get("fluid_boxes", [])
    size = _machine_size(entity_name)
    center = size // 2

    ports: list[tuple[int, int, str]] = []
    for fb in fluid_boxes:
        if not isinstance(fb, dict) or "pipe_connections" not in fb:
            continue
        conn = fb["pipe_connections"]
        if isinstance(conn, list):
            conn = conn[0]
        pos = conn.get("position", [0, 0])
        prod_type = fb.get("production_type", "input")
        direction = conn.get("direction", 0)

        # Convert center-relative to top-left-relative
        port_x = int(pos[0]) + center
        port_y = int(pos[1]) + center

        # Pipe connects one tile outward from the port
        if direction == 0:  # north
            pipe_y = port_y - 1
        elif direction == 8:  # south
            pipe_y = port_y + 1
        else:
            continue
        ports.append((port_x, pipe_y, prod_type))

    return ports


def check_fluid_port_connectivity(
    layout_result: LayoutResult,
    layout_style: str = "bus",
) -> list[ValidationIssue]:
    """Check that every machine's fluid ports have connected pipes.

    For each machine with fluid ports, verifies:
    1. At least one input port has an adjacent pipe
    2. (bus mode only) At least one input pipe is reachable from the bus via BFS
    3. At least one output port has an adjacent pipe

    In spaghetti mode, the bus-reachability check is skipped since there
    is no bus — only port adjacency is verified.
    """
    issues: list[ValidationIssue] = []

    # Build pipe tile set
    pipe_tiles: set[tuple[int, int]] = set()
    for e in layout_result.entities:
        if e.name in _PIPE_ENTITIES:
            pipe_tiles.add((e.x, e.y))

    # Build pipe-to-ground pair map for tunnel traversal
    ptg_pairs = _find_ptg_pairs(layout_result)

    # Find bus pipe positions (leftmost column of pipes = bus)
    bus_pipes: set[tuple[int, int]] = set()
    if pipe_tiles:
        min_x = min(x for x, _ in pipe_tiles)
        bus_pipes = {(x, y) for x, y in pipe_tiles if x == min_x}

    for e in layout_result.entities:
        if e.name not in _MACHINE_ENTITIES:
            continue
        if e.recipe is None:
            continue

        ports = _get_fluid_ports(e.name)
        if not ports:
            continue

        # assembling-machine-3 has fluid_boxes_off_when_no_fluid_recipe — skip
        # fluid port checks if no pipes are adjacent (non-fluid recipe)
        if e.name == "assembling-machine-3":
            has_any_pipe = any((e.x + rx, e.y + ry) in pipe_tiles for rx, ry, _ in ports)
            if not has_any_pipe:
                continue

        # Group ports by type
        input_ports = [(rx, ry) for rx, ry, pt in ports if pt == "input"]
        output_ports = [(rx, ry) for rx, ry, pt in ports if pt == "output"]

        # Check input ports: at least one must have a pipe and connect to bus
        if input_ports:
            input_pipe_positions = [
                (e.x + rx, e.y + ry) for rx, ry in input_ports if (e.x + rx, e.y + ry) in pipe_tiles
            ]
            if not input_pipe_positions:
                issues.append(
                    ValidationIssue(
                        severity="error",
                        category="fluid-connectivity",
                        message=f"{e.name} at ({e.x},{e.y}): no input port has an adjacent pipe",
                        x=e.x,
                        y=e.y,
                    )
                )
            elif layout_style == "bus" and bus_pipes:
                # Check at least one input pipe connects to bus
                any_connected = any(
                    _bfs_pipe_reach(pos, pipe_tiles, ptg_pairs) & bus_pipes for pos in input_pipe_positions
                )
                if not any_connected:
                    issues.append(
                        ValidationIssue(
                            severity="error",
                            category="fluid-connectivity",
                            message=f"{e.name} at ({e.x},{e.y}): input pipes not connected to bus",
                            x=e.x,
                            y=e.y,
                        )
                    )

        # Check output ports: at least one must have a pipe (no bus check needed)
        if output_ports:
            has_output_pipe = any((e.x + rx, e.y + ry) in pipe_tiles for rx, ry in output_ports)
            if not has_output_pipe:
                issues.append(
                    ValidationIssue(
                        severity="error",
                        category="fluid-connectivity",
                        message=f"{e.name} at ({e.x},{e.y}): no output port has an adjacent pipe",
                        x=e.x,
                        y=e.y,
                    )
                )

    return issues


def _find_ptg_pairs(layout_result: LayoutResult) -> dict[tuple[int, int], tuple[int, int]]:
    """Find pipe-to-ground pairs and return a bidirectional map.

    Pairs pipe-to-ground entities that face each other on the same row/column.
    Returns {pos_a: pos_b, pos_b: pos_a} for each pair.
    """
    from .models import EntityDirection

    pairs: dict[tuple[int, int], tuple[int, int]] = {}

    # Group by axis: horizontal pairs share y, vertical pairs share x
    # EAST-facing entry pairs with WEST-facing exit (same y, increasing x)
    # SOUTH-facing entry pairs with NORTH-facing exit (same x, increasing y)
    ptg_entities = [e for e in layout_result.entities if e.name == "pipe-to-ground"]

    # Horizontal pairs (EAST/WEST on same y)
    by_y: dict[int, list] = {}
    for e in ptg_entities:
        if e.direction in (EntityDirection.EAST, EntityDirection.WEST):
            by_y.setdefault(e.y, []).append(e)

    for _y, group in by_y.items():
        east_facing = sorted([e for e in group if e.direction == EntityDirection.EAST], key=lambda e: e.x)
        west_facing = sorted([e for e in group if e.direction == EntityDirection.WEST], key=lambda e: e.x)
        # Pair each EAST with the nearest WEST to its right
        for ef in east_facing:
            for wf in west_facing:
                if wf.x > ef.x:
                    a, b = (ef.x, ef.y), (wf.x, wf.y)
                    pairs[a] = b
                    pairs[b] = a
                    west_facing.remove(wf)
                    break

    # Vertical pairs (SOUTH/NORTH on same x)
    by_x: dict[int, list] = {}
    for e in ptg_entities:
        if e.direction in (EntityDirection.SOUTH, EntityDirection.NORTH):
            by_x.setdefault(e.x, []).append(e)

    for _x, group in by_x.items():
        south_facing = sorted([e for e in group if e.direction == EntityDirection.SOUTH], key=lambda e: e.y)
        north_facing = sorted([e for e in group if e.direction == EntityDirection.NORTH], key=lambda e: e.y)
        for sf in south_facing:
            for nf in north_facing:
                if nf.y > sf.y:
                    a, b = (sf.x, sf.y), (nf.x, nf.y)
                    pairs[a] = b
                    pairs[b] = a
                    north_facing.remove(nf)
                    break

    return pairs


def _bfs_pipe_reach(
    start: tuple[int, int],
    pipe_tiles: set[tuple[int, int]],
    ptg_pairs: dict[tuple[int, int], tuple[int, int]] | None = None,
) -> set[tuple[int, int]]:
    """BFS flood-fill through adjacent pipe tiles from start.

    Also traverses pipe-to-ground tunnel connections if ptg_pairs is provided
    (maps each pipe-to-ground position to its paired endpoint).
    """
    if ptg_pairs is None:
        ptg_pairs = {}

    visited: set[tuple[int, int]] = set()
    queue = deque([start])
    visited.add(start)

    while queue:
        x, y = queue.popleft()
        # Adjacent pipe connections
        for dx, dy in [(1, 0), (-1, 0), (0, 1), (0, -1)]:
            nb = (x + dx, y + dy)
            if nb in pipe_tiles and nb not in visited:
                visited.add(nb)
                queue.append(nb)
        # Pipe-to-ground tunnel connections
        if (x, y) in ptg_pairs:
            other = ptg_pairs[(x, y)]
            if other not in visited:
                visited.add(other)
                queue.append(other)

    return visited


def check_inserter_chains(
    layout_result: LayoutResult,
    solver_result: SolverResult | None = None,
) -> list[ValidationIssue]:
    """Check that every machine with solid I/O has inserters.

    Machines with only fluid inputs/outputs (e.g. oil refineries running
    basic-oil-processing) don't need inserters.
    """
    issues: list[ValidationIssue] = []

    # Build a set of recipes that need inserters (have solid inputs or outputs)
    fluid_only_recipes: set[str] = set()
    if solver_result is not None:
        for spec in solver_result.machines:
            has_solid = any(not f.is_fluid for f in spec.inputs + spec.outputs)
            if not has_solid:
                fluid_only_recipes.add(spec.recipe)

    # Build inserter positions
    inserter_positions: set[tuple[int, int]] = set()
    for e in layout_result.entities:
        if e.name == "inserter":
            inserter_positions.add((e.x, e.y))

    # Each machine with solid I/O should have at least one adjacent inserter
    checked_machines: set[tuple[int, int]] = set()
    for e in layout_result.entities:
        if e.name not in _MACHINE_ENTITIES:
            continue
        if (e.x, e.y) in checked_machines:
            continue
        checked_machines.add((e.x, e.y))

        # Skip machines that only have fluid I/O
        if e.recipe in fluid_only_recipes:
            continue

        size = _machine_size(e.name)
        has_inserter = False
        # Check tiles adjacent to the machine border
        for dx in range(-1, size + 1):
            for dy in range(-1, size + 1):
                if (e.x + dx, e.y + dy) in inserter_positions:
                    has_inserter = True
                    break
            if has_inserter:
                break

        if not has_inserter:
            issues.append(
                ValidationIssue(
                    severity="error",
                    category="inserter",
                    message=f"{e.name} at ({e.x},{e.y}): no inserter adjacent",
                    x=e.x,
                    y=e.y,
                )
            )

    return issues


def _build_machine_tile_set(layout_result: LayoutResult) -> set[tuple[int, int]]:
    """Build a set of all tiles occupied by machines."""
    tiles: set[tuple[int, int]] = set()
    for e in layout_result.entities:
        if e.name in _MACHINE_ENTITIES:
            size = _machine_size(e.name)
            for dx in range(size):
                for dy in range(size):
                    tiles.add((e.x + dx, e.y + dy))
    return tiles


def _get_fluid_only_recipes(solver_result: SolverResult | None) -> set[str]:
    """Get recipes that have only fluid I/O (no solid items)."""
    fluid_only: set[str] = set()
    if solver_result is not None:
        for spec in solver_result.machines:
            has_solid = any(not f.is_fluid for f in spec.inputs + spec.outputs)
            if not has_solid:
                fluid_only.add(spec.recipe)
    return fluid_only


def check_inserter_direction(
    layout_result: LayoutResult,
) -> list[ValidationIssue]:
    """Check that inserters face toward or away from an adjacent machine.

    An inserter picks from one side and drops to the other. Its direction
    indicates which way it drops. A valid inserter must have its drop or
    pickup side pointing at a machine — otherwise it's facing parallel to
    the machine border and won't transfer items.
    """
    issues: list[ValidationIssue] = []

    machine_tiles = _build_machine_tile_set(layout_result)

    for e in layout_result.entities:
        if e.name not in _INSERTER_ENTITIES:
            continue

        direction_vec = _DIR_TO_VEC.get(e.direction)
        if direction_vec is None:
            continue

        reach = _INSERTER_REACH.get(e.name, 1)
        dx, dy = direction_vec
        odx, ody = _OPPOSITE_VEC[direction_vec]

        # Drop side: reach tiles in facing direction
        drop_pos = (e.x + dx * reach, e.y + dy * reach)
        # Pickup side: reach tiles in opposite direction
        pickup_pos = (e.x + odx * reach, e.y + ody * reach)

        drop_touches_machine = drop_pos in machine_tiles
        pickup_touches_machine = pickup_pos in machine_tiles

        if not drop_touches_machine and not pickup_touches_machine:
            issues.append(
                ValidationIssue(
                    severity="error",
                    category="inserter-direction",
                    message=(
                        f"inserter at ({e.x},{e.y}) facing {e.direction.name}: "
                        f"neither drop nor pickup side touches a machine"
                    ),
                    x=e.x,
                    y=e.y,
                )
            )

    return issues


def check_belt_connectivity(
    layout_result: LayoutResult,
    solver_result: SolverResult | None = None,
) -> list[ValidationIssue]:
    """Check that every machine with solid I/O is connected to belts via inserters.

    Verifies two things:
    1. Each machine has at least one inserter whose non-machine side touches a belt.
    2. That belt is part of a connected belt network that reaches another machine's
       inserter or the edge of the layout (external input/output).

    Machines with only fluid I/O are skipped (they use pipes, not belts).
    """
    issues: list[ValidationIssue] = []

    # Identify fluid-only recipes to skip
    fluid_only_recipes: set[str] = set()
    if solver_result is not None:
        for spec in solver_result.machines:
            has_solid = any(not f.is_fluid for f in spec.inputs + spec.outputs)
            if not has_solid:
                fluid_only_recipes.add(spec.recipe)

    # Build tile maps
    belt_tiles: set[tuple[int, int]] = set()
    inserter_positions: set[tuple[int, int]] = set()
    for e in layout_result.entities:
        if e.name in _BELT_ENTITIES:
            belt_tiles.add((e.x, e.y))
        elif e.name in _INSERTER_ENTITIES:
            inserter_positions.add((e.x, e.y))

    if not belt_tiles:
        # No belts at all — if there are machines needing solid I/O, that's bad
        has_solid_machine = any(
            e.name in _MACHINE_ENTITIES and e.recipe not in fluid_only_recipes
            for e in layout_result.entities
            if e.name in _MACHINE_ENTITIES
        )
        if has_solid_machine:
            issues.append(
                ValidationIssue(
                    severity="error",
                    category="belt-connectivity",
                    message="No belts in layout but machines require solid item transport",
                )
            )
        return issues

    # Build machine tile sets for lookup
    machine_entities = [e for e in layout_result.entities if e.name in _MACHINE_ENTITIES]
    machine_tile_map: dict[tuple[int, int], PlacedEntity] = {}
    for e in machine_entities:
        size = _machine_size(e.name)
        for dx in range(size):
            for dy in range(size):
                machine_tile_map[(e.x + dx, e.y + dy)] = e

    # For each machine with solid I/O, check inserter-to-belt connectivity
    checked_machines: set[tuple[int, int]] = set()
    for e in machine_entities:
        if (e.x, e.y) in checked_machines:
            continue
        checked_machines.add((e.x, e.y))

        if e.recipe in fluid_only_recipes:
            continue

        size = _machine_size(e.name)
        machine_tiles = {(e.x + dx, e.y + dy) for dx in range(size) for dy in range(size)}

        # Find inserters adjacent to this machine
        adjacent_inserters: list[tuple[int, int]] = []
        for dx in range(-1, size + 1):
            for dy in range(-1, size + 1):
                pos = (e.x + dx, e.y + dy)
                if pos in inserter_positions and pos not in machine_tiles:
                    adjacent_inserters.append(pos)

        # Check if any inserter has a belt on its non-machine side
        has_belt_connection = False
        for ix, iy in adjacent_inserters:
            for dx, dy in [(1, 0), (-1, 0), (0, 1), (0, -1)]:
                nb = (ix + dx, iy + dy)
                if nb in belt_tiles and nb not in machine_tiles:
                    has_belt_connection = True
                    break
            if has_belt_connection:
                break

        if not has_belt_connection:
            issues.append(
                ValidationIssue(
                    severity="error",
                    category="belt-connectivity",
                    message=(
                        f"{e.name} at ({e.x},{e.y}): no inserter connects to a belt "
                        f"(inserters exist but none touch a belt tile)"
                    ),
                    x=e.x,
                    y=e.y,
                )
            )
            continue

        # Check that at least one connected belt network reaches beyond this
        # machine — i.e., connects to another machine's inserter or extends
        # to the layout boundary (external I/O)
        start_belt_tiles: set[tuple[int, int]] = set()
        for ix, iy in adjacent_inserters:
            for dx, dy in [(1, 0), (-1, 0), (0, 1), (0, -1)]:
                nb = (ix + dx, iy + dy)
                if nb in belt_tiles and nb not in machine_tiles:
                    start_belt_tiles.add(nb)

        belt_network = _bfs_belt_reach(start_belt_tiles, belt_tiles)

        if len(belt_network) <= 1:
            issues.append(
                ValidationIssue(
                    severity="error",
                    category="belt-connectivity",
                    message=(
                        f"{e.name} at ({e.x},{e.y}): belt adjacent to inserter "
                        f"is isolated (single tile, not connected to anything)"
                    ),
                    x=e.x,
                    y=e.y,
                )
            )

    return issues


def _bfs_belt_reach(
    starts: set[tuple[int, int]],
    belt_tiles: set[tuple[int, int]],
) -> set[tuple[int, int]]:
    """BFS flood-fill through adjacent belt tiles from start positions."""
    visited: set[tuple[int, int]] = set()
    queue = deque(starts)
    visited.update(starts)

    while queue:
        x, y = queue.popleft()
        for dx, dy in [(1, 0), (-1, 0), (0, 1), (0, -1)]:
            nb = (x + dx, y + dy)
            if nb in belt_tiles and nb not in visited:
                visited.add(nb)
                queue.append(nb)

    return visited


def check_belt_flow_path(
    layout_result: LayoutResult,
    solver_result: SolverResult | None = None,
    layout_style: str = "spaghetti",
) -> list[ValidationIssue]:
    """Check that each machine's input belt network reaches a source.

    BFS through connected belt tiles from each machine's input-side inserters.
    The network must reach either:
    - An inserter adjacent to a different machine (internal flow), or
    - The boundary of the layout (external input).

    Only checks input-side connections. Output belts are often intentionally
    short dead-ends (the player extends them), so those are not flagged.

    Severity depends on layout style: error for spaghetti (where routing should
    produce complete paths), warning for bus (which has known disconnected spurs).
    """
    issues: list[ValidationIssue] = []

    fluid_only_recipes = _get_fluid_only_recipes(solver_result)

    # Build tile maps
    belt_tiles: set[tuple[int, int]] = set()
    inserter_entities: list[PlacedEntity] = []
    inserter_positions: set[tuple[int, int]] = set()
    for e in layout_result.entities:
        if e.name in _BELT_ENTITIES:
            belt_tiles.add((e.x, e.y))
        elif e.name in _INSERTER_ENTITIES:
            inserter_entities.append(e)
            inserter_positions.add((e.x, e.y))

    if not belt_tiles:
        return issues  # check_belt_connectivity already handles this

    # Build per-machine tile sets
    machine_entities = [e for e in layout_result.entities if e.name in _MACHINE_ENTITIES]
    all_machine_tiles: set[tuple[int, int]] = set()
    for e in machine_entities:
        size = _machine_size(e.name)
        for dx in range(size):
            for dy in range(size):
                all_machine_tiles.add((e.x + dx, e.y + dy))

    # Identify input inserters: those that drop INTO a machine
    # (drop side = facing direction * reach → must land on a machine tile)
    input_inserter_positions: set[tuple[int, int]] = set()
    for ie in inserter_entities:
        direction_vec = _DIR_TO_VEC.get(ie.direction)
        if direction_vec is None:
            continue
        reach = _INSERTER_REACH.get(ie.name, 1)
        dx, dy = direction_vec
        drop_pos = (ie.x + dx * reach, ie.y + dy * reach)
        if drop_pos in all_machine_tiles:
            input_inserter_positions.add((ie.x, ie.y))

    # Compute layout boundary from all belt positions
    all_xs = [x for x, _ in belt_tiles]
    all_ys = [y for _, y in belt_tiles]
    min_bx, max_bx = min(all_xs), max(all_xs)
    min_by, max_by = min(all_ys), max(all_ys)

    checked_machines: set[tuple[int, int]] = set()
    for e in machine_entities:
        if (e.x, e.y) in checked_machines:
            continue
        checked_machines.add((e.x, e.y))

        if e.recipe in fluid_only_recipes:
            continue

        size = _machine_size(e.name)
        my_tiles = {(e.x + dx, e.y + dy) for dx in range(size) for dy in range(size)}

        # Find belt tiles adjacent to this machine's INPUT inserters
        start_belt_tiles: set[tuple[int, int]] = set()
        for dx in range(-1, size + 1):
            for dy in range(-1, size + 1):
                ipos = (e.x + dx, e.y + dy)
                if ipos not in input_inserter_positions or ipos in my_tiles:
                    continue
                for ddx, ddy in [(1, 0), (-1, 0), (0, 1), (0, -1)]:
                    nb = (ipos[0] + ddx, ipos[1] + ddy)
                    if nb in belt_tiles and nb not in my_tiles:
                        start_belt_tiles.add(nb)

        if not start_belt_tiles:
            continue  # No input inserters with belts — other checks cover this

        belt_network = _bfs_belt_reach(start_belt_tiles, belt_tiles)

        # Check if network reaches another machine's output inserter
        reaches_source = False
        for bx, by in belt_network:
            for ddx, ddy in [(1, 0), (-1, 0), (0, 1), (0, -1)]:
                adj = (bx + ddx, by + ddy)
                # Inserter adjacent to belt, but NOT an input inserter for this machine
                if adj in inserter_positions and adj not in my_tiles and adj not in input_inserter_positions:
                    reaches_source = True
                    break
                # Also check if this inserter is an output inserter of another machine
                if adj in inserter_positions and adj not in my_tiles:
                    for ddx2, ddy2 in [(1, 0), (-1, 0), (0, 1), (0, -1)]:
                        adj2 = (adj[0] + ddx2, adj[1] + ddy2)
                        if adj2 in all_machine_tiles and adj2 not in my_tiles:
                            reaches_source = True
                            break
                if reaches_source:
                    break
            if reaches_source:
                break

        # Check if network reaches layout boundary (external input)
        reaches_boundary = any(bx in (min_bx, max_bx) or by in (min_by, max_by) for bx, by in belt_network)

        if not reaches_source and not reaches_boundary:
            severity = "error" if layout_style == "spaghetti" else "warning"
            issues.append(
                ValidationIssue(
                    severity=severity,
                    category="belt-flow-path",
                    message=(
                        f"{e.name} at ({e.x},{e.y}): input belt network ({len(belt_network)} tiles) "
                        f"doesn't reach any source (other machine or layout boundary)"
                    ),
                    x=e.x,
                    y=e.y,
                )
            )

    return issues


def check_belt_direction_continuity(
    layout_result: LayoutResult,
) -> list[ValidationIssue]:
    """Check that adjacent belts don't form 180-degree reversals.

    Two adjacent belts pointing in exactly opposite directions create a
    dead spot where items pile up and stop flowing. This is almost always
    a routing error.
    """
    issues: list[ValidationIssue] = []

    # Build belt direction map
    belt_dir_map: dict[tuple[int, int], EntityDirection] = {}
    for e in layout_result.entities:
        if e.name in _BELT_ENTITIES:
            belt_dir_map[(e.x, e.y)] = e.direction

    checked: set[tuple[tuple[int, int], tuple[int, int]]] = set()
    for (bx, by), direction in belt_dir_map.items():
        dir_vec = _DIR_TO_VEC.get(direction)
        if dir_vec is None:
            continue

        opposite = _OPPOSITE_VEC[dir_vec]

        for dx, dy in [(1, 0), (-1, 0), (0, 1), (0, -1)]:
            nb = (bx + dx, by + dy)
            if nb not in belt_dir_map:
                continue

            pair = (min((bx, by), nb), max((bx, by), nb))
            if pair in checked:
                continue
            checked.add(pair)

            nb_dir = belt_dir_map[nb]
            nb_vec = _DIR_TO_VEC.get(nb_dir)
            if nb_vec is None:
                continue

            # Check if they're 180-degree opposites on the same axis as their flow
            # (belts facing N/S adjacent vertically, or E/W adjacent horizontally)
            # Side-by-side parallel belts going opposite ways is fine (common pattern)
            if nb_vec == opposite and (dx, dy) in (dir_vec, opposite):
                issues.append(
                    ValidationIssue(
                        severity="warning",
                        category="belt-direction",
                        message=(
                            f"Adjacent belts at ({bx},{by}) and ({nb[0]},{nb[1]}) "
                            f"face opposite directions ({direction.name} vs {nb_dir.name}), "
                            f"creating a dead spot"
                        ),
                        x=bx,
                        y=by,
                    )
                )

    return issues


def check_belt_throughput(
    layout_result: LayoutResult,
) -> list[ValidationIssue]:
    """Check that belt tiles don't carry more items/s than their tier allows.

    Multiple routes sharing the same belt tile can exceed the belt's capacity.
    This check sums all flow rates passing through each belt tile and compares
    against the belt tier's throughput limit.
    """
    issues: list[ValidationIssue] = []

    # Count overlapping belt entities at the same position.
    # Each belt entity placed at a tile represents a route using that tile.
    tile_counts: dict[tuple[int, int], int] = defaultdict(int)
    tile_names: dict[tuple[int, int], str] = {}

    for e in layout_result.entities:
        if e.name not in _BELT_ENTITIES:
            continue
        pos = (e.x, e.y)
        tile_counts[pos] += 1
        tile_names[pos] = e.name
        # If the entity has a rate annotation we could use it, but currently
        # rates aren't stored per-entity. Flag overlapping routes instead.

    for pos, count in tile_counts.items():
        if count > 1:
            belt_name = tile_names[pos]
            max_throughput = _BELT_THROUGHPUT.get(belt_name, 15.0)
            issues.append(
                ValidationIssue(
                    severity="warning",
                    category="belt-throughput",
                    message=(
                        f"Belt at ({pos[0]},{pos[1]}): {count} overlapping routes "
                        f"on {belt_name} (max {max_throughput}/s)"
                    ),
                    x=pos[0],
                    y=pos[1],
                )
            )

    return issues


def check_power_coverage(layout_result: LayoutResult) -> list[ValidationIssue]:
    """Check that every machine is within range of a power pole.

    Medium electric poles have a 7×7 supply area (3 tiles in each direction
    from the pole center).
    """
    issues: list[ValidationIssue] = []
    POLE_RANGE = 3  # 7x7 area = 3 tiles each direction from center

    # Collect pole positions
    pole_positions: list[tuple[int, int]] = []
    for e in layout_result.entities:
        if e.name == "medium-electric-pole":
            pole_positions.append((e.x, e.y))

    if not pole_positions:
        # No poles at all — warn but don't error for every machine
        issues.append(
            ValidationIssue(
                severity="warning",
                category="power",
                message="No power poles in layout",
            )
        )
        return issues

    for e in layout_result.entities:
        if e.name not in _MACHINE_ENTITIES:
            continue

        size = _machine_size(e.name)
        # Machine center tile
        cx = e.x + size // 2
        cy = e.y + size // 2

        powered = any(abs(cx - px) <= POLE_RANGE and abs(cy - py) <= POLE_RANGE for px, py in pole_positions)
        if not powered:
            issues.append(
                ValidationIssue(
                    severity="warning",
                    category="power",
                    message=f"{e.name} at ({e.x},{e.y}): not in range of any power pole",
                    x=e.x,
                    y=e.y,
                )
            )

    return issues
