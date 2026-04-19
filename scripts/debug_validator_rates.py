#!/usr/bin/env python3
"""
Debug script to trace the lane_rate propagation in compute_lane_rates_impl
for the tier2_electronic_circuit_from_ore snapshot.

This simulates the Rust validator walker and finds where copper-plate rates
go to 0 when they should be non-zero.
"""

import json
import sys
import base64
import gzip
from collections import defaultdict, deque

SNAPSHOT_PATH = "crates/core/target/tmp/snapshot-tier2_electronic_circuit_from_ore.fls"
# The snapshot is regenerated on each test run.

def load_snapshot(path):
    with open(path, 'rb') as f:
        f.read(4)  # skip 4-byte header
        data = f.read()
    decoded = base64.b64decode(data)
    decompressed = gzip.decompress(decoded)
    return json.loads(decompressed)

d = load_snapshot(SNAPSHOT_PATH)
ents = d['layout']['entities']
by_pos = {(e.get('x', 0), e.get('y', 0)): e for e in ents}
dir_vec = {'North': (0, -1), 'South': (0, 1), 'East': (1, 0), 'West': (-1, 0)}


def is_surface_belt(n): return 'transport-belt' in n and 'underground' not in n
def is_ug_belt(n): return 'underground-belt' in n
def is_splitter(n): return 'splitter' in n
def is_belt(n): return any(s in n for s in ['transport-belt', 'underground-belt', 'splitter'])
def is_machine(n): return n in [
    'assembling-machine-1', 'assembling-machine-2', 'assembling-machine-3',
    'electric-furnace', 'stone-furnace', 'chemical-plant', 'oil-refinery'
]
def is_inserter(n): return n == 'inserter'


def splitter_second_tile(e):
    if e['direction'] in ['North', 'South']:
        return (e['x'] + 1, e['y'])
    else:
        return (e['x'], e['y'] + 1)


# --- Build belt structures ---
belt_dir_map = {}  # pos -> direction str
ug_output_tiles = set()
ug_input_dir = {}  # pos -> direction (input only)
splitter_sibling = {}
belt_carries = {}

for e in ents:
    name = e['name']
    io = e.get('io_type')
    pos = (e['x'], e['y'])
    direction = e.get('direction')
    if is_surface_belt(name):
        belt_dir_map[pos] = direction
        belt_carries[pos] = e.get('carries')
    elif is_ug_belt(name):
        if io == 'output':
            belt_dir_map[pos] = direction
            ug_output_tiles.add(pos)
            belt_carries[pos] = e.get('carries')
        elif io == 'input':
            ug_input_dir[pos] = direction
            belt_carries[pos] = e.get('carries')
    elif is_splitter(name):
        belt_dir_map[pos] = direction
        belt_carries[pos] = e.get('carries')
        sec = splitter_second_tile(e)
        belt_dir_map[sec] = direction
        belt_carries[sec] = e.get('carries')
        splitter_sibling[pos] = sec
        splitter_sibling[sec] = pos


# --- Build UG pairs ---
def build_ug_pairs():
    used_outputs = set()
    pairs = {}  # input -> output (and output -> input)
    for inp_pos in sorted(ug_input_dir.keys()):
        inp_d = ug_input_dir[inp_pos]
        dx, dy = dir_vec[inp_d]
        best_out = None
        best_dist = 99999
        for out_pos in ug_output_tiles:
            if out_pos in used_outputs:
                continue
            out_e = by_pos.get(out_pos)
            if not out_e or out_e.get('direction') != inp_d:
                continue
            rx = out_pos[0] - inp_pos[0]
            ry = out_pos[1] - inp_pos[1]
            if dx != 0:
                if ry != 0 or (rx > 0) != (dx > 0):
                    continue
                dist = abs(rx)
            else:
                if rx != 0 or (ry > 0) != (dy > 0):
                    continue
                dist = abs(ry)
            if dist > 1 and dist < best_dist:
                best_dist = dist
                best_out = out_pos
        if best_out:
            pairs[inp_pos] = best_out
            pairs[best_out] = inp_pos
            used_outputs.add(best_out)
    return pairs


ug_pairs = build_ug_pairs()
ug_output_to_input = {out: inp for inp, out in ug_pairs.items()
                       if inp in ug_input_dir and out in ug_output_tiles}

