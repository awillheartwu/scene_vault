"""Strict configuration parsing for one screenshot-processing request."""

from __future__ import annotations

import math
import ntpath
import os
from dataclasses import dataclass
from pathlib import Path, PureWindowsPath
from typing import Any

from ..errors import InvalidPayloadError

SUPPORTED_OUTPUT_SUFFIXES = frozenset({".bmp", ".jpeg", ".jpg", ".png", ".webp"})

_PAYLOAD_FIELDS = frozenset(
    {
        "inputPath",
        "annotatedOutputPath",
        "avatarOutputPath",
        "characterName",
        "detectFace",
        "annotate",
        "cropAvatar",
        "yunetModelPath",
        "sfaceModelPath",
        "recognizer",
        "arcfaceModelPath",
        "detection",
        "annotation",
        "crop",
    }
)
_DETECTION_FIELDS = frozenset(
    {
        "scoreThreshold",
        "nmsThreshold",
        "topK",
        "areaWeight",
        "confidenceWeight",
        "centerWeight",
        "sharpnessWeight",
        "edgePenaltyWeight",
        "edgeMarginRatio",
        "minSharpness",
        "blurPenaltyWeight",
    }
)
_ANNOTATION_FIELDS = frozenset(
    {
        "textColor",
        "strokeColor",
        "strokeWidth",
        "padding",
        "faceBoxExpansion",
        "faceTextPosition",
        "fallbackPosition",
        "fontPath",
        "fontSize",
        "textOffsetX",
        "textOffsetY",
    }
)
_CROP_FIELDS = frozenset(
    {
        "aspectRatio",
        "scaleX",
        "scaleTop",
        "scaleBottom",
        "minSize",
    }
)
_FALLBACK_POSITIONS = frozenset(
    {"top_left", "top_right", "bottom_left", "bottom_right"}
)
_FACE_TEXT_POSITIONS = frozenset(
    {"above", "right", "below", "left", "custom"}
)


@dataclass(frozen=True, slots=True)
class YuNetConfig:
    model_path: Path
    score_threshold: float = 0.6
    nms_threshold: float = 0.3
    top_k: int = 5000
    area_weight: float = 1.5
    confidence_weight: float = 1.0
    center_weight: float = 3.0
    sharpness_weight: float = 1.2
    edge_penalty_weight: float = 2.0
    edge_margin_ratio: float = 0.18
    min_sharpness: float = 120.0
    blur_penalty_weight: float = 1.5


@dataclass(frozen=True, slots=True)
class AnnotationConfig:
    text_color: tuple[int, int, int] = (80, 220, 255)
    stroke_color: tuple[int, int, int] = (0, 0, 0)
    stroke_width: int = 2
    padding: int = 32
    # Symmetric pixel expansion of the detected face reference box used for
    # text placement; padding remains the canvas safety/text gap.
    face_box_expansion: int = 0
    face_text_position: str = "above"
    fallback_position: str = "top_left"
    # Custom text placement relative to the face box when
    # face_text_position == "custom"; multiples of box width/height.
    text_offset_x: float | None = None
    text_offset_y: float | None = None
    font_path: Path | None = None
    font_size: int = 48


@dataclass(frozen=True, slots=True)
class CropConfig:
    aspect_ratio: tuple[int, int] = (1, 1)
    scale_x: float = 1.8
    scale_top: float = 1.3
    scale_bottom: float = 1.8
    min_size: int = 224


