import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock

from scene_vault_ai.errors import InvalidPayloadError
from scene_vault_ai.vision.config import BigFacePolicy, ProcessingRequest
from scene_vault_ai.vision.detector import map_scaled_faces, prefer_scaled_faces
from scene_vault_ai.vision.processor import ScreenshotProcessor, dependencies_available
from scene_vault_ai.vision.types import FaceBox


def payload():
    root = Path(tempfile.gettempdir())
    return {
        "inputPath": str(root / "source.png"),
        "yunetModelPath": str(root / "yunet.onnx"),
        "annotate": False,
        "cropAvatar": False,
    }


class BigFaceValidationTests(unittest.TestCase):
    def test_missing_group_keeps_historical_behaviour(self):
        policy = ProcessingRequest.from_payload(payload()).big_face_policy
        self.assertEqual(policy.mode, "off")
        self.assertEqual(policy.min_image_side, 1600)
        self.assertEqual(policy.min_face_size, 600)

    def test_null_group_keeps_historical_behaviour(self):
        request = ProcessingRequest.from_payload({**payload(), "bigFace": None})
        self.assertEqual(request.big_face_policy.mode, "off")

    def test_group_defaults_to_auto(self):
        request = ProcessingRequest.from_payload({**payload(), "bigFace": {}})
        self.assertEqual(request.big_face_policy.mode, "auto")
        self.assertEqual(request.big_face_policy.min_image_side, 1600)
        self.assertEqual(request.big_face_policy.min_face_size, 600)

    def test_accepts_explicit_values(self):
        policy = ProcessingRequest.from_payload({
            **payload(),
            "bigFace": {"mode": "normalized", "minImageSide": 0, "minFaceSize": 0},
        }).big_face_policy
        self.assertEqual(policy.mode, "normalized")
        self.assertEqual(policy.min_image_side, 0)
        self.assertEqual(policy.min_face_size, 0)

    def test_rejects_invalid_groups_and_fields(self):
        invalid = [
            {"mode": "fast"},
            {"mode": None},
            {"mode": 1},
            {"minImageSide": -1},
            {"minImageSide": 20_000},
            {"minImageSide": 1.5},
            {"minImageSide": True},
            {"minFaceSize": -1},
            {"minFaceSize": 8_193},
            {"minFaceSize": "600"},
            {"unknown": 1},
        ]
        for group in invalid:
            with self.subTest(group=group), self.assertRaises(InvalidPayloadError):
                ProcessingRequest.from_payload({**payload(), "bigFace": group})
        for group in ([], "auto", 1):
            with self.subTest(group=group), self.assertRaises(InvalidPayloadError):
                ProcessingRequest.from_payload({**payload(), "bigFace": group})


class BigFaceRuleTests(unittest.TestCase):
    policy = BigFacePolicy(mode="auto", min_image_side=1600, min_face_size=600)

    def test_empty_scaled_result_never_wins(self):
        native = [FaceBox(100, 100, 500, 500, 0.62)]
        self.assertFalse(prefer_scaled_faces(native, [], self.policy))

    def test_empty_native_result_defers_to_scaled(self):
        scaled = [FaceBox(100, 100, 500, 500, 0.62)]
        self.assertTrue(prefer_scaled_faces([], scaled, self.policy))

    def test_adopts_a_clearly_more_complete_face(self):
        native = [FaceBox(200, 120, 500, 500, 0.62), FaceBox(760, 130, 500, 500, 0.61)]
        scaled = [FaceBox(120, 80, 640, 640, 0.93)]
        self.assertTrue(prefer_scaled_faces(native, scaled, self.policy))

    def test_keeps_native_when_scaled_is_smaller(self):
        native = [FaceBox(100, 100, 500, 500, 0.90)]
        scaled = [FaceBox(120, 80, 200, 200, 0.95)]
        self.assertFalse(prefer_scaled_faces(native, scaled, self.policy))

    def test_keeps_native_when_scaled_loses_confidence(self):
        native = [FaceBox(100, 100, 400, 400, 0.95)]
        scaled = [FaceBox(120, 80, 800, 800, 0.50)]
        self.assertFalse(prefer_scaled_faces(native, scaled, self.policy))

    def test_keeps_native_below_the_minimum_face_size(self):
        native = [FaceBox(100, 100, 100, 100, 0.95)]
        scaled = [FaceBox(120, 80, 500, 500, 0.95)]
        self.assertFalse(prefer_scaled_faces(native, scaled, self.policy))

    def test_thresholds_are_inclusive(self):
        native = [FaceBox(0, 0, 600, 500, 0.90)]
        scaled = [FaceBox(10, 10, 600, 750, 0.85)]
        self.assertTrue(prefer_scaled_faces(native, scaled, self.policy))


