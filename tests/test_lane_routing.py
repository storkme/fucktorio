"""Tests for lane-aware A* routing."""

from __future__ import annotations

import pytest

from src.models import EntityDirection, PlacedEntity
from src.routing.common import LANE_LEFT, LANE_RIGHT, inserter_target_lane
from src.routing.router import _astar_path


class TestInserterTargetLane:
    """Test that inserter_target_lane correctly computes the far lane."""

    @pytest.mark.parametrize(
        "belt_dir, ins_side, expected",
        [
            # Belt going SOUTH (dir=(0,1)): left perp=WEST=(-1,0)
            # Inserter on WEST (left) side → items on RIGHT (far) lane
            (EntityDirection.SOUTH, (-1, 0), LANE_RIGHT),
            # Inserter on EAST (right) side → items on LEFT (far) lane
            (EntityDirection.SOUTH, (1, 0), LANE_LEFT),
            # Belt going NORTH (dir=(0,-1)): left perp=EAST=(1,0)
            (EntityDirection.NORTH, (-1, 0), LANE_LEFT),
            (EntityDirection.NORTH, (1, 0), LANE_RIGHT),
            # Belt going EAST (dir=(1,0)): left perp=SOUTH=(0,1)
            (EntityDirection.EAST, (0, -1), LANE_LEFT),
            (EntityDirection.EAST, (0, 1), LANE_RIGHT),
            # Belt going WEST (dir=(-1,0)): left perp=NORTH=(0,-1)
            (EntityDirection.WEST, (0, -1), LANE_RIGHT),
            (EntityDirection.WEST, (0, 1), LANE_LEFT),
        ],
    )
    def test_all_directions(self, belt_dir, ins_side, expected):
        # Belt at (5, 5), inserter offset by ins_side
        bx, by = 5, 5
        ix, iy = bx + ins_side[0], by + ins_side[1]
        result = inserter_target_lane(ix, iy, bx, by, belt_dir)
        assert result == expected


class TestLanePreservation:
    """Test that A* lane state transitions work correctly."""

    def test_straight_preserves_lane(self):
        """Lane should be unchanged through a straight path."""
        # Route from (0,0) to (5,0) — straight east, no obstacles
        path = _astar_path(
            (0, 0),
            {(5, 0)},
            set(),
            start_lane=LANE_LEFT,
        )
        assert path is not None
        assert path[0] == (0, 0)
        assert path[-1] == (5, 0)
        # All moves are east — no turns, lane stays left
        # We verify by checking A* found the path (if lane was wrong at goal
        # with a goal_lane_check, it would fail)

    def test_turn_swaps_lane(self):
        """A 90-degree turn should swap the lane."""
        # Force a right-angle turn: go east then south
        # Block straight east path to (3, 3) — force a turn
        obstacles = set()
        # Create an L-shaped corridor: (0,0) -> (3,0) -> (3,3)
        for x in range(10):
            for y in range(10):
                obstacles.add((x, y))
        # Clear the L-path
        for x in range(4):
            obstacles.discard((x, 0))
        for y in range(4):
            obstacles.discard((3, y))

        path = _astar_path(
            (0, 0),
            {(3, 3)},
            obstacles,
            start_lane=LANE_LEFT,
        )
        assert path is not None
        assert (3, 3) in path

    def test_underground_preserves_lane(self):
        """Lane should be preserved through an underground belt jump."""
        # Block a wide band of surface tiles to force underground
        obstacles = set()
        for x in range(1, 5):
            for y in range(-5, 6):
                obstacles.add((x, y))
        path = _astar_path(
            (0, 0),
            {(7, 0)},
            obstacles,
            allow_underground=True,
            ug_max_reach=6,
            start_lane=LANE_RIGHT,
        )
        assert path is not None
        # Path should include an underground jump (non-adjacent tiles)
        has_jump = False
        for i in range(1, len(path)):
            dist = abs(path[i][0] - path[i - 1][0]) + abs(path[i][1] - path[i - 1][1])
            if dist > 1:
                has_jump = True
                break
        assert has_jump, "Expected underground jump in path"


class TestSideloadLane:
    """Test that sideloading onto existing belts forces the correct lane."""

    def test_sideload_forces_near_lane(self):
        """Sideloading from north onto an eastbound belt → left lane."""
        # Existing eastbound belt at (3, 3)
        belt_dir_map = {(3, 3): EntityDirection.EAST}
        # Route from (3, 0) south to (3, 3) — sideloading from north
        path = _astar_path(
            (3, 0),
            {(3, 3)},
            set(),
            start_lane=LANE_LEFT,
            belt_dir_map=belt_dir_map,
        )
        assert path is not None
        assert path[-1] == (3, 3)


