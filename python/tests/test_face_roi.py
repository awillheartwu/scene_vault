import unittest
import tempfile
from pathlib import Path
from unittest.mock import Mock, patch

from scene_vault_ai.errors import InvalidPayloadError
from scene_vault_ai.protocol import Request
from scene_vault_ai.service import AiService
from scene_vault_ai.vision.config import ProcessingRequest
from scene_vault_ai.vision.processor import ScreenshotProcessor, dependencies_available
from scene_vault_ai.vision.types import FaceBox


def payload():
    return {
        "inputPath": "/tmp/source.png",
        "yunetModelPath": "/tmp/yunet.onnx",
        "annotate": False,
        "cropAvatar": False,
    }


class FaceRoiValidationTests(unittest.TestCase):
    def test_optional_and_full_image_roi(self):
        self.assertIsNone(ProcessingRequest.from_payload(payload()).face_roi)
        self.assertIsNone(ProcessingRequest.from_payload({**payload(), "faceRoi": None}).face_roi)
        roi = ProcessingRequest.from_payload({
            **payload(), "faceRoi": {"x": 0, "y": 0, "width": 1, "height": 1}
        }).face_roi
        self.assertEqual((roi.x, roi.y, roi.width, roi.height), (0, 0, 1, 1))

    def test_rejects_malformed_roi(self):
        valid = {"x": 0.2, "y": 0.2, "width": 0.4, "height": 0.4}
        invalid = [[], "roi", 1, True, {}, {**valid, "extra": 0}]
        for key in valid:
            invalid.append({k: v for k, v in valid.items() if k != key})
            for value in (None, True, "0.2", float("nan"), float("inf"),
                          -float("inf"), -0.1, 1.1, 10 ** 400):
                invalid.append({**valid, key: value})
        for key in ("width", "height"):
            invalid.extend([{**valid, key: 0}, {**valid, key: 0.9}])
        for roi in invalid:
            with self.subTest(roi=roi), self.assertRaises(InvalidPayloadError):
                ProcessingRequest.from_payload({**payload(), "faceRoi": roi})

    def test_roi_requires_detection(self):
        with self.assertRaises(InvalidPayloadError) as caught:
            ProcessingRequest.from_payload({
                **payload(), "detectFace": False,
                "faceRoi": {"x": 0, "y": 0, "width": 1, "height": 1},
            })
        self.assertEqual(caught.exception.details["field"], "faceRoi")


