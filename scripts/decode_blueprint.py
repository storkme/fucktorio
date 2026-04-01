#!/usr/bin/env python3
"""
Decode and analyse a Factorio blueprint string, or search factorio.school.

Usage:
    python scripts/decode_blueprint.py <blueprint_string>
    python scripts/decode_blueprint.py https://www.factorio.school/view/-OPaz0GzpyKTlYyhcEiy
    python scripts/decode_blueprint.py --search "blue circuit"
    python scripts/decode_blueprint.py --search "blue circuit" --recent
    python scripts/decode_blueprint.py  # reads from stdin

Flags:
    -v / --verbose        Dump all entities with positions
    --json                Dump raw decoded JSON
    --render              Render to HTML (written to scripts/blueprints/<id>.html)
    --search <query>      Search factorio.school by title (most popular by default)
    --recent              Sort search results by most recent instead of most popular
"""

import argparse
import base64
import json
import re
import sys
import urllib.parse
import urllib.request
import zlib
from collections import Counter
from pathlib import Path

DIR_NAMES = {0: "N", 2: "NE", 4: "E", 6: "SE", 8: "S", 10: "SW", 12: "W", 14: "NW"}

BLUEPRINTS_DIR = Path(__file__).parent / "blueprints"
SEARCH_INDEX = BLUEPRINTS_DIR / "search_index.json"


def decode(s: str) -> dict:
    s = s.strip()
    raw = base64.b64decode(s[1:])  # skip version byte
    return json.loads(zlib.decompress(raw))


def _safe_filename(title: str) -> str:
    return re.sub(r"[^\w\-]+", "_", title).strip("_").lower()


def _load_search_index() -> dict:
    if SEARCH_INDEX.exists():
        return json.loads(SEARCH_INDEX.read_text())
    return {}


def _save_search_index(index: dict) -> None:
    BLUEPRINTS_DIR.mkdir(exist_ok=True)
    tmp = SEARCH_INDEX.with_suffix(".tmp")
    tmp.write_text(json.dumps(index, indent=2, sort_keys=True))
    tmp.replace(SEARCH_INDEX)  # atomic on POSIX, best-effort on Windows


def _fetch_json(url: str) -> dict:
    req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0"})
    with urllib.request.urlopen(req) as r:
        return json.loads(r.read())


def _search_results(query: str, recent: bool = False) -> list[dict]:
    """Fetch all pages of search results from factorio.school."""
    endpoint = "filtered" if recent else "top"
    page = 1
    total_pages = 1
    all_results = []
    while page <= total_pages:
        params = urllib.parse.urlencode({"title": query})
        url = f"https://www.factorio.school/api/blueprintSummaries/{endpoint}/page/{page}?{params}"
        data = _fetch_json(url)
        pagination = data.get("_metadata", {}).get("pagination", {})
        total_pages = pagination.get("numberOfPages", 1)
        all_results.extend(data.get("_data", []))
        page += 1
    return all_results


def search_factorio_school(query: str, recent: bool = False, fetch_count: int | None = None) -> None:
    """Search factorio.school and print results, saving metadata to the index.

    fetch_count: None = list only, 0 = download all, N = download top N.
    """
    order_label = "most recent" if recent else "most popular"
    all_results = _search_results(query, recent=recent)
    to_fetch = all_results if fetch_count == 0 else (all_results[:fetch_count] if fetch_count else [])

    fetch_label = f", downloading {len(to_fetch)}" if to_fetch else ""
    print(f"Found {len(all_results)} results for '{query}' ({order_label}{fetch_label}):\n")

    index = _load_search_index()
    for bp in all_results:
        key = bp["key"]
        title = bp.get("title", "(no title)")
        upvotes = bp.get("voteSummary", {}).get("numberOfUpvotes", 0)
        slug = _safe_filename(title)
        marker = "  " if bp not in to_fetch else "->"
        print(f"  {marker} [{upvotes:3d} ▲] {title}")
        print(f"         https://www.factorio.school/view/{key}")
        index[key] = {
            "key": key,
            "title": title,
            "slug": slug,
            "upvotes": upvotes,
            "imgurId": bp.get("imgurImage", {}).get("imgurId"),
        }

    _save_search_index(index)
    print(f"\nMetadata saved to: {SEARCH_INDEX}", file=sys.stderr)

    if to_fetch:
        print(f"\nDownloading {len(to_fetch)} blueprints...")
        for i, bp in enumerate(to_fetch, 1):
            key = bp["key"]
            title = bp.get("title", "(no title)")
            url = f"https://www.factorio.school/view/{key}"
            try:
                fetch_factorio_school(url, quiet=True)
                print(f"  [{i}/{len(to_fetch)}] OK  {title}")
            except Exception as e:
                print(f"  [{i}/{len(to_fetch)}] ERR {title}: {e}", file=sys.stderr)


