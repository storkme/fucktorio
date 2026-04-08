"""Generate a single HTML page showing all balancer templates in an N×M grid.

Run: uv run python scripts/viz_balancer_grid.py
Produces: test_viz/balancer-grid.html

Each cell renders a tile-based diagram of the balancer with colour-coded
entity types, direction arrows, and input/output port highlights.
"""

from __future__ import annotations

from pathlib import Path

from src.bus.balancer_library import BALANCER_TEMPLATES, BalancerTemplate

TILE_PX = 24  # pixels per tile
OUT_DIR = Path("test_viz")

# Factorio direction → unicode arrow
_DIR_ARROW = {0: "\u2191", 2: "\u2192", 4: "\u2193", 6: "\u2190"}  # N E S W

# Entity type → fill colour
_COLOURS = {
    "transport-belt": "#ffd966",
    "underground-belt": "#a4c2f4",
    "splitter": "#b6d7a8",
}


def _splitter_tiles(x: int, y: int, direction: int) -> list[tuple[int, int]]:
    """Return the two tiles a splitter occupies."""
    if direction in (0, 4):  # N/S: spans 2 tiles horizontally
        return [(x, y), (x + 1, y)]
    else:  # E/W: spans 2 tiles vertically
        return [(x, y), (x, y + 1)]


def render_template_svg(tmpl: BalancerTemplate) -> str:
    """Render a single template as an inline SVG element."""
    w = tmpl.width * TILE_PX + 2
    h = tmpl.height * TILE_PX + 2

    input_set = set(tmpl.input_tiles)
    output_set = set(tmpl.output_tiles)

    lines = [f'<svg width="{w}" height="{h}" xmlns="http://www.w3.org/2000/svg">']
    lines.append(f'<rect width="{w}" height="{h}" fill="#1a1a2e" rx="3"/>')

    # Grid lines
    for gx in range(tmpl.width + 1):
        px = gx * TILE_PX + 1
        lines.append(f'<line x1="{px}" y1="1" x2="{px}" y2="{h - 1}" stroke="#333" stroke-width="0.5"/>')
    for gy in range(tmpl.height + 1):
        py = gy * TILE_PX + 1
        lines.append(f'<line x1="1" y1="{py}" x2="{w - 1}" y2="{py}" stroke="#333" stroke-width="0.5"/>')

    # Entities
    for e in tmpl.entities:
        colour = _COLOURS.get(e.name, "#888")
        arrow = _DIR_ARROW.get(e.direction, "?")

        if e.name == "splitter":
            tiles = _splitter_tiles(e.x, e.y, e.direction)
            for tx, ty in tiles:
                px = tx * TILE_PX + 1
                py = ty * TILE_PX + 1
                lines.append(
                    f'<rect x="{px}" y="{py}" width="{TILE_PX}" height="{TILE_PX}" fill="{colour}" stroke="#555" stroke-width="0.5" rx="2"/>'
                )
            # Arrow in center of splitter
            cx = (tiles[0][0] + tiles[1][0] + 1) * TILE_PX / 2 + 1
            cy = (tiles[0][1] + tiles[1][1] + 1) * TILE_PX / 2 + 1
            lines.append(
                f'<text x="{cx}" y="{cy}" text-anchor="middle" dominant-baseline="central" '
                f'font-size="{TILE_PX * 0.6}" fill="#333">{arrow}</text>'
            )
        else:
            px = e.x * TILE_PX + 1
            py = e.y * TILE_PX + 1
            # Darker shade for underground outputs (exit points)
            if e.name == "underground-belt" and e.io_type == "output":
                colour = "#6d9eeb"
            lines.append(
                f'<rect x="{px}" y="{py}" width="{TILE_PX}" height="{TILE_PX}" fill="{colour}" stroke="#555" stroke-width="0.5" rx="2"/>'
            )
            cx = px + TILE_PX / 2
            cy = py + TILE_PX / 2
            fs = TILE_PX * 0.55
            lines.append(
                f'<text x="{cx}" y="{cy}" text-anchor="middle" dominant-baseline="central" '
                f'font-size="{fs}" fill="#333">{arrow}</text>'
            )

    # Highlight input/output ports
    for ix, iy in input_set:
        px = ix * TILE_PX + 1
        py = iy * TILE_PX + 1
        lines.append(
            f'<rect x="{px}" y="{py}" width="{TILE_PX}" height="{TILE_PX}" fill="none" stroke="#00ff88" stroke-width="2" rx="2"/>'
        )
    for ox, oy in output_set:
        px = ox * TILE_PX + 1
        py = oy * TILE_PX + 1
        lines.append(
            f'<rect x="{px}" y="{py}" width="{TILE_PX}" height="{TILE_PX}" fill="none" stroke="#ff6688" stroke-width="2" rx="2"/>'
        )

    lines.append("</svg>")
    return "\n".join(lines)


