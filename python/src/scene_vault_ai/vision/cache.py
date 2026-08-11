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


@dataclass(frozen=True, slots=True)
class ModelFileIdentity:
    path: Path
    size: int
    modified_ns: int

    @classmethod
    def read(cls, path: Path) -> "ModelFileIdentity | None":
        try:
            stat = path.stat()
        except OSError:
            return None
        return cls(
            path=path,
            size=stat.st_size,
            modified_ns=stat.st_mtime_ns,
        )


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
    _detector: tuple[YuNetConfig, ModelFileIdentity, FaceDetector] | None = field(
        default=None,
        init=False,
        repr=False,
    )
    _sface: tuple[ModelFileIdentity, SfaceFeatureExtractor] | None = field(
        default=None,
        init=False,
        repr=False,
    )
    _arcface: tuple[ModelFileIdentity, ArcfaceFeatureExtractor] | None = field(
        default=None,
        init=False,
        repr=False,
    )

    def get_detector(self, config: YuNetConfig) -> FaceDetector:
        identity = ModelFileIdentity.read(config.model_path)
        if (
            identity is not None
            and self._detector is not None
            and self._detector[:2] == (config, identity)
        ):
            return self._detector[2]
        detector = self.detector_factory(config)
        identity = ModelFileIdentity.read(config.model_path)
        self._detector = (config, identity, detector) if identity is not None else None
        return detector

    def get_sface(self, model_path: Path) -> SfaceFeatureExtractor:
        identity = ModelFileIdentity.read(model_path)
        if identity is not None and self._sface is not None and self._sface[0] == identity:
            return self._sface[1]
        extractor = self.sface_factory(SfaceConfig(model_path))
        identity = ModelFileIdentity.read(model_path)
        self._sface = (identity, extractor) if identity is not None else None
        return extractor

    def get_arcface(self, model_path: Path) -> ArcfaceFeatureExtractor:
        identity = ModelFileIdentity.read(model_path)
        if identity is not None and self._arcface is not None and self._arcface[0] == identity:
            return self._arcface[1]
        extractor = self.arcface_factory(ArcfaceConfig(model_path))
        identity = ModelFileIdentity.read(model_path)
        self._arcface = (identity, extractor) if identity is not None else None
        return extractor

    def discard_detector(self, config: YuNetConfig) -> None:
        if self._detector is not None and self._detector[0] == config:
            self._detector = None

    def discard_sface(self, model_path: Path) -> None:
        if self._sface is not None and self._sface[0].path == model_path:
            self._sface = None

    def discard_arcface(self, model_path: Path) -> None:
        if self._arcface is not None and self._arcface[0].path == model_path:
            self._arcface = None

    def clear(self) -> None:
        self._detector = None
        self._sface = None
        self._arcface = None
