"""Command-line and standard-input boundary used by the Rust host."""

from __future__ import annotations

import argparse
import json
import logging
import os
import sys
from time import perf_counter
from collections.abc import Sequence
from typing import Any

from .errors import ErrorInfo, InvalidJsonError, InvalidRequestError, SceneVaultAiError
from .protocol import PROTOCOL_VERSION, Request, Response
from .service import AiService

logger = logging.getLogger(__name__)
MAX_REQUEST_BYTES = 1024 * 1024


def _write_response(response: Response) -> None:
    # Binary stdout keeps UTF-8 regardless of the Windows ANSI codepage, so
    # Chinese paths/names in responses reach the Rust host intact.
    payload = json.dumps(
        response.to_dict(),
        ensure_ascii=False,
        separators=(",", ":"),
        allow_nan=False,
    )
    sys.stdout.buffer.write(payload.encode("utf-8") + b"\n")
    sys.stdout.buffer.flush()


def _health(service: AiService) -> int:
    _write_response(
        Response(
            ok=True,
            action="health",
            data=service.health(),
        )
    )
    return 0


def _request(service: AiService) -> int:
    response = _handle_request_bytes(service, sys.stdin.buffer.read())
    _write_response(response)
    return 0 if response.ok else 1


def _worker(service: AiService) -> int:
    """Handles newline-delimited requests until stdin reaches EOF.

    Each malformed or failed request produces one error response and leaves
    the worker available for the next line. The host owns process timeout and
    restart policy; this loop stays deliberately serial.
    """

    while raw_request := sys.stdin.buffer.readline(MAX_REQUEST_BYTES + 1):
        if len(raw_request) > MAX_REQUEST_BYTES:
            if not raw_request.endswith(b"\n"):
                _discard_line_remainder()
            response = _error_response(
                InvalidRequestError(
                    "request exceeds the worker line size limit",
                    details={"maxBytes": MAX_REQUEST_BYTES},
                )
            )
        elif not raw_request.strip():
            continue
        else:
            response = _handle_request_bytes(service, raw_request)
        _write_response(response)
    return 0


def _handle_request_bytes(service: AiService, raw_bytes: bytes) -> Response:
    started = perf_counter()
    request: Request | None = None
    try:
        # The Rust host always writes UTF-8; on Windows the default text-mode
        # stdin decodes with the ANSI codepage (e.g. GBK), which corrupts CJK
        # character names. Read the raw bytes and decode UTF-8 explicitly.
        raw_request = json.loads(
            raw_bytes.decode("utf-8"),
            object_pairs_hook=_unique_object,
            parse_constant=_reject_json_constant,
        )
        if not isinstance(raw_request, dict):
            raise InvalidRequestError("request must be a JSON object")
        request = Request.from_dict(raw_request)
        _stderr_log(
            "info",
            "request",
            "request_started",
            requestId=request.request_id,
            message=request.action,
            outcome="started",
        )
        response = service.handle(
            request,
            progress=lambda stage, percent: _stderr_progress(
                stage,
                percent,
                request_id=request.request_id,
            ),
            log_event=lambda event, fields: _stderr_log(
                "debug",
                "model_cache",
                event,
                requestId=request.request_id,
                **fields,
            ),
        )
    except UnicodeDecodeError as error:
        response = _error_response(
            InvalidJsonError(
                "request is not valid UTF-8",
                details={"offset": error.start},
            )
        )
    except json.JSONDecodeError as error:
        response = _error_response(
            InvalidJsonError(
                "request is not valid JSON",
                details={
                    "line": error.lineno,
                    "column": error.colno,
                },
            )
        )
    except SceneVaultAiError as error:
        response = _error_response(error)
    except Exception:
        logger.exception("unexpected request-boundary failure")
        response = Response(
            ok=False,
            action="invalid",
            error=ErrorInfo(
                code="internal_error",
                message="an unexpected request error occurred",
            ),
        )

    duration_ms = round((perf_counter() - started) * 1000.0, 3)
    error_code = response.error.code if response.error is not None else None
    _stderr_log(
        "info" if response.ok else "error",
        "request",
        "request_succeeded" if response.ok else "request_failed",
        requestId=response.request_id or (request.request_id if request else None),
        message=response.action,
        durationMs=duration_ms,
        outcome="succeeded" if response.ok else "failed",
        errorCode=error_code,
    )
    return response


def _stderr_log(level: str, module: str, event: str, **fields: object) -> None:
    payload: dict[str, object] = {
        "level": level,
        "module": module,
        "event": event,
    }
    payload.update({key: value for key, value in fields.items() if value is not None})
    print(
        "SVLOG " + json.dumps(payload, ensure_ascii=False, separators=(",", ":"), allow_nan=False),
        file=sys.stderr,
        flush=True,
    )


def _stderr_progress(
    stage: str,
    percent: float,
    *,
    request_id: str | None = None,
) -> None:
    """Streams one SVPROGRESS line per processing stage on stderr.

    The Rust host reads these lines incrementally and forwards them as
    progress events; stdout keeps the single-response contract untouched.
    """

    payload: dict[str, str | float] = {"stage": stage, "percent": percent}
    if request_id is not None:
        payload["requestId"] = request_id
    print(
        "SVPROGRESS "
        + json.dumps(
            payload,
            separators=(",", ":"),
        ),
        file=sys.stderr,
        flush=True,
    )


def _discard_line_remainder() -> None:
    while chunk := sys.stdin.buffer.readline(MAX_REQUEST_BYTES + 1):
        if chunk.endswith(b"\n"):
            return


def _unique_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, item in pairs:
        if key in value:
            raise InvalidRequestError(
                "request contains a duplicate JSON field",
                details={"field": key},
            )
        value[key] = item
    return value


def _reject_json_constant(value: str) -> None:
    raise InvalidJsonError(
        "request contains a non-finite JSON number",
        details={"value": value},
    )


def _error_response(error: SceneVaultAiError) -> Response:
    return Response(
        ok=False,
        action="invalid",
        error=error.to_info(),
    )


def _configure_logging() -> None:
    level_name = os.environ.get("SCENE_VAULT_AI_LOG_LEVEL", "WARNING").upper()
    level = getattr(logging, level_name, logging.WARNING)
    logging.basicConfig(
        level=level,
        stream=sys.stderr,
        format="%(levelname)s %(name)s: %(message)s",
    )


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="scene-vault-ai")
    parser.add_argument(
        "--protocol-version",
        action="version",
        version=str(PROTOCOL_VERSION),
    )
    parser.add_argument("command", choices=("health", "request", "worker"))
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    _configure_logging()
    args = build_parser().parse_args(argv)
    service = AiService()

    if args.command == "health":
        return _health(service)
    if args.command == "worker":
        return _worker(service)
    return _request(service)
