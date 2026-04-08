"""
Extract 64×64 item/entity icons from Factorio game data and copy them to
web/public/icons/.

Source dirs: graphics/base/icons/, graphics/alt1/icons/, graphics/quality/icons/
Format: each file is a 120×64 mipmap chain (64+32+16+8=120). Crop (0,0,64,64).
"""

from pathlib import Path

from PIL import Image

REPO = Path(__file__).parent.parent
SRC_DIRS = [
    REPO / "graphics" / "base" / "icons",
    REPO / "graphics" / "alt1" / "icons",
    REPO / "graphics" / "quality" / "icons",
]
OUT_DIR = REPO / "web" / "public" / "icons"
OUT_DIR.mkdir(parents=True, exist_ok=True)


def extract_icon(src: Path) -> Image.Image:
    img = Image.open(src).convert("RGBA")
    # Mipmap chain: full-res icon is leftmost 64×64
    if img.width > 64:
        return img.crop((0, 0, 64, 64))
    return img.resize((64, 64), Image.LANCZOS)


def main():
    new, updated = 0, 0
    for src_dir in SRC_DIRS:
        if not src_dir.exists():
            print(f"  [SKIP] {src_dir} not found")
            continue
        for src in sorted(src_dir.glob("*.png")):
            dest = OUT_DIR / src.name
            icon = extract_icon(src)
            existed = dest.exists()
            icon.save(dest)
            if existed:
                updated += 1
            else:
                new += 1

    print(f"Done: {new} new, {updated} updated → {OUT_DIR}")


if __name__ == "__main__":
    main()