@unittest.skipUnless(dependencies_available(), "vision dependencies are required")
class FaceRoiProcessingTests(unittest.TestCase):
    def setUp(self):
        from PIL import Image

        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.source = self.root / "source.png"
        Image.new("RGB", (200, 100), (50, 70, 90)).save(self.source)
        self.roi = {"x": 0.25, "y": 0.2, "width": 0.5, "height": 0.6}

    def request(self, final=False, **overrides):
        value = {**payload(), "inputPath": str(self.source), "faceRoi": self.roi}
        if final:
            value.update({
                "annotate": True, "cropAvatar": True, "characterName": "Aurora",
                "annotatedOutputPath": str(self.root / "annotated.png"),
                "avatarOutputPath": str(self.root / "avatar.png"),
            })
        return ProcessingRequest.from_payload({**value, **overrides})

    def test_roi_policy_defaults_and_validation(self):
        self.assertEqual(self.request().roi_policy.expand_ratio, 0.15)
        self.assertEqual(self.request().roi_policy.multiple_faces, "error")
        configured = self.request(roi={"expandRatio": 0.3, "multipleFaces": "largest"})
        self.assertEqual(configured.roi_policy.expand_ratio, 0.3)
        self.assertEqual(configured.roi_policy.multiple_faces, "largest")
        for invalid in [
            {"expandRatio": 0.9},
            {"expandRatio": -0.1},
            {"multipleFaces": "closest"},
            {"unknown": 1},
        ]:
            with self.assertRaises(InvalidPayloadError):
                self.request(roi=invalid)

    def processor(self, detector, final=False, **overrides):
        return ScreenshotProcessor(
            self.request(final, **overrides), detector_factory=lambda _: detector
        )

    def test_expand_ratio_controls_the_retry_crop(self):
        from scene_vault_ai.errors import RoiNoFaceError

        for ratio, expected in [(0.0, (60, 100)), (0.2, (84, 140))]:
            with self.subTest(ratio=ratio):
                detector = Mock()
                detector.detect_faces.return_value = ([], [])
                processor = self.processor(detector, roi={"expandRatio": ratio})
                with self.assertRaises(RoiNoFaceError):
                    processor.process()
                # The second call is the retry pass on the expanded crop.
                cropped = detector.detect_faces.call_args_list[1].args[0]
                self.assertEqual(cropped.shape[:2], expected)

    def test_multiple_faces_policy_picks_the_requested_face(self):
        from scene_vault_ai.errors import RoiMultipleFacesError

        small = FaceBox(55, 25, 10, 10, 0.9)
        large = FaceBox(70, 30, 40, 40, 0.5)
        for strategy, expected in [("largest", large), ("sharpest", small)]:
            with self.subTest(strategy=strategy):
                detector = Mock()
                # The bigger box is the blurrier one, so the policies differ.
                detector.detect_faces.return_value = ([small, large], [900.0, 10.0])
                result = self.processor(
                    detector, roi={"multipleFaces": strategy}
                ).process()
                self.assertEqual(result.face_box, expected)
                self.assertEqual(result.face_count, 1)

        detector = Mock()
        detector.detect_faces.return_value = ([small, large], [900.0, 10.0])
        with self.assertRaises(RoiMultipleFacesError):
            self.processor(detector).process()

    def test_full_image_selection_shared_by_features_annotation_and_avatar(self):
        target = FaceBox(55, 25, 20, 20, 0.8)
        outside = FaceBox(150, 10, 50, 80, 0.99)
        for final in (False, True):
            with self.subTest(final=final):
                detector = Mock()
                detector.detect_faces.return_value = ([outside, target], [900.0, 123.0])
                processor = self.processor(detector, final)
                extractor = Mock(MODEL_ID="test", MODEL_VERSION="v1")
                # Processor reads the model ID/version from the extractor class.
                type(extractor).MODEL_ID = "test"
                type(extractor).MODEL_VERSION = "v1"
                extractor.extract.return_value = [0.6, 0.8]
                processor.feature_extractor = extractor
                with patch("scene_vault_ai.vision.processor.ImageAnnotator.annotate") as annotate, \
                     patch("scene_vault_ai.vision.processor.AvatarCropper.crop") as crop:
                    from PIL import Image
                    annotate.return_value = Image.new("RGB", (200, 100))
                    crop.return_value = Image.new("RGB", (20, 20))
                    result = processor.process()
                self.assertEqual(result.face_box, target)
                self.assertEqual(result.face_count, 1)
                self.assertEqual(result.face_sharpness, 123.0)
                self.assertEqual(result.face_area_ratio, 0.02)
                self.assertEqual(result.face_feature, [0.6, 0.8])
                self.assertEqual(extractor.extract.call_args.args[0].shape, (100, 200, 3))
                self.assertEqual(extractor.extract.call_args.args[1], target)
                detector.detect_faces.assert_called_once()
                if final:
                    self.assertEqual(annotate.call_args.args[2], target)
                    self.assertEqual(crop.call_args.args[1], target)
                    self.assertTrue(result.annotated_path.is_file())
                    self.assertTrue(result.avatar_path.is_file())
                else:
                    annotate.assert_not_called()
                    crop.assert_not_called()

    def test_retry_expands_crop_and_translates_box_and_all_landmarks(self):
        points = ((20.5, 20.5), (30, 20), (25, 25), (21, 29), (29, 29))
        local = FaceBox(20, 15, 20, 20, 0.9, points)
        detector = Mock()
        # The second face is in the padding, outside the original ROI.
        detector.detect_faces.side_effect = [
            ([FaceBox(170, 0, 10, 10)], [1.0]),
            ([local, FaceBox(0, 0, 4, 4)], [77.0, 88.0]),
        ]
        processor = self.processor(detector)
        result = processor.process()
        self.assertEqual(detector.detect_faces.call_args.args[0].shape, (78, 130, 3))
        self.assertEqual(result.face_box, FaceBox(
            55, 26, 20, 20, 0.9, tuple((x + 35, y + 11) for x, y in points)
        ))
        self.assertEqual(result.face_sharpness, 77.0)
        self.assertEqual(result.face_count, 1)
        self.assertEqual(local.landmarks, points)
        self.assertEqual(processor.request.face_roi.x, 0.25)

    def test_retry_crop_is_clipped_at_image_edges(self):
        for roi in (
            {"x": 0, "y": 0, "width": 0.5, "height": 0.6},
            {"x": 0.5, "y": 0.4, "width": 0.5, "height": 0.6},
        ):
            with self.subTest(roi=roi):
                detector = Mock()
                detector.detect_faces.side_effect = [([], []), ([FaceBox(20, 20, 10, 10)], [1.0])]
                self.processor(detector, faceRoi=roi).process()
                self.assertEqual(detector.detect_faces.call_args.args[0].shape, (69, 115, 3))

    def test_roi_errors_are_structured_and_write_no_outputs(self):
        inside = FaceBox(60, 30, 10, 10)
        other = FaceBox(90, 30, 10, 10)
        cases = [
            ([([inside, other], [1.0, 2.0])], "roi_multiple_faces", 1),
            ([([], []), ([inside, other], [1.0, 2.0])], "roi_multiple_faces", 2),
            ([([], []), ([], [])], "roi_no_face", 2),
            ([([], []), ([FaceBox(0, 0, 4, 4)], [1.0])], "roi_no_face", 2),
        ]
        for results, code, calls in cases:
            with self.subTest(code=code, calls=calls):
                detector = Mock()
                detector.detect_faces.side_effect = results
                service = AiService()
                with patch.object(type(service.model_cache), "get_detector", return_value=detector), \
                     patch.object(type(service.model_cache), "discard_detector") as discard:
                    response = service.handle(Request(1, "processScreenshot", {
                        **payload(), "inputPath": str(self.source), "faceRoi": self.roi,
                        "annotate": True, "cropAvatar": True, "characterName": "Aurora",
                        "annotatedOutputPath": str(self.root / "annotated.png"),
                        "avatarOutputPath": str(self.root / "avatar.png"),
                    }, request_id="roi-test"))
                self.assertFalse(response.ok)
                self.assertEqual(response.to_dict()["error"]["code"], code)
                self.assertEqual(response.request_id, "roi-test")
                self.assertEqual(detector.detect_faces.call_count, calls)
                discard.assert_not_called()
                self.assertFalse((self.root / "annotated.png").exists())
                self.assertFalse((self.root / "avatar.png").exists())

    def test_face_center_on_roi_boundary_is_included(self):
        face = FaceBox(40, 10, 20, 20)
        detector = Mock()
        detector.detect_faces.return_value = ([face], [1.0])
        self.assertEqual(self.processor(detector).process().face_box, face)
        detector.detect_faces.assert_called_once()

    def test_no_roi_preserves_face_count_and_no_face_degradation(self):
        for faces in ([], [FaceBox(10, 10, 20, 20), FaceBox(70, 30, 40, 40)]):
            with self.subTest(faces=faces):
                detector = Mock()
                detector.detect_faces.return_value = (faces, [1.0] * len(faces))
                result = self.processor(detector, faceRoi=None).process()
                self.assertEqual(result.face_count, len(faces))
                self.assertEqual(result.face_box, faces[-1] if faces else None)
                self.assertEqual(result.warnings, () if faces else ("face_not_detected",))
                detector.detect_faces.assert_called_once()


if __name__ == "__main__":
    unittest.main()
