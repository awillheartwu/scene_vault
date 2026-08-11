"""Single-image processing with local, atomic output writes."""

from __future__ import annotations

import importlib.util
import os
import tempfile
from collections.abc import Callable, Sequence
from dataclasses import dataclass, field
from pathlib import Path
from time import perf_counter
from typing import Any

from ..errors import (
    CapabilityUnavailableError,
    DetectionError,
    ImageDecodeError,
    InputNotFoundError,
    InputReadError,
    OutputWriteError,
    SceneVaultAiError,
)
from .annotator import ImageAnnotator
from .cache import VisionModelCache
from .config import ProcessingRequest, YuNetConfig
from .cropper import AvatarCropper
from .detector import FaceDetector, YuNetFaceDetector, select_primary_face
from .recognizer import (
    ArcfaceConfig,
    ArcfaceFeatureExtractor,
    SfaceConfig,
    SfaceFeatureExtractor,
)
from .types import FaceBox

_VISION_MODULES = {
    "opencv": "cv2",
    "numpy": "numpy",
    "pillow": "PIL",
}
_IMAGE_FORMATS = {
    ".bmp": "BMP",
    ".jpeg": "JPEG",
    ".jpg": "JPEG",
    ".png": "PNG",
    ".webp": "WEBP",
}


def dependency_status() -> dict[str, bool]:
    return {
        name: importlib.util.find_spec(module) is not None
        for name, module in _VISION_MODULES.items()
    }


def dependencies_available() -> bool:
    return all(dependency_status().values())


@dataclass(frozen=True, slots=True)
class ProcessingTimings:
    processor_init_ms: float = 0.0
    read_ms: float = 0.0
    detect_ms: float = 0.0
    feature_ms: float = 0.0
    annotate_ms: float = 0.0
    crop_ms: float = 0.0
    write_ms: float = 0.0
    process_total_ms: float = 0.0
    service_total_ms: float = 0.0

    def to_dict(self) -> dict[str, float]:
        return {
            "processorInitMs": self.processor_init_ms,
            "readMs": self.read_ms,
            "detectMs": self.detect_ms,
            "featureMs": self.feature_ms,
            "annotateMs": self.annotate_ms,
            "cropMs": self.crop_ms,
            "writeMs": self.write_ms,
            "processTotalMs": self.process_total_ms,
            "serviceTotalMs": self.service_total_ms,
        }


@dataclass(frozen=True, slots=True)
class ProcessingResult:
    input_path: Path
    image_width: int
    image_height: int
    annotated_path: Path | None
    avatar_path: Path | None
    face_box: FaceBox | None
    face_feature: list[float] | None = None
    face_feature_model_id: str | None = None
    face_feature_model_version: str | None = None
    face_count: int | None = None
    face_sharpness: float | None = None
    face_area_ratio: float | None = None
    warnings: tuple[str, ...] = ()
    timings: ProcessingTimings = field(default_factory=ProcessingTimings)

    def to_dict(self) -> dict[str, object]:
        return {
            "inputPath": str(self.input_path),
            "imageWidth": self.image_width,
            "imageHeight": self.image_height,
            "annotatedPath": (
                str(self.annotated_path) if self.annotated_path else None
            ),
            "avatarPath": str(self.avatar_path) if self.avatar_path else None,
            "faceDetected": self.face_box is not None,
            "faceBox": self.face_box.to_dict() if self.face_box else None,
            "faceFeature": self.face_feature,
            "faceFeatureModelId": self.face_feature_model_id,
            "faceFeatureModelVersion": self.face_feature_model_version,
            "faceCount": self.face_count,
            "faceSharpness": self.face_sharpness,
            "faceAreaRatio": self.face_area_ratio,
            "warnings": list(self.warnings),
            "timings": self.timings.to_dict(),
        }


DetectorFactory = Callable[[YuNetConfig], FaceDetector]
ProgressCallback = Callable[[str, float], None]