@dataclass(frozen=True, slots=True)
class ProcessingRequest:
    input_path: Path
    annotated_output_path: Path | None
    avatar_output_path: Path | None
    character_name: str | None
    detect_face: bool
    annotate: bool
    crop_avatar: bool
    yunet: YuNetConfig | None
    sface_model_path: Path | None
    annotation: AnnotationConfig
    crop: CropConfig
    recognizer: str = "sface"
    arcface_model_path: Path | None = None

    @classmethod
    def from_payload(cls, payload: dict[str, Any]) -> "ProcessingRequest":
        _reject_unknown_fields(payload, _PAYLOAD_FIELDS, "payload")

        input_path = _required_local_path(payload, "inputPath")
        annotate = _boolean(payload, "annotate", True)
        crop_avatar = _boolean(payload, "cropAvatar", True)
        detect_face = _boolean(payload, "detectFace", True)

        annotated_output_path = _optional_local_path(
            payload.get("annotatedOutputPath"),
            "annotatedOutputPath",
        )
        avatar_output_path = _optional_local_path(
            payload.get("avatarOutputPath"),
            "avatarOutputPath",
        )
        character_name = _character_name(payload.get("characterName"))

        if not any((annotate, crop_avatar, detect_face)):
            raise _invalid(
                "at least one processing operation must be enabled",
                "detectFace",
            )
        if annotate and annotated_output_path is None:
            raise _invalid(
                "annotatedOutputPath is required when annotate is enabled",
                "annotatedOutputPath",
            )
        if not annotate and annotated_output_path is not None:
            raise _invalid(
                "annotatedOutputPath is only allowed when annotate is enabled",
                "annotatedOutputPath",
            )
        if annotate and character_name is None:
            raise _invalid(
                "characterName is required when annotate is enabled",
                "characterName",
            )
        if crop_avatar and avatar_output_path is None:
            raise _invalid(
                "avatarOutputPath is required when cropAvatar is enabled",
                "avatarOutputPath",
            )
        if not crop_avatar and avatar_output_path is not None:
            raise _invalid(
                "avatarOutputPath is only allowed when cropAvatar is enabled",
                "avatarOutputPath",
            )
        if crop_avatar and not detect_face:
            raise _invalid("cropAvatar requires detectFace", "cropAvatar")

        model_path = _optional_local_path(
            payload.get("yunetModelPath"),
            "yunetModelPath",
        )
        if detect_face and model_path is None:
            raise _invalid(
                "yunetModelPath is required when detectFace is enabled",
                "yunetModelPath",
            )
        sface_model_path = _optional_local_path(
            payload.get("sfaceModelPath"),
            "sfaceModelPath",
        )
        recognizer = _recognizer(payload.get("recognizer"))
        arcface_model_path = _optional_local_path(
            payload.get("arcfaceModelPath"),
            "arcfaceModelPath",
        )
        if recognizer == "arcface" and arcface_model_path is None:
            raise _invalid(
                "arcfaceModelPath is required when recognizer is arcface",
                "arcfaceModelPath",
            )

        detection_payload = _object(payload.get("detection"), "detection")
        annotation_payload = _object(payload.get("annotation"), "annotation")
        crop_payload = _object(payload.get("crop"), "crop")
        _reject_unknown_fields(detection_payload, _DETECTION_FIELDS, "detection")
        _reject_unknown_fields(annotation_payload, _ANNOTATION_FIELDS, "annotation")
        _reject_unknown_fields(crop_payload, _CROP_FIELDS, "crop")

        yunet = (
            YuNetConfig(
                model_path=model_path,
                score_threshold=_number(
                    detection_payload,
                    "scoreThreshold",
                    0.6,
                    minimum=0.0,
                    maximum=1.0,
                ),
                nms_threshold=_number(
                    detection_payload,
                    "nmsThreshold",
                    0.3,
                    minimum=0.0,
                    maximum=1.0,
                ),
                top_k=_integer(
                    detection_payload,
                    "topK",
                    5000,
                    minimum=1,
                    maximum=100_000,
                ),
                area_weight=_number(
                    detection_payload,
                    "areaWeight",
                    1.5,
                    minimum=0.0,
                    maximum=100.0,
                ),
                confidence_weight=_number(
                    detection_payload,
                    "confidenceWeight",
                    1.0,
                    minimum=0.0,
                    maximum=100.0,
                ),
                center_weight=_number(
                    detection_payload,
                    "centerWeight",
                    3.0,
                    minimum=0.0,
                    maximum=100.0,
                ),
                sharpness_weight=_number(
                    detection_payload,
                    "sharpnessWeight",
                    1.2,
                    minimum=0.0,
                    maximum=100.0,
                ),
                edge_penalty_weight=_number(
                    detection_payload,
                    "edgePenaltyWeight",
                    2.0,
                    minimum=0.0,
                    maximum=100.0,
                ),
                edge_margin_ratio=_number(
                    detection_payload,
                    "edgeMarginRatio",
                    0.18,
                    minimum=0.0,
                    maximum=0.49,
                ),
                min_sharpness=_number(
                    detection_payload,
                    "minSharpness",
                    120.0,
                    minimum=0.01,
                    maximum=1_000_000.0,
                ),
                blur_penalty_weight=_number(
                    detection_payload,
                    "blurPenaltyWeight",
                    1.5,
                    minimum=0.0,
                    maximum=100.0,
                ),
            )
            if model_path is not None
            else None
        )

        annotation = AnnotationConfig(
            text_color=_color(annotation_payload.get("textColor"), (80, 220, 255)),
            stroke_color=_color(annotation_payload.get("strokeColor"), (0, 0, 0)),
            stroke_width=_integer(
                annotation_payload,
                "strokeWidth",
                2,
                minimum=0,
                maximum=64,
            ),
            padding=_integer(
                annotation_payload,
                "padding",
                32,
                minimum=0,
                maximum=4096,
            ),
            face_box_expansion=_integer(
                annotation_payload,
                "faceBoxExpansion",
                0,
                minimum=0,
                maximum=4096,
            ),
            face_text_position=_choice(
                annotation_payload,
                "faceTextPosition",
                "above",
                _FACE_TEXT_POSITIONS,
            ),
            fallback_position=_choice(
                annotation_payload,
                "fallbackPosition",
                "top_left",
                _FALLBACK_POSITIONS,
            ),
            text_offset_x=_optional_offset(
                annotation_payload,
                "textOffsetX",
            ),
            text_offset_y=_optional_offset(
                annotation_payload,
                "textOffsetY",
            ),
            font_path=_optional_local_path(
                annotation_payload.get("fontPath"),
                "annotation.fontPath",
            ),
            font_size=_integer(
                annotation_payload,
                "fontSize",
                48,
                minimum=1,
                maximum=512,
            ),
        )
        crop = CropConfig(
            aspect_ratio=_ratio(crop_payload.get("aspectRatio", "1:1")),
            scale_x=_number(
                crop_payload,
                "scaleX",
                1.8,
                minimum=0.1,
                maximum=20.0,
            ),
            scale_top=_number(
                crop_payload,
                "scaleTop",
                1.3,
                minimum=0.1,
                maximum=20.0,
            ),
            scale_bottom=_number(
                crop_payload,
                "scaleBottom",
                1.8,
                minimum=0.1,
                maximum=20.0,
            ),
            min_size=_integer(
                crop_payload,
                "minSize",
                224,
                minimum=1,
                maximum=16_384,
            ),
        )

        output_paths = [
            path
            for path in (annotated_output_path, avatar_output_path)
            if path is not None
        ]
        for output_path in output_paths:
            if output_path.suffix.casefold() not in SUPPORTED_OUTPUT_SUFFIXES:
                raise InvalidPayloadError(
                    "output image extension is not supported",
                    details={
                        "field": "outputPath",
                        "path": str(output_path),
                        "supportedExtensions": sorted(SUPPORTED_OUTPUT_SUFFIXES),
                    },
                )

        path_keys = [_comparison_key(input_path), *map(_comparison_key, output_paths)]
        if len(path_keys) != len(set(path_keys)):
            raise InvalidPayloadError(
                "input and output paths must all be different",
                details={"field": "outputPath"},
            )

        return cls(
            input_path=input_path,
            annotated_output_path=annotated_output_path,
            avatar_output_path=avatar_output_path,
            character_name=character_name,
            detect_face=detect_face,
            annotate=annotate,
            crop_avatar=crop_avatar,
            yunet=yunet,
            sface_model_path=sface_model_path,
            recognizer=recognizer,
            arcface_model_path=arcface_model_path,
            annotation=annotation,
            crop=crop,
        )


