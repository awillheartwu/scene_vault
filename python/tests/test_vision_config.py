import math
import unittest
from pathlib import Path

from scene_vault_ai.errors import InvalidPayloadError
from scene_vault_ai.vision.config import ProcessingRequest


def valid_payload() -> dict[str, object]:
    return {
        "inputPath": r"D:\Screenshots\001.png",
        "annotatedOutputPath": r"D:\Cache\001.png",
        "avatarOutputPath": r"D:\Cache\avatars\001.png",
        "characterName": "星见 Aurora",
        "yunetModelPath": r"D:\Models\yunet.onnx",
    }


class VisionConfigTests(unittest.TestCase):
    def test_builds_a_strict_single_image_request(self) -> None:
        payload = valid_payload()
        payload["detection"] = {
            "scoreThreshold": 0.7,
            "topK": 1000,
        }
        payload["annotation"] = {
            "fontSize": 42,
            "fallbackPosition": "bottom_right",
        }
        payload["crop"] = {
            "aspectRatio": "3:4",
            "scaleX": 2.0,
        }

        request = ProcessingRequest.from_payload(payload)

        self.assertTrue(request.detect_face)
        self.assertTrue(request.annotate)
        self.assertTrue(request.crop_avatar)
        self.assertEqual(request.character_name, "星见 Aurora")
        self.assertEqual(request.yunet.score_threshold, 0.7)
        self.assertEqual(request.annotation.font_size, 42)
        self.assertEqual(request.crop.aspect_ratio, (3, 4))

    def test_accepts_custom_text_position_with_offsets(self) -> None:
        payload = valid_payload()
        payload["annotation"] = {
            "faceTextPosition": "custom",
            "textOffsetX": -0.3,
            "textOffsetY": 1.25,
        }

        request = ProcessingRequest.from_payload(payload)

        self.assertEqual(request.annotation.face_text_position, "custom")
        self.assertEqual(request.annotation.text_offset_x, -0.3)
        self.assertEqual(request.annotation.text_offset_y, 1.25)

    def test_custom_text_offsets_are_optional_and_clamped(self) -> None:
        payload = valid_payload()
        payload["annotation"] = {
            "faceTextPosition": "custom",
        }
        request = ProcessingRequest.from_payload(payload)
        self.assertIsNone(request.annotation.text_offset_x)
        self.assertIsNone(request.annotation.text_offset_y)

        for key in ("textOffsetX", "textOffsetY"):
            with self.subTest(key=key):
                bad = valid_payload()
                bad["annotation"] = {"faceTextPosition": "custom", key: 3.5}
                with self.assertRaises(InvalidPayloadError):
                    ProcessingRequest.from_payload(bad)

    def test_rejects_unknown_text_offset_field(self) -> None:
        payload = valid_payload()
        payload["annotation"] = {"textOffsetZ": 0.5}
        with self.assertRaises(InvalidPayloadError):
            ProcessingRequest.from_payload(payload)

    def test_rejects_relative_uri_unc_and_device_paths(self) -> None:
        invalid_paths = (
            "relative/input.png",
            "file:///D:/input.png",
            r"\\nas\archive\input.png",
            r"\\.\PIPE\input.png",
        )

        for invalid_path in invalid_paths:
            with self.subTest(path=invalid_path):
                payload = valid_payload()
                payload["inputPath"] = invalid_path
                with self.assertRaises(InvalidPayloadError):
                    ProcessingRequest.from_payload(payload)

    def test_rejects_equal_input_or_output_paths(self) -> None:
        payload = valid_payload()
        payload["avatarOutputPath"] = payload["annotatedOutputPath"]

        with self.assertRaisesRegex(InvalidPayloadError, "different"):
            ProcessingRequest.from_payload(payload)

        payload = valid_payload()
        payload["annotatedOutputPath"] = payload["inputPath"]
        with self.assertRaisesRegex(InvalidPayloadError, "different"):
            ProcessingRequest.from_payload(payload)

        payload = valid_payload()
        payload["annotatedOutputPath"] = r"D:\Screenshots\temp\..\001.png"
        with self.assertRaisesRegex(InvalidPayloadError, "different"):
            ProcessingRequest.from_payload(payload)

    def test_rejects_unknown_fields_at_every_level(self) -> None:
        payload = valid_payload()
        payload["unknown"] = True
        with self.assertRaisesRegex(InvalidPayloadError, "unknown"):
            ProcessingRequest.from_payload(payload)

        for section in ("detection", "annotation", "crop"):
            with self.subTest(section=section):
                payload = valid_payload()
                payload[section] = {"unknown": True}
                with self.assertRaisesRegex(InvalidPayloadError, "unknown"):
                    ProcessingRequest.from_payload(payload)

    def test_parses_optional_sface_model_path(self) -> None:
        payload = valid_payload()
        payload["sfaceModelPath"] = r"D:\Models\sface.onnx"
        request = ProcessingRequest.from_payload(payload)
        self.assertEqual(request.sface_model_path, Path(r"D:\Models\sface.onnx"))

        payload.pop("sfaceModelPath")
        request = ProcessingRequest.from_payload(payload)
        self.assertIsNone(request.sface_model_path)

        payload["sfaceModelPath"] = "relative/model.onnx"
        with self.assertRaisesRegex(InvalidPayloadError, "absolute"):
            ProcessingRequest.from_payload(payload)

    def test_parses_arcface_recognizer_and_model_path(self) -> None:
        payload = valid_payload()
        payload["recognizer"] = "arcface"
        payload["arcfaceModelPath"] = r"D:\Models\w600k_r50.onnx"

        request = ProcessingRequest.from_payload(payload)

        self.assertEqual(request.recognizer, "arcface")
        self.assertEqual(
            request.arcface_model_path, Path(r"D:\Models\w600k_r50.onnx")
        )

    def test_arcface_requires_its_model_path(self) -> None:
        payload = valid_payload()
        payload["recognizer"] = "arcface"
        with self.assertRaises(InvalidPayloadError):
            ProcessingRequest.from_payload(payload)

    def test_rejects_unknown_recognizer(self) -> None:
        payload = valid_payload()
        payload["recognizer"] = "facenet"
        with self.assertRaises(InvalidPayloadError):
            ProcessingRequest.from_payload(payload)

    def test_rejects_wrong_types_and_out_of_range_numbers(self) -> None:
        invalid_detection_values = (
            {"scoreThreshold": True},
            {"scoreThreshold": math.nan},
            {"scoreThreshold": 1.1},
            {"topK": 0},
            {"edgeMarginRatio": 0.5},
            {"minSharpness": -1},
        )
        for detection in invalid_detection_values:
            with self.subTest(detection=detection):
                payload = valid_payload()
                payload["detection"] = detection
                with self.assertRaises(InvalidPayloadError):
                    ProcessingRequest.from_payload(payload)

        payload = valid_payload()
        payload["annotation"] = {"fontSize": "48"}
        with self.assertRaises(InvalidPayloadError):
            ProcessingRequest.from_payload(payload)

        payload = valid_payload()
        payload["crop"] = {"scaleX": 0}
        with self.assertRaises(InvalidPayloadError):
            ProcessingRequest.from_payload(payload)

    def test_rejects_invalid_operation_combinations(self) -> None:
        with self.assertRaisesRegex(InvalidPayloadError, "characterName"):
            payload = valid_payload()
            del payload["characterName"]
            ProcessingRequest.from_payload(payload)

        with self.assertRaisesRegex(InvalidPayloadError, "requires detectFace"):
            ProcessingRequest.from_payload(
                {
                    "inputPath": r"D:\Screenshots\001.png",
                    "avatarOutputPath": r"D:\Cache\avatar.png",
                    "detectFace": False,
                    "annotate": False,
                    "cropAvatar": True,
                }
            )

        with self.assertRaisesRegex(InvalidPayloadError, "only allowed"):
            ProcessingRequest.from_payload(
                {
                    "inputPath": r"D:\Screenshots\001.png",
                    "annotatedOutputPath": r"D:\Cache\001.png",
                    "detectFace": True,
                    "annotate": False,
                    "cropAvatar": False,
                    "yunetModelPath": r"D:\Models\yunet.onnx",
                }
            )

    def test_rejects_unsupported_output_extension_and_control_names(self) -> None:
        payload = valid_payload()
        payload["annotatedOutputPath"] = r"D:\Cache\001.gif"
        with self.assertRaisesRegex(InvalidPayloadError, "extension"):
            ProcessingRequest.from_payload(payload)

        payload = valid_payload()
        payload["characterName"] = "Aurora\nInjected"
        with self.assertRaisesRegex(InvalidPayloadError, "control"):
            ProcessingRequest.from_payload(payload)


if __name__ == "__main__":
    unittest.main()
