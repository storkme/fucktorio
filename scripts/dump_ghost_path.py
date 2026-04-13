#!/usr/bin/env python3
"""Dump the full path of a ghost-routed spec."""

import base64
import gzip
import json
import sys


def decode_fls(path):
    with open(path, "rb") as f:
        raw = f.read()
    payload = base64.b64decode(raw[4:])
    return json.loads(gzip.decompress(payload))


def main():
    path = sys.argv[1]
    spec_filter = sys.argv[2]
    data = decode_fls(path)
    events = data.get("trace", {}).get("events", [])

    for e in events:
        if e.get("phase") != "GhostSpecRouted":
            continue
        d = e["data"]
        if spec_filter not in d["spec_key"]:
            continue
        tiles = [tuple(t) for t in d.get("tiles", [])]
        crossings = set(tuple(t) for t in d.get("crossing_tiles", []))
        print(f"{d['spec_key']}: {len(tiles)} tiles")
        for i, t in enumerate(tiles):
            marker = " X" if tuple(t) in crossings else ""
            print(f"  {i}: {t}{marker}")


if __name__ == "__main__":
    main()
