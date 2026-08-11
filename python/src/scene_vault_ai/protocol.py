"""Versioned messages shared with the Rust process boundary."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any

from .errors import ErrorInfo, InvalidRequestError

PROTOCOL_VERSION = 1
_REQUEST_FIELDS = frozenset({"protocolVersion", "requestId", "action", "payload"})


@dataclass(frozen=True, slots=True)
class Request:
    protocol_version: int
    action: str
    payload: dict[str, Any] = field(default_factory=dict)
    request_id: str | None = None

    @classmethod
    def from_dict(cls, value: dict[str, Any]) -> "Request":
        unknown_fields = sorted(set(value) - _REQUEST_FIELDS)
        if unknown_fields:
            raise InvalidRequestError(
                "request contains unknown fields",
                details={"fields": unknown_fields},
            )

        protocol_version = value.get("protocolVersion")
        request_id = value.get("requestId")
        action = value.get("action")
        payload = value.get("payload", {})

        if type(protocol_version) is not int:
            raise InvalidRequestError(
                "protocolVersion must be an integer",
                details={"field": "protocolVersion"},
            )
        if request_id is not None and (
            not isinstance(request_id, str)
            or not request_id
            or request_id != request_id.strip()
            or len(request_id) > 128
        ):
            raise InvalidRequestError(
                "requestId must be a non-empty string of at most 128 characters",
                details={"field": "requestId"},
            )
        if not isinstance(action, str) or not action or action != action.strip():
            raise InvalidRequestError(
                "action must be a non-empty string without surrounding whitespace",
                details={"field": "action"},
            )
        if not isinstance(payload, dict):
            raise InvalidRequestError(
                "payload must be an object",
                details={"field": "payload"},
            )

        return cls(
            protocol_version=protocol_version,
            action=action,
            payload=payload,
            request_id=request_id,
        )


@dataclass(frozen=True, slots=True)
class Response:
    ok: bool
    action: str
    data: dict[str, Any] | None = None
    error: ErrorInfo | None = None
    protocol_version: int = PROTOCOL_VERSION
    request_id: str | None = None

    def to_dict(self) -> dict[str, Any]:
        return {
            "protocolVersion": self.protocol_version,
            "requestId": self.request_id,
            "ok": self.ok,
            "action": self.action,
            "data": self.data,
            "error": self.error.to_dict() if self.error else None,
        }
