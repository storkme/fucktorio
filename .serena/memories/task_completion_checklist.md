# What To Do When a Task Is Completed

1. **Run ruff** to check linting and formatting:
   ```bash
   ruff check src/ tests/
   ruff format --check src/ tests/
   ```

2. **Run tests**:
   ```bash
   pytest tests/ -x
   ```

3. **If layout engine changes were made**, also:
   - Generate viz: `pytest tests/test_spaghetti.py::TestSpaghettiVisualization::test_viz_iron_gear_wheel --viz -x`
   - Visually inspect `test_viz/iron-gear-wheel-10s.html`
   - Verify the fix is actually running (the `_evaluate` function catches ALL exceptions silently)
   - Don't trust error count drops alone — check WHY errors changed

4. **Performance check**: One search attempt should take <2s with the Rust A*. If >10s, something is wrong.
