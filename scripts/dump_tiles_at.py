#!/usr/bin/env python3
"""Dump all entities at a y range."""

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
    y_min = int(sys.argv[2])
    y_max = int(sys.argv[3]) if len(sys.argv) > 3 else y_min
    x_min = int(sys.argv[4]) if len(sys.argv) > 4 else 0
    x_max = int(sys.argv[5]) if len(sys.argv) > 5 else 50
    data = decode_fls(path)
    ents = data.get("layout", {}).get("entities", [])

    for e in ents:
        if y_min <= e["y"] <= y_max and x_min <= e["x"] <= x_max:
            seg = e.get("segment_id", "")
            dir_ = e.get("direction", "")
            print(f"  ({e['x']},{e['y']}) {e['name']:<25} {dir_:<6} {seg}")


if __name__ == "__main__":
    main()