# --- Machine structures ---
machine_tile_set = set()
machine_by_tile = {}
machine_entity_map = {}

for e in ents:
    if is_machine(e['name']):
        pos = (e['x'], e['y'])
        machine_entity_map[pos] = e
        for dx in range(3):
            for dy in range(3):
                t = (e['x'] + dx, e['y'] + dy)
                machine_tile_set.add(t)
                machine_by_tile[t] = pos

solver = d.get('solver', {})
recipe_to_spec = {m['recipe']: m for m in solver.get('machines', [])}


def inserter_target_lane(ix, iy, dx, dy, belt_d):
    bdx, bdy = dir_vec[belt_d]
    left_dx, left_dy = -bdy, bdx
    dot = dx * left_dx + dy * left_dy
    return 'left' if dot > 0 else 'right'


# --- Lane injections ---
lane_injections = defaultdict(lambda: [0.0, 0.0])
for ins in ents:
    if not is_inserter(ins['name']):
        continue
    direction = ins['direction']
    dx, dy = dir_vec[direction]
    x, y = ins['x'], ins['y']
    reach = 1
    drop_pos = (x + dx * reach, y + dy * reach)
    pickup_pos = (x - dx * reach, y - dy * reach)
    if pickup_pos not in machine_tile_set or drop_pos not in belt_dir_map:
        continue
    mpos = machine_by_tile.get(pickup_pos)
    if mpos is None:
        continue
    me = machine_entity_map.get(mpos)
    if me is None:
        continue
    spec = recipe_to_spec.get(me.get('recipe', ''))
    if spec is None:
        continue
    carried = belt_carries.get(drop_pos)
    if carried is None:
        continue
    rate = next((o['rate'] for o in spec['outputs'] if o['item'] == carried), 0.0)
    if rate <= 0:
        continue
    belt_d = belt_dir_map[drop_pos]
    lane = inserter_target_lane(x, y, dx, dy, belt_d)
    if lane == 'left':
        lane_injections[drop_pos][0] += rate
    else:
        lane_injections[drop_pos][1] += rate

print("=== Lane injections for copper-plate ===")
cp_injections = {pos: rates for pos, rates in lane_injections.items()
                 if belt_carries.get(pos) == 'copper-plate'}
for pos, rates in sorted(cp_injections.items()):
    print(f"  {pos}: L={rates[0]:.3f} R={rates[1]:.3f}")
print(f"  Total tiles with injection: {len(cp_injections)}")
print(f"  Total injected: {sum(r[0]+r[1] for r in cp_injections.values()):.3f}/s")

# --- Build feeders ---
feeders = {}
for pos, belt_d in belt_dir_map.items():
    if pos in ug_output_tiles:
        continue
    dx, dy = dir_vec[belt_d]
    left_dx, left_dy = -dy, dx
    tile_feeders = []
    for ddx, ddy in [(1, 0), (-1, 0), (0, 1), (0, -1)]:
        nx, ny = pos[0] + ddx, pos[1] + ddy
        nb = (nx, ny)
        if nb not in belt_dir_map:
            continue
        nd = belt_dir_map[nb]
        ndx, ndy = dir_vec[nd]
        if (nx + ndx, ny + ndy) != pos:
            continue
        if nd == belt_d:
            ft = 0
        else:
            dot = (nx - pos[0]) * left_dx + (ny - pos[1]) * left_dy
            ft = 1 if dot > 0 else 2
        tile_feeders.append((nb, ft))
    if tile_feeders:
        feeders[pos] = tile_feeders

# --- In-degrees ---
in_degree = {pos: 0 for pos in belt_dir_map}
for pos, tile_feeders in feeders.items():
    in_degree[pos] = len(tile_feeders)

# Splitter unification
visited_pairs = set()
for a, b in splitter_sibling.items():
    key = (min(a, b), max(a, b))
    if key in visited_pairs:
        continue
    visited_pairs.add(key)
    total = in_degree.get(a, 0) + in_degree.get(b, 0)
    in_degree[a] = total
    in_degree[b] = total

