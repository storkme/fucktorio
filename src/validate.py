"""Functional blueprint validation: checks that layouts actually work in Factorio."""

from __future__ import annotations

from collections import defaultdict, deque
from dataclasses import dataclass

from .models import EntityDirection, LayoutResult, MachineSpec, PlacedEntity, SolverResult
from .routing.common import _MACHINE_SIZE, _UG_MAX_REACH, inserter_target_lane

# Derive machine entity sets from the single source of truth in routing.common.
_MACHINE_ENTITIES = set(_MACHINE_SIZE.keys())
_3x3_ENTITIES = {k for k, v in _MACHINE_SIZE.items() if v == 3}
_5x5_ENTITIES = {k for k, v in _MACHINE_SIZE.items() if v == 5}
_PIPE_ENTITIES = {"pipe", "pipe-to-ground"}
_SURFACE_BELT_ENTITIES = {"transport-belt", "fast-transport-belt", "express-transport-belt"}
_UG_BELT_ENTITIES = {"underground-belt", "fast-underground-belt", "express-underground-belt"}
_SPLITTER_ENTITIES = {"splitter", "fast-splitter", "express-splitter"}
_BELT_ENTITIES = _SURFACE_BELT_ENTITIES | _UG_BELT_ENTITIES | _SPLITTER_ENTITIES

# Map underground belt entity name to the corresponding surface belt tier
_UG_TO_SURFACE_TIER: dict[str, str] = {
    "underground-belt": "transport-belt",
    "fast-underground-belt": "fast-transport-belt",
    "express-underground-belt": "express-transport-belt",
}

_INSERTER_ENTITIES = {"inserter", "long-handed-inserter", "fast-inserter", "stack-inserter"}


def _belt_dir_map_from(entities: list[PlacedEntity]) -> dict[tuple[int, int], EntityDirection]:
    """Build belt direction map, expanding splitters to both occupied tiles."""
    bdm: dict[tuple[int, int], EntityDirection] = {}
    for e in entities:
        if e.name not in _BELT_ENTITIES:
            continue
        bdm[(e.x, e.y)] = e.direction
        if e.name in _SPLITTER_ENTITIES:
            # Splitters occupy 2 tiles perpendicular to their direction
            if e.direction in (EntityDirection.NORTH, EntityDirection.SOUTH):
                bdm[(e.x + 1, e.y)] = e.direction
            else:
                bdm[(e.x, e.y + 1)] = e.direction
    return bdm

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
    "underground-belt": 15.0,
}

