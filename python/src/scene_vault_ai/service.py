"""Application service for protocol actions."""

from __future__ import annotations

import logging
import platform
from collections.abc import Callable
from dataclasses import dataclass, field, replace
from time import perf_counter

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
                request_id=request.request_id,
            )

        if request.action == "health":
            if request.payload:
                return _failure(
                    request.action,
                    InvalidPayloadError(
                        "health payload must be empty",
                        details={"field": "payload"},
                    ),
                    request_id=request.request_id,
                )
            return Response(
                ok=True,
                action="health",
                data=self.health(),
                request_id=request.request_id,
            )

        if request.action == "processScreenshot":
            service_started = perf_counter()
            try:
                processing_request = ProcessingRequest.from_payload(request.payload)
                processor_started = perf_counter()
                processor = ScreenshotProcessor(
                    processing_request,
                    progress=progress,
                )
                processor_init_ms = _elapsed_ms(processor_started)
                result = processor.process()
                result = replace(
                    result,
                    timings=replace(
                        result.timings,
                        processor_init_ms=processor_init_ms,
                        service_total_ms=_elapsed_ms(service_started),
                    ),
                )
            except SceneVaultAiError as error:
                return _failure(
                    request.action,
                    error,
                    request_id=request.request_id,
                )
            except Exception:
                logger.exception("unexpected screenshot-processing failure")
                return Response(
                    ok=False,
                    action=request.action,
                    error=ErrorInfo(
                        code="internal_error",
                        message="an unexpected screenshot-processing error occurred",
                    ),
                    request_id=request.request_id,
                )
            return Response(
                ok=True,
                action=request.action,
                data=result.to_dict(),
                request_id=request.request_id,
            )

        return _failure(
            request.action,
            UnsupportedActionError(
                "the requested action is not supported",
                details={"action": request.action},
            ),
            request_id=request.request_id,
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


def _failure(
    action: str,
    error: SceneVaultAiError,
    *,
    request_id: str | None = None,
) -> Response:
    return Response(
        ok=False,
        action=action,
        error=error.to_info(),
        request_id=request_id,
    )


def _elapsed_ms(started: float) -> float:
    return round((perf_counter() - started) * 1000.0, 3)
