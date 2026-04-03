"""Shared rendering theme for the visualizer and showcase.

The theme is a JavaScript object with draw* methods, emitted as inline JS
into self-contained HTML files.
"""

from __future__ import annotations

from pathlib import Path

_JS_DIR = Path(__file__).parent / "js"

_JS_FILES = [
    "utils.js",
    "theme-schematic.js",
]

THEME_JS = "\n".join((_JS_DIR / f).read_text() for f in _JS_FILES)
