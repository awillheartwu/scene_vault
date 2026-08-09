"""SFace face-feature extraction for the Face Bank suggestion flow.

The extractor only turns one detected primary face into a fixed-length
feature vector; similarity matching against the sample bank stays on the
Rust side, so this module never holds per-character state.
"""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Any

from ..errors import CapabilityUnavailableError, ResourceNotFoundError
from .types import FaceBox


@dataclass(frozen=True, slots=True)
class SfaceConfig:
    model_path: Path


class SfaceFeatureExtractor:
    """OpenCV Zoo SFace (face_recognition_sface_2021dec.onnx).

    The model identity travels with every feature so Rust can refuse to
    compare embeddings from different models (different vector spaces).
    """

    MODEL_ID = "opencv-sface"
    MODEL_VERSION = "2021dec"

    def __init__(self, config: SfaceConfig) -> None:
        if not config.model_path.is_file():
            raise ResourceNotFoundError(
                "SFace model was not found",
                details={"path": str(config.model_path)},
            )
        import cv2

        create = getattr(cv2, "FaceRecognizerSF", None)
        factory = getattr(create, "create", None) or getattr(
            cv2, "FaceRecognizerSF_create", None
        )
        if factory is None:
            raise CapabilityUnavailableError(
                "this OpenCV build does not support SFace FaceRecognizerSF",
                details={
                    "capability": "sfaceFeature",
                    "installExtra": "vision",
                },
            )
        self.recognizer = factory(str(config.model_path), "")

    def extract(self, image_bgr: Any, face: FaceBox) -> list[float]:
        """Aligns the primary face and returns the SFace feature vector."""
        import numpy as np

        # Passing the five YuNet landmarks (1x14 box) is REQUIRED for
        # determinism: with a bare 1x4 bounding box, alignCrop produces
        # different aligned crops across processes (verified 2026-08-08),
        # which silently corrupts Face Bank matching. With landmarks the
        # crop and feature are bit-identical across processes.
        if face.landmarks is not None:
            values: list[float] = [face.x, face.y, face.width, face.height]
            for point_x, point_y in face.landmarks:
                values.extend((point_x, point_y))
            box = np.array(values, dtype=np.float32).reshape(1, 14)
        else:
            box = np.array(
                [face.x, face.y, face.width, face.height],
                dtype=np.float32,
            )
        aligned = self.recognizer.alignCrop(image_bgr, box)
        feature = self.recognizer.feature(aligned)
        return [float(value) for value in feature.reshape(-1)]


@dataclass(frozen=True, slots=True)
class ArcfaceConfig:
    model_path: Path


class ArcfaceFeatureExtractor:
    """InsightFace ArcFace R50 (w600k_r50.onnx) feature extractor.

    Benchmark-only for now: the runtime pipeline still uses SFace until the
    A/B numbers decide. Preprocessing mirrors InsightFace's ArcFaceONNX:
    similarity-transform alignment of the five YuNet landmarks to the
    112x112 template, RGB blob normalized with (x - 127.5) / 127.5.
    """

    MODEL_ID = "arcface-r50"
    MODEL_VERSION = "w600k-r50"
    INPUT_SIZE = 112
    EMBEDDING_DIM = 512

    # InsightFace arcface_dst template, ordered (left eye, right eye, nose
    # tip, left mouth corner, right mouth corner) — the same order YuNet
    # reports its five landmarks, so no reordering is needed.
    TEMPLATE = (
        (38.2946, 51.6963),
        (73.5318, 51.5014),
        (56.0252, 71.7366),
        (41.5493, 92.3655),
        (70.7299, 92.2041),
    )

    def __init__(self, config: ArcfaceConfig) -> None:
        if not config.model_path.is_file():
            raise ResourceNotFoundError(
                "ArcFace model was not found",
                details={"path": str(config.model_path)},
            )
        import cv2

        self._net = cv2.dnn.readNetFromONNX(str(config.model_path))

    def extract(self, image_bgr: Any, face: FaceBox) -> list[float]:
        """Aligns the face with its five landmarks and returns the 512-d
        ArcFace embedding."""
        if face.landmarks is None:
            raise ValueError(
                "ArcFace alignment requires the five facial landmarks"
            )
        import cv2
        import numpy as np

        source = np.array(face.landmarks, dtype=np.float32)
        target = np.array(self.TEMPLATE, dtype=np.float32)
        transform, _ = cv2.estimateAffinePartial2D(
            source, target, method=cv2.LMEDS
        )
        if transform is None:
            raise ValueError("face alignment failed")
        aligned = cv2.warpAffine(
            image_bgr,
            transform,
            (self.INPUT_SIZE, self.INPUT_SIZE),
            borderValue=0.0,
        )
        blob = cv2.dnn.blobFromImage(
            aligned,
            1.0 / 127.5,
            (self.INPUT_SIZE, self.INPUT_SIZE),
            (127.5, 127.5, 127.5),
            swapRB=True,
        )
        self._net.setInput(blob)
        embedding = self._net.forward()
        return [float(value) for value in embedding.reshape(-1)]
