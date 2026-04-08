#!/usr/bin/env python3
"""Analyze a corpus of community Factorio blueprints and export corpus.json for the web app.

Usage:
    # From a directory of .txt/.json/.bp files:
    uv run python scripts/analysis/mine_corpus.py <corpus_dir> [OPTIONS]

    # From stdin — one blueprint string per line (pipe from jq, etc.):
    jq -r '.blueprint_string' *.json | uv run python scripts/analysis/mine_corpus.py --stdin [OPTIONS]

    # Tab-separated name+string pairs from stdin:
    echo "my-factory\\t0eJy..." | uv run python scripts/analysis/mine_corpus.py --stdin [OPTIONS]

Options:
    --stdin             Read blueprint strings from stdin (one per line, or name<TAB>string)
    --out <path>        Output path for corpus.json (default: ./corpus.json)
    --filter-bus        Only include blueprints classified as bus layouts
    --product <name>    Filter by final_product (can be repeated)
    --min-machines N    Minimum machine count filter
    --workers N         Parallel workers (default: cpu_count)
    --stats-csv <path>  Also write stats CSV to this path
    --verbose           Show per-blueprint progress
"""

from __future__ import annotations

import argparse
import csv
import dataclasses
import json
import logging
import os
import sys
from concurrent.futures import ProcessPoolExecutor, as_completed
from pathlib import Path

# Allow running as a script
sys.path.insert(0, str(Path(__file__).parent.parent.parent))

from src.analysis import analyze_blueprint
from src.analysis.batch import aggregate_stats, load_blueprints, print_summary
from src.analysis.export import to_layout_result
from src.analysis.stats import BlueprintStats, extract_stats

logging.basicConfig(level=logging.INFO, format="%(levelname)s %(message)s")
logger = logging.getLogger(__name__)


def _analyze_one(args: tuple[str, str]) -> tuple[str, dict | None, BlueprintStats | None]:
    """Top-level worker function (must be picklable for ProcessPoolExecutor)."""
    name, bp_string = args
    try:
        graph = analyze_blueprint(bp_string)
        stats = extract_stats(graph)
        layout = to_layout_result(graph)
        return name, layout, stats
    except Exception as e:
        logger.warning("Failed %s: %s", name, e)
        return name, None, None


def _stats_to_dict(stats: BlueprintStats) -> dict:
    """Convert BlueprintStats to a JSON-serialisable dict."""
    d = dataclasses.asdict(stats)
    # throughput_estimates is already a plain dict; everything else is numeric/bool/str/list
    return d


def _agg_to_dict(agg) -> dict:
    """Flatten AggregateStats to a plain dict of key→median values."""
    result: dict = {
        "total": agg.count,
        "success": agg.success_count,
        "failure": agg.failure_count,
    }
    for field_name, dist in agg.distributions.items():
        result[f"mean_{field_name}"] = round(dist.mean, 3)
        result[f"median_{field_name}"] = round(dist.median, 3)
    return result


def _write_csv(results: list[tuple[str, BlueprintStats | None]], path: Path) -> None:
    """Write stats to a CSV file — one row per blueprint."""
    rows = [(name, s) for name, s in results if s is not None]
    if not rows:
        logger.warning("No successful analyses to write to CSV")
        return

    # Get field names from the first stats object
    sample = dataclasses.asdict(rows[0][1])
    fieldnames = ["source_name"] + list(sample.keys())

    with path.open("w", newline="", encoding="utf-8") as f:
        writer = csv.DictWriter(f, fieldnames=fieldnames)
        writer.writeheader()
        for name, stats in rows:
            row = {"source_name": name}
            d = dataclasses.asdict(stats)
            for k, v in d.items():
                # Serialise list/dict fields as JSON strings
                if isinstance(v, (list, dict)):
                    row[k] = json.dumps(v)
                else:
                    row[k] = v
            writer.writerow(row)

    logger.info("Wrote %d rows to %s", len(rows), path)


