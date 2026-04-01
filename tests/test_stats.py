"""Tests for blueprint statistics extraction."""

import pytest

from src.analysis import analyze_blueprint
from src.analysis.stats import detect_final_product, estimate_throughput, extract_stats
from src.blueprint import build_blueprint

# The processing-unit blueprint from earlier testing
PROCESSING_UNIT_BP = (
    "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"
)


class TestFinalProductDetection:
    def test_iron_gear_wheel(self, iron_gear_solver_result, iron_gear_layout):
        bp_str = build_blueprint(iron_gear_layout, label="test")
        graph = analyze_blueprint(bp_str)
        product = detect_final_product(graph)
        assert product == "iron-gear-wheel"

    def test_processing_unit(self):
        graph = analyze_blueprint(PROCESSING_UNIT_BP)
        product = detect_final_product(graph)
        assert product == "processing-unit"


class TestThroughputEstimates:
    def test_iron_gear_wheel(self, iron_gear_solver_result, iron_gear_layout):
        bp_str = build_blueprint(iron_gear_layout, label="test")
        graph = analyze_blueprint(bp_str)
        throughput = estimate_throughput(graph)
        assert "iron-gear-wheel" in throughput
        assert throughput["iron-gear-wheel"] > 0

    def test_processing_unit(self):
        graph = analyze_blueprint(PROCESSING_UNIT_BP)
        throughput = estimate_throughput(graph)
        assert "processing-unit" in throughput
        assert throughput["processing-unit"] > 0


class TestExtractStats:
    def test_iron_gear_roundtrip(self, iron_gear_solver_result, iron_gear_layout):
        bp_str = build_blueprint(iron_gear_layout, label="test")
        graph = analyze_blueprint(bp_str)
        stats = extract_stats(graph)

        assert stats.final_product == "iron-gear-wheel"
        assert stats.machine_count > 0
        assert stats.belt_tiles > 0
        assert stats.inserter_count > 0
        assert stats.belts_per_machine > 0
        assert stats.inserters_per_machine > 0
        assert stats.bbox_area > 0
        assert stats.density > 0
        assert stats.machines_without_inserters == 0
        assert "iron-gear-wheel" in stats.throughput_estimates

        print("\n--- Iron Gear Wheel Stats ---")
        print(f"  Final product: {stats.final_product}")
        print(f"  Machines: {stats.machine_count}")
        print(f"  Belts/machine: {stats.belts_per_machine:.1f}")
        print(f"  Inserters/machine: {stats.inserters_per_machine:.1f}")
        print(f"  Density: {stats.density:.2f}")
        print(f"  Throughput: {stats.throughput_estimates}")

    def test_processing_unit(self):
        graph = analyze_blueprint(PROCESSING_UNIT_BP)
        stats = extract_stats(graph)

        assert stats.final_product == "processing-unit"
        assert stats.machine_count == 6
        assert stats.recipe_count == 1  # all machines make processing-unit
        assert stats.beacon_count == 22
        assert stats.belt_tiles > 0
        assert stats.pipe_tiles > 0
        assert stats.belts_per_machine == pytest.approx(14.8, abs=1.0)
        assert stats.pipes_per_machine == pytest.approx(5.2, abs=1.0)
        assert stats.beacons_per_machine == pytest.approx(3.7, abs=0.5)
        assert stats.poles_per_machine == pytest.approx(2.3, abs=0.5)

        # Machine gaps should be 1 (adjacent machines with 1-tile gap)
        assert len(stats.machine_gaps) > 0
        assert 1 in stats.machine_gaps

        print("\n--- Processing Unit Stats ---")
        print(f"  Final product: {stats.final_product}")
        print(f"  Machines: {stats.machine_count}, Recipes: {stats.recipe_count}")
        print(f"  Belts/machine: {stats.belts_per_machine:.1f}")
        print(f"  Pipes/machine: {stats.pipes_per_machine:.1f}")
        print(f"  Inserters/machine: {stats.inserters_per_machine:.1f}")
        print(f"  Beacons/machine: {stats.beacons_per_machine:.1f}")
        print(f"  Poles/machine: {stats.poles_per_machine:.1f}")
        print(f"  Bbox: {stats.bbox_width}x{stats.bbox_height} ({stats.bbox_area} tiles)")
        print(f"  Density: {stats.density:.2f}")
        print(f"  Machine gaps: {stats.machine_gaps}")
        print(f"  Belt networks: {stats.belt_networks}")
        print(f"  Avg turn density: {stats.avg_turn_density:.2f}")
        print(f"  Avg UG ratio: {stats.avg_underground_ratio:.1%}")
        print(f"  Networks labeled: {stats.networks_labeled}/{stats.networks_labeled + stats.networks_unlabeled}")
        print(f"  Orphan networks: {stats.orphan_networks}")
        print(f"  Throughput: {stats.throughput_estimates}")
