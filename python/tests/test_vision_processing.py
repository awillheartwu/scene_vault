import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from scene_vault_ai.protocol import PROTOCOL_VERSION, Request
from scene_vault_ai.service import AiService
from scene_vault_ai.vision import ProcessingRequest, ScreenshotProcessor
from scene_vault_ai.vision.config import AnnotationConfig, CropConfig, YuNetConfig
from scene_vault_ai.vision.recognizer import (
    ArcfaceConfig,
    ArcfaceFeatureExtractor,
    _fingerprinted_version,
)
from scene_vault_ai.vision.processor import dependencies_available
from scene_vault_ai.vision.types import FaceBox


class NoFaceDetector:
    def detect_faces(self, _image_bgr: object) -> tuple[list[FaceBox], list[float]]:
        return [], []

    def detect_primary_face(self, _image_bgr: object) -> None:
        return None


class FixedFaceDetector:
    def __init__(self, face: FaceBox) -> None:
        self.face = face

    def detect_faces(self, _image_bgr: object) -> tuple[list[FaceBox], list[float]]:
        return [self.face], [1.0]

    def detect_primary_face(self, _image_bgr: object) -> FaceBox:
        return self.face


class MultiFaceDetector:
    def __init__(self, faces: list[FaceBox]) -> None:
        self.faces = faces

    def detect_faces(self, _image_bgr: object) -> tuple[list[FaceBox], list[float]]:
        return self.faces, [1.0] * len(self.faces)

    def detect_primary_face(self, _image_bgr: object) -> FaceBox:
        return self.faces[0]


