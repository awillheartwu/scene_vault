"""Process-local model cache for the serial vision worker."""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass, field
from pathlib import Path

from .config import YuNetConfig
from .detector import FaceDetector, YuNetFaceDetector
from .recognizer import (
    ArcfaceConfig,
    ArcfaceFeatureExtractor,
    SfaceConfig,
    SfaceFeatureExtractor,
)

DetectorFactory = Callable[[YuNetConfig], FaceDetector]
SfaceFactory = Callable[[SfaceConfig], SfaceFeatureExtractor]
ArcfaceFactory = Callable[[ArcfaceConfig], ArcfaceFeatureExtractor]


@dataclass(slots=True)
class VisionModelCache:
    """Caches one active instance per model type.

    The worker is intentionally single-flight, so OpenCV objects are never
    shared by concurrent requests. A changed path or YuNet configuration
    replaces the previous entry instead of growing an unbounded cache.
    Failed construction is not cached, allowing a repaired model to be
    retried by the next request.
    """

    detector_factory: DetectorFactory = YuNetFaceDetector
    sface_factory: SfaceFactory = SfaceFeatureExtractor
    arcface_factory: ArcfaceFactory = ArcfaceFeatureExtractor
    _detector: tuple[YuNetConfig, FaceDetector] | None = field(
        default=None,
        init=False,
        repr=False,
    )
    _sface: tuple[Path, SfaceFeatureExtractor] | None = field(
        default=None,
        init=False,
        repr=False,
    )
    _arcface: tuple[Path, ArcfaceFeatureExtractor] | None = field(
        default=None,
        init=False,
        repr=False,
    )

    def get_detector(self, config: YuNetConfig) -> FaceDetector:
        if self._detector is not None and self._detector[0] == config:
            return self._detector[1]
        detector = self.detector_factory(config)
        self._detector = (config, detector)
        return detector

    def get_sface(self, model_path: Path) -> SfaceFeatureExtractor:
        if self._sface is not None and self._sface[0] == model_path:
            return self._sface[1]
        extractor = self.sface_factory(SfaceConfig(model_path))
        self._sface = (model_path, extractor)
        return extractor

    def get_arcface(self, model_path: Path) -> ArcfaceFeatureExtractor:
        if self._arcface is not None and self._arcface[0] == model_path:
            return self._arcface[1]
        extractor = self.arcface_factory(ArcfaceConfig(model_path))
        self._arcface = (model_path, extractor)
        return extractor

    def discard_detector(self, config: YuNetConfig) -> None:
        if self._detector is not None and self._detector[0] == config:
            self._detector = None

    def discard_sface(self, model_path: Path) -> None:
        if self._sface is not None and self._sface[0] == model_path:
            self._sface = None

    def discard_arcface(self, model_path: Path) -> None:
        if self._arcface is not None and self._arcface[0] == model_path:
            self._arcface = None

    def clear(self) -> None:
        self._detector = None
        self._sface = None
        self._arcface = None
