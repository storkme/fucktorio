"""Gallery sweep: generate layouts with varied weight profiles for comparison.

Produces N HTML visualizations with different loss weight combinations
so you can visually compare how different tuning affects the layout.
"""

from __future__ import annotations

import itertools
from pathlib import Path

import numpy as np

from ..blueprint import build_blueprint
from ..models import SolverResult
from ..visualize import visualize
from .layout import ml_layout
from .loss import DEFAULT_WEIGHTS


def sweep(
    solver_result: SolverResult,
    n: int = 6,
    output_dir: str = "ml_sweep",
    label_prefix: str = "ML sweep",
) -> list[Path]:
    """Generate N layouts with varied weight profiles.

    Varies 'edge' and 'compact' weights while keeping 'overlap' fixed.
    Each layout is exported as an HTML visualization.

    Args:
        solver_result: Solved production chain.
        n: Number of weight combinations to try.
        output_dir: Directory for output HTML files.
        label_prefix: Prefix for blueprint labels.

    Returns:
        List of paths to generated HTML files.
    """
    out = Path(output_dir)
    out.mkdir(parents=True, exist_ok=True)

    # Generate weight profiles by varying edge and compact
    edge_values = np.linspace(0.5, 3.0, max(2, int(np.ceil(np.sqrt(n)))))
    compact_values = np.linspace(0.05, 0.5, max(2, int(np.ceil(np.sqrt(n)))))

    profiles: list[dict[str, float]] = []
    for edge_w, compact_w in itertools.product(edge_values, compact_values):
        profiles.append(
            {
                **DEFAULT_WEIGHTS,
                "edge": float(edge_w),
                "compact": float(compact_w),
            }
        )
        if len(profiles) >= n:
            break

    paths: list[Path] = []
    for i, weights in enumerate(profiles):
        tag = f"e{weights['edge']:.1f}_c{weights['compact']:.2f}"
        label = f"{label_prefix} [{tag}]"

        print(f"[{i + 1}/{len(profiles)}] weights: edge={weights['edge']:.1f}, compact={weights['compact']:.2f}")

        layout_result = ml_layout(solver_result, weights=weights)
        bp_string = build_blueprint(layout_result, label=label)

        filename = f"sweep_{i:02d}_{tag}.html"
        filepath = out / filename
        visualize(
            bp_string,
            solver_result=solver_result,
            open_browser=False,
            output_path=str(filepath),
        )
        paths.append(filepath)
        print(f"  → {filepath} ({len(layout_result.entities)} entities)")

    print(f"\nGenerated {len(paths)} layouts in {out}/")
    return paths