def _reject_unknown_fields(
    value: dict[str, Any],
    allowed: frozenset[str],
    field: str,
) -> None:
    unknown_fields = sorted(set(value) - allowed)
    if unknown_fields:
        raise InvalidPayloadError(
            f"{field} contains unknown fields",
            details={"field": field, "fields": unknown_fields},
        )


def _required_local_path(payload: dict[str, Any], key: str) -> Path:
    path = _optional_local_path(payload.get(key), key)
    if path is None:
        raise _invalid(f"{key} is required", key)
    return path


def _optional_local_path(value: object, field: str) -> Path | None:
    if value is None:
        return None
    if not isinstance(value, str):
        raise _invalid(f"{field} must be a string path", field)
    if not value or value != value.strip():
        raise _invalid(
            f"{field} must be a non-empty path without surrounding whitespace",
            field,
        )
    if "\x00" in value:
        raise _invalid(f"{field} contains a null byte", field)
    if value.casefold().startswith("file:"):
        raise _invalid(f"{field} must be a filesystem path, not a URI", field)
    if not _is_absolute(value):
        raise _invalid(f"{field} must be an absolute path", field)
    if _is_unc(value):
        raise _invalid(f"{field} must be local, not a UNC/NAS path", field)
    if _is_windows_device_path(value):
        raise _invalid(f"{field} must not use a Windows device namespace", field)
    return Path(value)


def _character_name(value: object) -> str | None:
    if value is None:
        return None
    if not isinstance(value, str):
        raise _invalid("characterName must be a string", "characterName")
    text = value.strip()
    if not text:
        raise _invalid("characterName must not be empty", "characterName")
    if len(text) > 256:
        raise _invalid(
            "characterName must contain at most 256 characters",
            "characterName",
        )
    if any(ord(character) < 32 or ord(character) == 127 for character in text):
        raise _invalid(
            "characterName must not contain control characters",
            "characterName",
        )
    return text