def fetch_factorio_school(url: str, quiet: bool = False) -> tuple[str, dict]:
    """Fetch blueprint from factorio.school. Returns (blueprint_string, metadata)."""
    bp_id = url.rstrip("/").split("/")[-1]
    api_url = f"https://facorio-blueprints.firebaseio.com/blueprints/{bp_id}.json"
    with urllib.request.urlopen(api_url) as r:
        data = json.loads(r.read())

    BLUEPRINTS_DIR.mkdir(exist_ok=True)
    slug = _safe_filename(data.get("title", bp_id))
    cache_path = BLUEPRINTS_DIR / f"{bp_id}_{slug}.json"
    cache_path.write_text(json.dumps(data, indent=2))
    if not quiet:
        print(f"Cached to: {cache_path}", file=sys.stderr)

    # Update search index with any extra metadata we now have
    index = _load_search_index()
    index[bp_id] = {
        **index.get(bp_id, {}),
        "key": bp_id,
        "title": data.get("title", ""),
        "slug": slug,
        "upvotes": data.get("numberOfFavorites", index.get(bp_id, {}).get("upvotes")),
        "tags": data.get("tags", []),
        "description": data.get("descriptionMarkdown", "")[:300],
        "cache_file": cache_path.name,
    }
    _save_search_index(index)

    return data["blueprintString"], data


def analyse(bp: dict, verbose: bool = False):
    entities = bp.get("entities", [])
    label = bp.get("label", "(no label)")
    print(f"Label: {label}")
    print(f"Total entities: {len(entities)}")

    types = Counter(e["name"] for e in entities)
    print("\nEntity counts:")
    for name, count in sorted(types.items(), key=lambda x: -x[1]):
        print(f"  {name}: {count}")

    recipes = Counter(e["recipe"] for e in entities if e.get("recipe"))
    if recipes:
        print("\nRecipes:")
        for recipe, count in sorted(recipes.items(), key=lambda x: -x[1]):
            print(f"  {recipe}: {count}")

    if entities:
        xs = [e["position"]["x"] for e in entities]
        ys = [e["position"]["y"] for e in entities]
        w = max(xs) - min(xs) + 1
        h = max(ys) - min(ys) + 1
        print(f"\nBounding box: {w:.0f} x {h:.0f}  (x=[{min(xs):.1f},{max(xs):.1f}] y=[{min(ys):.1f},{max(ys):.1f}])")

    inserters = [e for e in entities if "inserter" in e["name"]]
    if inserters:
        print(f"\nInserters ({len(inserters)}):")
        for t in sorted(set(e["name"] for e in inserters)):
            subset = [e for e in inserters if e["name"] == t]
            dirs = Counter(DIR_NAMES.get(e.get("direction", 0), "?") for e in subset)
            print(f"  {t}: {len(subset)} — {dict(dirs)}")

    pipes = [e for e in entities if "pipe" in e["name"]]
    if pipes:
        print(f"\nPipes ({len(pipes)}):")
        for t in sorted(set(e["name"] for e in pipes)):
            print(f"  {t}: {sum(1 for e in pipes if e['name'] == t)}")

    if verbose and entities:
        ox = min(e["position"]["x"] for e in entities)
        oy = min(e["position"]["y"] for e in entities)
        print("\nAll entities (normalized):")
        for e in sorted(entities, key=lambda e: (e["position"]["y"], e["position"]["x"])):
            x = e["position"]["x"] - ox
            y = e["position"]["y"] - oy
            extras = []
            if e.get("recipe"):
                extras.append(f'recipe={e["recipe"]}')
            if e.get("direction"):
                extras.append(f'dir={DIR_NAMES.get(e["direction"], e["direction"])}')
            mods = [item["id"]["name"] for item in e.get("items", []) if "id" in item]
            if mods:
                extras.append(f"mods={mods}")
            suffix = " — " + ", ".join(extras) if extras else ""
            print(f"  ({x:5.1f},{y:5.1f}) {e['name']}{suffix}")