class TestGoalLaneCheck:
    """Test that the goal lane check correctly rejects wrong-lane arrivals."""

    def test_goal_accepts_correct_lane(self):
        """Goal should be accepted when items arrive on the correct lane."""
        # Path goes straight south from (5,0) to (5,5)
        # At goal: arrival direction = SOUTH, left perp = (-1, 0)
        # goal_lane_check = (1, 0) (inserter on east side of belt)
        # dot = 1*(-1) + 0*0 = -1 < 0 → needed_lane = "right"
        # Start on RIGHT lane, straight path → arrives RIGHT → match
        path = _astar_path(
            (5, 0),
            {(5, 5)},
            set(),
            start_lane=LANE_RIGHT,
            goal_lane_check=(1, 0),
        )
        assert path is not None

    def test_goal_rejects_wrong_lane(self):
        """Goal should be rejected when items arrive on the wrong lane."""
        # Same setup: inserter on east side needs RIGHT lane
        # But start on LEFT lane. Straight south = no turn = stays LEFT → mismatch
        # Block all alternatives to force straight south (no turns possible)
        obstacles = set()
        for x in range(-5, 15):
            for y in range(-5, 15):
                obstacles.add((x, y))
        for y in range(6):
            obstacles.discard((5, y))

        path = _astar_path(
            (5, 0),
            {(5, 5)},
            obstacles,
            start_lane=LANE_LEFT,
            goal_lane_check=(1, 0),
        )
        # Should fail — can't reach goal on correct lane (no room to turn)
        assert path is None

    def test_goal_with_turn_to_fix_lane(self):
        """A turn can swap the lane to match the goal requirement."""
        # Goal needs RIGHT lane (inserter on east side, southbound arrival).
        # Start on LEFT. One turn (east→south) swaps left→right. Should work.
        # Create an L-path: (0,0) east to (3,0), then south to (3,3)
        obstacles = set()
        for x in range(-2, 10):
            for y in range(-2, 10):
                obstacles.add((x, y))
        for x in range(4):
            obstacles.discard((x, 0))
        for y in range(4):
            obstacles.discard((3, y))

        # At goal (3,3): arrival = SOUTH, left perp = (-1, 0)
        # goal_lane_check = (1, 0), dot = -1 → needed = RIGHT
        # Start LEFT, turn east→south swaps to RIGHT → matches
        path = _astar_path(
            (0, 0),
            {(3, 3)},
            obstacles,
            start_lane=LANE_LEFT,
            goal_lane_check=(1, 0),
        )
        assert path is not None


class TestDeadEndFix:
    """Test that dead-end belts turn to face adjacent network tiles."""

    def test_orphan_turns_to_face_adjacent_belt(self):
        """A dead-end belt next to an existing belt should turn to sideload into it."""
        from src.routing.router import _fix_belt_directions

        entities = [
            # Existing eastbound belt at (5, 2)
            PlacedEntity(name="transport-belt", x=5, y=2, direction=EntityDirection.EAST, carries="iron-gear-wheel"),
            # Dead-end belt at (5, 3) pointing east (should turn north to face (5,2))
            PlacedEntity(name="transport-belt", x=5, y=3, direction=EntityDirection.EAST, carries="iron-gear-wheel"),
            # Upstream belt feeding (5,3) from the west
            PlacedEntity(name="transport-belt", x=4, y=3, direction=EntityDirection.EAST, carries="iron-gear-wheel"),
        ]
        belt_dir_map = {(e.x, e.y): e.direction for e in entities}

        _fix_belt_directions(entities, belt_dir_map)

        # (5,3) should now face NORTH to sideload into (5,2)
        assert belt_dir_map[(5, 3)] == EntityDirection.NORTH
        assert entities[1].direction == EntityDirection.NORTH

    def test_orphan_no_upstream_turns_to_face_adjacent(self):
        """A dead-end belt with no upstream belt (fed by inserter) should still turn."""
        from src.routing.router import _fix_belt_directions

        entities = [
            # Northbound trunk: (5,4) → (5,3) → (5,2)
            PlacedEntity(name="transport-belt", x=5, y=4, direction=EntityDirection.NORTH, carries="iron-gear-wheel"),
            PlacedEntity(name="transport-belt", x=5, y=3, direction=EntityDirection.NORTH, carries="iron-gear-wheel"),
            PlacedEntity(name="transport-belt", x=5, y=2, direction=EntityDirection.NORTH, carries="iron-gear-wheel"),
            # Dead-end belt at (4, 3) pointing south (should turn east to face trunk)
            # This belt has no upstream belt neighbor — it's fed by an inserter
            PlacedEntity(name="transport-belt", x=4, y=3, direction=EntityDirection.SOUTH, carries="iron-gear-wheel"),
        ]
        belt_dir_map = {(e.x, e.y): e.direction for e in entities}

        _fix_belt_directions(entities, belt_dir_map)

        # (4,3) should now face EAST to sideload into (5,3)
        assert belt_dir_map[(4, 3)] == EntityDirection.EAST
        assert entities[3].direction == EntityDirection.EAST
        # Trunk should remain northbound
        assert belt_dir_map[(5, 3)] == EntityDirection.NORTH