def build_html() -> str:
    """Build a single HTML page with all templates in an N×M grid."""
    max_n = max(n for n, _ in BALANCER_TEMPLATES.keys())
    max_m = max(m for _, m in BALANCER_TEMPLATES.keys())

    rows: list[str] = []
    rows.append("""<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Balancer Template Grid</title>
<style>
body { background: #0d1117; color: #c9d1d9; font-family: system-ui, sans-serif; margin: 20px; }
h1 { text-align: center; color: #58a6ff; }
.summary { text-align: center; margin-bottom: 20px; color: #8b949e; }
table { border-collapse: collapse; margin: 0 auto; }
th { background: #161b22; color: #58a6ff; padding: 8px 12px; border: 1px solid #30363d; font-size: 14px; }
td { border: 1px solid #30363d; padding: 8px; text-align: center; vertical-align: top; background: #0d1117; }
td.empty { background: #161b22; }
.cell-label { font-size: 11px; color: #8b949e; margin-top: 4px; }
.cell-label .dims { color: #58a6ff; }
.cell-label .entities { color: #d2a8ff; }
.legend { display: flex; justify-content: center; gap: 20px; margin: 16px 0; font-size: 13px; }
.legend-item { display: flex; align-items: center; gap: 6px; }
.legend-swatch { width: 16px; height: 16px; border-radius: 3px; border: 1px solid #555; }
.coverage { text-align: center; margin: 10px 0; }
.covered { color: #3fb950; } .missing { color: #f85149; }
</style>
</head>
<body>
<h1>Balancer Template Grid</h1>
""")

    total = max_n * max_m - 1  # exclude 1×1
    covered = len(BALANCER_TEMPLATES)
    rows.append(f'<div class="summary">{covered}/{total} templates generated (excluding 1&times;1 identity)</div>')

    # Legend
    rows.append('<div class="legend">')
    rows.append('<div class="legend-item"><div class="legend-swatch" style="background:#ffd966"></div> Belt</div>')
    rows.append('<div class="legend-item"><div class="legend-swatch" style="background:#a4c2f4"></div> UG Input</div>')
    rows.append('<div class="legend-item"><div class="legend-swatch" style="background:#6d9eeb"></div> UG Output</div>')
    rows.append('<div class="legend-item"><div class="legend-swatch" style="background:#b6d7a8"></div> Splitter</div>')
    rows.append(
        '<div class="legend-item"><div class="legend-swatch" style="background:none;border:2px solid #00ff88"></div> Input port</div>'
    )
    rows.append(
        '<div class="legend-item"><div class="legend-swatch" style="background:none;border:2px solid #ff6688"></div> Output port</div>'
    )
    rows.append("</div>")

    # Coverage bar
    rows.append('<div class="coverage">')
    missing = []
    for n in range(1, max_n + 1):
        for m in range(1, max_m + 1):
            if (n, m) != (1, 1) and (n, m) not in BALANCER_TEMPLATES:
                missing.append(f"{n}&rarr;{m}")
    if missing:
        rows.append(f'<span class="missing">Missing: {", ".join(missing)}</span>')
    else:
        rows.append('<span class="covered">Full coverage!</span>')
    rows.append("</div>")

    # Grid table
    rows.append("<table>")
    # Header row
    rows.append("<tr><th>N \\ M</th>")
    for m in range(1, max_m + 1):
        rows.append(f"<th>{m}</th>")
    rows.append("</tr>")

    for n in range(1, max_n + 1):
        rows.append(f"<tr><th>{n}</th>")
        for m in range(1, max_m + 1):
            if (n, m) == (1, 1):
                rows.append('<td class="empty"><span style="color:#484f58">identity</span></td>')
            elif (n, m) in BALANCER_TEMPLATES:
                tmpl = BALANCER_TEMPLATES[(n, m)]
                svg = render_template_svg(tmpl)
                label = (
                    f'<div class="cell-label">'
                    f'<span class="dims">{tmpl.width}W &times; {tmpl.height}H</span> '
                    f'<span class="entities">({len(tmpl.entities)} ent)</span></div>'
                )
                rows.append(f"<td>{svg}{label}</td>")
            else:
                rows.append('<td class="empty"><span style="color:#f85149">--</span></td>')
        rows.append("</tr>")

    rows.append("</table>")
    rows.append("</body></html>")
    return "\n".join(rows)


def main() -> None:
    OUT_DIR.mkdir(exist_ok=True)
    content = build_html()
    out_path = OUT_DIR / "balancer-grid.html"
    out_path.write_text(content)
    print(f"Wrote {out_path} ({len(BALANCER_TEMPLATES)} templates)")


if __name__ == "__main__":
    main()
