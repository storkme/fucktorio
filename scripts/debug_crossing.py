"""Inspect the iron-plate tap-off crossing area in the Python bus layout."""

from src.bus import bus_layout
from src.solver.solver import solve

result = solve("electronic-circuit", 10.0, available_inputs={"iron-plate", "copper-plate"})
layout = bus_layout(result)

ents = [e for e in layout.entities if 30 <= (e.y or 0) <= 50 and 3 <= (e.x or 0) <= 20]
ents.sort(key=lambda e: (e.y, e.x))
for e in ents:
    print(f"({e.x:3},{e.y:3}) {e.name:30} {str(e.direction):10} {e.carries or '':20} {e.io_type or ''}")
