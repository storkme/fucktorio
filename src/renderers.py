"""Shared rendering themes for the visualizer and showcase.

Each theme is a JavaScript object with draw* methods. These are emitted as
inline JS into self-contained HTML files.

Both `visualize.py` and `showcase.py` import THEME_JS to get the rendering
code for all themes + the dispatch helpers.

Themes:
  - schematic: clean, colorful, diagrammatic (the original style)
  - factorio: dark, industrial, game-realistic
"""

from __future__ import annotations

from pathlib import Path

_JS_DIR = Path(__file__).parent / "js"

_JS_FILES = [
    "utils.js",
    "theme-schematic.js",
    "theme-factorio.js",
    "theme-dispatch.js",
]

THEME_JS = "\n".join((_JS_DIR / f).read_text() for f in _JS_FILES)
