"""Stable, machine-readable errors exposed at the process boundary."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Mapping


@dataclass(frozen=True, slots=True)
class ErrorInfo:
    """Serialized error information returned to the Rust host."""

    code: str
    message: str
    details: Mapping[str, Any] | None = None

    def to_dict(self) -> dict[str, Any]:
        value: dict[str, Any] = {
            "code": self.code,
            "message": self.message,
        }
        if self.details:
            value["details"] = dict(self.details)
        return value


class SceneVaultAiError(Exception):
    """Base class for failures that are safe to return to the caller."""

    code = "processing_failed"

    def __init__(
        self,
        message: str,
        *,
        details: Mapping[str, Any] | None = None,
    ) -> None:
        super().__init__(message)
        self.message = message
        self.details = details

    def to_info(self) -> ErrorInfo:
        return ErrorInfo(
            code=self.code,
            message=self.message,
            details=self.details,
        )


class InvalidRequestError(SceneVaultAiError, ValueError):
    code = "invalid_request"


class InvalidJsonError(SceneVaultAiError, ValueError):
    code = "invalid_json"


class InvalidPayloadError(SceneVaultAiError, ValueError):
    code = "invalid_payload"


class UnsupportedProtocolError(SceneVaultAiError):
    code = "unsupported_protocol_version"


class UnsupportedActionError(SceneVaultAiError):
    code = "unsupported_action"


class CapabilityUnavailableError(SceneVaultAiError):
    code = "capability_unavailable"


class InputNotFoundError(SceneVaultAiError):
    code = "input_not_found"


class InputReadError(SceneVaultAiError):
    code = "input_read_failed"


class ResourceNotFoundError(SceneVaultAiError):
    code = "resource_not_found"


class ImageDecodeError(SceneVaultAiError):
    code = "image_decode_failed"


class DetectionError(SceneVaultAiError):
    code = "face_detection_failed"


class OutputWriteError(SceneVaultAiError):
    code = "output_write_failed"