# Per-lane capacity (half of total belt throughput)
_LANE_CAPACITY = {
    "transport-belt": 7.5,
    "fast-transport-belt": 15.0,
    "express-transport-belt": 22.5,
    "underground-belt": 7.5,
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
    layout_style: str = "spaghetti",
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
    issues.extend(check_output_belt_coverage(layout_result, solver_result))
    if layout_style == "spaghetti":
        issues.extend(check_belt_network_topology(layout_result, solver_result))
    issues.extend(check_belt_junctions(layout_result))
    issues.extend(check_underground_belt_pairs(layout_result))
    issues.extend(check_underground_belt_sideloading(layout_result))
    issues.extend(check_belt_loops(layout_result))
    issues.extend(check_belt_item_isolation(layout_result))
    issues.extend(check_belt_inserter_conflict(layout_result))
    issues.extend(check_belt_flow_reachability(layout_result, solver_result, layout_style=layout_style))
    issues.extend(check_lane_throughput(layout_result, solver_result))
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
    layout_style: str = "spaghetti",
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

    In Factorio, pipe-to-ground entry and exit both face the SAME direction
    (the direction of flow), distinguished by io_type ("input"/"output").
    Entry at position A facing EAST pairs with the nearest output facing
    EAST to its right on the same y.

    Returns {pos_a: pos_b, pos_b: pos_a} for each pair.
    """
    from .models import EntityDirection

    pairs: dict[tuple[int, int], tuple[int, int]] = {}
    ptg_entities = [e for e in layout_result.entities if e.name == "pipe-to-ground"]

    # Group by (direction, axis_value): EAST/WEST share y, SOUTH/NORTH share x
    groups: dict[tuple[int, int], list] = {}
    for e in ptg_entities:
        key = (e.direction, e.y) if e.direction in (EntityDirection.EAST, EntityDirection.WEST) else (e.direction, e.x)
        groups.setdefault(key, []).append(e)

    for (_dir, _axis), group in groups.items():
        inputs = sorted(
            [e for e in group if e.io_type == "input"],
            key=lambda e: (e.x, e.y),
        )
        outputs = sorted(
            [e for e in group if e.io_type == "output"],
            key=lambda e: (e.x, e.y),
        )

        # Pair each input with the nearest output in the flow direction
        remaining_outputs = list(outputs)
        for inp in inputs:
            for out in remaining_outputs:
                # Output must be "ahead" of input in the flow direction
                ahead = (
                    (_dir == EntityDirection.EAST and out.x > inp.x)
                    or (_dir == EntityDirection.WEST and out.x < inp.x)
                    or (_dir == EntityDirection.SOUTH and out.y > inp.y)
                    or (_dir == EntityDirection.NORTH and out.y < inp.y)
                )
                if not ahead:
                    continue

                a, b = (inp.x, inp.y), (out.x, out.y)
                pairs[a] = b
                pairs[b] = a
                remaining_outputs.remove(out)
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

    # Build inserter positions by reach category
    short_inserter_positions: set[tuple[int, int]] = set()
    long_inserter_positions: set[tuple[int, int]] = set()
    for e in layout_result.entities:
        if e.name in _INSERTER_ENTITIES:
            if e.name == "long-handed-inserter":
                long_inserter_positions.add((e.x, e.y))
            else:
                short_inserter_positions.add((e.x, e.y))

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
        # Check for short-reach inserters (1 tile from border)
        for dx in range(-1, size + 1):
            for dy in range(-1, size + 1):
                if (e.x + dx, e.y + dy) in short_inserter_positions:
                    has_inserter = True
                    break
            if has_inserter:
                break
        # Check for long-handed inserters (2 tiles from border)
        if not has_inserter:
            for dx in range(-2, size + 2):
                for dy in range(-2, size + 2):
                    if (e.x + dx, e.y + dy) in long_inserter_positions:
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
    ug_pairs: dict[tuple[int, int], tuple[int, int]] | None = None,
) -> set[tuple[int, int]]:
    """BFS flood-fill through adjacent belt tiles from start positions.

    Traverses underground belt tunnels via ug_pairs mapping.
    """
    visited: set[tuple[int, int]] = set()
    queue = deque(starts)
    visited.update(starts)

    while queue:
        x, y = queue.popleft()
        neighbors: list[tuple[int, int]] = [(x + dx, y + dy) for dx, dy in [(1, 0), (-1, 0), (0, 1), (0, -1)]]
        if ug_pairs and (x, y) in ug_pairs:
            neighbors.append(ug_pairs[(x, y)])
        for nb in neighbors:
            if nb in belt_tiles and nb not in visited:
                visited.add(nb)
                queue.append(nb)

    return visited


def _build_ug_pairs(
    layout_result: LayoutResult,
) -> dict[tuple[int, int], tuple[int, int]]:
    """Build underground belt pair map: entry↔exit.

    Scans underground-belt entities and matches input/output pairs
    by direction and alignment. Returns bidirectional mapping.
    """
    # Group by direction and axis
    ug_inputs: list[PlacedEntity] = []
    ug_outputs: list[PlacedEntity] = []
    for e in layout_result.entities:
        if e.name in _UG_BELT_ENTITIES:
            if e.io_type == "input":
                ug_inputs.append(e)
            elif e.io_type == "output":
                ug_outputs.append(e)

    pairs: dict[tuple[int, int], tuple[int, int]] = {}
    used_outputs: set[tuple[int, int]] = set()

    for inp in ug_inputs:
        d = _DIR_TO_VEC.get(inp.direction)
        if d is None:
            continue
        dx, dy = d
        # Find nearest matching output in the same direction along the line
        best_out = None
        best_dist = float("inf")
        for out in ug_outputs:
            if (out.x, out.y) in used_outputs:
                continue
            if out.direction != inp.direction:
                continue
            # Must be along the direction line
            rx, ry = out.x - inp.x, out.y - inp.y
            if dx != 0:
                if ry != 0 or (rx > 0) != (dx > 0):
                    continue
                dist = abs(rx)
            else:
                if rx != 0 or (ry > 0) != (dy > 0):
                    continue
                dist = abs(ry)
            if 1 < dist < best_dist:
                best_dist = dist
                best_out = out

        if best_out is not None:
            pairs[(inp.x, inp.y)] = (best_out.x, best_out.y)
            pairs[(best_out.x, best_out.y)] = (inp.x, inp.y)
            used_outputs.add((best_out.x, best_out.y))

    return pairs


def _build_splitter_siblings(
    layout_result: LayoutResult,
) -> dict[tuple[int, int], tuple[int, int]]:
    """Map each splitter tile to its sibling tile.

    A splitter occupies 2 tiles perpendicular to its direction. This map
    lets BFS traverse between both sides of a splitter, which is needed
    because items entering either input can exit either output.
    """
    siblings: dict[tuple[int, int], tuple[int, int]] = {}
    for e in layout_result.entities:
        if e.name not in _SPLITTER_ENTITIES:
            continue
        if e.direction in (EntityDirection.NORTH, EntityDirection.SOUTH):
            siblings[(e.x, e.y)] = (e.x + 1, e.y)
            siblings[(e.x + 1, e.y)] = (e.x, e.y)
        else:
            siblings[(e.x, e.y)] = (e.x, e.y + 1)
            siblings[(e.x, e.y + 1)] = (e.x, e.y)
    return siblings


def _bfs_belt_downstream(
    starts: set[tuple[int, int]],
    belt_dir_map: dict[tuple[int, int], EntityDirection],
    ug_pairs: dict[tuple[int, int], tuple[int, int]] | None = None,
    splitter_siblings: dict[tuple[int, int], tuple[int, int]] | None = None,
) -> set[tuple[int, int]]:
    """BFS following belt directions forward — where do items end up?

    Traverses underground belt tunnels via ug_pairs mapping.
    Traverses splitter siblings so items entering either side can exit either side.
    """
    visited: set[tuple[int, int]] = set()
    queue = deque(starts & belt_dir_map.keys())
    visited.update(queue)

    while queue:
        x, y = queue.popleft()
        d = belt_dir_map.get((x, y))
        if d is None:
            continue
        dx, dy = _DIR_TO_VEC[d]

        # Check for underground tunnel jump
        if ug_pairs and (x, y) in ug_pairs:
            paired = ug_pairs[(x, y)]
            if paired not in visited:
                visited.add(paired)
                queue.append(paired)

        # Splitter sibling: items can cross to the other side
        if splitter_siblings and (x, y) in splitter_siblings:
            sib = splitter_siblings[(x, y)]
            if sib not in visited:
                visited.add(sib)
                queue.append(sib)

        nb = (x + dx, y + dy)
        if nb in belt_dir_map and nb not in visited:
            visited.add(nb)
            queue.append(nb)

    return visited


def _bfs_belt_upstream(
    starts: set[tuple[int, int]],
    belt_dir_map: dict[tuple[int, int], EntityDirection],
    ug_pairs: dict[tuple[int, int], tuple[int, int]] | None = None,
    splitter_siblings: dict[tuple[int, int], tuple[int, int]] | None = None,
) -> set[tuple[int, int]]:
    """BFS tracing backward against belt flow — where can items come from?

    A neighbor (nx,ny) feeds into (x,y) if the neighbor's direction
    points at (x,y): (nx + ndx, ny + ndy) == (x, y).
    Traverses underground belt tunnels via ug_pairs mapping.
    Traverses splitter siblings so items on either side are reachable.
    """
    visited: set[tuple[int, int]] = set()
    queue = deque(starts & belt_dir_map.keys())
    visited.update(queue)

    while queue:
        x, y = queue.popleft()

        # Check for underground tunnel jump (reverse)
        if ug_pairs and (x, y) in ug_pairs:
            paired = ug_pairs[(x, y)]
            if paired not in visited:
                visited.add(paired)
                queue.append(paired)

        # Splitter sibling: items can cross from either side
        if splitter_siblings and (x, y) in splitter_siblings:
            sib = splitter_siblings[(x, y)]
            if sib not in visited:
                visited.add(sib)
                queue.append(sib)

        for ddx, ddy in [(1, 0), (-1, 0), (0, 1), (0, -1)]:
            nx, ny = x + ddx, y + ddy
            nd = belt_dir_map.get((nx, ny))
            if nd is None or (nx, ny) in visited:
                continue
            ndx, ndy = _DIR_TO_VEC[nd]
            if (nx + ndx, ny + ndy) == (x, y):
                visited.add((nx, ny))
                queue.append((nx, ny))

    return visited


def check_belt_flow_path(
    layout_result: LayoutResult,
    solver_result: SolverResult | None = None,
    layout_style: str = "spaghetti",
) -> list[ValidationIssue]:
    """Check that each machine's belt networks reach valid sources/sinks.

    For inputs: BFS through connected belt tiles from each machine's input-side
    inserters. The network must reach either an inserter adjacent to a different
    machine (internal flow) or the layout boundary (external input).

    For outputs: BFS from each machine's output-side inserters. The network must
    reach either another machine's input inserter or the layout boundary.

    Boundary checks require the network to have at least 3 tiles to avoid
    falsely passing dead stubs that happen to sit at the layout edge.

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

    # Classify inserters in a single pass: input (drops into machine) vs output (picks from machine)
    input_inserter_positions: set[tuple[int, int]] = set()
    output_inserter_positions: set[tuple[int, int]] = set()
    for ie in inserter_entities:
        direction_vec = _DIR_TO_VEC.get(ie.direction)
        if direction_vec is None:
            continue
        reach = _INSERTER_REACH.get(ie.name, 1)
        dx, dy = direction_vec
        drop_pos = (ie.x + dx * reach, ie.y + dy * reach)
        pickup_pos = (ie.x - dx * reach, ie.y - dy * reach)
        if drop_pos in all_machine_tiles:
            input_inserter_positions.add((ie.x, ie.y))
        if pickup_pos in all_machine_tiles:
            output_inserter_positions.add((ie.x, ie.y))

    # Compute layout boundary from all belt positions
    all_xs = [x for x, _ in belt_tiles]
    all_ys = [y for _, y in belt_tiles]
    min_bx, max_bx = min(all_xs), max(all_xs)
    min_by, max_by = min(all_ys), max(all_ys)

    def _belt_tiles_near_inserters(
        machine_entity: PlacedEntity,
        machine_tiles: set[tuple[int, int]],
        target_inserter_positions: set[tuple[int, int]],
    ) -> set[tuple[int, int]]:
        """Find belt tiles adjacent to a machine's inserters of a given type."""
        size = _machine_size(machine_entity.name)
        result: set[tuple[int, int]] = set()
        for dx in range(-1, size + 1):
            for dy in range(-1, size + 1):
                ipos = (machine_entity.x + dx, machine_entity.y + dy)
                if ipos not in target_inserter_positions or ipos in machine_tiles:
                    continue
                for ddx, ddy in [(1, 0), (-1, 0), (0, 1), (0, -1)]:
                    nb = (ipos[0] + ddx, ipos[1] + ddy)
                    if nb in belt_tiles and nb not in machine_tiles:
                        result.add(nb)
        return result

    def _network_reaches_boundary(network: set[tuple[int, int]]) -> bool:
        """Check if a belt network reaches the layout boundary.

        Requires at least 3 tiles to avoid falsely passing dead stubs
        that happen to sit at the layout edge.
        """
        return len(network) >= 3 and any(bx in (min_bx, max_bx) or by in (min_by, max_by) for bx, by in network)

    # Build recipe -> has_solid_output lookup for output checks
    solid_output_recipes: set[str] = set()
    if solver_result:
        for ms in solver_result.machines:
            if any(not o.is_fluid for o in ms.outputs):
                solid_output_recipes.add(ms.recipe)

    checked_machines: set[tuple[int, int]] = set()
    for e in machine_entities:
        if (e.x, e.y) in checked_machines:
            continue
        checked_machines.add((e.x, e.y))

        if e.recipe in fluid_only_recipes:
            continue

        size = _machine_size(e.name)
        my_tiles = {(e.x + dx, e.y + dy) for dx in range(size) for dy in range(size)}

        # --- Input path checking ---
        input_belt_starts = _belt_tiles_near_inserters(e, my_tiles, input_inserter_positions)
        if input_belt_starts:
            belt_network = _bfs_belt_reach(input_belt_starts, belt_tiles)

            # Check if network reaches another machine's output inserter
            reaches_source = False
            for bx, by in belt_network:
                for ddx, ddy in [(1, 0), (-1, 0), (0, 1), (0, -1)]:
                    adj = (bx + ddx, by + ddy)
                    if adj in inserter_positions and adj not in my_tiles and adj not in input_inserter_positions:
                        reaches_source = True
                        break
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

            if not reaches_source and not _network_reaches_boundary(belt_network):
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

        # --- Output path checking ---
        has_solid_output = e.recipe in solid_output_recipes if solver_result else True
        if not has_solid_output:
            continue

        output_belt_starts = _belt_tiles_near_inserters(e, my_tiles, output_inserter_positions)
        if not output_belt_starts:
            continue

        belt_network = _bfs_belt_reach(output_belt_starts, belt_tiles)

        reaches_sink = False
        for bx, by in belt_network:
            for ddx, ddy in [(1, 0), (-1, 0), (0, 1), (0, -1)]:
                adj = (bx + ddx, by + ddy)
                if adj in input_inserter_positions and adj not in my_tiles:
                    reaches_sink = True
                    break
            if reaches_sink:
                break

        if not reaches_sink and not _network_reaches_boundary(belt_network):
            severity = "error" if layout_style == "spaghetti" else "warning"
            issues.append(
                ValidationIssue(
                    severity=severity,
                    category="belt-flow-path",
                    message=(
                        f"{e.name} at ({e.x},{e.y}): output belt network ({len(belt_network)} tiles) "
                        f"doesn't reach any sink (other machine or layout boundary)"
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
    belt_dir_map = _belt_dir_map_from(layout_result.entities)

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


def check_output_belt_coverage(
    layout_result: LayoutResult,
    solver_result: SolverResult | None = None,
) -> list[ValidationIssue]:
    """Check that every machine with solid outputs has an output inserter with a belt.

    An output inserter picks FROM the machine and drops onto a belt. This check
    verifies that the drop side of at least one such inserter has a belt entity,
    ensuring products can actually leave the machine.

    Machines with only fluid outputs are skipped (they use pipes).
    """
    issues: list[ValidationIssue] = []

    # Identify recipes whose outputs are entirely fluid
    fluid_output_recipes: set[str] = set()
    if solver_result is not None:
        for spec in solver_result.machines:
            has_solid_output = any(not f.is_fluid for f in spec.outputs)
            if not has_solid_output:
                fluid_output_recipes.add(spec.recipe)

    machine_tiles = _build_machine_tile_set(layout_result)
    belt_tiles: set[tuple[int, int]] = set()
    for e in layout_result.entities:
        if e.name in _BELT_ENTITIES:
            belt_tiles.add((e.x, e.y))

    checked: set[tuple[int, int]] = set()
    for e in layout_result.entities:
        if e.name not in _MACHINE_ENTITIES:
            continue
        if (e.x, e.y) in checked:
            continue
        checked.add((e.x, e.y))

        if e.recipe in fluid_output_recipes:
            continue

        size = _machine_size(e.name)
        my_tiles = set()
        for dx in range(size):
            for dy in range(size):
                my_tiles.add((e.x + dx, e.y + dy))

        # Find output inserters: inserters whose pickup side touches this machine
        has_output_belt = False
        for ins in layout_result.entities:
            if ins.name not in _INSERTER_ENTITIES:
                continue
            direction_vec = _DIR_TO_VEC.get(ins.direction)
            if direction_vec is None:
                continue

            reach = _INSERTER_REACH.get(ins.name, 1)
            dx, dy = direction_vec
            odx, ody = _OPPOSITE_VEC[direction_vec]

            pickup_pos = (ins.x + odx * reach, ins.y + ody * reach)
            drop_pos = (ins.x + dx * reach, ins.y + dy * reach)

            if pickup_pos in my_tiles and drop_pos not in machine_tiles and drop_pos in belt_tiles:
                has_output_belt = True
                break

        if not has_output_belt:
            issues.append(
                ValidationIssue(
                    severity="error",
                    category="output-belt",
                    message=(f"{e.name} at ({e.x},{e.y}): no output inserter has a belt at its drop position"),
                    x=e.x,
                    y=e.y,
                )
            )

    return issues


def check_belt_network_topology(
    layout_result: LayoutResult,
    solver_result: SolverResult | None = None,
) -> list[ValidationIssue]:
    """Check that belt networks form valid connected topologies.

    Three checks:
    1. Shared input networks: all machines consuming the same external input
       must be reachable from a single connected belt network.
    2. Output reaches boundary: output belt networks must extend to the layout
       boundary (not just be a single dead-end stub).
    3. Shared output networks: all machines producing the same external output
       must have their output belts connected into a single network that reaches
       the layout boundary.
    """
    issues: list[ValidationIssue] = []
    if solver_result is None:
        return issues

    # Build tile maps
    belt_tiles: set[tuple[int, int]] = set()
    belt_carries: dict[tuple[int, int], str | None] = {}
    for e in layout_result.entities:
        if e.name in _BELT_ENTITIES:
            belt_tiles.add((e.x, e.y))
            belt_carries[(e.x, e.y)] = e.carries

    if not belt_tiles:
        return issues

    machine_tiles = _build_machine_tile_set(layout_result)

    # Identify inserters and classify as input/output
    input_inserter_belt_tiles: dict[tuple[int, int], list[tuple[int, int]]] = {}  # machine_pos → belt tiles
    output_inserter_belt_tiles: dict[tuple[int, int], list[tuple[int, int]]] = {}
    machine_positions: dict[tuple[int, int], PlacedEntity] = {}
    for e in layout_result.entities:
        if e.name in _MACHINE_ENTITIES:
            machine_positions[(e.x, e.y)] = e

    # Build per-machine tile lookup
    machine_by_tile: dict[tuple[int, int], tuple[int, int]] = {}
    for e in layout_result.entities:
        if e.name in _MACHINE_ENTITIES:
            size = _machine_size(e.name)
            for dx in range(size):
                for dy in range(size):
                    machine_by_tile[(e.x + dx, e.y + dy)] = (e.x, e.y)

    for ins in layout_result.entities:
        if ins.name not in _INSERTER_ENTITIES:
            continue
        direction_vec = _DIR_TO_VEC.get(ins.direction)
        if direction_vec is None:
            continue
        reach = _INSERTER_REACH.get(ins.name, 1)
        dx, dy = direction_vec
        odx, ody = _OPPOSITE_VEC[direction_vec]
        drop_pos = (ins.x + dx * reach, ins.y + dy * reach)
        pickup_pos = (ins.x + odx * reach, ins.y + ody * reach)

        if drop_pos in machine_tiles and pickup_pos in belt_tiles:
            mpos = machine_by_tile.get(drop_pos)
            if mpos:
                input_inserter_belt_tiles.setdefault(mpos, []).append(pickup_pos)
        elif pickup_pos in machine_tiles and drop_pos in belt_tiles:
            mpos = machine_by_tile.get(pickup_pos)
            if mpos:
                output_inserter_belt_tiles.setdefault(mpos, []).append(drop_pos)

    ug_pairs = _build_ug_pairs(layout_result)

    # Layout boundary
    all_xs = [x for x, _ in belt_tiles]
    all_ys = [y for _, y in belt_tiles]
    min_bx, max_bx = min(all_xs), max(all_xs)
    min_by, max_by = min(all_ys), max(all_ys)

    def _on_boundary(pos: tuple[int, int]) -> bool:
        return pos[0] in (min_bx, max_bx) or pos[1] in (min_by, max_by)

    # Group machines by recipe
    recipe_machines: dict[str, list[tuple[int, int]]] = {}
    for pos, e in machine_positions.items():
        recipe_machines.setdefault(e.recipe, []).append(pos)

    # Get external input/output item names
    external_input_items = {f.item for f in solver_result.external_inputs if not f.is_fluid}
    external_output_items = {f.item for f in solver_result.external_outputs if not f.is_fluid}

    # Map: item → which recipes consume it (external inputs only)
    item_to_consumer_recipes: dict[str, set[str]] = {}
    for spec in solver_result.machines:
        for inp in spec.inputs:
            if inp.item in external_input_items and not inp.is_fluid:
                item_to_consumer_recipes.setdefault(inp.item, set()).add(spec.recipe)

    # Map: item → which recipes produce it (external outputs only)
    item_to_producer_recipes: dict[str, set[str]] = {}
    for spec in solver_result.machines:
        for out in spec.outputs:
            if out.item in external_output_items and not out.is_fluid:
                item_to_producer_recipes.setdefault(out.item, set()).add(spec.recipe)

    def _check_network(
        item: str,
        direction: str,
        belt_starts: list[tuple[int, int]],
        machine_list: list[tuple[int, int]],
    ) -> None:
        """Validate a belt network for an external item (input or output).

        Checks:
        1. All belt_starts are on one connected network.
        2. The network reaches the layout boundary.
        3. The boundary tiles form a contiguous segment on a single edge.
        """
        if not belt_starts:
            return

        # Filter belt tiles to only those carrying this item (prevents
        # BFS from leaking into adjacent networks for different items)
        item_belt_tiles = {pos for pos, carry in belt_carries.items() if carry == item}

        # BFS the full network from all start tiles
        full_network = _bfs_belt_reach(set(belt_starts), item_belt_tiles, ug_pairs)

        # Check connectivity: BFS from just the first start should reach all others
        if len(belt_starts) > 1:
            first_network = _bfs_belt_reach({belt_starts[0]}, item_belt_tiles, ug_pairs)
            unreachable = [bt for bt in belt_starts[1:] if bt not in first_network]
            if unreachable:
                issues.append(
                    ValidationIssue(
                        severity="error",
                        category="belt-topology",
                        message=(
                            f"{item} {direction}: {len(unreachable) + 1} disconnected "
                            f"belt networks for {len(machine_list)} machines "
                            f"(should be a single connected network)"
                        ),
                    )
                )
                return  # skip further checks if not even connected

        # Find boundary tiles in the network
        boundary_tiles = [t for t in full_network if _on_boundary(t)]
        if not boundary_tiles:
            issues.append(
                ValidationIssue(
                    severity="error",
                    category="belt-topology",
                    message=(
                        f"{item} {direction}: belt network ({len(full_network)} tiles) doesn't reach layout boundary"
                    ),
                )
            )
            return

        # Check boundary tiles form one contiguous group (adjacency flood-fill)
        boundary_set = set(boundary_tiles)
        bfs_visited: set[tuple[int, int]] = set()
        bfs_queue = deque([boundary_tiles[0]])
        bfs_visited.add(boundary_tiles[0])
        while bfs_queue:
            bx, by = bfs_queue.popleft()
            for ddx, ddy in [(1, 0), (-1, 0), (0, 1), (0, -1)]:
                nb = (bx + ddx, by + ddy)
                if nb in boundary_set and nb not in bfs_visited:
                    bfs_visited.add(nb)
                    bfs_queue.append(nb)
        if len(bfs_visited) < len(boundary_set):
            issues.append(
                ValidationIssue(
                    severity="warning",
                    category="belt-topology",
                    message=(
                        f"{item} {direction}: belt network reaches layout boundary "
                        f"at multiple separate locations (ideally one contiguous "
                        f"entry/exit point)"
                    ),
                )
            )

    # CHECK: Input networks
    for item, recipes in item_to_consumer_recipes.items():
        input_belt_starts: list[tuple[int, int]] = []
        consuming_machines: list[tuple[int, int]] = []
        for recipe in recipes:
            for mpos in recipe_machines.get(recipe, []):
                if mpos in input_inserter_belt_tiles:
                    matched = [p for p in input_inserter_belt_tiles[mpos] if belt_carries.get(p) == item]
                    if matched:
                        input_belt_starts.extend(matched)
                        consuming_machines.append(mpos)
        _check_network(item, "input", input_belt_starts, consuming_machines)

    # CHECK: Output networks
    for item, recipes in item_to_producer_recipes.items():
        output_belt_starts: list[tuple[int, int]] = []
        producing_machines: list[tuple[int, int]] = []
        for recipe in recipes:
            for mpos in recipe_machines.get(recipe, []):
                if mpos in output_inserter_belt_tiles:
                    matched = [p for p in output_inserter_belt_tiles[mpos] if belt_carries.get(p) == item]
                    if matched:
                        output_belt_starts.extend(matched)
                        producing_machines.append(mpos)
        _check_network(item, "output", output_belt_starts, producing_machines)

    return issues


def check_belt_junctions(
    layout_result: LayoutResult,
) -> list[ValidationIssue]:
    """Check that belt T-junctions use valid sideloading geometry.

    At a T-junction, a branch belt feeding into a trunk belt must be
    perpendicular to the trunk. Same-direction feeding from the side
    (parallel merge) doesn't work in Factorio — items back up.

    A feeder belt at position (nx, ny) "feeds into" trunk at (x, y) if
    the feeder's direction vector points from (nx, ny) to (x, y).
    """
    issues: list[ValidationIssue] = []

    belt_dir: dict[tuple[int, int], EntityDirection] = {}
    belt_carry: dict[tuple[int, int], str | None] = {}
    for e in layout_result.entities:
        if e.name in _BELT_ENTITIES:
            belt_dir[(e.x, e.y)] = e.direction
            belt_carry[(e.x, e.y)] = e.carries

    for (x, y), direction in belt_dir.items():
        dx, dy = _DIR_TO_VEC[direction]

        # Find feeders: adjacent belts whose direction points at (x, y)
        for nx, ny in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)]:
            if (nx, ny) not in belt_dir:
                continue
            # Only check same-item feeders
            if belt_carry.get((nx, ny)) != belt_carry.get((x, y)):
                continue

            nd = belt_dir[(nx, ny)]
            ndx, ndy = _DIR_TO_VEC[nd]
            # Does this neighbor point at (x, y)?
            if (nx + ndx, ny + ndy) != (x, y):
                continue

            # This neighbor feeds into (x, y). Check if perpendicular.
            is_perpendicular = (ndx * dx + ndy * dy) == 0
            is_from_behind = ndx == dx and ndy == dy  # same direction

            if is_from_behind:
                # Same direction feeding from behind — this is just a straight
                # belt continuation, which is fine
                continue

            if not is_perpendicular:
                # Head-on collision: belt pointing opposite to the trunk
                is_head_on = ndx == -dx and ndy == -dy
                issues.append(
                    ValidationIssue(
                        severity="error" if is_head_on else "warning",
                        category="belt-junction",
                        message=(
                            f"Belt at ({nx},{ny}) feeds HEAD-ON into ({x},{y})"
                            if is_head_on
                            else f"Belt at ({nx},{ny}) feeds into ({x},{y}) from an invalid angle (not perpendicular)"
                        ),
                        x=x,
                        y=y,
                    )
                )

    return issues


def check_underground_belt_pairs(
    layout_result: LayoutResult,
) -> list[ValidationIssue]:
    """Check underground belt pairing: every input has a matching output.

    Validates:
    - Each UG input has a matching output (same direction, same axis)
    - Distance between pairs does not exceed max reach for the tier
    - No intermediate UG belt of same tier intercepts the pair
    """
    issues: list[ValidationIssue] = []

    # Collect all UG belts grouped by io_type
    ug_inputs: list[PlacedEntity] = []
    ug_outputs: list[PlacedEntity] = []
    all_ug: list[PlacedEntity] = []
    for e in layout_result.entities:
        if e.name in _UG_BELT_ENTITIES:
            all_ug.append(e)
            if e.io_type == "input":
                ug_inputs.append(e)
            elif e.io_type == "output":
                ug_outputs.append(e)

    used_outputs: set[tuple[int, int]] = set()

    for inp in ug_inputs:
        d = _DIR_TO_VEC.get(inp.direction)
        if d is None:
            continue
        dx, dy = d
        surface_tier = _UG_TO_SURFACE_TIER.get(inp.name, "transport-belt")
        max_reach = _UG_MAX_REACH.get(surface_tier, 4)

        # Find nearest matching output along direction
        best_out: PlacedEntity | None = None
        best_dist = float("inf")
        for out in ug_outputs:
            if (out.x, out.y) in used_outputs:
                continue
            if out.direction != inp.direction:
                continue
            if out.name != inp.name:
                continue
            rx, ry = out.x - inp.x, out.y - inp.y
            if dx != 0:
                if ry != 0 or (rx > 0) != (dx > 0):
                    continue
                dist = abs(rx)
            else:
                if rx != 0 or (ry > 0) != (dy > 0):
                    continue
                dist = abs(ry)
            if 1 < dist < best_dist:
                best_dist = dist
                best_out = out

        if best_out is None:
            issues.append(
                ValidationIssue(
                    severity="error",
                    category="underground-belt",
                    message=(
                        f"Unpaired underground belt input at ({inp.x},{inp.y}) "
                        f"facing {inp.direction.name}: no matching output found"
                    ),
                    x=inp.x,
                    y=inp.y,
                )
            )
            continue

        used_outputs.add((best_out.x, best_out.y))

        # Check distance does not exceed max reach
        # max_reach = underground gap tiles, so max entry-to-exit distance = max_reach + 1
        dist = best_dist
        if dist > max_reach + 1:
            issues.append(
                ValidationIssue(
                    severity="error",
                    category="underground-belt",
                    message=(
                        f"Underground belt pair ({inp.x},{inp.y})->({best_out.x},{best_out.y}) "
                        f"distance {int(dist)} exceeds max reach {max_reach} for {surface_tier}"
                    ),
                    x=inp.x,
                    y=inp.y,
                )
            )

        # Check for intercepting UG belts of the same tier between the pair
        for ug in all_ug:
            if ug is inp or ug is best_out:
                continue
            if ug.name != inp.name or ug.direction != inp.direction:
                continue
            rx, ry = ug.x - inp.x, ug.y - inp.y
            if dx != 0:
                if ry != 0 or (rx > 0) != (dx > 0):
                    continue
                udist = abs(rx)
            else:
                if rx != 0 or (ry > 0) != (dy > 0):
                    continue
                udist = abs(ry)
            if 0 < udist < dist:
                issues.append(
                    ValidationIssue(
                        severity="warning",
                        category="underground-belt",
                        message=(
                            f"Underground belt at ({ug.x},{ug.y}) intercepts pair "
                            f"({inp.x},{inp.y})->({best_out.x},{best_out.y})"
                        ),
                        x=ug.x,
                        y=ug.y,
                    )
                )

    # Check for unpaired outputs (outputs not matched to any input)
    for out in ug_outputs:
        if (out.x, out.y) not in used_outputs:
            issues.append(
                ValidationIssue(
                    severity="error",
                    category="underground-belt",
                    message=(
                        f"Unpaired underground belt output at ({out.x},{out.y}) "
                        f"facing {out.direction.name}: no matching input found"
                    ),
                    x=out.x,
                    y=out.y,
                )
            )

    return issues


def check_underground_belt_sideloading(
    layout_result: LayoutResult,
) -> list[ValidationIssue]:
    """Check underground belt exit sideloading geometry.

    For each UG belt output (exit), checks what's on the tile it exits
    onto. If it's a belt:
    - Perpendicular sideload: valid (feeds near lane)
    - Head-on collision (opposite direction, same axis): error
    """
    issues: list[ValidationIssue] = []

    belt_dir: dict[tuple[int, int], EntityDirection] = {}
    for e in layout_result.entities:
        if e.name in _BELT_ENTITIES:
            belt_dir[(e.x, e.y)] = e.direction

    for e in layout_result.entities:
        if e.name not in _UG_BELT_ENTITIES or e.io_type != "output":
            continue

        d = _DIR_TO_VEC.get(e.direction)
        if d is None:
            continue
        dx, dy = d
        exit_tile = (e.x + dx, e.y + dy)

        if exit_tile not in belt_dir:
            continue

        target_dir = belt_dir[exit_tile]
        tdx, tdy = _DIR_TO_VEC[target_dir]

        # Check relationship between UG exit direction and target belt direction
        dot = dx * tdx + dy * tdy

        if dot < 0:
            # Head-on: UG exit flows into a belt coming toward it
            issues.append(
                ValidationIssue(
                    severity="error",
                    category="underground-belt",
                    message=(
                        f"Underground belt exit at ({e.x},{e.y}) facing {e.direction.name} "
                        f"collides head-on with belt at ({exit_tile[0]},{exit_tile[1]}) "
                        f"facing {target_dir.name}"
                    ),
                    x=e.x,
                    y=e.y,
                )
            )

    return issues


def check_belt_loops(
    layout_result: LayoutResult,
) -> list[ValidationIssue]:
    """Check for belt loops (cycles where items circulate forever).

    Follows each belt's direction forward. If we revisit a tile,
    there's a loop. Items caught in a loop never reach their destination.
    """
    issues: list[ValidationIssue] = []

    belt_dir_map = _belt_dir_map_from(layout_result.entities)

    # Track which tiles we've already confirmed as loop-free or part of a reported loop
    confirmed: set[tuple[int, int]] = set()
    reported_loops: set[frozenset[tuple[int, int]]] = set()

    for start in belt_dir_map:
        if start in confirmed:
            continue

        # Follow the chain forward
        visited_order: list[tuple[int, int]] = []
        visited_set: set[tuple[int, int]] = set()
        cur = start

        while cur in belt_dir_map and cur not in visited_set:
            visited_set.add(cur)
            visited_order.append(cur)
            d = belt_dir_map[cur]
            dx, dy = _DIR_TO_VEC[d]
            cur = (cur[0] + dx, cur[1] + dy)

        if cur in visited_set:
            # Found a cycle — extract the loop tiles
            cycle_start_idx = visited_order.index(cur)
            loop_tiles = frozenset(visited_order[cycle_start_idx:])

            if loop_tiles not in reported_loops:
                reported_loops.add(loop_tiles)
                # Pick a representative tile for the error location
                rep = min(loop_tiles)
                issues.append(
                    ValidationIssue(
                        severity="error",
                        category="belt-loop",
                        message=(f"Belt loop detected: {len(loop_tiles)} tiles form a cycle near ({rep[0]},{rep[1]})"),
                        x=rep[0],
                        y=rep[1],
                    )
                )

        confirmed |= visited_set

    return issues


def check_belt_item_isolation(
    layout_result: LayoutResult,
) -> list[ValidationIssue]:
    """Check that belts carrying different items don't feed into each other.

    Unlike pipes, belts don't auto-merge by adjacency — but if belt A's
    direction points at belt B and they carry different items, items from A
    will physically flow onto B, contaminating it.
    """
    issues: list[ValidationIssue] = []

    belt_dir: dict[tuple[int, int], EntityDirection] = {}
    belt_carry: dict[tuple[int, int], str | None] = {}
    ug_inputs: set[tuple[int, int]] = set()
    for e in layout_result.entities:
        if e.name in _BELT_ENTITIES:
            belt_dir[(e.x, e.y)] = e.direction
            belt_carry[(e.x, e.y)] = e.carries
            if e.name in _UG_BELT_ENTITIES and e.io_type == "input":
                ug_inputs.add((e.x, e.y))

    seen: set[tuple[tuple[int, int], tuple[int, int]]] = set()
    for (ax, ay), ad in belt_dir.items():
        # Skip underground belt entries — items go underground, not to adjacent tile
        if (ax, ay) in ug_inputs:
            continue
        dx, dy = _DIR_TO_VEC[ad]
        bx, by = ax + dx, ay + dy
        if (bx, by) not in belt_dir:
            continue
        ac = belt_carry.get((ax, ay))
        bc = belt_carry.get((bx, by))
        if ac and bc and ac != bc:
            pair = ((ax, ay), (bx, by))
            if pair not in seen:
                seen.add(pair)
                issues.append(
                    ValidationIssue(
                        severity="error",
                        category="belt-item-isolation",
                        message=(f"Belt at ({ax},{ay}) carries {ac} but feeds into ({bx},{by}) which carries {bc}"),
                        x=ax,
                        y=ay,
                    )
                )

    return issues


def check_belt_inserter_conflict(
    layout_result: LayoutResult,
) -> list[ValidationIssue]:
    issues: list[ValidationIssue] = []

    belt_tiles: set[tuple[int, int]] = set()
    for e in layout_result.entities:
        if e.name in _BELT_ENTITIES:
            belt_tiles.add((e.x, e.y))

    drop_map: dict[tuple[int, int], list[tuple[PlacedEntity, str]]] = defaultdict(list)
    for e in layout_result.entities:
        if e.name not in _INSERTER_ENTITIES or e.carries is None:
            continue
        dv = _DIR_TO_VEC.get(e.direction)
        if dv is None:
            continue
        reach = 2 if e.name == "long-handed-inserter" else 1
        drop_x = e.x + dv[0] * reach
        drop_y = e.y + dv[1] * reach
        if (drop_x, drop_y) in belt_tiles:
            drop_map[(drop_x, drop_y)].append((e, e.carries))

    for (bx, by), inserters in drop_map.items():
        items = {item for _, item in inserters}
        if len(items) >= 2:
            sorted_items = sorted(items)
            issues.append(
                ValidationIssue(
                    severity="error",
                    category="belt-item-isolation",
                    message=(
                        f"Belt at ({bx},{by}): inserters drop conflicting items"
                        f" {sorted_items[0]!r} and {sorted_items[1]!r}"
                    ),
                    x=bx,
                    y=by,
                )
            )

    return issues


def check_belt_flow_reachability(
    layout_result: LayoutResult,
    solver_result: SolverResult | None = None,
    layout_style: str = "spaghetti",
) -> list[ValidationIssue]:
    """Check that items can physically flow through belts to/from machines.

    Uses directional BFS that follows actual belt directions, unlike the
    adjacency-based checks. For each machine:
    - Input check: trace upstream from the input inserter's belt tile to
      verify items can reach it from the boundary or another machine's output.
    - Output check: trace downstream from the output inserter's belt tile to
      verify items can leave toward the boundary or another machine's input.
    """
    issues: list[ValidationIssue] = []
    if solver_result is None:
        return issues

    fluid_only_recipes = _get_fluid_only_recipes(solver_result)

    # Build belt direction map
    belt_dir_map = _belt_dir_map_from(layout_result.entities)

    if not belt_dir_map:
        return issues

    # Build underground belt pair map for tunnel traversal
    ug_pairs = _build_ug_pairs(layout_result)
    splitter_siblings = _build_splitter_siblings(layout_result)

    # Build machine tile maps
    machine_tiles = _build_machine_tile_set(layout_result)
    machine_by_tile: dict[tuple[int, int], tuple[int, int]] = {}
    for e in layout_result.entities:
        if e.name in _MACHINE_ENTITIES:
            size = _machine_size(e.name)
            for dx in range(size):
                for dy in range(size):
                    machine_by_tile[(e.x + dx, e.y + dy)] = (e.x, e.y)

    # Classify inserters as input or output, recording their belt tiles
    # input_belt_tiles: set of tiles where input inserters pick from
    # output_belt_tiles: set of tiles where output inserters drop onto
    input_belt_tiles: set[tuple[int, int]] = set()
    output_belt_tiles: set[tuple[int, int]] = set()
    # Per-machine: which belt tiles are input/output for that machine
    machine_input_belts: dict[tuple[int, int], list[tuple[int, int]]] = {}
    machine_output_belts: dict[tuple[int, int], list[tuple[int, int]]] = {}

    for ins in layout_result.entities:
        if ins.name not in _INSERTER_ENTITIES:
            continue
        direction_vec = _DIR_TO_VEC.get(ins.direction)
        if direction_vec is None:
            continue
        reach = _INSERTER_REACH.get(ins.name, 1)
        dx, dy = direction_vec
        odx, ody = _OPPOSITE_VEC[direction_vec]
        drop_pos = (ins.x + dx * reach, ins.y + dy * reach)
        pickup_pos = (ins.x + odx * reach, ins.y + ody * reach)

        if drop_pos in machine_tiles and pickup_pos in belt_dir_map:
            mpos = machine_by_tile.get(drop_pos)
            if mpos:
                input_belt_tiles.add(pickup_pos)
                machine_input_belts.setdefault(mpos, []).append(pickup_pos)
        elif pickup_pos in machine_tiles and drop_pos in belt_dir_map:
            mpos = machine_by_tile.get(pickup_pos)
            if mpos:
                output_belt_tiles.add(drop_pos)
                machine_output_belts.setdefault(mpos, []).append(drop_pos)

    # Compute layout boundary from belt positions
    all_xs = [x for x, _ in belt_dir_map]
    all_ys = [y for _, y in belt_dir_map]
    min_bx, max_bx = min(all_xs), max(all_xs)
    min_by, max_by = min(all_ys), max(all_ys)

    def _on_boundary(pos: tuple[int, int]) -> bool:
        return pos[0] in (min_bx, max_bx) or pos[1] in (min_by, max_by)

    severity = "error" if layout_style == "spaghetti" else "warning"

    # Check inputs: can items reach each machine's input inserter?
    checked: set[tuple[int, int]] = set()
    for e in layout_result.entities:
        if e.name not in _MACHINE_ENTITIES:
            continue
        mpos = (e.x, e.y)
        if mpos in checked:
            continue
        checked.add(mpos)
        if e.recipe in fluid_only_recipes:
            continue

        belts = machine_input_belts.get(mpos, [])
        if not belts:
            continue  # other checks catch missing inserters

        belt_set = set(belts)
        upstream = _bfs_belt_upstream(belt_set, belt_dir_map, ug_pairs=ug_pairs, splitter_siblings=splitter_siblings)
        # Exclude start tiles — they may be on the boundary but that
        # doesn't mean items can reach them from outside
        upstream_beyond_start = upstream - belt_set
        reaches_source = any(_on_boundary(t) for t in upstream_beyond_start) or bool(
            upstream_beyond_start & output_belt_tiles
        )
        if not reaches_source:
            issues.append(
                ValidationIssue(
                    severity=severity,
                    category="belt-flow-reachability",
                    message=(
                        f"{e.name} at ({e.x},{e.y}): items can't reach input "
                        f"(no upstream path from boundary or another machine's output)"
                    ),
                    x=e.x,
                    y=e.y,
                )
            )

    # Check outputs: can items leave each machine's output inserter?
    checked.clear()
    for e in layout_result.entities:
        if e.name not in _MACHINE_ENTITIES:
            continue
        mpos = (e.x, e.y)
        if mpos in checked:
            continue
        checked.add(mpos)
        if e.recipe in fluid_only_recipes:
            continue

        belts = machine_output_belts.get(mpos, [])
        if not belts:
            continue  # other checks catch missing output belts

        belt_set = set(belts)
        downstream = _bfs_belt_downstream(
            belt_set, belt_dir_map, ug_pairs=ug_pairs, splitter_siblings=splitter_siblings,
        )
        downstream_beyond_start = downstream - belt_set
        reaches_sink = any(_on_boundary(t) for t in downstream_beyond_start) or bool(
            downstream_beyond_start & input_belt_tiles
        )
        if not reaches_sink:
            issues.append(
                ValidationIssue(
                    severity=severity,
                    category="belt-flow-reachability",
                    message=(
                        f"{e.name} at ({e.x},{e.y}): items can't leave output "
                        f"(no downstream path to boundary or another machine's input)"
                    ),
                    x=e.x,
                    y=e.y,
                )
            )

    return issues


def _inserter_target_lane(
    ins_x: int,
    ins_y: int,
    belt_x: int,
    belt_y: int,
    belt_dir: EntityDirection,
) -> str:
    """Return which lane an inserter places items on (the far lane)."""
    return inserter_target_lane(ins_x, ins_y, belt_x, belt_y, belt_dir)


def _classify_belt_feeders(
    belt_dir_map: dict[tuple[int, int], EntityDirection],
    ug_output_tiles: set[tuple[int, int]] | None = None,
    ug_output_to_input: dict[tuple[int, int], tuple[int, int]] | None = None,
) -> dict[tuple[int, int], list[tuple[tuple[int, int], str]]]:
    """For each belt tile, return feeders and their type.

    Returns {(x,y): [(feeder_pos, feed_type), ...]}
    where feed_type is 'straight', 'sideload_left', or 'sideload_right'.

    Underground outputs in belt_dir_map feed adjacent tiles but only receive
    from their tunnel (paired input), not from adjacent surface belts.
    """
    _ug_outputs = ug_output_tiles or set()
    _ug_o2i = ug_output_to_input or {}

    feeders: dict[tuple[int, int], list[tuple[tuple[int, int], str]]] = {}
    for (bx, by), belt_d in belt_dir_map.items():
        tile_feeders = []
        bdx, bdy = _DIR_TO_VEC[belt_d]
        # Left perpendicular
        left_dx, left_dy = -bdy, bdx

        if (bx, by) in _ug_outputs:
            # UG output: only feeder is the tunnel (paired input's upstream).
            # We model this as in-degree 0 — rates injected during propagation.
            pass
        else:
            for ddx, ddy in [(1, 0), (-1, 0), (0, 1), (0, -1)]:
                nx, ny = bx + ddx, by + ddy
                nd = belt_dir_map.get((nx, ny))
                if nd is None:
                    continue
                ndx, ndy = _DIR_TO_VEC[nd]
                # Does this neighbor output to (bx, by)?
                if (nx + ndx, ny + ndy) != (bx, by):
                    continue
                # Classify: straight or sideload
                if nd == belt_d:
                    tile_feeders.append(((nx, ny), "straight"))
                else:
                    # Which side does the sideload come from?
                    rel_x, rel_y = nx - bx, ny - by
                    dot = rel_x * left_dx + rel_y * left_dy
                    if dot > 0:
                        tile_feeders.append(((nx, ny), "sideload_left"))
                    else:
                        tile_feeders.append(((nx, ny), "sideload_right"))
        if tile_feeders:
            feeders[(bx, by)] = tile_feeders
    return feeders


def compute_lane_rates(
    layout_result: LayoutResult,
    solver_result: SolverResult | None = None,
) -> dict[tuple[int, int], dict[str, float]]:
    """Compute per-lane throughput for every surface belt tile.

    Returns ``{(x, y): {"left": rate, "right": rate}}`` via topological
    propagation of inserter output rates through the belt network.
    """
    if solver_result is None:
        return {}

    # Build belt maps — surface belts, underground outputs, and splitters.
    # Underground outputs feed adjacent tiles like surface belts and inherit
    # rates from their paired input's upstream belt.
    belt_dir_map: dict[tuple[int, int], EntityDirection] = {}
    ug_output_tiles: set[tuple[int, int]] = set()
    ug_output_to_input: dict[tuple[int, int], tuple[int, int]] = {}
    ug_input_dir: dict[tuple[int, int], EntityDirection] = {}
    for e in layout_result.entities:
        if e.name in _SURFACE_BELT_ENTITIES:
            belt_dir_map[(e.x, e.y)] = e.direction
        elif e.name in _UG_BELT_ENTITIES:
            if e.io_type == "output":
                belt_dir_map[(e.x, e.y)] = e.direction
                ug_output_tiles.add((e.x, e.y))
            elif e.io_type == "input":
                ug_input_dir[(e.x, e.y)] = e.direction
        elif e.name in _SPLITTER_ENTITIES:
            belt_dir_map[(e.x, e.y)] = e.direction
            if e.direction in (EntityDirection.NORTH, EntityDirection.SOUTH):
                belt_dir_map[(e.x + 1, e.y)] = e.direction
            else:
                belt_dir_map[(e.x, e.y + 1)] = e.direction

    if not belt_dir_map:
        return {}

    # Build underground pair map: output → input (for tunnel rate propagation)
    ug_pairs = _build_ug_pairs(layout_result)
    for (ix, iy), (ox, oy) in list(ug_pairs.items()):
        if (ix, iy) in ug_input_dir:
            ug_output_to_input[(ox, oy)] = (ix, iy)

    # Build machine tile maps
    machine_tiles = _build_machine_tile_set(layout_result)
    machine_by_tile: dict[tuple[int, int], tuple[int, int]] = {}
    machine_entity: dict[tuple[int, int], PlacedEntity] = {}
    for e in layout_result.entities:
        if e.name in _MACHINE_ENTITIES:
            machine_entity[(e.x, e.y)] = e
            size = _machine_size(e.name)
            for dx in range(size):
                for dy in range(size):
                    machine_by_tile[(e.x + dx, e.y + dy)] = (e.x, e.y)

    # Build recipe → MachineSpec lookup
    recipe_to_spec: dict[str, MachineSpec] = {}
    for spec in solver_result.machines:
        recipe_to_spec[spec.recipe] = spec

    # Find output inserters and their injection rates per lane
    # lane_injections: {(bx, by): {"left": rate, "right": rate}}
    lane_injections: dict[tuple[int, int], dict[str, float]] = {}
    belt_carries: dict[tuple[int, int], str | None] = {}
    for e in layout_result.entities:
        if e.name in _BELT_ENTITIES:
            belt_carries[(e.x, e.y)] = e.carries

    for ins in layout_result.entities:
        if ins.name not in _INSERTER_ENTITIES:
            continue
        direction_vec = _DIR_TO_VEC.get(ins.direction)
        if direction_vec is None:
            continue
        reach = _INSERTER_REACH.get(ins.name, 1)
        dx, dy = direction_vec
        odx, ody = _OPPOSITE_VEC[direction_vec]
        drop_pos = (ins.x + dx * reach, ins.y + dy * reach)
        pickup_pos = (ins.x + odx * reach, ins.y + ody * reach)

        # Output inserter: picks from machine, drops onto belt
        if pickup_pos not in machine_tiles or drop_pos not in belt_dir_map:
            continue

        mpos = machine_by_tile.get(pickup_pos)
        if mpos is None or mpos not in machine_entity:
            continue

        me = machine_entity[mpos]
        spec = recipe_to_spec.get(me.recipe or "")
        if spec is None:
            continue

        # Find the rate for the item this belt carries
        carried_item = belt_carries.get(drop_pos)
        if carried_item is None:
            continue

        rate = 0.0
        for out in spec.outputs:
            if out.item == carried_item:
                rate = out.rate
                break
        if rate <= 0:
            continue

        # Determine target lane
        belt_d = belt_dir_map[drop_pos]
        lane = _inserter_target_lane(ins.x, ins.y, drop_pos[0], drop_pos[1], belt_d)

        entry = lane_injections.setdefault(drop_pos, {"left": 0.0, "right": 0.0})
        entry[lane] += rate

    # Classify feeders for each belt tile
    feeders = _classify_belt_feeders(belt_dir_map, ug_output_tiles, ug_output_to_input)

    # Topological-order propagation
    # Compute in-degree (number of belt feeders)
    in_degree: dict[tuple[int, int], int] = {pos: 0 for pos in belt_dir_map}
    for pos, tile_feeders in feeders.items():
        in_degree[pos] = len(tile_feeders)

    # Start from tiles with in-degree 0
    queue = deque(pos for pos, deg in in_degree.items() if deg == 0)
    lane_rates: dict[tuple[int, int], dict[str, float]] = {}
    processed: set[tuple[int, int]] = set()

    # Initialize all tiles
    for pos in belt_dir_map:
        inj = lane_injections.get(pos, {"left": 0.0, "right": 0.0})
        lane_rates[pos] = {"left": inj["left"], "right": inj["right"]}

    # Build splitter sibling map for 50/50 distribution
    splitter_sibling: dict[tuple[int, int], tuple[int, int]] = {}
    for e in layout_result.entities:
        if e.name in _SPLITTER_ENTITIES:
            if e.direction in (EntityDirection.NORTH, EntityDirection.SOUTH):
                splitter_sibling[(e.x, e.y)] = (e.x + 1, e.y)
                splitter_sibling[(e.x + 1, e.y)] = (e.x, e.y)
            else:
                splitter_sibling[(e.x, e.y)] = (e.x, e.y + 1)
                splitter_sibling[(e.x, e.y + 1)] = (e.x, e.y)

    # Track which splitter tiles have all inputs accumulated (in_degree reached 0)
    splitter_input_ready: set[tuple[int, int]] = set()

    def _propagate_tile(tile: tuple[int, int]) -> None:
        """Propagate one tile's rates to its downstream neighbour."""
        d = belt_dir_map.get(tile)
        if d is None:
            return
        ddx, ddy = _DIR_TO_VEC[d]
        downstream = (tile[0] + ddx, tile[1] + ddy)

        if downstream not in belt_dir_map:
            return

        downstream_d = belt_dir_map[downstream]
        downstream_dx, downstream_dy = _DIR_TO_VEC[downstream_d]
        left_dx, left_dy = -downstream_dy, downstream_dx

        my_rates = lane_rates[tile]

        if d == downstream_d or (ddx, ddy) == (downstream_dx, downstream_dy):
            lane_rates[downstream]["left"] += my_rates["left"]
            lane_rates[downstream]["right"] += my_rates["right"]
        else:
            behind_downstream = (downstream[0] - downstream_dx, downstream[1] - downstream_dy)
            if tile == behind_downstream:
                lane_rates[downstream]["left"] += my_rates["left"]
                lane_rates[downstream]["right"] += my_rates["right"]
            else:
                # Check if this is a 90-degree turn or a sideload.
                # Turn: downstream has NO straight feeder (only this perpendicular one).
                # Sideload: downstream also has a straight feeder from behind.
                ds_feeders = feeders.get(downstream, [])
                has_straight = any(ft == "straight" for _, ft in ds_feeders)

                if has_straight:
                    # Sideload: all items go onto one lane
                    rel_x = tile[0] - downstream[0]
                    rel_y = tile[1] - downstream[1]
                    dot = rel_x * left_dx + rel_y * left_dy
                    total = my_rates["left"] + my_rates["right"]
                    if dot > 0:
                        lane_rates[downstream]["left"] += total
                    else:
                        lane_rates[downstream]["right"] += total
                else:
                    # 90-degree turn: lanes rotate with the belt.
                    # CW turn: left→right, right→left.
                    # CCW turn: left→left, right→right.
                    cross = ddx * downstream_dy - ddy * downstream_dx
                    if cross > 0:
                        # Clockwise turn
                        lane_rates[downstream]["right"] += my_rates["left"]
                        lane_rates[downstream]["left"] += my_rates["right"]
                    else:
                        # Counter-clockwise turn
                        lane_rates[downstream]["left"] += my_rates["left"]
                        lane_rates[downstream]["right"] += my_rates["right"]

        in_degree[downstream] -= 1
        if in_degree[downstream] <= 0 and downstream not in processed:
            queue.append(downstream)

    while queue:
        pos = queue.popleft()
        if pos in processed:
            continue

        # Underground output: inherit rates from the belt feeding the paired input.
        # Tunnel preserves lanes.  Defer until the upstream tile is processed.
        if pos in ug_output_tiles:
            paired_input = ug_output_to_input.get(pos)
            if paired_input is not None:
                inp_d = ug_input_dir.get(paired_input)
                if inp_d is not None:
                    idx, idy = _DIR_TO_VEC[inp_d]
                    behind = (paired_input[0] - idx, paired_input[1] - idy)
                    if behind in belt_dir_map and behind not in processed:
                        # Defer — re-queue after the upstream tile is processed
                        queue.append(pos)
                        continue
                    if behind in lane_rates:
                        lane_rates[pos]["left"] += lane_rates[behind]["left"]
                        lane_rates[pos]["right"] += lane_rates[behind]["right"]

        sib = splitter_sibling.get(pos)
        if sib is not None and sib not in processed:
            # Splitter tile: wait until sibling's inputs are also ready
            splitter_input_ready.add(pos)
            if sib not in splitter_input_ready:
                continue  # defer — sibling will trigger processing of both
            # Both tiles have accumulated inputs — redistribute 50/50
            total_left = lane_rates[pos]["left"] + lane_rates[sib]["left"]
            total_right = lane_rates[pos]["right"] + lane_rates[sib]["right"]
            for tile in (pos, sib):
                lane_rates[tile]["left"] = total_left / 2
                lane_rates[tile]["right"] = total_right / 2
            # Process both (sibling was deferred, process it first)
            for tile in (sib, pos):
                processed.add(tile)
                _propagate_tile(tile)
            continue

        processed.add(pos)
        _propagate_tile(pos)

    return lane_rates


def check_lane_throughput(
    layout_result: LayoutResult,
    solver_result: SolverResult | None = None,
) -> list[ValidationIssue]:
    """Check per-lane belt throughput using full lane simulation.

    Each belt has two lanes (left/right), each with half the belt's total
    throughput. Items enter specific lanes: inserters → far lane, sideloads
    → near lane. Turns swap lanes. Flags tiles where either lane exceeds
    its per-lane capacity.
    """
    issues: list[ValidationIssue] = []
    lane_rates = compute_lane_rates(layout_result, solver_result)
    if not lane_rates:
        return issues

    # Build belt name map for capacity lookup (surface belts + UG outputs)
    belt_name_map: dict[tuple[int, int], str] = {}
    for e in layout_result.entities:
        if e.name in _SURFACE_BELT_ENTITIES:
            belt_name_map[(e.x, e.y)] = e.name
        elif e.name in _UG_BELT_ENTITIES and e.io_type == "output":
            belt_name_map[(e.x, e.y)] = _UG_TO_SURFACE_TIER.get(e.name, "transport-belt")

    for pos, rates in lane_rates.items():
        belt_name = belt_name_map.get(pos, "transport-belt")
        cap = _LANE_CAPACITY.get(belt_name, 7.5)
        for lane_name in ("left", "right"):
            if rates[lane_name] > cap + 0.01:
                issues.append(
                    ValidationIssue(
                        severity="error",
                        category="lane-throughput",
                        message=(
                            f"Belt at ({pos[0]},{pos[1]}): {lane_name} lane "
                            f"{rates[lane_name]:.1f}/s exceeds {belt_name} "
                            f"per-lane capacity {cap}/s"
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