# UG behind deps
behind_to_ug_outputs = defaultdict(list)
for ug_out in ug_output_tiles:
    paired_input = ug_output_to_input.get(ug_out)
    if paired_input is None:
        continue
    inp_d = ug_input_dir.get(paired_input)
    if inp_d is None:
        continue
    idx, idy = dir_vec[inp_d]
    behind = (paired_input[0] - idx, paired_input[1] - idy)
    if behind in belt_dir_map and behind != ug_out:
        behind_to_ug_outputs[behind].append(ug_out)
        in_degree[ug_out] = in_degree.get(ug_out, 0) + 1

# --- Print in_degrees for the critical copper-plate path ---
print("\n=== In-degrees for copper-plate balancer region (x=10-15, y=10-30) ===")
cp_tiles = {pos for pos in belt_dir_map if belt_carries.get(pos) == 'copper-plate'}
for pos in sorted(cp_tiles):
    if 10 <= pos[0] <= 15 and 10 <= pos[1] <= 30:
        feedlist = feeders.get(pos, [])
        ug_dep = behind_to_ug_outputs.get(pos, [])
        print(f"  {pos} dir={belt_dir_map[pos]:5s} in_deg={in_degree.get(pos,0):2d} "
              f"feeders={feedlist} ug_deps={ug_dep} ug_out={pos in ug_output_tiles}")

# --- Initialize lane_rates ---
lane_rates = {pos: list(lane_injections.get(pos, [0.0, 0.0])) for pos in belt_dir_map}

# --- Run topological sort with debug tracing for copper-plate ---
processed = set()
splitter_input_ready = set()
splitter_retries = defaultdict(int)
MAX_RETRIES = 3
queue = deque(pos for pos, deg in in_degree.items() if deg <= 0)

TRACE_ITEMS = {'copper-plate'}
TRACE_REGION = lambda pos: 10 <= pos[0] <= 18 and 10 <= pos[1] <= 40

def do_propagate(tile):
    if tile not in belt_dir_map:
        return
    d = belt_dir_map[tile]
    dx, dy = dir_vec[d]
    downstream = (tile[0] + dx, tile[1] + dy)
    if downstream not in belt_dir_map:
        if TRACE_REGION(tile) and belt_carries.get(tile) in TRACE_ITEMS:
            print(f"    do_propagate({tile}): downstream {downstream} NOT in belt_dir_map, skipping")
        return

    my_rates = lane_rates.get(tile, [0.0, 0.0])
    ds_d = belt_dir_map[downstream]
    ds_dx, ds_dy = dir_vec[ds_d]
    left_dx, left_dy = -ds_dy, ds_dx

    ds_rates = lane_rates.setdefault(downstream, [0.0, 0.0])

    if d == ds_d:
        ds_rates[0] += my_rates[0]
        ds_rates[1] += my_rates[1]
    else:
        behind_ds = (downstream[0] - ds_dx, downstream[1] - ds_dy)
        if tile == behind_ds:
            ds_rates[0] += my_rates[0]
            ds_rates[1] += my_rates[1]
        else:
            ds_feeders = feeders.get(downstream, [])
            has_straight = any(ft == 0 for _, ft in ds_feeders)
            if has_straight:
                rel_x = tile[0] - downstream[0]
                rel_y = tile[1] - downstream[1]
                dot = rel_x * left_dx + rel_y * left_dy
                total = my_rates[0] + my_rates[1]
                if dot > 0:
                    ds_rates[0] += total
                else:
                    ds_rates[1] += total
            else:
                ddx, ddy = dir_vec[d]
                cross = ddx * ds_dy - ddy * ds_dx
                if cross > 0:
                    ds_rates[1] += my_rates[0]
                    ds_rates[0] += my_rates[1]
                else:
                    ds_rates[0] += my_rates[0]
                    ds_rates[1] += my_rates[1]

    in_degree[downstream] = in_degree.get(downstream, 0) - 1
    if in_degree[downstream] <= 0:
        queue.append(downstream)

    sib = splitter_sibling.get(downstream)
    if sib:
        in_degree[sib] = in_degree.get(sib, 0) - 1
        if in_degree[sib] <= 0:
            queue.append(sib)


def notify_ug_deps(tile):
    for ug_out in behind_to_ug_outputs.get(tile, []):
        in_degree[ug_out] = in_degree.get(ug_out, 0) - 1
        if in_degree[ug_out] <= 0:
            queue.append(ug_out)


