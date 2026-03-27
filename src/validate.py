"""Functional blueprint validation: checks that layouts actually work in Factorio."""

from __future__ import annotations

from collections import deque
from dataclasses import dataclass

from .models import LayoutResult, SolverResult

_3x3_ENTITIES = {
    "assembling-machine-1",
    "assembling-machine-2",
    "assembling-machine-3",
    "chemical-plant",
}
_5x5_ENTITIES = {"oil-refinery"}
_MACHINE_ENTITIES = _3x3_ENTITIES | _5x5_ENTITIES
_PIPE_ENTITIES = {"pipe", "pipe-to-ground"}


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
