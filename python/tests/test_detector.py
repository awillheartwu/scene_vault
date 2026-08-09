import unittest
from pathlib import Path

from scene_vault_ai.vision.config import YuNetConfig
from scene_vault_ai.vision.detector import (
    _clamp_face,
    _resolve_yunet_factory,
    select_primary_face,
)
from scene_vault_ai.vision.types import FaceBox


class DetectorScoringTests(unittest.TestCase):
    def setUp(self) -> None:
        self.config = YuNetConfig(model_path=Path("unused.onnx"))

    def test_prefers_a_clear_central_face_over_an_edge_face(self) -> None:
        edge_face = FaceBox(
            x=0,
            y=120,
            width=260,
            height=260,
            confidence=0.99,
        )
        central_face = FaceBox(
            x=430,
            y=220,
            width=140,
            height=140,
            confidence=0.85,
        )

        selected = select_primary_face(
            [edge_face, central_face],
            image_width=1000,
            image_height=600,
            config=self.config,
            sharpness_values=[30.0, 180.0],
        )

        self.assertEqual(selected, central_face)

    def test_sharpness_can_break_otherwise_equal_candidates(self) -> None:
        left = FaceBox(300, 230, 120, 120, 0.9)
        right = FaceBox(580, 230, 120, 120, 0.9)

        selected = select_primary_face(
            [left, right],
            image_width=1000,
            image_height=600,
            config=self.config,
            sharpness_values=[20.0, 240.0],
        )

        self.assertEqual(selected, right)

    def test_clamps_faces_to_the_image_and_rejects_invalid_values(self) -> None:
        face = _clamp_face(
            x=-10.5,
            y=-5.0,
            width=40.0,
            height=30.0,
            confidence=1.5,
            image_width=100,
            image_height=100,
        )

        self.assertEqual(face, FaceBox(0, 0, 30, 25, 1.0))
        self.assertIsNone(
            _clamp_face(
                x=110,
                y=110,
                width=20,
                height=20,
                confidence=0.5,
                image_width=100,
                image_height=100,
            )
        )

    def test_supports_both_opencv_yunet_factory_shapes(self) -> None:
        class ModernFactory:
            @staticmethod
            def create(*_args: object) -> str:
                return "modern"

        class ModernCv2:
            FaceDetectorYN = ModernFactory

        class LegacyCv2:
            @staticmethod
            def FaceDetectorYN_create(*_args: object) -> str:
                return "legacy"

        self.assertEqual(_resolve_yunet_factory(ModernCv2)(), "modern")
        self.assertEqual(_resolve_yunet_factory(LegacyCv2)(), "legacy")


if __name__ == "__main__":
    unittest.main()