class ScreenshotProcessor:
    def __init__(
        self,
        request: ProcessingRequest,
        *,
        detector_factory: DetectorFactory | None = None,
        model_cache: VisionModelCache | None = None,
        progress: ProgressCallback | None = None,
    ) -> None:
        status = dependency_status()
        missing = sorted(name for name, available in status.items() if not available)
        if missing:
            raise CapabilityUnavailableError(
                "vision dependencies are not installed",
                details={
                    "capability": "processScreenshot",
                    "missingDependencies": missing,
                    "installExtra": "vision",
                },
            )
        if not request.input_path.is_file():
            raise InputNotFoundError(
                "screenshot was not found",
                details={"path": str(request.input_path)},
            )
        self.request = request
        self.detector_factory = detector_factory or YuNetFaceDetector
        self.model_cache = model_cache if detector_factory is None else None
        self.progress = progress
        self.feature_extractor: SfaceFeatureExtractor | ArcfaceFeatureExtractor | None = None
        self.extractor_error: str | None = None
        if request.recognizer == "arcface":
            assert request.arcface_model_path is not None
            try:
                self.feature_extractor = (
                    self.model_cache.get_arcface(request.arcface_model_path)
                    if self.model_cache is not None
                    else ArcfaceFeatureExtractor(
                        ArcfaceConfig(request.arcface_model_path)
                    )
                )
            except Exception as error:
                self.extractor_error = f"arcface_feature_failed: {error}"
        elif request.sface_model_path is not None:
            try:
                self.feature_extractor = (
                    self.model_cache.get_sface(request.sface_model_path)
                    if self.model_cache is not None
                    else SfaceFeatureExtractor(SfaceConfig(request.sface_model_path))
                )
            except Exception as error:
                self.extractor_error = f"sface_feature_failed: {error}"

    def process(self) -> ProcessingResult:
        process_started = perf_counter()
        phase_started = perf_counter()
        image_bgr, image = self._load_image()
        read_ms = _elapsed_ms(phase_started)
        image_width, image_height = image.size
        self._progress("read", 5.0)

        self._progress("detect_face", 20.0)
        phase_started = perf_counter()
        faces, sharpness_values, face_count = self._detect_faces(image_bgr)
        detect_ms = _elapsed_ms(phase_started)
        if faces:
            yunet_config = self.request.yunet
            if yunet_config is None:
                raise RuntimeError(
                    "face detection returned results without configuration"
                )
            face_box = select_primary_face(
                faces,
                image_width=image_width,
                image_height=image_height,
                config=yunet_config,
                sharpness_values=sharpness_values,
            )
        else:
            face_box = None
        face_sharpness = (
            sharpness_values[faces.index(face_box)]
            if face_box is not None and faces
            else None
        )
        face_area_ratio = (
            (face_box.width * face_box.height) / (image_width * image_height)
            if face_box is not None
            else None
        )
        warnings: list[str] = []
        if self.request.detect_face and face_box is None:
            warnings.append("face_not_detected")
        if self.request.crop_avatar and face_box is None:
            warnings.append("avatar_not_generated")
        face_feature: list[float] | None = None
        feature_ms = 0.0
        if face_box is not None:
            if self.extractor_error is not None:
                warnings.append(self.extractor_error)
            elif self.feature_extractor is not None:
                self._progress("extract_feature", 45.0)
                phase_started = perf_counter()
                try:
                    face_feature = self.feature_extractor.extract(image_bgr, face_box)
                    model_id = type(self.feature_extractor).MODEL_ID
                    model_version = getattr(
                        self.feature_extractor,
                        "model_version",
                        type(self.feature_extractor).MODEL_VERSION,
                    )
                except Exception as error:
                    # Optional feature extraction must never fail the whole
                    # screenshot processing pipeline.
                    warnings.append(f"feature_failed: {error}")
                    self._discard_failed_extractor()
                    face_feature = None
                    model_id = None
                    model_version = None
                feature_ms = _elapsed_ms(phase_started)

        outputs: list[tuple[Any, Path]] = []
        annotate_ms = 0.0
        crop_ms = 0.0

        annotated_path = None
        if self.request.annotate:
            self._progress("annotate", 75.0)
            phase_started = perf_counter()
            if (
                self.request.character_name is None
                or self.request.annotated_output_path is None
            ):
                raise RuntimeError("validated annotation request became incomplete")
            annotated = ImageAnnotator(self.request.annotation).annotate(
                image,
                self.request.character_name,
                face_box,
            )
            annotated_path = self.request.annotated_output_path
            outputs.append((annotated, annotated_path))
            annotate_ms = _elapsed_ms(phase_started)

        avatar_path = None
        if (
            self.request.crop_avatar
            and face_box is not None
            and self.request.avatar_output_path is not None
        ):
            self._progress("crop_avatar", 90.0)
            phase_started = perf_counter()
            avatar = AvatarCropper(self.request.crop).crop(image, face_box)
            avatar_path = self.request.avatar_output_path
            outputs.append((avatar, avatar_path))
            crop_ms = _elapsed_ms(phase_started)

        phase_started = perf_counter()
        _save_images_atomically(outputs)
        write_ms = _elapsed_ms(phase_started)
        self._progress("done", 100.0)

        return ProcessingResult(
            input_path=self.request.input_path,
            image_width=image_width,
            image_height=image_height,
            annotated_path=annotated_path,
            avatar_path=avatar_path,
            face_box=face_box,
            face_feature=face_feature,
            face_feature_model_id=model_id if face_feature is not None else None,
            face_feature_model_version=(
                model_version if face_feature is not None else None
            ),
            face_count=face_count,
            face_sharpness=face_sharpness,
            face_area_ratio=face_area_ratio,
            warnings=tuple(warnings),
            timings=ProcessingTimings(
                read_ms=read_ms,
                detect_ms=detect_ms,
                feature_ms=feature_ms,
                annotate_ms=annotate_ms,
                crop_ms=crop_ms,
                write_ms=write_ms,
                process_total_ms=_elapsed_ms(process_started),
            ),
        )

    def _progress(self, stage: str, percent: float) -> None:
        if self.progress is not None:
            self.progress(stage, percent)

    def _discard_failed_extractor(self) -> None:
        if self.model_cache is None:
            return
        if self.request.recognizer == "arcface":
            assert self.request.arcface_model_path is not None
            self.model_cache.discard_arcface(self.request.arcface_model_path)
        elif self.request.sface_model_path is not None:
            self.model_cache.discard_sface(self.request.sface_model_path)

    def _load_image(self) -> tuple[Any, Any]:
        try:
            import cv2
            import numpy as np
            from PIL import Image

            image_bytes = np.fromfile(self.request.input_path, dtype=np.uint8)
        except OSError as error:
            raise InputReadError(
                "screenshot could not be read",
                details={"path": str(self.request.input_path)},
            ) from error

        if image_bytes.size == 0:
            raise ImageDecodeError(
                "screenshot is empty",
                details={"path": str(self.request.input_path)},
            )

        try:
            image_bgr = cv2.imdecode(image_bytes, cv2.IMREAD_COLOR)
        except Exception as error:
            raise ImageDecodeError(
                "screenshot could not be decoded",
                details={"path": str(self.request.input_path)},
            ) from error
        if image_bgr is None:
            raise ImageDecodeError(
                "screenshot could not be decoded",
                details={"path": str(self.request.input_path)},
            )

        try:
            image_rgb = cv2.cvtColor(image_bgr, cv2.COLOR_BGR2RGB)
            image = Image.fromarray(image_rgb)
        except Exception as error:
            raise ImageDecodeError(
                "decoded screenshot has an unsupported pixel layout",
                details={"path": str(self.request.input_path)},
            ) from error
        return image_bgr, image

    def _detect_faces(
        self, image_bgr: Any
    ) -> tuple[list[FaceBox], list[float], int | None]:
        if not self.request.detect_face:
            return [], [], None
        if self.request.yunet is None:
            raise RuntimeError("validated face-detection request became incomplete")

        try:
            detector = (
                self.model_cache.get_detector(self.request.yunet)
                if self.model_cache is not None
                else self.detector_factory(self.request.yunet)
            )
        except SceneVaultAiError:
            raise
        except Exception as error:
            raise DetectionError(
                "face detector initialization failed",
                details={"backend": "yunet"},
            ) from error

        try:
            faces, sharpness_values = detector.detect_faces(image_bgr)
            return faces, sharpness_values, len(faces)
        except Exception as error:
            if self.model_cache is not None:
                self.model_cache.discard_detector(self.request.yunet)
            if isinstance(error, SceneVaultAiError):
                raise
            raise DetectionError(
                "face detection failed",
                details={"backend": "yunet"},
            ) from error


