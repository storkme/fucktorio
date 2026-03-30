"""Tests for the blueprint exporter."""

from draftsman.blueprintable import get_blueprintable_from_string

from src.blueprint import build_blueprint
from src.models import EntityDirection, LayoutResult, PlacedEntity


class TestBlueprint:
    def test_valid_string(self):
        lr = LayoutResult(
            entities=[
                PlacedEntity(name="assembling-machine-3", x=0, y=0, recipe="iron-gear-wheel"),
                PlacedEntity(name="inserter", x=1, y=-1, direction=EntityDirection.SOUTH),
                PlacedEntity(name="transport-belt", x=1, y=-2, direction=EntityDirection.EAST),
            ],
            width=3,
            height=5,
        )
        bp_str = build_blueprint(lr, label="test")
        assert isinstance(bp_str, str)
        assert len(bp_str) > 10

        # Re-import should succeed
        bp = get_blueprintable_from_string(bp_str)
        assert len(bp.entities) == 3

    def test_roundtrip(self):
        lr = LayoutResult(
            entities=[
                PlacedEntity(name="transport-belt", x=0, y=0, direction=EntityDirection.EAST),
                PlacedEntity(name="transport-belt", x=1, y=0, direction=EntityDirection.EAST),
            ],
            width=2,
            height=1,
        )
        bp_str = build_blueprint(lr)
        bp = get_blueprintable_from_string(bp_str)
        assert bp.label == "Generated Factory"
        names = [e.name for e in bp.entities]
        assert names.count("transport-belt") == 2

    def test_end_to_end(self, viz):
        """Full pipeline produces a re-importable blueprint."""
        from src.pipeline import produce

        bp_str = produce(
            "electronic-circuit",
            rate=30,
            inputs=["iron-plate", "copper-plate"],
        )
        viz(bp_str, "electronic-circuit-30s")
        bp = get_blueprintable_from_string(bp_str)
        assert len(bp.entities) > 50

        # Check assemblers have recipes set
        asms = [e for e in bp.entities if e.name == "assembling-machine-3"]
        assert len(asms) > 0
        for asm in asms:
            assert asm.recipe is not None

