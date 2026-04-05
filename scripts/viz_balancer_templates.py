"""Stamp each balancer template into an empty layout and viz it.

Run: .venv/bin/python scripts/viz_balancer_templates.py
Produces: test_viz/balancer-{n}x{m}.html for each template.
"""

from __future__ import annotations

from pathlib import Path

from src.blueprint import build_blueprint
from src.bus.balancer_library import BALANCER_TEMPLATES, BalancerTemplate
from src.models import EntityDirection, LayoutResult, PlacedEntity
from src.visualize import visualize

_DIR_MAP = {
    0: EntityDirection.NORTH,
    2: EntityDirection.EAST,
    4: EntityDirection.SOUTH,
    6: EntityDirection.WEST,
}


def stamp_template(template: BalancerTemplate, origin_x: int = 0, origin_y: int = 0) -> list[PlacedEntity]:
    return [
        PlacedEntity(
            name=e.name,
            x=origin_x + e.x,
            y=origin_y + e.y,
            direction=_DIR_MAP[e.direction],
            io_type=e.io_type,
            carries="iron-plate",
        )
        for e in template.entities
    ]


def main() -> None:
    out_dir = Path("test_viz")
    out_dir.mkdir(exist_ok=True)

    for (n, m), tmpl in sorted(BALANCER_TEMPLATES.items()):
        entities = stamp_template(tmpl)
        layout = LayoutResult(
            entities=entities,
            connections=[],
            width=tmpl.width,
            height=tmpl.height,
        )
        bp_str = build_blueprint(layout, label=f"balancer {n}->{m}")
        html_path = out_dir / f"balancer-{n}x{m}.html"
        visualize(
            bp_str,
            output_path=str(html_path),
            open_browser=False,
            layout_result=layout,
        )
        print(f"wrote {html_path}")


if __name__ == "__main__":
    main()
