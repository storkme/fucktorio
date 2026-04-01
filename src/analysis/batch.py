"""Batch blueprint analysis: load, analyze, and aggregate statistics."""

from __future__ import annotations

import json
import logging
import statistics
from dataclasses import dataclass, field, fields
from pathlib import Path

from . import analyze_blueprint
from .stats import BlueprintStats, extract_stats

logger = logging.getLogger(__name__)

_BLUEPRINT_EXTENSIONS = {".txt", ".blueprint", ".bp", ".json"}


@dataclass
class DistributionStats:
    """Summary statistics for a single metric across blueprints."""

    count: int = 0
    mean: float = 0.0
    median: float = 0.0
    min: float = 0.0
    max: float = 0.0
    stdev: float = 0.0


@dataclass
class AggregateStats:
    """Aggregated statistics across a collection of blueprints."""

    count: int = 0
    success_count: int = 0
    failure_count: int = 0
    distributions: dict[str, DistributionStats] = field(default_factory=dict)
    by_product: dict[str, list[BlueprintStats]] = field(default_factory=dict)


def load_blueprints(path: str) -> list[tuple[str, str]]:
    """Load blueprint strings from a file or directory.

    Handles:
    - Directory: scan for .txt, .blueprint, .bp, .json files
    - JSON file: array of strings, array of objects with "blueprint_string" key,
      or a single object with a "blueprints" array
    - Text file: one blueprint string per line, or a single string

    Returns list of (source_name, blueprint_string) tuples.
    """
    p = Path(path)

    if p.is_dir():
        return _load_directory(p)
    elif p.is_file():
        if p.suffix == ".json":
            return _load_json_file(p)
        else:
            return _load_text_file(p)
    else:
        logger.error("Path does not exist: %s", path)
        return []


def _load_directory(dirpath: Path) -> list[tuple[str, str]]:
    """Scan a directory for blueprint files."""
    results: list[tuple[str, str]] = []
    for f in sorted(dirpath.iterdir()):
        if f.is_file() and f.suffix in _BLUEPRINT_EXTENSIONS:
            try:
                loaded = load_blueprints(str(f))
                results.extend(loaded)
            except Exception as e:
                logger.warning("Failed to load %s: %s", f.name, e)
    return results


def _load_json_file(filepath: Path) -> list[tuple[str, str]]:
    """Load blueprints from a JSON file."""
    text = filepath.read_text(encoding="utf-8").strip()
    data = json.loads(text)
    name = filepath.stem

    if isinstance(data, list):
        results = []
        for i, item in enumerate(data):
            if isinstance(item, str) and item.startswith("0"):
                results.append((f"{name}[{i}]", item))
            elif isinstance(item, dict):
                bp_str = _extract_bp_string(item)
                if bp_str:
                    item_name = item.get("name", item.get("label", f"{name}[{i}]"))
                    results.append((str(item_name), bp_str))
        return results

    if isinstance(data, dict):
        # Object with "blueprints" array
        if "blueprints" in data:
            return _load_json_file_from_data(data["blueprints"], name)
        # Single object with blueprint string
        bp_str = _extract_bp_string(data)
        if bp_str:
            return [(name, bp_str)]

    return []


def _load_json_file_from_data(data: list, name: str) -> list[tuple[str, str]]:
    """Load from a list of blueprint entries."""
    results = []
    for i, item in enumerate(data):
        if isinstance(item, str) and item.startswith("0"):
            results.append((f"{name}[{i}]", item))
        elif isinstance(item, dict):
            bp_str = _extract_bp_string(item)
            if bp_str:
                item_name = item.get("name", item.get("label", f"{name}[{i}]"))
                results.append((str(item_name), bp_str))
    return results


def _extract_bp_string(obj: dict) -> str | None:
    """Extract a blueprint string from a JSON object, trying common field names."""
    for key in ("blueprint_string", "blueprint", "string", "bp", "data"):
        val = obj.get(key)
        if isinstance(val, str) and val.startswith("0"):
            return val
    return None


def _load_text_file(filepath: Path) -> list[tuple[str, str]]:
    """Load blueprints from a text file (one per line or single string)."""
    text = filepath.read_text(encoding="utf-8").strip()
    name = filepath.stem

    if "\n" in text:
        # Multiple lines — one blueprint per line
        results = []
        for i, line in enumerate(text.splitlines()):
            line = line.strip()
            if line.startswith("0") and len(line) > 10:
                results.append((f"{name}[{i}]", line))
        return results

    # Single string
    if text.startswith("0") and len(text) > 10:
        return [(name, text)]

    return []


