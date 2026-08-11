from __future__ import annotations

import unittest
from tempfile import TemporaryDirectory
from pathlib import Path
from typing import Any, cast

from scene_vault_ai.vision.cache import VisionModelCache
from scene_vault_ai.vision.config import YuNetConfig
from scene_vault_ai.vision.recognizer import (
    ArcfaceConfig,
    ArcfaceFeatureExtractor,
    SfaceConfig,
    SfaceFeatureExtractor,
)
from scene_vault_ai.vision.types import FaceBox


class _FakeDetector:
    def detect_faces(self, _image_bgr: Any) -> tuple[list[FaceBox], list[float]]:
        return [], []

    def detect_primary_face(self, _image_bgr: Any) -> FaceBox | None:
        return None


class ModelCacheTests(unittest.TestCase):
    def test_reuses_matching_models_and_replaces_changed_configuration(self) -> None:
        detector_calls: list[YuNetConfig] = []
        sface_calls: list[Path] = []
        arcface_calls: list[Path] = []

        def detector_factory(config: YuNetConfig) -> _FakeDetector:
            detector_calls.append(config)
            return _FakeDetector()

        def sface_factory(config: SfaceConfig) -> SfaceFeatureExtractor:
            sface_calls.append(config.model_path)
            return cast(SfaceFeatureExtractor, object())

        def arcface_factory(config: ArcfaceConfig) -> ArcfaceFeatureExtractor:
            arcface_calls.append(config.model_path)
            return cast(ArcfaceFeatureExtractor, object())

        with TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            yunet_path = root / "yunet.onnx"
            sface_path = root / "sface.onnx"
            arcface_path = root / "arcface.onnx"
            for path in (yunet_path, sface_path, arcface_path):
                path.write_bytes(b"model")
            cache = VisionModelCache(
                detector_factory=detector_factory,
                sface_factory=sface_factory,
                arcface_factory=arcface_factory,
            )
            first_yunet = YuNetConfig(yunet_path)
            changed_yunet = YuNetConfig(yunet_path, score_threshold=0.75)

            self.assertIs(cache.get_detector(first_yunet), cache.get_detector(first_yunet))
            self.assertIsNot(cache.get_detector(first_yunet), cache.get_detector(changed_yunet))
            self.assertIs(cache.get_sface(sface_path), cache.get_sface(sface_path))
            self.assertIs(cache.get_arcface(arcface_path), cache.get_arcface(arcface_path))

            self.assertEqual(detector_calls, [first_yunet, changed_yunet])
            self.assertEqual(sface_calls, [sface_path])
            self.assertEqual(arcface_calls, [arcface_path])

    def test_reloads_a_model_replaced_at_the_same_path(self) -> None:
        calls = 0

        def detector_factory(_config: YuNetConfig) -> _FakeDetector:
            nonlocal calls
            calls += 1
            return _FakeDetector()

        with TemporaryDirectory() as temporary_directory:
            model_path = Path(temporary_directory) / "yunet.onnx"
            model_path.write_bytes(b"first")
            cache = VisionModelCache(detector_factory=detector_factory)
            config = YuNetConfig(model_path)
            first = cache.get_detector(config)

            model_path.write_bytes(b"replacement-with-a-different-size")

            self.assertIsNot(cache.get_detector(config), first)
            self.assertEqual(calls, 2)

    def test_does_not_cache_failed_model_construction(self) -> None:
        attempts = 0
        expected = _FakeDetector()

        def flaky_factory(_config: YuNetConfig) -> _FakeDetector:
            nonlocal attempts
            attempts += 1
            if attempts == 1:
                raise RuntimeError("model is temporarily unavailable")
            return expected

        with TemporaryDirectory() as temporary_directory:
            model_path = Path(temporary_directory) / "yunet.onnx"
            model_path.write_bytes(b"model")
            cache = VisionModelCache(detector_factory=flaky_factory)
            config = YuNetConfig(model_path)

            with self.assertRaisesRegex(RuntimeError, "temporarily unavailable"):
                cache.get_detector(config)

            self.assertIs(cache.get_detector(config), expected)
            self.assertEqual(attempts, 2)

    def test_clear_discards_all_cached_instances(self) -> None:
        detector_calls = 0

        def detector_factory(_config: YuNetConfig) -> _FakeDetector:
            nonlocal detector_calls
            detector_calls += 1
            return _FakeDetector()

        with TemporaryDirectory() as temporary_directory:
            model_path = Path(temporary_directory) / "yunet.onnx"
            model_path.write_bytes(b"model")
            cache = VisionModelCache(detector_factory=detector_factory)
            config = YuNetConfig(model_path)
            first = cache.get_detector(config)

            cache.clear()

            self.assertIsNot(cache.get_detector(config), first)
            self.assertEqual(detector_calls, 2)

    def test_targeted_discard_only_removes_matching_entry(self) -> None:
        detector_calls = 0

        def detector_factory(_config: YuNetConfig) -> _FakeDetector:
            nonlocal detector_calls
            detector_calls += 1
            return _FakeDetector()

        with TemporaryDirectory() as temporary_directory:
            root = Path(temporary_directory)
            model_path = root / "yunet.onnx"
            other_path = root / "other.onnx"
            model_path.write_bytes(b"model")
            other_path.write_bytes(b"other")
            cache = VisionModelCache(detector_factory=detector_factory)
            config = YuNetConfig(model_path)
            other = YuNetConfig(other_path)
            first = cache.get_detector(config)

            cache.discard_detector(other)
            self.assertIs(cache.get_detector(config), first)
            cache.discard_detector(config)

            self.assertIsNot(cache.get_detector(config), first)
            self.assertEqual(detector_calls, 2)


if __name__ == "__main__":
    unittest.main()