class ArcfaceIdentityTests(unittest.TestCase):
    """Model identity and guard rails that do not need the vision stack."""

    def test_model_identity_constants(self) -> None:
        self.assertEqual(ArcfaceFeatureExtractor.MODEL_ID, "arcface-r50")
        self.assertEqual(ArcfaceFeatureExtractor.MODEL_VERSION, "w600k-r50")
        self.assertEqual(ArcfaceFeatureExtractor.EMBEDDING_DIM, 512)
        # InsightFace arcface_dst template: left eye, right eye, nose tip,
        # left mouth corner, right mouth corner — the YuNet landmark order.
        self.assertEqual(
            ArcfaceFeatureExtractor.TEMPLATE,
            (
                (38.2946, 51.6963),
                (73.5318, 51.5014),
                (56.0252, 71.7366),
                (41.5493, 92.3655),
                (70.7299, 92.2041),
            ),
        )

    def test_missing_model_raises_resource_error(self) -> None:
        from scene_vault_ai.errors import ResourceNotFoundError

        with self.assertRaises(ResourceNotFoundError):
            ArcfaceFeatureExtractor(
                ArcfaceConfig(Path("missing-arcface.onnx"))
            )

    def test_model_version_fingerprint_changes_with_model_contents(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            model_path = Path(temporary_directory) / "model.onnx"
            model_path.write_bytes(b"first-model")
            first = _fingerprinted_version("v1", model_path)
            model_path.write_bytes(b"second-model")
            second = _fingerprinted_version("v1", model_path)

        self.assertTrue(first.startswith("v1+sha256:"))
        self.assertNotEqual(first, second)


@unittest.skipUnless(
    dependencies_available(),
    "optional vision dependencies are not installed",
)
class VisionProcessingTests(unittest.TestCase):
    def _request(
        self,
        root: Path,
        *,
        sface_model_path: Path | None,
        recognizer: str = "sface",
        arcface_model_path: Path | None = None,
    ) -> ProcessingRequest:
        return ProcessingRequest(
            input_path=root / "输入.png",
            annotated_output_path=root / "输出.png",
            avatar_output_path=None,
            character_name="星见",
            detect_face=True,
            annotate=True,
            crop_avatar=False,
            yunet=YuNetConfig(model_path=root / "yunet.onnx"),
            sface_model_path=sface_model_path,
            recognizer=recognizer,
            arcface_model_path=arcface_model_path,
            annotation=AnnotationConfig(),
            crop=CropConfig(),
        )

    def test_face_feature_absent_without_sface_model(self) -> None:
        from PIL import Image

        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            Image.new("RGB", (480, 270), color=(20, 30, 40)).save(root / "输入.png")
            processor = ScreenshotProcessor(
                self._request(root, sface_model_path=None),
                detector_factory=lambda _config: FixedFaceDetector(
                    FaceBox(x=10, y=10, width=60, height=60)
                ),
            )
            result = processor.process()
            self.assertIsNone(result.face_feature)
            self.assertIsNone(result.to_dict()["faceFeature"])
            self.assertIsNone(result.to_dict()["faceFeatureModelId"])
            self.assertIsNone(result.to_dict()["faceFeatureModelVersion"])
            self.assertNotIn(
                "sface",
                " ".join(result.warnings),
            )

    def test_face_feature_carries_model_identity(self) -> None:
        from unittest.mock import patch

        from PIL import Image

        from scene_vault_ai.vision import processor as processor_module

        class FakeSfaceExtractor:
            MODEL_ID = "opencv-sface"
            MODEL_VERSION = "2021dec"

            def __init__(self, _config: object) -> None:
                pass

            def extract(self, _image_bgr: object, _face: object) -> list[float]:
                return [0.25, 0.5, 0.75]

        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            Image.new("RGB", (480, 270), color=(20, 30, 40)).save(root / "输入.png")
            with patch.object(
                processor_module,
                "SfaceFeatureExtractor",
                FakeSfaceExtractor,
            ):
                processor = ScreenshotProcessor(
                    self._request(root, sface_model_path=root / "sface.onnx"),
                    detector_factory=lambda _config: FixedFaceDetector(
                        FaceBox(x=10, y=10, width=60, height=60)
                    ),
                )
                result = processor.process()
            self.assertEqual(result.face_feature, [0.25, 0.5, 0.75])
            payload = result.to_dict()
            self.assertEqual(payload["faceFeatureModelId"], "opencv-sface")
            self.assertEqual(payload["faceFeatureModelVersion"], "2021dec")

    def test_reports_face_count_for_multi_face_images(self) -> None:
        from PIL import Image

        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            Image.new("RGB", (480, 270), color=(20, 30, 40)).save(root / "输入.png")
            processor = ScreenshotProcessor(
                self._request(root, sface_model_path=None),
                detector_factory=lambda _config: MultiFaceDetector(
                    [
                        FaceBox(x=10, y=10, width=60, height=60),
                        FaceBox(x=200, y=100, width=80, height=80),
                    ]
                ),
            )
            result = processor.process()
            self.assertEqual(result.face_count, 2)
            self.assertEqual(result.to_dict()["faceCount"], 2)
            self.assertAlmostEqual(result.face_sharpness, 1.0)
            self.assertIsNotNone(result.face_box)

    def test_arcface_recognizer_uses_arcface_extractor(self) -> None:
        from unittest.mock import patch

        from PIL import Image

        from scene_vault_ai.vision import processor as processor_module

        class FakeArcfaceExtractor:
            MODEL_ID = "arcface-r50"
            MODEL_VERSION = "w600k-r50"

            def __init__(self, _config: object) -> None:
                pass

            def extract(self, _image_bgr: object, _face: object) -> list[float]:
                return [0.125] * 512

        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            Image.new("RGB", (480, 270), color=(20, 30, 40)).save(root / "输入.png")
            with patch.object(
                processor_module,
                "ArcfaceFeatureExtractor",
                FakeArcfaceExtractor,
            ):
                processor = ScreenshotProcessor(
                    self._request(
                        root,
                        sface_model_path=None,
                        recognizer="arcface",
                        arcface_model_path=root / "w600k.onnx",
                    ),
                    detector_factory=lambda _config: FixedFaceDetector(
                        FaceBox(x=10, y=10, width=60, height=60)
                    ),
                )
                result = processor.process()
            self.assertEqual(result.face_feature, [0.125] * 512)
            payload = result.to_dict()
            self.assertEqual(payload["faceFeatureModelId"], "arcface-r50")
            self.assertEqual(payload["faceFeatureModelVersion"], "w600k-r50")

    @unittest.skipUnless(
        os.environ.get("SCENE_VAULT_TEST_MODELS")
        and os.environ.get("SCENE_VAULT_TEST_FACE_IMAGE"),
        "set SCENE_VAULT_TEST_MODELS (dir with yunet+sface onnx) and "
        "SCENE_VAULT_TEST_FACE_IMAGE (a screenshot with one face)",
    )
    def test_aligncrop_is_deterministic_across_processes(self) -> None:
        """Regression guard for the 2026-08-08 alignCrop drift bug: a bare
        1x4 bounding box produced different aligned crops across processes.
        Extracting the same image in two independent processes must give
        bit-identical features (cosine ~1.0)."""
        import json
        import math
        import subprocess
        import sys

        models = Path(os.environ["SCENE_VAULT_TEST_MODELS"])
        yunet = models / "face_detection_yunet_2023mar.onnx"
        sface = models / "face_recognition_sface_2021dec.onnx"
        image_path = Path(os.environ["SCENE_VAULT_TEST_FACE_IMAGE"])
        script = (
            "import json,sys;"
            "import numpy as np, cv2;"
            "from pathlib import Path;"
            "from scene_vault_ai.vision.config import YuNetConfig;"
            "from scene_vault_ai.vision.detector import YuNetFaceDetector;"
            "from scene_vault_ai.vision.recognizer import SfaceConfig, "
            "SfaceFeatureExtractor;"
            "img=cv2.imdecode(np.fromfile(sys.argv[1],dtype=np.uint8),"
            "cv2.IMREAD_COLOR);"
            "det=YuNetFaceDetector(YuNetConfig(model_path=Path(sys.argv[2])));"
            "ex=SfaceFeatureExtractor(SfaceConfig(Path(sys.argv[3])));"
            "face=det.detect_primary_face(img);"
            "json.dump({'feature': ex.extract(img, face) if face else None},"
            "sys.stdout)"
        )
        env = dict(os.environ)
        features = []
        for _ in range(2):
            run = subprocess.run(
                [
                    sys.executable,
                    "-c",
                    script,
                    str(image_path),
                    str(yunet),
                    str(sface),
                ],
                capture_output=True,
                text=True,
                env=env,
                check=True,
            )
            features.append(json.loads(run.stdout)["feature"])
        self.assertIsNotNone(features[0])
        first, second = features
        dot = sum(a * b for a, b in zip(first, second, strict=True))
        norm = math.sqrt(sum(a * a for a in first)) * math.sqrt(
            sum(b * b for b in second)
        )
        self.assertGreater(dot / norm, 0.9999)

    def test_missing_sface_model_degrades_with_warning(self) -> None:
        from PIL import Image

        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            Image.new("RGB", (480, 270), color=(20, 30, 40)).save(root / "输入.png")
            processor = ScreenshotProcessor(
                self._request(root, sface_model_path=root / "missing-sface.onnx"),
                detector_factory=lambda _config: FixedFaceDetector(
                    FaceBox(x=10, y=10, width=60, height=60)
                ),
            )
            result = processor.process()
            self.assertIsNone(result.face_feature)
            self.assertIsNone(result.to_dict()["faceFeature"])
            self.assertTrue(
                any(
                    warning.startswith("sface_feature_failed")
                    for warning in result.warnings
                ),
                result.warnings,
            )

    def test_reports_stage_progress_in_order(self) -> None:
        from unittest.mock import patch

        from PIL import Image

        from scene_vault_ai.vision import processor as processor_module

        class FakeSfaceExtractor:
            MODEL_ID = "opencv-sface"
            MODEL_VERSION = "2021dec"

            def __init__(self, _config: object) -> None:
                self.model_version = "2021dec+sha256:test"

            def extract(self, _image_bgr: object, _face: object) -> list[float]:
                return [0.25, 0.5, 0.75]

        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            Image.new("RGB", (480, 270), color=(20, 30, 40)).save(root / "输入.png")
            (root / "sface.onnx").write_bytes(b"model")
            events: list[tuple[str, float]] = []
            with patch.object(processor_module, "SfaceFeatureExtractor", FakeSfaceExtractor):
                processor = ScreenshotProcessor(
                    self._request(root, sface_model_path=root / "sface.onnx"),
                    detector_factory=lambda _config: FixedFaceDetector(
                        FaceBox(x=10, y=10, width=60, height=60)
                    ),
                    progress=lambda stage, percent: events.append((stage, percent)),
                )
                processor.process()
            stages = [stage for stage, _ in events]
            self.assertEqual(
                stages,
                ["read", "detect_face", "extract_feature", "annotate", "done"],
            )
            self.assertEqual(events[-1], ("done", 100.0))
            percents = [percent for _, percent in events]
            self.assertEqual(percents, sorted(percents))

    def test_unicode_path_and_chinese_name_are_annotated(self) -> None:
        from PIL import Image, ImageChops

        with tempfile.TemporaryDirectory(prefix="场景仓库-") as temporary_directory:
            root = Path(temporary_directory)
            source_path = root / "输入截图.png"
            output_path = root / "输出标注.png"
            source = Image.new("RGB", (480, 270), color=(20, 30, 40))
            source.save(source_path)

            response = AiService().handle(
                Request(
                    protocol_version=PROTOCOL_VERSION,
                    action="processScreenshot",
                    payload={
                        "inputPath": str(source_path),
                        "annotatedOutputPath": str(output_path),
                        "characterName": "星见",
                        "detectFace": False,
                        "annotate": True,
                        "cropAvatar": False,
                    },
                )
            )

            self.assertTrue(response.ok, response.error)
            self.assertTrue(output_path.is_file())
            self.assertEqual(response.data["imageWidth"], 480)
            self.assertEqual(response.data["imageHeight"], 270)
            self.assertFalse(response.data["faceDetected"])
            self.assertEqual(response.data["warnings"], [])
            with Image.open(output_path) as annotated:
                difference = ImageChops.difference(source, annotated.convert("RGB"))
                self.assertIsNotNone(difference.getbbox())

    def test_cli_process_request_round_trip_uses_json_only(self) -> None:
        from PIL import Image

        with tempfile.TemporaryDirectory(prefix="截图-") as temporary_directory:
            root = Path(temporary_directory)
            source_path = root / "输入.png"
            output_path = root / "输出.png"
            Image.new("RGB", (320, 180), color=(40, 50, 60)).save(source_path)
            request = {
                "protocolVersion": PROTOCOL_VERSION,
                "action": "processScreenshot",
                "payload": {
                    "inputPath": str(source_path),
                    "annotatedOutputPath": str(output_path),
                    "characterName": "星见",
                    "detectFace": False,
                    "annotate": True,
                    "cropAvatar": False,
                },
            }

            process = subprocess.run(
                [sys.executable, "-m", "scene_vault_ai", "request"],
                input=json.dumps(request, ensure_ascii=False).encode("utf-8"),
                check=False,
                capture_output=True,
            )
            stdout = process.stdout.decode("utf-8")
            stderr = process.stderr.decode("utf-8")
            response = json.loads(stdout)

            self.assertEqual(
                process.returncode,
                0,
                stderr or stdout,
            )
            self.assertIn("SVPROGRESS", stderr)
            self.assertIn('"stage":"done"', stderr)
            self.assertEqual(len(stdout.strip().splitlines()), 1)
            self.assertTrue(response["ok"])
            self.assertEqual(response["data"]["annotatedPath"], str(output_path))
            self.assertTrue(output_path.is_file())

    def test_cli_worker_processes_multiple_images_in_one_process(self) -> None:
        from PIL import Image

        with tempfile.TemporaryDirectory(prefix="worker-截图-") as temporary_directory:
            root = Path(temporary_directory)
            requests = []
            output_paths = []
            for index in range(2):
                source_path = root / f"输入-{index}.png"
                output_path = root / f"输出-{index}.png"
                Image.new(
                    "RGB",
                    (320, 180),
                    color=(40 + index, 50, 60),
                ).save(source_path)
                output_paths.append(output_path)
                requests.append(
                    {
                        "protocolVersion": PROTOCOL_VERSION,
                        "requestId": f"worker-image-{index}",
                        "action": "processScreenshot",
                        "payload": {
                            "inputPath": str(source_path),
                            "annotatedOutputPath": str(output_path),
                            "characterName": "星见",
                            "detectFace": False,
                            "annotate": True,
                            "cropAvatar": False,
                        },
                    }
                )
            stdin = b"\n".join(
                json.dumps(request, ensure_ascii=False).encode("utf-8")
                for request in requests
            ) + b"\n"

            process = subprocess.run(
                [sys.executable, "-m", "scene_vault_ai", "worker"],
                input=stdin,
                check=False,
                capture_output=True,
            )
            stdout = process.stdout.decode("utf-8")
            stderr = process.stderr.decode("utf-8")
            responses = [json.loads(line) for line in stdout.splitlines()]

            self.assertEqual(process.returncode, 0, stderr or stdout)
            self.assertEqual(len(responses), 2)
            self.assertTrue(all(response["ok"] for response in responses))
            self.assertEqual(
                [response["requestId"] for response in responses],
                ["worker-image-0", "worker-image-1"],
            )
            self.assertTrue(all(path.is_file() for path in output_paths))
            self.assertIn('"requestId":"worker-image-0"', stderr)
            self.assertIn('"requestId":"worker-image-1"', stderr)

    def test_no_face_is_a_successful_degradation(self) -> None:
        from PIL import Image

        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            source_path = root / "source.png"
            model_path = root / "yunet.onnx"
            annotated_path = root / "annotated.png"
            avatar_path = root / "avatar.png"
            Image.new("RGB", (640, 360), color=(10, 20, 30)).save(source_path)
            model_path.touch()
            request = ProcessingRequest.from_payload(
                {
                    "inputPath": str(source_path),
                    "annotatedOutputPath": str(annotated_path),
                    "avatarOutputPath": str(avatar_path),
                    "characterName": "Aurora",
                    "yunetModelPath": str(model_path),
                }
            )

            result = ScreenshotProcessor(
                request,
                detector_factory=lambda _config: NoFaceDetector(),
            ).process()

            self.assertTrue(annotated_path.is_file())
            self.assertFalse(avatar_path.exists())
            self.assertIsNone(result.avatar_path)
            self.assertEqual(
                result.warnings,
                ("face_not_detected", "avatar_not_generated"),
            )

    def test_fake_face_generates_annotation_and_avatar(self) -> None:
        from PIL import Image

        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            source_path = root / "source.png"
            model_path = root / "yunet.onnx"
            annotated_path = root / "annotated.png"
            avatar_path = root / "avatar.png"
            Image.new("RGB", (640, 480), color=(80, 100, 120)).save(source_path)
            model_path.touch()
            request = ProcessingRequest.from_payload(
                {
                    "inputPath": str(source_path),
                    "annotatedOutputPath": str(annotated_path),
                    "avatarOutputPath": str(avatar_path),
                    "characterName": "Aurora",
                    "yunetModelPath": str(model_path),
                }
            )
            face = FaceBox(260, 120, 120, 120, 0.95)

            result = ScreenshotProcessor(
                request,
                detector_factory=lambda _config: FixedFaceDetector(face),
            ).process()

            self.assertEqual(result.face_box, face)
            self.assertEqual(result.warnings, ())
            self.assertTrue(annotated_path.is_file())
            self.assertTrue(avatar_path.is_file())
            with Image.open(avatar_path) as avatar:
                self.assertGreater(avatar.width, 0)
                self.assertEqual(avatar.width, avatar.height)
            self.assertFalse(
                any(path.name.startswith(".") for path in root.iterdir())
            )

    def test_corrupt_and_missing_images_return_stable_error_codes(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            corrupt_path = root / "corrupt.png"
            missing_path = root / "missing.png"
            output_path = root / "output.png"
            corrupt_path.write_bytes(b"not an image")

            corrupt_response = self._annotation_request(
                corrupt_path,
                output_path,
            )
            missing_response = self._annotation_request(
                missing_path,
                output_path,
            )

            self.assertFalse(corrupt_response.ok)
            self.assertEqual(corrupt_response.error.code, "image_decode_failed")
            self.assertFalse(missing_response.ok)
            self.assertEqual(missing_response.error.code, "input_not_found")
            self.assertFalse(output_path.exists())

    def test_missing_yunet_model_is_structured(self) -> None:
        from PIL import Image

        with tempfile.TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            source_path = root / "source.png"
            missing_model = root / "missing.onnx"
            Image.new("RGB", (320, 180), color=(10, 20, 30)).save(source_path)

            response = AiService().handle(
                Request(
                    protocol_version=PROTOCOL_VERSION,
                    action="processScreenshot",
                    payload={
                        "inputPath": str(source_path),
                        "detectFace": True,
                        "annotate": False,
                        "cropAvatar": False,
                        "yunetModelPath": str(missing_model),
                    },
                )
            )

            self.assertFalse(response.ok)
            self.assertEqual(response.error.code, "resource_not_found")
            self.assertEqual(
                response.error.details["resource"],
                "yunet_model",
            )

    @staticmethod
    def _annotation_request(source_path: Path, output_path: Path):
        return AiService().handle(
            Request(
                protocol_version=PROTOCOL_VERSION,
                action="processScreenshot",
                payload={
                    "inputPath": str(source_path),
                    "annotatedOutputPath": str(output_path),
                    "characterName": "Aurora",
                    "detectFace": False,
                    "annotate": True,
                    "cropAvatar": False,
                },
            )
        )


_REAL_YUNET_MODEL_VALUE = os.environ.get("SCENE_VAULT_TEST_YUNET_MODEL")
_REAL_YUNET_MODEL = (
    Path(_REAL_YUNET_MODEL_VALUE)
    if _REAL_YUNET_MODEL_VALUE
    else Path("__missing_yunet_model__")
)


@unittest.skipUnless(
    dependencies_available() and _REAL_YUNET_MODEL.is_file(),
    "set SCENE_VAULT_TEST_YUNET_MODEL to run the real YuNet smoke test",
)
class RealYuNetSmokeTests(unittest.TestCase):
    def test_real_yunet_model_loads_and_processes_an_image(self) -> None:
        from PIL import Image

        with tempfile.TemporaryDirectory() as temporary_directory:
            source_path = Path(temporary_directory) / "blank.png"
            Image.new("RGB", (640, 360), color=(32, 32, 32)).save(source_path)
            request = ProcessingRequest.from_payload(
                {
                    "inputPath": str(source_path),
                    "detectFace": True,
                    "annotate": False,
                    "cropAvatar": False,
                    "yunetModelPath": str(_REAL_YUNET_MODEL.resolve()),
                }
            )

            result = ScreenshotProcessor(request).process()

            self.assertEqual(result.image_width, 640)
            self.assertEqual(result.image_height, 360)
            self.assertIsNone(result.annotated_path)
            self.assertIsNone(result.avatar_path)


if __name__ == "__main__":
    unittest.main()
