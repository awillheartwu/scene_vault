"""YuNet face detection and deterministic primary-face selection."""

from __future__ import annotations

import math
from typing import Any, Protocol

from ..errors import DetectionError, ResourceNotFoundError
from .config import YuNetConfig
from .types import FaceBox


class FaceDetector(Protocol):
    def detect_faces(
        self, image_bgr: Any
    ) -> tuple[list[FaceBox], list[float]]:
        """Return every detected face with aligned sharpness values."""

    def detect_primary_face(self, image_bgr: Any) -> FaceBox | None:
        """Return the best face or ``None`` when no face was detected."""


class YuNetFaceDetector:
    def __init__(self, config: YuNetConfig) -> None:
        import cv2

        if not config.model_path.is_file():
            raise ResourceNotFoundError(
                "YuNet model was not found",
                details={
                    "resource": "yunet_model",
                    "path": str(config.model_path),
                },
            )

        create = _resolve_yunet_factory(cv2)
        if create is None:
            raise DetectionError(
                "this OpenCV build does not support YuNet FaceDetectorYN",
                details={"backend": "yunet"},
            )

        self.config = config
        self._cv2 = cv2
        try:
            self._detector = create(
                str(config.model_path),
                "",
                (320, 320),
                config.score_threshold,
                config.nms_threshold,
                config.top_k,
            )
        except Exception as error:
            raise DetectionError(
                "YuNet could not load the configured model",
                details={
                    "backend": "yunet",
                    "path": str(config.model_path),
                },
            ) from error

    def detect_faces(
        self, image_bgr: Any
    ) -> tuple[list[FaceBox], list[float]]:
        """Return (faces, sharpness values) for every detected face. The
        lists stay aligned; used by the processor for face counting and by
        primary-face selection for candidate ranking."""
        image_height, image_width = image_bgr.shape[:2]
        try:
            self._detector.setInputSize((image_width, image_height))
            _, faces = self._detector.detect(image_bgr)
        except Exception as error:
            raise DetectionError(
                "YuNet failed while detecting faces",
                details={"backend": "yunet"},
            ) from error

        if faces is None or len(faces) == 0:
            return [], []

        candidates: list[FaceBox] = []
        for raw_face in faces:
            landmarks = tuple(
                (float(raw_face[4 + 2 * i]), float(raw_face[5 + 2 * i]))
                for i in range(5)
            )
            try:
                face = _clamp_face(
                    x=float(raw_face[0]),
                    y=float(raw_face[1]),
                    width=float(raw_face[2]),
                    height=float(raw_face[3]),
                    confidence=float(raw_face[-1]),
                    landmarks=landmarks,
                    image_width=image_width,
                    image_height=image_height,
                )
            except (IndexError, TypeError, ValueError):
                continue
            if face is not None:
                candidates.append(face)

        if not candidates:
            return [], []

        try:
            gray = self._cv2.cvtColor(image_bgr, self._cv2.COLOR_BGR2GRAY)
            sharpness_values = [
                _face_sharpness(gray, face, self._cv2) for face in candidates
            ]
        except Exception as error:
            raise DetectionError(
                "YuNet could not score detected faces",
                details={"backend": "yunet"},
            ) from error

        return candidates, sharpness_values

    def detect_all_faces(self, image_bgr: Any) -> list[FaceBox]:
        """Every detected face ordered by sharpness descending. Used by the
        benchmark to reject multi-face images (their filename label is
        ambiguous)."""
        candidates, sharpness_values = self.detect_faces(image_bgr)
        return [
            face
            for face, _ in sorted(
                zip(candidates, sharpness_values, strict=True),
                key=lambda item: item[1],
                reverse=True,
            )
        ]

    def detect_primary_face(self, image_bgr: Any) -> FaceBox | None:
        """Return the best face or ``None`` when no face was detected."""
        candidates, sharpness_values = self.detect_faces(image_bgr)
        if not candidates:
            return None
        image_height, image_width = image_bgr.shape[:2]
        return select_primary_face(
            candidates,
            image_width=image_width,
            image_height=image_height,
            config=self.config,
            sharpness_values=sharpness_values,
        )


