#!/usr/bin/env python3
"""Inspect a specific ghost-routed spec from a .fls snapshot."""
import sys
import json
import gzip
import base64


def decode_fls(path):
    with open(path, "rb") as f:
        raw = f.read()
    payload = base64.b64decode(raw[4:])
    return json.loads(gzip.decompress(payload))


def main():
    path = sys.argv[1] if len(sys.argv) > 1 else "crates/core/target/tmp/snapshot-tier4_ghost.fls"
    spec_filter = sys.argv[2] if len(sys.argv) > 2 else None
    data = decode_fls(path)
    events = data.get("trace", {}).get("events", [])

    for e in events:
        if e.get("phase") != "GhostSpecRouted":
            continue
        d = e["data"]
        if spec_filter and spec_filter not in d["spec_key"]:
            continue
        tiles = [tuple(t) for t in d.get("tiles", [])]
        crossings = [tuple(t) for t in d.get("crossing_tiles", [])]
        print(f"{d['spec_key']}")
        print(f"  path_len={d['path_len']}, turns={d['turns']}, crossings={len(crossings)}")
        if tiles:
            print(f"  start={tiles[0]}, end={tiles[-1]}")
            xs = [t[0] for t in tiles]
            ys = [t[1] for t in tiles]
            print(f"  x range: {min(xs)}..{max(xs)}, y range: {min(ys)}..{max(ys)}")
        if crossings:
            print(f"  crossing tiles: {crossings[:20]}{'...' if len(crossings) > 20 else ''}")
        print()


if __name__ == "__main__":
    main()
