"""Versioned messages shared with the Rust process boundary."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any

from .errors import ErrorInfo, InvalidRequestError

PROTOCOL_VERSION = 1
_REQUEST_FIELDS = frozenset({"protocolVersion", "action", "payload"})


@dataclass(frozen=True, slots=True)
class Request:
    protocol_version: int
    action: str
    payload: dict[str, Any] = field(default_factory=dict)

    @classmethod
    def from_dict(cls, value: dict[str, Any]) -> "Request":
        unknown_fields = sorted(set(value) - _REQUEST_FIELDS)
        if unknown_fields:
            raise InvalidRequestError(
                "request contains unknown fields",
                details={"fields": unknown_fields},
            )

        protocol_version = value.get("protocolVersion")
        action = value.get("action")
        payload = value.get("payload", {})

        if type(protocol_version) is not int:
            raise InvalidRequestError(
                "protocolVersion must be an integer",
                details={"field": "protocolVersion"},
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
        )


@dataclass(frozen=True, slots=True)
class Response:
    ok: bool
    action: str
    data: dict[str, Any] | None = None
    error: ErrorInfo | None = None
    protocol_version: int = PROTOCOL_VERSION

    def to_dict(self) -> dict[str, Any]:
        return {
            "protocolVersion": self.protocol_version,
            "ok": self.ok,
            "action": self.action,
            "data": self.data,
            "error": self.error.to_dict() if self.error else None,
        }