def select_primary_face(
    candidates: list[FaceBox],
    *,
    image_width: int,
    image_height: int,
    config: YuNetConfig,
    sharpness_values: list[float] | None = None,
) -> FaceBox | None:
    """Select one face using pure, independently testable scoring logic."""

    if not candidates:
        return None
    if image_width <= 0 or image_height <= 0:
        raise ValueError("image dimensions must be positive")

    if sharpness_values is None:
        sharpness_values = [config.min_sharpness] * len(candidates)
    if len(sharpness_values) != len(candidates):
        raise ValueError("sharpness_values must match the candidate count")

    image_center = (image_width / 2, image_height / 2)
    image_area = max(1, image_width * image_height)
    diagonal_half = max(
        1.0,
        math.hypot(image_width / 2, image_height / 2),
    )

    def score(item: tuple[FaceBox, float]) -> float:
        face, sharpness = item
        center_x, center_y = face.center
        distance = math.hypot(
            center_x - image_center[0],
            center_y - image_center[1],
        )
        center_score = max(0.0, 1.0 - distance / diagonal_half)
        safe_sharpness = max(0.0, sharpness) if math.isfinite(sharpness) else 0.0
        sharpness_score = min(
            safe_sharpness / max(1.0, config.min_sharpness),
            2.0,
        )
        blur_penalty = max(
            0.0,
            1.0 - safe_sharpness / max(1.0, config.min_sharpness),
        )

        horizontal_edge = (
            center_x < image_width * config.edge_margin_ratio
            or center_x > image_width * (1.0 - config.edge_margin_ratio)
        )
        vertical_edge = (
            center_y < image_height * config.edge_margin_ratio
            or center_y > image_height * (1.0 - config.edge_margin_ratio)
        )
        edge_penalty = float(horizontal_edge) + 0.5 * float(vertical_edge)

        return (
            face.area / image_area * config.area_weight
            + face.confidence * config.confidence_weight
            + center_score * config.center_weight
            + sharpness_score * config.sharpness_weight
            - edge_penalty * config.edge_penalty_weight
            - blur_penalty * config.blur_penalty_weight
        )

    return max(zip(candidates, sharpness_values, strict=True), key=score)[0]


def _resolve_yunet_factory(cv2: Any) -> Any | None:
    face_detector_class = getattr(cv2, "FaceDetectorYN", None)
    class_factory = getattr(face_detector_class, "create", None)
    if callable(class_factory):
        return class_factory

    legacy_factory = getattr(cv2, "FaceDetectorYN_create", None)
    return legacy_factory if callable(legacy_factory) else None


def _clamp_face(
    *,
    x: float,
    y: float,
    width: float,
    height: float,
    confidence: float,
    landmarks: tuple[tuple[float, float], ...] | None = None,
    image_width: int,
    image_height: int,
) -> FaceBox | None:
    if not all(math.isfinite(value) for value in (x, y, width, height, confidence)):
        return None
    if width <= 0 or height <= 0:
        return None

    left = max(0, int(math.floor(x)))
    top = max(0, int(math.floor(y)))
    right = min(image_width, int(math.ceil(x + width)))
    bottom = min(image_height, int(math.ceil(y + height)))
    if right <= left or bottom <= top:
        return None
    return FaceBox(
        x=left,
        y=top,
        width=right - left,
        height=bottom - top,
        confidence=min(1.0, max(0.0, confidence)),
        landmarks=landmarks,
    )


def _face_sharpness(gray: Any, face: FaceBox, cv2: Any) -> float:
    right = min(gray.shape[1], face.x + face.width)
    bottom = min(gray.shape[0], face.y + face.height)
    roi = gray[face.y:bottom, face.x:right]
    return float(cv2.Laplacian(roi, cv2.CV_64F).var()) if roi.size else 0.0