def _save_images_atomically(outputs: Sequence[tuple[Any, Path]]) -> None:
    """Encode all images first, then atomically replace each destination."""

    staged: list[tuple[Path, Path]] = []
    active_path: Path | None = None
    try:
        for image, output_path in outputs:
            active_path = output_path
            output_path.parent.mkdir(parents=True, exist_ok=True)
            file_descriptor, temporary_name = tempfile.mkstemp(
                prefix=f".{output_path.stem}-",
                suffix=output_path.suffix,
                dir=output_path.parent,
            )
            os.close(file_descriptor)
            temporary_path = Path(temporary_name)
            staged.append((temporary_path, output_path))

            image_format = _IMAGE_FORMATS[output_path.suffix.casefold()]
            image.save(temporary_path, format=image_format)
            with temporary_path.open("rb+") as file:
                os.fsync(file.fileno())

        for temporary_path, output_path in staged:
            active_path = output_path
            os.replace(temporary_path, output_path)
    except (KeyError, OSError, ValueError) as error:
        raise OutputWriteError(
            "processed image could not be written",
            details={"path": str(active_path) if active_path else None},
        ) from error
    finally:
        for temporary_path, _ in staged:
            try:
                temporary_path.unlink(missing_ok=True)
            except OSError:
                pass


def _elapsed_ms(started: float) -> float:
    return round((perf_counter() - started) * 1000.0, 3)