step = 0
while queue:
    pos = queue.popleft()
    if pos in processed:
        continue

    step += 1
    trace = TRACE_REGION(pos) and belt_carries.get(pos) in TRACE_ITEMS

    # UG output: inherit from behind paired input
    if pos in ug_output_tiles:
        paired_input = ug_output_to_input.get(pos)
        if paired_input:
            inp_d = ug_input_dir.get(paired_input)
            if inp_d:
                idx, idy = dir_vec[inp_d]
                behind = (paired_input[0] - idx, paired_input[1] - idy)
                if behind in lane_rates:
                    behind_rates = lane_rates[behind]
                    cur = lane_rates.setdefault(pos, [0.0, 0.0])
                    cur[0] += behind_rates[0]
                    cur[1] += behind_rates[1]
                    if trace:
                        print(f"  UG_OUT {pos}: inherited from behind={behind} rates={behind_rates} -> now {cur}")
                else:
                    if trace:
                        print(f"  UG_OUT {pos}: behind={behind} NOT in lane_rates!")

    # Splitter: wait for sibling
    sib = splitter_sibling.get(pos)
    if sib:
        if sib not in processed:
            splitter_input_ready.add(pos)
            if sib not in splitter_input_ready:
                retry = splitter_retries[pos]
                if retry < MAX_RETRIES:
                    splitter_retries[pos] += 1
                    queue.append(pos)
                    continue
                # gave up - process with current rates
                if trace:
                    print(f"  SPLITTER {pos}: GAVE UP waiting for sibling {sib}, processing with rates {lane_rates.get(pos)}")
                processed.add(pos)
                do_propagate(pos)
                notify_ug_deps(pos)
                continue
            else:
                # Both ready - average
                pos_rates = lane_rates.get(pos, [0.0, 0.0])
                sib_rates = lane_rates.get(sib, [0.0, 0.0])
                total_l = pos_rates[0] + sib_rates[0]
                total_r = pos_rates[1] + sib_rates[1]
                if trace or (TRACE_REGION(sib) and belt_carries.get(sib) in TRACE_ITEMS):
                    print(f"  SPLITTER_AVG {pos}+{sib}: pos_rates={pos_rates} sib_rates={sib_rates} -> avg=[{total_l/2:.3f},{total_r/2:.3f}]")
                for tile in [pos, sib]:
                    r = lane_rates.setdefault(tile, [0.0, 0.0])
                    r[0] = total_l / 2.0
                    r[1] = total_r / 2.0
                for tile in [sib, pos]:
                    processed.add(tile)
                    do_propagate(tile)
                    notify_ug_deps(tile)
                continue

    processed.add(pos)
    if trace:
        print(f"  PROCESS {pos} dir={belt_dir_map.get(pos,'?'):5s} rates={lane_rates.get(pos,[0,0])} in_deg_was={in_degree.get(pos,0)}")
    do_propagate(pos)
    notify_ug_deps(pos)

# --- Report on the copper-plate path ---
print("\n=== Final lane_rates for copper-plate path (x=10-20, y=6-40) ===")
for pos in sorted(belt_dir_map.keys()):
    if belt_carries.get(pos) == 'copper-plate' and 10 <= pos[0] <= 20 and 6 <= pos[1] <= 40:
        rates = lane_rates.get(pos, [0.0, 0.0])
        in_q = pos in processed
        print(f"  {pos} dir={belt_dir_map.get(pos,'?'):5s} rates=[{rates[0]:.3f},{rates[1]:.3f}] processed={in_q}")

print(f"\n=== Key failing tiles ===")
for pos in [(11, 36), (16, 36), (17, 36), (20, 36), (23, 36)]:
    rates = lane_rates.get(pos, [0, 0])
    print(f"  {pos}: rates={rates} processed={pos in processed}")

print(f"\n=== Unprocessed tiles with copper-plate (x=8-18, y=10-42) ===")
for pos in sorted(belt_dir_map.keys()):
    if pos not in processed and belt_carries.get(pos) == 'copper-plate':
        if 8 <= pos[0] <= 18 and 10 <= pos[1] <= 42:
            print(f"  UNPROCESSED: {pos} dir={belt_dir_map.get(pos,'?')} in_deg={in_degree.get(pos,0)}")
