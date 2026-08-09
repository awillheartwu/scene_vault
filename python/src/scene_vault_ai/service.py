"""Application service for protocol actions."""

from __future__ import annotations

import logging
import platform
from collections.abc import Callable
from dataclasses import dataclass, field

from . import __version__
from .errors import (
    ErrorInfo,
    InvalidPayloadError,
    SceneVaultAiError,
    UnsupportedActionError,
    UnsupportedProtocolError,
)
from .protocol import PROTOCOL_VERSION, Request, Response
from .providers import AiProvider
from .vision import ProcessingRequest, ScreenshotProcessor
from .vision.processor import dependency_status

logger = logging.getLogger(__name__)


@dataclass(slots=True)
class AiService:
    providers: dict[str, AiProvider] = field(default_factory=dict)

    def handle(
        self,
        request: Request,
        progress: Callable[[str, float], None] | None = None,
    ) -> Response:
        if request.protocol_version != PROTOCOL_VERSION:
            return _failure(
                request.action,
                UnsupportedProtocolError(
                    "the request protocol version is not supported",
                    details={
                        "received": request.protocol_version,
                        "expected": PROTOCOL_VERSION,
                    },
                ),
            )

        if request.action == "health":
            if request.payload:
                return _failure(
                    request.action,
                    InvalidPayloadError(
                        "health payload must be empty",
                        details={"field": "payload"},
                    ),
                )
            return Response(
                ok=True,
                action="health",
                data=self.health(),
            )

        if request.action == "processScreenshot":
            try:
                processing_request = ProcessingRequest.from_payload(request.payload)
                result = ScreenshotProcessor(
                    processing_request,
                    progress=progress,
                ).process()
            except SceneVaultAiError as error:
                return _failure(request.action, error)
            except Exception:
                logger.exception("unexpected screenshot-processing failure")
                return Response(
                    ok=False,
                    action=request.action,
                    error=ErrorInfo(
                        code="internal_error",
                        message="an unexpected screenshot-processing error occurred",
                    ),
                )
            return Response(
                ok=True,
                action=request.action,
                data=result.to_dict(),
            )

        return _failure(
            request.action,
            UnsupportedActionError(
                "the requested action is not supported",
                details={"action": request.action},
            ),
        )

    def health(self) -> dict[str, object]:
        dependencies = dependency_status()
        vision_available = all(dependencies.values())
        return {
            "status": "ready" if vision_available else "degraded",
            "engineVersion": __version__,
            "pythonVersion": platform.python_version(),
            "providers": sorted(self.providers),
            "capabilities": ["processScreenshot"] if vision_available else [],
            "vision": {
                "dependencies": dependencies,
                "faceDetector": "yunet",
                "modelProvisioning": "request",
                "available": vision_available,
            },
        }


def _failure(action: str, error: SceneVaultAiError) -> Response:
    return Response(
        ok=False,
        action=action,
        error=error.to_info(),
    )
