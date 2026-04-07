"""
Extract static north-facing entity sprites (body + shadow composited) from Factorio
game data, using draftsman for metadata and Pillow for image processing.

Output: graphics/entity-frames/{slug}.png
Canvas is sized to the machine's tile footprint at OUTPUT_TILE_PX, with extra padding
on the right/bottom for shadow overflow. Shadow rendered at ~45% alpha behind body.

Space Age / quality mod entities use hardcoded geometry (draftsman returns width=0
for those) derived from the .lua sidecar files in the entity directories.
"""

import os
from pathlib import Path
from PIL import Image
from draftsman.data import entities as entities_data

REPO = Path(__file__).parent.parent
GRAPHICS_BASE  = REPO / "graphics" / "base"
GRAPHICS_ALT1  = REPO / "graphics" / "alt1"
GRAPHICS_QUALITY = REPO / "graphics" / "quality"
OUT_DIR = REPO / "graphics" / "entity-frames"
OUT_DIR.mkdir(exist_ok=True)

OUTPUT_TILE_PX = 64   # pixels per tile in output images
SHADOW_ALPHA   = 0.45
SHADOW_PAD_RIGHT  = 2.5  # extra canvas tiles for shadow overflow
SHADOW_PAD_BOTTOM = 1.5

# All machines to extract: slug → (tile_w, tile_h)
MACHINES = {
    # Base-game machines (draftsman has full metadata)
    "assembling-machine-1":  (3, 3),
    "assembling-machine-2":  (3, 3),
    "assembling-machine-3":  (3, 3),
    "electric-furnace":      (3, 3),
    "steel-furnace":         (2, 2),
    "stone-furnace":         (2, 2),
    "chemical-plant":        (3, 3),
    "oil-refinery":          (5, 5),
    "centrifuge":            (3, 3),
    "lab":                   (3, 3),
    "rocket-silo":           (9, 9),
    # Space Age / quality machines (geometry from .lua sidecars)
    "electromagnetic-plant": (4, 4),
    "cryogenic-plant":       (5, 5),
    "biochamber":            (3, 3),
    "biolab":                (5, 5),
    "foundry":               (5, 5),
    "recycler":              (2, 4),
    "crusher":               (2, 3),
}

# by_pixel(px, py) → tile shift = px/32, py/32  (Factorio: 1 tile = 32 px)
def bp(px, py): return [px / 32, py / 32]

# Hardcoded layer specs for Space Age / quality entities.
# body/shadow keys contain the same fields as draftsman layer dicts.
# 'filenames' → list of file suffixes to stitch horizontally (lines_per_file rows each).
SPACE_AGE_OVERRIDES = {
    "electromagnetic-plant": {
        "body": {
            "filename": "__space-age__/graphics/entity/electromagnetic-plant/electromagnetic-plant-base.png",
            "width": 238, "height": 252, "scale": 0.5, "shift": bp(0, 0),
        },
        "shadow": {
            "filename": "__space-age__/graphics/entity/electromagnetic-plant/electromagnetic-plant-base-shadow.png",
            "width": 262, "height": 242, "scale": 0.5, "shift": bp(5.5, 6.5),
            "draw_as_shadow": True,
        },
    },
    "cryogenic-plant": {
        "body": {
            "filename": "__space-age__/graphics/entity/cryogenic-plant/cryogenic-plant-main.png",
            "width": 380, "height": 396, "scale": 0.5, "shift": bp(0, 0),
        },
        "shadow": {
            "filename": "__space-age__/graphics/entity/cryogenic-plant/cryogenic-plant-shadow.png",
            "width": 462, "height": 310, "scale": 0.5, "shift": bp(35.5, 7.0),
            "draw_as_shadow": True,
        },
    },
    "biochamber": {
        "body": {
            "filename": "__space-age__/graphics/entity/biochamber/biochamber.png",
            "width": 238, "height": 268, "scale": 0.5, "shift": bp(0, 0),
        },
        "shadow": {
            "filename": "__space-age__/graphics/entity/biochamber/biochamber-shadow.png",
            "width": 268, "height": 190, "scale": 0.5, "shift": bp(18.0, 1.5),
            "draw_as_shadow": True,
        },
    },
    "biolab": {
        "body": {
            "filename": "__space-age__/graphics/entity/biolab/biolab-anim.png",
            "width": 366, "height": 404, "line_length": 8, "scale": 0.5, "shift": bp(2, -5),
        },
        "shadow": {
            "filename": "__space-age__/graphics/entity/biolab/biolab-shadow.png",
            "width": 476, "height": 262, "line_length": 8, "scale": 0.5, "shift": bp(39.5, 21.0),
            "draw_as_shadow": True,
        },
    },
    "foundry": {
        "body": {
            # split across -1.png / -2.png; frame 0 is in file 1
            "filename": "__space-age__/graphics/entity/foundry/foundry-main-1.png",
            "width": 376, "height": 398, "scale": 0.5, "shift": bp(0, -6),
        },
        "shadow": {
            "filename": "__space-age__/graphics/entity/foundry/foundry-shadow-1.png",
            "width": 514, "height": 214, "scale": 0.5, "shift": bp(47.5, 29.0),
            "draw_as_shadow": True,
        },
    },
    "recycler": {
        "body": {
            "filename": "__quality__/graphics/entity/recycler/recycler-N.png",
            "width": 170, "height": 304, "line_length": 8, "scale": 0.5, "shift": bp(2, -6.5),
        },
        "shadow": {
            "filename": "__quality__/graphics/entity/recycler/recycler-N-shadow.png",
            "width": 234, "height": 252, "line_length": 8, "scale": 0.5, "shift": bp(28, 2),
            "draw_as_shadow": True,
        },
    },
    "crusher": {
        "body": {
            "filename": "__space-age__/graphics/entity/crusher/crusher-vertical.png",
            "width": 140, "height": 194, "line_length": 8, "scale": 0.5, "shift": bp(2, -15.5),
        },
        "shadow": {
            "filename": "__space-age__/graphics/entity/crusher/crusher-vertical-shadow.png",
            "width": 50, "height": 78, "line_length": 1, "scale": 0.5, "shift": bp(38, -26),
            "draw_as_shadow": True,
        },
    },
}


