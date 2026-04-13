#!/usr/bin/env python3
"""Analyze ghost routing crossings from a .fls snapshot."""
import base64
import gzip
import json
import sys
from collections import defaultdict


def decode_fls(path):
    with open(path, "rb") as f:
        raw = f.read()
    # Skip 4-byte length prefix, base64 decode, gunzip
    payload = base64.b64decode(raw[4:])
    return json.loads(gzip.decompress(payload))


def main(path):
    data = decode_fls(path)
    events = data.get("trace", {}).get("events", [])

    routed = {e["data"]["spec_key"]: e["data"] for e in events if e.get("phase") == "GhostSpecRouted"}
    print(f"Total routed specs: {len(routed)}")

    by_type = defaultdict(int)
    for k in routed:
        by_type[k.split(":")[0]] += 1
    print("Specs by type:", dict(by_type))
    print()

    all_crossing_tiles = set()
    crossings_per_spec = {}
    for k, d in routed.items():
        ct = [tuple(t) for t in d.get("crossing_tiles", [])]
        crossings_per_spec[k] = ct
        all_crossing_tiles.update(ct)
    print(f"Unique crossing tiles: {len(all_crossing_tiles)}")

    tile_to_specs = defaultdict(list)
    for k, tiles in crossings_per_spec.items():
        for t in tiles:
            tile_to_specs[t].append(k)

    hist = defaultdict(int)
    for _t, specs in tile_to_specs.items():
        hist[len(specs)] += 1
    print("Tile spec-count histogram:", dict(hist))
    print()

    # Horizontal runs of adjacent crossings on same y
    by_y = defaultdict(list)
    for t in all_crossing_tiles:
        by_y[t[1]].append(t[0])

    runs_by_y = {}
    for y, xs in by_y.items():
        xs_sorted = sorted(xs)
        runs = []
        cur = [xs_sorted[0]]
        for x in xs_sorted[1:]:
            if x == cur[-1] + 1:
                cur.append(x)
            else:
                runs.append(cur[:])
                cur = [x]
        runs.append(cur)
        runs_by_y[y] = runs

    # Count run lengths
    run_len_hist = defaultdict(int)
    total_runs = 0
    for _y, runs in runs_by_y.items():
        for r in runs:
            run_len_hist[len(r)] += 1
            total_runs += 1
    print(f"Horizontal-run length distribution ({total_runs} total runs):")
    for length in sorted(run_len_hist.keys()):
        print(f"  length={length}: {run_len_hist[length]} runs")
    print()

    # Show the long runs and the horizontals involved
    print("Runs with length >= 3 (the hard cases):")
    long_runs = []
    for y, runs in runs_by_y.items():
        for r in runs:
            if len(r) >= 3:
                long_runs.append((y, r))
    long_runs.sort(key=lambda v: (-len(v[1]), v[0]))
    for y, run in long_runs[:20]:
        specs_here = set()
        for x in run:
            specs_here.update(tile_to_specs[(x, y)])
        trunks = sorted([s for s in specs_here if s.startswith("trunk:")])
        horiz = sorted([s for s in specs_here if not s.startswith("trunk:")])
        print(f"  y={y} x={run[0]}..{run[-1]} ({len(run)} tiles)")
        print(f"    horiz specs ({len(horiz)}): {horiz}")
        print(f"    trunks ({len(trunks)}): {trunks[:4]}{' ...' if len(trunks) > 4 else ''}")

    print()
    # Total crossing tiles involving 2 specs (per-tile perpendicular case)
    # vs more (multi-spec conflict)
    per_tile_2 = sum(1 for t, specs in tile_to_specs.items() if len(specs) == 2)
    multi = sum(1 for t, specs in tile_to_specs.items() if len(specs) > 2)
    print(f"Tiles with exactly 2 specs: {per_tile_2} (per-tile template candidates)")
    print(f"Tiles with 3+ specs: {multi} (multi-spec conflicts)")


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "crates/core/target/tmp/snapshot-tier4_ghost.fls")