class BigFaceMappingTests(unittest.TestCase):
    def test_maps_boxes_and_landmarks_back_to_the_original(self):
        points = ((30.0, 20.0), (40.0, 20.0), (35.0, 30.0), (31.0, 40.0), (39.0, 40.0))
        mapped = map_scaled_faces(
            [FaceBox(30, 20, 160, 160, 0.9, points)],
            scale=0.25,
            image_width=2048,
            image_height=1024,
        )
        self.assertEqual(
            mapped,
            [FaceBox(120, 80, 640, 640, 0.9, tuple((x * 4, y * 4) for x, y in points))],
        )

    def test_clamps_boxes_at_the_original_edges(self):
        mapped = map_scaled_faces(
            [FaceBox(0, 0, 200, 200, 0.8)],
            scale=0.5,
            image_width=300,
            image_height=300,
        )
        self.assertEqual(mapped, [FaceBox(0, 0, 300, 300, 0.8)])

    def test_drops_boxes_that_fall_completely_outside(self):
        mapped = map_scaled_faces(
            [FaceBox(10, 10, 20, 20, 0.8)],
            scale=0.01,
            image_width=100,
            image_height=100,
        )
        self.assertEqual(mapped, [])

    def test_rejects_non_positive_or_non_finite_scale(self):
        for scale in (0.0, -1.0, float("nan"), float("inf")):
            with self.subTest(scale=scale), self.assertRaises(ValueError):
                map_scaled_faces([], scale=scale, image_width=10, image_height=10)


