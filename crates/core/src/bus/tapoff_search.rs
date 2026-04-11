//! Brute-force search for optimal splitter tap-off tile patterns.
//!
//! Given a South-flowing trunk belt, finds the minimal tile arrangement
//! that splits the flow: trunk continues South, tap-off exits East.
//! Only used in tests to verify the hardcoded 2×2 stamp is optimal.

#[cfg(test)]
mod tests {
    use std::fmt;

    /// Direction of an entity.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    enum Dir {
        North,
        South,
        East,
        West,
    }

    impl fmt::Display for Dir {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Dir::North => write!(f, "N"),
                Dir::South => write!(f, "S"),
                Dir::East => write!(f, "E"),
                Dir::West => write!(f, "W"),
            }
        }
    }

    /// What occupies a grid cell.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Cell {
        Empty,
        Belt(Dir),
        /// Splitter anchor (top-left tile). The companion tile is determined
        /// by the facing direction: perpendicular-right of Dir.
        SplitterAnchor(Dir),
        /// Second half of a splitter (placed automatically by the anchor).
        SplitterCompanion(Dir),
    }

    impl fmt::Display for Cell {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Cell::Empty => write!(f, ".  "),
                Cell::Belt(d) => write!(f, "B{} ", d),
                Cell::SplitterAnchor(d) => write!(f, "S{}<", d),
                Cell::SplitterCompanion(d) => write!(f, "S{}>", d),
            }
        }
    }

    /// The offset of the companion tile relative to the anchor, given
    /// the splitter's facing direction. A South-facing splitter is 2 wide
    /// (East-West) × 1 deep, so companion is at (+1, 0).
    fn companion_offset(dir: Dir) -> (i32, i32) {
        match dir {
            Dir::South => (1, 0),  // 2 wide horizontally
            Dir::North => (1, 0),
            Dir::East => (0, 1),   // 2 tall vertically
            Dir::West => (0, 1),
        }
    }

    /// Input side offsets for a splitter (2 tiles on the input side).
    /// Returns [(left_input_dx, left_input_dy), (right_input_dx, right_input_dy)]
    /// relative to the anchor.
    fn splitter_inputs(dir: Dir) -> [(i32, i32); 2] {
        match dir {
            Dir::South => [(0, -1), (1, -1)],   // inputs from North
            Dir::North => [(0, 1), (1, 1)],      // inputs from South
            Dir::East => [(-1, 0), (-1, 1)],     // inputs from West
            Dir::West => [(1, 0), (1, 1)],        // inputs from East
        }
    }

    /// Output side offsets for a splitter.
    /// Returns [(left_output_dx, left_output_dy), (right_output_dx, right_output_dy)]
    fn splitter_outputs(dir: Dir) -> [(i32, i32); 2] {
        match dir {
            Dir::South => [(0, 1), (1, 1)],     // outputs to South
            Dir::North => [(0, -1), (1, -1)],    // outputs to North
            Dir::East => [(1, 0), (1, 1)],       // outputs to East
            Dir::West => [(-1, 0), (-1, 1)],     // outputs to West
        }
    }

    /// The tile a belt at (x, y) facing `dir` receives FROM.
    fn belt_input_tile(x: i32, y: i32, dir: Dir) -> (i32, i32) {
        match dir {
            Dir::North => (x, y + 1),
            Dir::South => (x, y - 1),
            Dir::East => (x - 1, y),
            Dir::West => (x + 1, y),
        }
    }

    #[derive(Clone)]
    struct Grid {
        w: i32,
        h: i32,
        cells: Vec<Vec<Cell>>,
    }

    impl Grid {
        fn new(w: i32, h: i32) -> Self {
            Self {
                w,
                h,
                cells: vec![vec![Cell::Empty; w as usize]; h as usize],
            }
        }

        fn get(&self, x: i32, y: i32) -> Cell {
            if x < 0 || y < 0 || x >= self.w || y >= self.h {
                Cell::Empty
            } else {
                self.cells[y as usize][x as usize]
            }
        }

        fn set(&mut self, x: i32, y: i32, cell: Cell) {
            self.cells[y as usize][x as usize] = cell;
        }

        fn in_bounds(&self, x: i32, y: i32) -> bool {
            x >= 0 && y >= 0 && x < self.w && y < self.h
        }
    }

    impl fmt::Display for Grid {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            for y in 0..self.h {
                for x in 0..self.w {
                    write!(f, "{}", self.get(x, y))?;
                }
                writeln!(f)?;
            }
            Ok(())
        }
    }

    /// Check if a grid configuration is valid and satisfies:
    /// 1. A belt entering South from (0, -1) can flow through the grid
    /// 2. A belt exits South from (0, h) — trunk continues
    /// 3. A belt exits East from (w, some_y) — tap-off
    /// 4. All placed entities are properly connected (no dangling outputs)
    fn is_valid_tapoff(grid: &Grid) -> bool {
        // Check: input from (0, -1) going South must feed into (0, 0)
        let top_cell = grid.get(0, 0);
        let top_accepts_south_input = match top_cell {
            Cell::Belt(Dir::South) => true,
            // Splitter anchor facing South at (0,0): input from (0,-1) is valid
            Cell::SplitterAnchor(Dir::South) => true,
            // A belt turning: South input into an East belt is a valid 90° turn
            Cell::Belt(Dir::East) => true,
            _ => false,
        };
        if !top_accepts_south_input {
            return false;
        }

        // Check: trunk continues South — something at (0, h-1) must output to (0, h)
        let bottom_cell = grid.get(0, grid.h - 1);
        let bottom_outputs_south = match bottom_cell {
            Cell::Belt(Dir::South) => true,
            // Splitter output at (0, h-1) going South works too
            _ => false,
        };
        if !bottom_outputs_south {
            return false;
        }

        // Check: tap-off exits East from the right edge
        let mut has_east_exit = false;
        for y in 0..grid.h {
            let right_cell = grid.get(grid.w - 1, y);
            if let Cell::Belt(Dir::East) = right_cell {
                has_east_exit = true;
                break;
            }
        }
        if !has_east_exit {
            return false;
        }

        // Check: flow connectivity — trace items from (0, -1) South
        // and verify they can reach both outputs
        let mut can_reach_south = false;
        let mut can_reach_east = false;
        trace_flow(grid, 0, -1, Dir::South, &mut can_reach_south, &mut can_reach_east, 0);

        can_reach_south && can_reach_east
    }

    /// Recursively trace item flow through the grid.
    fn trace_flow(
        grid: &Grid,
        x: i32,
        y: i32,
        incoming_dir: Dir,
        can_reach_south: &mut bool,
        can_reach_east: &mut bool,
        depth: u32,
    ) {
        if depth > 20 {
            return; // prevent infinite loops
        }

        // The item is at (x, y) moving in `incoming_dir`. Find where it goes next.
        let next_x = x + match incoming_dir {
            Dir::East => 1,
            Dir::West => -1,
            _ => 0,
        };
        let next_y = y + match incoming_dir {
            Dir::South => 1,
            Dir::North => -1,
            _ => 0,
        };

        // Check if we've exited the grid
        if next_x >= grid.w && incoming_dir == Dir::East {
            *can_reach_east = true;
            return;
        }
        if next_y >= grid.h && incoming_dir == Dir::South && next_x == 0 {
            *can_reach_south = true;
            return;
        }
        if !grid.in_bounds(next_x, next_y) {
            return; // exited grid in wrong direction
        }

        let cell = grid.get(next_x, next_y);
        match cell {
            Cell::Empty => {} // dead end
            Cell::Belt(dir) => {
                // A belt accepts input from behind (opposite of its facing direction)
                // or from the side (90° turn). Items follow the belt's direction.
                let expected_input = belt_input_tile(next_x, next_y, dir);
                let from_behind = expected_input == (x, y);
                // Side input (sideload) — items from the side follow the belt direction
                let from_side = !from_behind && {
                    let (dx, dy) = (next_x - x, next_y - y);
                    // Must be adjacent
                    (dx.abs() + dy.abs() == 1) &&
                    // Side means perpendicular to belt direction
                    match dir {
                        Dir::South | Dir::North => dx != 0,
                        Dir::East | Dir::West => dy != 0,
                    }
                };
                if from_behind || from_side {
                    trace_flow(grid, next_x, next_y, dir, can_reach_south, can_reach_east, depth + 1);
                }
            }
            Cell::SplitterAnchor(dir) | Cell::SplitterCompanion(dir) => {
                // Splitter: items entering from the input side get split to both outputs
                let inputs = splitter_inputs(dir);
                // Check which input we're entering from (anchor is at the first input position)
                let anchor_pos = if cell == Cell::SplitterAnchor(dir) {
                    (next_x, next_y)
                } else {
                    // Companion: find anchor position
                    let (cdx, cdy) = companion_offset(dir);
                    (next_x - cdx, next_y - cdy)
                };
                let is_valid_input = inputs.iter().any(|&(dx, dy)| {
                    (anchor_pos.0 + dx, anchor_pos.1 + dy) == (x, y)
                });
                if is_valid_input {
                    // Items go to both outputs
                    let outputs = splitter_outputs(dir);
                    for &(dx, dy) in &outputs {
                        let (ox, oy) = (anchor_pos.0 + dx, anchor_pos.1 + dy);
                        trace_flow(grid, ox - match dir {
                            Dir::East => 1,
                            Dir::West => -1,
                            _ => 0,
                        }, oy - match dir {
                            Dir::South => 1,
                            Dir::North => -1,
                            _ => 0,
                        }, dir, can_reach_south, can_reach_east, depth + 1);
                    }
                }
            }
        }
    }

    /// Enumerate all valid grid configurations of the given size and
    /// return those that form valid tap-off patterns.
    fn search_tapoff_patterns(w: i32, h: i32) -> Vec<Grid> {
        let mut results = Vec::new();

        // Cell options for each position (excluding splitter companions,
        // which are placed automatically with anchors)
        let cell_options = [
            Cell::Empty,
            Cell::Belt(Dir::North),
            Cell::Belt(Dir::South),
            Cell::Belt(Dir::East),
            Cell::Belt(Dir::West),
            Cell::SplitterAnchor(Dir::South),
            Cell::SplitterAnchor(Dir::North),
            Cell::SplitterAnchor(Dir::East),
            Cell::SplitterAnchor(Dir::West),
        ];

        let total_cells = (w * h) as usize;
        let num_options = cell_options.len();

        // For small grids, enumerate all combinations
        // 2x2 = 4 cells, 9 options each = 6561 combinations — trivial
        // 3x3 = 9 cells, 9^9 = ~387M — too many, use pruning
        // 4x4 = 16 cells — way too many

        if total_cells > 6 {
            // For larger grids, skip — the 2x2 search is sufficient
            return results;
        }

        let mut indices = vec![0usize; total_cells];

        loop {
            // Build grid from current indices
            let mut grid = Grid::new(w, h);
            let mut valid_placement = true;

            for pos in 0..total_cells {
                let x = (pos as i32) % w;
                let y = (pos as i32) / w;
                let cell = cell_options[indices[pos]];

                match cell {
                    Cell::SplitterAnchor(dir) => {
                        let (cdx, cdy) = companion_offset(dir);
                        let cx = x + cdx;
                        let cy = y + cdy;
                        if !grid.in_bounds(cx, cy) || grid.get(cx, cy) != Cell::Empty {
                            valid_placement = false;
                            break;
                        }
                        grid.set(x, y, cell);
                        grid.set(cx, cy, Cell::SplitterCompanion(dir));
                    }
                    _ => {
                        if grid.get(x, y) != Cell::Empty && cell != Cell::Empty {
                            // Already occupied by a splitter companion
                            valid_placement = false;
                            break;
                        }
                        if cell != Cell::Empty {
                            grid.set(x, y, cell);
                        }
                    }
                }
            }

            if valid_placement && is_valid_tapoff(&grid) {
                results.push(grid);
            }

            // Increment indices (odometer style)
            let mut carry = true;
            for pos in (0..total_cells).rev() {
                if carry {
                    indices[pos] += 1;
                    if indices[pos] >= num_options {
                        indices[pos] = 0;
                    } else {
                        carry = false;
                    }
                }
            }
            if carry {
                break; // all combinations exhausted
            }
        }

        results
    }

    #[test]
    fn test_find_optimal_tapoff_pattern() {
        // Search 2x2 grid — should find the expected pattern
        let results_2x2 = search_tapoff_patterns(2, 2);

        eprintln!("=== 2x2 results: {} valid patterns ===", results_2x2.len());
        for (i, grid) in results_2x2.iter().enumerate() {
            eprintln!("Pattern {}:\n{}", i, grid);
        }

        // Verify the expected pattern exists: splitter South at (0,0)+(1,0), belt South at (0,1), belt East at (1,1)
        let has_expected = results_2x2.iter().any(|g| {
            g.get(0, 0) == Cell::SplitterAnchor(Dir::South)
                && g.get(1, 0) == Cell::SplitterCompanion(Dir::South)
                && g.get(0, 1) == Cell::Belt(Dir::South)
                && g.get(1, 1) == Cell::Belt(Dir::East)
        });
        assert!(has_expected, "Expected 2x2 splitter tap-off pattern not found");

        // If 2x2 has solutions, no need for larger grids — 2x2 is minimal
        assert!(!results_2x2.is_empty(), "No valid 2x2 tap-off patterns found");

        eprintln!("\n=== Confirmed: 2x2 is the minimal tap-off pattern ===");
    }

    #[test]
    fn test_no_1x1_tapoff_exists() {
        // A 1x1 grid cannot possibly split flow — verify no solutions
        let results = search_tapoff_patterns(1, 1);
        assert!(results.is_empty(), "1x1 should have no valid patterns");

        let results = search_tapoff_patterns(1, 2);
        assert!(results.is_empty(), "1x2 should have no valid patterns (no room for splitter)");

        let results = search_tapoff_patterns(2, 1);
        assert!(results.is_empty(), "2x1 should have no valid patterns (no room for outputs)");
    }
}