def render(bp_string: str, stem: str) -> str:
    """Render blueprint string to HTML. Returns output path."""
    sys.path.insert(0, str(Path(__file__).parent.parent))
    from src.visualize import visualize

    BLUEPRINTS_DIR.mkdir(exist_ok=True)
    out = str(BLUEPRINTS_DIR / f"{stem}.html")
    visualize(bp_string, output_path=out, open_browser=False)
    return out


def main():
    parser = argparse.ArgumentParser(description="Decode and analyse a Factorio blueprint string.")
    parser.add_argument("blueprint", nargs="?", help="Blueprint string or factorio.school URL (reads stdin if omitted)")
    parser.add_argument("-v", "--verbose", action="store_true", help="Dump all entities with positions")
    parser.add_argument("--json", action="store_true", help="Dump raw decoded JSON")
    parser.add_argument("--render", action="store_true", help="Render to HTML in scripts/blueprints/")
    parser.add_argument("--search", metavar="QUERY", help="Search factorio.school by title")
    parser.add_argument(
        "--recent", action="store_true", help="With --search: sort by most recent instead of most popular"
    )
    parser.add_argument("--fetch-all", metavar="N", nargs="?", const=0, type=int,
                        help="With --search: download all results, or top N if given")
    args = parser.parse_args()

    if args.search:
        # fetch_all: None=list only, 0=all, N=top N
        search_factorio_school(args.search, recent=args.recent, fetch_count=args.fetch_all)
        return

    meta = {}
    stem = "blueprint"

    if args.blueprint and args.blueprint.startswith("https://www.factorio.school/view/"):
        bp_id = args.blueprint.rstrip("/").split("/")[-1]
        s, meta = fetch_factorio_school(args.blueprint)
        slug = _safe_filename(meta.get("title", bp_id))
        stem = f"{bp_id}_{slug}"
        print(f"Title: {meta.get('title', '?')}")
        if meta.get("descriptionMarkdown"):
            print(f"Description: {meta['descriptionMarkdown'][:200]}")
    elif args.blueprint:
        s = args.blueprint
    else:
        print("Paste blueprint string and press Ctrl+D:", file=sys.stderr)
        s = sys.stdin.read().strip()

    decoded = decode(s)

    if args.json:
        print(json.dumps(decoded, indent=2))
        return

    if "blueprint" in decoded:
        analyse(decoded["blueprint"], verbose=args.verbose)
        if args.render:
            out = render(s, stem)
            print(f"\nRendered: {out}")
    elif "blueprint_book" in decoded:
        book = decoded["blueprint_book"]
        print(f"Blueprint book: {book.get('label','(no label)')} — {len(book.get('blueprints',[]))} blueprints")
        for i, entry in enumerate(book.get("blueprints", [])):
            print(f"\n=== Blueprint {i+1} ===")
            if "blueprint" in entry:
                analyse(entry["blueprint"], verbose=args.verbose)
        if args.render:
            out = render(s, stem)
            print(f"\nRendered: {out}")
    else:
        print("Unknown blueprint format:", list(decoded.keys()))


if __name__ == "__main__":
    main()