def analyze_batch(
    blueprints: list[tuple[str, str]],
) -> list[tuple[str, BlueprintStats | None]]:
    """Analyze a batch of blueprints, returning stats for each.

    Failures are logged and returned as None.
    """
    results: list[tuple[str, BlueprintStats | None]] = []

    for name, bp_string in blueprints:
        try:
            graph = analyze_blueprint(bp_string)
            stats = extract_stats(graph)
            results.append((name, stats))
        except Exception as e:
            logger.warning("Failed to analyze %s: %s", name, e)
            results.append((name, None))

    return results


def aggregate_stats(results: list[tuple[str, BlueprintStats | None]]) -> AggregateStats:
    """Compute aggregate statistics across a batch of analyzed blueprints."""
    agg = AggregateStats()
    agg.count = len(results)
    successful = [(name, s) for name, s in results if s is not None]
    agg.success_count = len(successful)
    agg.failure_count = agg.count - agg.success_count

    if not successful:
        return agg

    stats_list = [s for _, s in successful]

    # Group by product
    for s in stats_list:
        product = s.final_product or "(unknown)"
        agg.by_product.setdefault(product, []).append(s)

    # Compute distributions for numeric fields
    numeric_fields = [
        "machine_count",
        "belt_tiles",
        "pipe_tiles",
        "inserter_count",
        "beacon_count",
        "pole_count",
        "bbox_area",
        "density",
        "belt_networks",
        "avg_belt_path_length",
        "avg_turn_density",
        "avg_underground_ratio",
        "pipe_networks",
        "avg_pipe_path_length",
        "input_inserters_per_machine",
        "output_inserters_per_machine",
        "belts_per_machine",
        "pipes_per_machine",
        "inserters_per_machine",
        "beacons_per_machine",
        "poles_per_machine",
        "machines_without_inserters",
        "orphan_networks",
    ]

    for field_name in numeric_fields:
        values = [getattr(s, field_name) for s in stats_list]
        values = [v for v in values if v is not None]
        if not values:
            continue
        agg.distributions[field_name] = DistributionStats(
            count=len(values),
            mean=statistics.mean(values),
            median=statistics.median(values),
            min=min(values),
            max=max(values),
            stdev=statistics.stdev(values) if len(values) > 1 else 0.0,
        )

    return agg


def print_summary(agg: AggregateStats) -> None:
    """Print a formatted summary of aggregate statistics."""
    print(f"\n{'=' * 60}")
    print(f"BLUEPRINT ANALYSIS SUMMARY")
    print(f"{'=' * 60}")
    print(f"  Analyzed: {agg.count} blueprints ({agg.success_count} ok, {agg.failure_count} failed)")

    if agg.by_product:
        print(f"\n--- By Final Product ---")
        for product, stats_list in sorted(agg.by_product.items(), key=lambda x: -len(x[1])):
            print(f"  {product}: {len(stats_list)} blueprints")

    if agg.distributions:
        # Key ratios that map to layout engine parameters
        key_metrics = [
            ("belts_per_machine", "Belts/machine"),
            ("pipes_per_machine", "Pipes/machine"),
            ("inserters_per_machine", "Inserters/machine"),
            ("beacons_per_machine", "Beacons/machine"),
            ("poles_per_machine", "Poles/machine"),
            ("density", "Entity density"),
            ("avg_turn_density", "Belt turn density"),
            ("avg_underground_ratio", "Underground ratio"),
            ("input_inserters_per_machine", "Input inserters/machine"),
            ("output_inserters_per_machine", "Output inserters/machine"),
        ]

        print(f"\n--- Key Ratios (for tuning layout engine) ---")
        print(f"  {'Metric':<30} {'Mean':>8} {'Median':>8} {'Min':>8} {'Max':>8} {'StdDev':>8}")
        print(f"  {'-' * 78}")
        for field_name, label in key_metrics:
            d = agg.distributions.get(field_name)
            if d is None:
                continue
            print(f"  {label:<30} {d.mean:>8.2f} {d.median:>8.2f} {d.min:>8.2f} {d.max:>8.2f} {d.stdev:>8.2f}")

        print(f"\n--- All Distributions ---")
        for field_name, d in sorted(agg.distributions.items()):
            print(f"  {field_name}: mean={d.mean:.2f}, median={d.median:.2f}, range=[{d.min:.1f}, {d.max:.1f}]")