def _load_from_stdin() -> list[tuple[str, str]]:
    """Read blueprint strings from stdin.

    Accepts two formats (mixed):
    - Plain blueprint string: ``0eJy...``
    - Tab-separated name + string: ``my-factory\\t0eJy...``

    Blank lines and lines not starting with '0' are skipped.
    """
    results: list[tuple[str, str]] = []
    for i, line in enumerate(sys.stdin):
        line = line.rstrip("\n")
        if not line:
            continue
        if "\t" in line:
            parts = line.split("\t", 1)
            name, bp = parts[0].strip(), parts[1].strip()
        else:
            name = f"stdin_{i + 1:04d}"
            bp = line.strip()
        if bp.startswith("0") and len(bp) > 10:
            results.append((name, bp))
        else:
            logger.debug("Skipping non-blueprint line %d: %.40s…", i + 1, line)
    return results


def main() -> None:
    parser = argparse.ArgumentParser(description="Analyze a Factorio blueprint corpus and export corpus.json")
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("corpus_dir", nargs="?", help="Path to corpus directory or file")
    source.add_argument("--stdin", action="store_true", help="Read blueprint strings from stdin")
    parser.add_argument("--out", default="corpus.json", help="Output path for corpus.json")
    parser.add_argument("--filter-bus", action="store_true", help="Only include bus layout blueprints")
    parser.add_argument(
        "--product",
        action="append",
        metavar="ITEM",
        dest="products",
        help="Filter by final_product (repeatable)",
    )
    parser.add_argument("--min-machines", type=int, default=0, help="Minimum machine count")
    parser.add_argument(
        "--workers",
        type=int,
        default=os.cpu_count() or 4,
        help="Parallel worker processes",
    )
    parser.add_argument("--stats-csv", metavar="PATH", help="Also write stats CSV")
    parser.add_argument("--verbose", action="store_true")
    args = parser.parse_args()

    if args.verbose:
        logging.getLogger().setLevel(logging.DEBUG)

    # Load blueprints
    if args.stdin:
        logger.info("Reading blueprint strings from stdin …")
        blueprints = _load_from_stdin()
    else:
        logger.info("Loading blueprints from %s …", args.corpus_dir)
        blueprints = load_blueprints(args.corpus_dir)
    logger.info("Found %d blueprint(s)", len(blueprints))

    if not blueprints:
        logger.error("No blueprints found — check the path and file formats")
        sys.exit(1)

    # Analyze in parallel
    logger.info("Analyzing with %d worker(s) …", args.workers)
    triples: list[tuple[str, dict | None, BlueprintStats | None]] = []

    with ProcessPoolExecutor(max_workers=args.workers) as executor:
        futures = {executor.submit(_analyze_one, bp): bp[0] for bp in blueprints}
        for i, future in enumerate(as_completed(futures), 1):
            name, layout, stats = future.result()
            triples.append((name, layout, stats))
            if args.verbose or i % 50 == 0:
                logger.info("[%d/%d] %s", i, len(blueprints), name)

    # Apply filters
    products_filter = set(args.products) if args.products else None

    corpus_entries = []
    stats_results: list[tuple[str, BlueprintStats | None]] = []

    for name, layout, stats in triples:
        stats_results.append((name, stats))
        if layout is None or stats is None:
            continue
        if args.filter_bus and not stats.is_bus_layout:
            continue
        if products_filter and stats.final_product not in products_filter:
            continue
        if stats.machine_count < args.min_machines:
            continue

        corpus_entries.append(
            {
                "name": name,
                "stats": _stats_to_dict(stats),
                "layout": layout,
            }
        )

    logger.info(
        "%d blueprint(s) pass filters (of %d analyzed)",
        len(corpus_entries),
        sum(1 for _, s in stats_results if s is not None),
    )

    # Aggregate stats (unfiltered)
    agg = aggregate_stats(stats_results)
    print_summary(agg)

    # Write corpus.json
    out_path = Path(args.out)
    corpus = {
        "blueprints": corpus_entries,
        "aggregate": _agg_to_dict(agg),
    }
    out_path.write_text(json.dumps(corpus, separators=(",", ":")), encoding="utf-8")
    logger.info("Wrote corpus.json to %s (%d blueprints)", out_path, len(corpus_entries))

    # Optionally write CSV
    if args.stats_csv:
        _write_csv(stats_results, Path(args.stats_csv))


if __name__ == "__main__":
    main()