def resolve_path(factorio_path: str) -> Path:
    if factorio_path.startswith("__base__/graphics/"):
        return GRAPHICS_BASE / factorio_path.removeprefix("__base__/graphics/")
    if factorio_path.startswith("__space-age__/graphics/"):
        return GRAPHICS_ALT1 / factorio_path.removeprefix("__space-age__/graphics/")
    if factorio_path.startswith("__quality__/graphics/"):
        return GRAPHICS_QUALITY / factorio_path.removeprefix("__quality__/graphics/")
    raise ValueError(f"Unknown prefix: {factorio_path}")


def crop_frame0(layer: dict) -> Image.Image:
    path = resolve_path(layer["filename"])
    sheet = Image.open(path).convert("RGBA")
    x_off = layer.get("x", 0)
    y_off = layer.get("y", 0)
    fw, fh = layer["width"], layer["height"]
    return sheet.crop((x_off, y_off, x_off + fw, y_off + fh))


def get_north_layers_from_draftsman(slug: str) -> list[dict]:
    e = entities_data.raw.get(slug, {})
    gs = e.get("graphics_set", {})
    anim = (
        gs.get("animation") or gs.get("idle_animation") or gs.get("on_animation")
        or e.get("animation") or e.get("on_animation")
        or _rocket_silo_body(e)
    )
    if not anim:
        return []
    node = anim.get("north", anim) if isinstance(anim, dict) else {}
    if "layers" in node:
        return node["layers"]
    elif "filename" in node:
        return [node]
    return []


def _rocket_silo_body(e: dict) -> dict | None:
    sprite = e.get("base_day_sprite") or e.get("base_front_sprite")
    if not sprite:
        return None
    node = sprite.get("north", sprite)
    if "filename" in node:
        return {"layers": [node]}
    return None


def place_on_canvas(canvas, sprite, shift_tiles, origin_tiles, scale, alpha_mult=1.0):
    px = OUTPUT_TILE_PX
    sw = int(sprite.width * scale)
    sh = int(sprite.height * scale)
    resized = sprite.resize((sw, sh), Image.LANCZOS)

    cx = int(-origin_tiles[0] * px)
    cy = int(-origin_tiles[1] * px)
    paste_x = int(cx + shift_tiles[0] * px - sw / 2)
    paste_y = int(cy + shift_tiles[1] * px - sh / 2)

    if alpha_mult < 1.0:
        r, g, b, a = resized.split()
        a = a.point(lambda v: int(v * alpha_mult))
        resized = Image.merge("RGBA", (r, g, b, a))

    canvas.paste(resized, (paste_x, paste_y), resized)


def extract(slug: str, tile_w: int, tile_h: int) -> Path | None:
    # Get layers: prefer override for Space Age machines, else draftsman
    override = SPACE_AGE_OVERRIDES.get(slug)
    if override:
        body_layer  = override.get("body")
        shadow_layer = override.get("shadow")
    else:
        layers = get_north_layers_from_draftsman(slug)
        if not layers:
            print(f"  [SKIP] no layers found for {slug}")
            return None
        # draftsman layers with width=0 are unusable
        body_layer   = next((l for l in layers if not l.get("draw_as_shadow") and l.get("width", 0) > 0), None)
        shadow_layer = next((l for l in layers if l.get("draw_as_shadow") and l.get("width", 0) > 0), None)

    if not body_layer:
        print(f"  [SKIP] no usable body layer for {slug}")
        return None

    px = OUTPUT_TILE_PX
    canvas_w = int((tile_w + SHADOW_PAD_RIGHT) * px)
    canvas_h = int((tile_h + SHADOW_PAD_BOTTOM) * px)
    canvas = Image.new("RGBA", (canvas_w, canvas_h), (0, 0, 0, 0))
    origin = (-tile_w / 2, -tile_h / 2)

    if shadow_layer:
        try:
            shadow = crop_frame0(shadow_layer)
            place_on_canvas(canvas, shadow, shadow_layer.get("shift", [0,0]), origin,
                            shadow_layer.get("scale", 0.5), SHADOW_ALPHA)
        except Exception as e:
            print(f"  [WARN] shadow failed: {e}")

    try:
        body = crop_frame0(body_layer)
        place_on_canvas(canvas, body, body_layer.get("shift", [0,0]), origin,
                        body_layer.get("scale", 0.5), 1.0)
    except Exception as e:
        print(f"  [ERROR] body failed: {e}")
        return None

    out_path = OUT_DIR / f"{slug}.png"
    canvas.save(out_path)
    return out_path


def main():
    print(f"Extracting entity frames → {OUT_DIR}\n")
    for slug, (tw, th) in MACHINES.items():
        print(f"{slug} ({tw}×{th})...")
        out = extract(slug, tw, th)
        if out:
            img = Image.open(out)
            print(f"  → {out.name} ({img.width}×{img.height})")
    print("\nDone.")


if __name__ == "__main__":
    main()