@unittest.skipUnless(dependencies_available(), "vision dependencies are required")
class BigFaceProcessingTests(unittest.TestCase):
    def setUp(self):
        import numpy as np
        from PIL import Image

        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.source = self.root / "source.png"
        pixels = np.asarray(Image.new("RGB", (2048, 1024), (40, 60, 80))).copy()
        noise = np.random.default_rng(7).integers(0, 256, size=(640, 640, 3), dtype=np.uint8)
        pixels[80:720, 120:760] = noise
        Image.fromarray(pixels).save(self.source)

    def request(self, **overrides):
        value = {**payload(), "inputPath": str(self.source)}
        return ProcessingRequest.from_payload({**value, **overrides})

    def processor(self, detector, **overrides):
        return ScreenshotProcessor(
            self.request(**overrides), detector_factory=lambda _: detector
        )

    def test_auto_adopts_the_scaled_whole_face(self):
        native = (
            [FaceBox(200, 120, 500, 500, 0.62), FaceBox(760, 130, 500, 500, 0.61)],
            [900.0, 800.0],
        )
        points = ((40.0, 30.0), (70.0, 30.0), (55.0, 60.0), (42.0, 90.0), (68.0, 90.0))
        scaled = ([FaceBox(30, 20, 160, 160, 0.93, points)], [55.0])
        detector = Mock()
        detector.detect_faces.side_effect = [native, scaled]
        result = self.processor(
            detector,
            bigFace={"mode": "auto"},
            faceRoi={"x": 0.02, "y": 0.02, "width": 0.6, "height": 0.9},
        ).process()
        import cv2
        import numpy as np

        self.assertEqual(detector.detect_faces.call_count, 2)
        self.assertEqual(detector.detect_faces.call_args_list[1].args[0].shape, (256, 512, 3))
        expected = FaceBox(
            120, 80, 640, 640, 0.93, tuple((x * 4, y * 4) for x, y in points)
        )
        self.assertEqual(result.face_box, expected)
        self.assertEqual(result.face_count, 1)
        decoded = cv2.imdecode(
            np.fromfile(self.source, dtype=np.uint8), cv2.IMREAD_COLOR
        )
        gray = cv2.cvtColor(decoded, cv2.COLOR_BGR2GRAY)
        self.assertAlmostEqual(
            result.face_sharpness,
            float(cv2.Laplacian(gray[80:720, 120:760], cv2.CV_64F).var()),
            places=6,
        )

    def test_auto_keeps_the_native_result_when_scaled_is_not_better(self):
        native = ([FaceBox(200, 120, 500, 500, 0.95), FaceBox(760, 130, 300, 300, 0.9)], [900.0, 30.0])
        detector = Mock()
        detector.detect_faces.side_effect = [native, ([FaceBox(30, 20, 60, 60, 0.4)], [1.0])]
        result = self.processor(detector, bigFace={"mode": "auto"}).process()
        self.assertEqual(detector.detect_faces.call_count, 2)
        self.assertEqual(result.face_count, 2)
        self.assertIn(result.face_box, native[0])

    def test_auto_keeps_native_when_the_scaled_pass_finds_nothing(self):
        native = ([FaceBox(200, 120, 500, 500, 0.7)], [12.0])
        detector = Mock()
        detector.detect_faces.side_effect = [native, ([], [])]
        result = self.processor(detector, bigFace={"mode": "auto"}).process()
        self.assertEqual(result.face_count, 1)
        self.assertEqual(result.face_box, native[0][0])
        self.assertEqual(result.face_sharpness, 12.0)

    def test_auto_defers_to_scaled_when_native_finds_nothing(self):
        detector = Mock()
        detector.detect_faces.side_effect = [
            ([], []),
            ([FaceBox(30, 20, 160, 160, 0.93)], [55.0]),
        ]
        result = self.processor(detector, bigFace={"mode": "auto"}).process()
        self.assertEqual(result.face_count, 1)
        self.assertEqual(result.face_box, FaceBox(120, 80, 640, 640, 0.93))

    def test_forced_normalized_runs_a_single_downscaled_pass(self):
        detector = Mock()
        detector.detect_faces.return_value = ([FaceBox(30, 20, 160, 160, 0.93)], [55.0])
        result = self.processor(detector, bigFace={"mode": "normalized"}).process()
        self.assertEqual(detector.detect_faces.call_count, 1)
        self.assertEqual(detector.detect_faces.call_args.args[0].shape, (256, 512, 3))
        self.assertEqual(result.face_box, FaceBox(120, 80, 640, 640, 0.93))

    def test_forced_normalized_returns_no_face_when_scaled_is_empty(self):
        detector = Mock()
        detector.detect_faces.return_value = ([], [])
        result = self.processor(detector, bigFace={"mode": "normalized"}).process()
        self.assertEqual(detector.detect_faces.call_count, 1)
        self.assertIsNone(result.face_box)
        self.assertEqual(result.face_count, 0)
        self.assertEqual(result.warnings, ("face_not_detected",))

    def test_auto_stays_single_pass_below_the_image_side_threshold(self):
        native = ([FaceBox(200, 120, 500, 500, 0.7)], [12.0])
        detector = Mock()
        detector.detect_faces.return_value = native
        result = self.processor(
            detector, bigFace={"mode": "auto", "minImageSide": 4000}
        ).process()
        self.assertEqual(detector.detect_faces.call_count, 1)
        self.assertEqual(result.face_box, native[0][0])

    def test_off_and_missing_policies_keep_the_single_native_pass(self):
        for overrides in ({}, {"bigFace": {"mode": "off"}}):
            with self.subTest(overrides=overrides):
                detector = Mock()
                detector.detect_faces.return_value = ([FaceBox(200, 120, 500, 500, 0.7)], [12.0])
                result = self.processor(detector, **overrides).process()
                self.assertEqual(detector.detect_faces.call_count, 1)
                self.assertEqual(result.face_box, FaceBox(200, 120, 500, 500, 0.7))


if __name__ == "__main__":
    unittest.main()