def _recognizer(value: object) -> str:
    if value is None:
        return "sface"
    if not isinstance(value, str) or value not in ("sface", "arcface"):
        raise _invalid(
            "recognizer must be 'sface' or 'arcface'",
            "recognizer",
        )
    return value


def _boolean(payload: dict[str, Any], key: str, default: bool) -> bool:
    value = payload.get(key, default)
    if type(value) is not bool:
        raise _invalid(f"{key} must be a boolean", key)
    return value


def _object(value: object, key: str) -> dict[str, Any]:
    if value is None:
        return {}
    if not isinstance(value, dict):
        raise _invalid(f"{key} must be an object", key)
    return value


def _color(value: object, default: tuple[int, int, int]) -> tuple[int, int, int]:
    if value is None:
        return default
    if not isinstance(value, list) or len(value) != 3:
        raise _invalid("colors must be RGB arrays with three channels", "color")
    if any(type(channel) is not int for channel in value):
        raise _invalid("color channels must be integers", "color")
    if any(channel < 0 or channel > 255 for channel in value):
        raise _invalid("color channels must be between 0 and 255", "color")
    return value[0], value[1], value[2]


def _integer(
    payload: dict[str, Any],
    key: str,
    default: int,
    *,
    minimum: int,
    maximum: int,
) -> int:
    value = payload.get(key, default)
    if type(value) is not int:
        raise _invalid(f"{key} must be an integer", key)
    if not minimum <= value <= maximum:
        raise _invalid(
            f"{key} must be between {minimum} and {maximum}",
            key,
        )
    return value


def _number(
    payload: dict[str, Any],
    key: str,
    default: float,
    *,
    minimum: float,
    maximum: float,
) -> float:
    value = payload.get(key, default)
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise _invalid(f"{key} must be a number", key)
    number = float(value)
    if not math.isfinite(number):
        raise _invalid(f"{key} must be finite", key)
    if not minimum <= number <= maximum:
        raise _invalid(
            f"{key} must be between {minimum} and {maximum}",
            key,
        )
    return number


def _optional_offset(payload: dict[str, Any], key: str) -> float | None:
    """Parses an optional face-relative text offset, clamped to +-3 box units."""
    value = payload.get(key)
    if value is None:
        return None
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise _invalid(f"{key} must be a number", key)
    number = float(value)
    if not math.isfinite(number):
        raise _invalid(f"{key} must be finite", key)
    if not -3.0 <= number <= 3.0:
        raise _invalid(f"{key} must be between -3 and 3", key)
    return number


def _choice(
    payload: dict[str, Any],
    key: str,
    default: str,
    choices: frozenset[str],
) -> str:
    value = payload.get(key, default)
    if not isinstance(value, str) or value not in choices:
        raise InvalidPayloadError(
            f"{key} is not supported",
            details={"field": key, "allowed": sorted(choices)},
        )
    return value


def _ratio(value: object) -> tuple[int, int]:
    if not isinstance(value, str) or value.count(":") != 1:
        raise _invalid("aspectRatio must look like '1:1'", "aspectRatio")
    try:
        width, height = (int(part) for part in value.split(":"))
    except ValueError as error:
        raise _invalid("aspectRatio must look like '1:1'", "aspectRatio") from error
    if width <= 0 or height <= 0 or width > 100 or height > 100:
        raise _invalid(
            "aspectRatio values must be between 1 and 100",
            "aspectRatio",
        )
    return width, height


def _is_absolute(value: str) -> bool:
    return Path(value).is_absolute() or PureWindowsPath(value).is_absolute()


def _is_unc(value: str) -> bool:
    normalized = value.replace("/", "\\").casefold()
    if normalized.startswith("\\\\?\\unc\\"):
        return True
    if normalized.startswith(("\\\\?\\", "\\\\.\\")):
        return False
    return normalized.startswith("\\\\")


def _is_windows_device_path(value: str) -> bool:
    normalized = value.replace("/", "\\").casefold()
    return normalized.startswith("\\\\.\\")


def _comparison_key(path: Path) -> str:
    value = str(path)
    windows_path = PureWindowsPath(value)
    if windows_path.is_absolute():
        return ntpath.normcase(ntpath.normpath(value))
    return os.path.normcase(os.path.abspath(value))


def _invalid(message: str, field: str) -> InvalidPayloadError:
    return InvalidPayloadError(message, details={"field": field})