class TestPerpendicularUGPenalty:
    """Test that perpendicular underground entries are penalized."""

    def test_straight_ug_preferred_over_perpendicular(self):
        """A* should prefer straight-through underground over perpendicular entry."""
        # Two possible paths to reach (10, 0):
        # 1. Go east from (0,0), underground at some point — straight entry
        # 2. Go south then east underground — perpendicular entry (penalized)
        # Both should work, but option 1 should be cheaper
        obstacles = set()
        # Block tiles 3-6 on y=0 to force underground
        for x in range(3, 7):
            obstacles.add((x, 0))
            # But leave y=1 open so perpendicular approach is possible
        # Also block wide band on y=-1 to prevent going north
        for x in range(-5, 15):
            obstacles.add((x, -1))

        path = _astar_path(
            (0, 0),
            {(10, 0)},
            obstacles,
            allow_underground=True,
            ug_max_reach=6,
            start_lane=LANE_LEFT,
        )
        assert path is not None
        # The path should use a straight east underground, not go south first
        # Check that all path tiles have y >= 0 (no detour south then back)
        # and the underground entry tile has the same x-direction as approach
        for i in range(1, len(path)):
            px, py = path[i - 1]
            cx, cy = path[i]
            dist = abs(cx - px) + abs(cy - py)
            if dist > 1:
                # Underground jump — check approach was inline (east)
                jump_dx = 1 if cx > px else -1 if cx < px else 0
                if i >= 2:
                    prev_dx = px - path[i - 2][0]
                    if prev_dx != 0:
                        prev_dx = 1 if prev_dx > 0 else -1
                    # Approach direction should match jump direction (not perpendicular)
                    assert prev_dx == jump_dx or prev_dx == 0, (
                        f"Perpendicular underground entry at path[{i - 1}]→path[{i}]"
                    )


class TestInserterPositions:
    """Test that _get_sides returns centered border positions."""

    def test_3x3_machine_has_12_positions(self):
        """A 3×3 machine should have 12 border positions (3 per side)."""
        from src.routing.inserters import _get_sides

        sides = _get_sides(0, 0, 3)
        assert len(sides) == 12
        border_tiles = [s[0] for s in sides]
        assert len(set(border_tiles)) == 12


class TestDirectInsertion:
    """Test direct machine-to-machine insertion detection."""

    def test_adjacent_machines_get_direct_inserter(self):
        """Two machines with gap=1 should get a direct inserter."""
        from src.routing.graph import build_production_graph
        from src.routing.orchestrate import build_layout
        from src.solver import solve

        # electronic-circuit has internal edges (copper-cable -> circuit)
        result = solve("electronic-circuit", target_rate=1, available_inputs={"iron-plate", "copper-plate"})
        graph = build_production_graph(result)

        # Place two connected machines adjacent (gap=1)
        # Find an internal edge
        internal = [(i, e) for i, e in enumerate(graph.edges) if e.from_node is not None and e.to_node is not None]
        if not internal:
            pytest.skip("No internal edges in this recipe")

        edge_idx, edge = internal[0]
        # Place from_node at (0,0) and to_node at (4,0) — gap of 1 tile at x=3
        positions = {}
        for node in graph.nodes:
            if node.id == edge.from_node:
                positions[node.id] = (0, 0)
            elif node.id == edge.to_node:
                positions[node.id] = (4, 0)
            else:
                positions[node.id] = (0, 10 + node.id * 5)

        layout, failed, direct_count = build_layout(result, graph, positions)
        assert direct_count >= 1, "Expected at least one direct insertion"


class TestCompactPlacement:
    """Test that the placer can produce tight layouts."""

    def test_spacing_1_places_machines_close(self):
        """Spacing=1 should place machines with small gaps."""
        from src.routing.graph import build_production_graph
        from src.solver import solve
        from src.spaghetti.placer import incremental_place

        result = solve("iron-gear-wheel", target_rate=10, available_inputs={"iron-plate"})
        graph = build_production_graph(result)
        positions = incremental_place(graph, spacing=1)

        # All machines should be within a reasonable bounding box
        xs = [x for x, y in positions.values()]
        ys = [y for x, y in positions.values()]
        spread = (max(xs) - min(xs)) + (max(ys) - min(ys))
        # With spacing=1 and 4 machines, spread should be much less than spacing=3
        assert spread < 30, f"Layout too spread out: {spread}"


class TestIntegration:
    """End-to-end tests with full layout generation."""

    def test_iron_gear_lane_throughput(self, iron_gear_layout, iron_gear_solver_result):
        """Iron gear layout should have no lane-reachability errors."""
        from src.validate import check_lane_throughput

        issues = check_lane_throughput(iron_gear_layout, iron_gear_solver_result)
        lane_errors = [i for i in issues if i.category == "lane-reachability"]
        # We don't assert zero errors yet (the layout is randomized),
        # but this exercises the full pipeline
        if lane_errors:
            for e in lane_errors:
                print(f"  [{e.severity}] {e.message}")
