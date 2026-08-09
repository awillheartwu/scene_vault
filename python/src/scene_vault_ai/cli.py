"""Command-line and standard-input boundary used by the Rust host."""

from __future__ import annotations

import argparse
import json
import logging
import os
import sys
from collections.abc import Sequence
from typing import Any

from .errors import ErrorInfo, InvalidJsonError, InvalidRequestError, SceneVaultAiError
from .protocol import PROTOCOL_VERSION, Request, Response
from .service import AiService

logger = logging.getLogger(__name__)


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
    try:
        # The Rust host always writes UTF-8; on Windows the default text-mode
        # stdin decodes with the ANSI codepage (e.g. GBK), which corrupts CJK
        # character names. Read the raw bytes and decode UTF-8 explicitly.
        raw_request = json.loads(
            sys.stdin.buffer.read().decode("utf-8"),
            object_pairs_hook=_unique_object,
            parse_constant=_reject_json_constant,
        )
        if not isinstance(raw_request, dict):
            raise InvalidRequestError("request must be a JSON object")
        request = Request.from_dict(raw_request)
        response = service.handle(request, progress=_stderr_progress)
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

    _write_response(response)
    return 0 if response.ok else 1


def _stderr_progress(stage: str, percent: float) -> None:
    """Streams one SVPROGRESS line per processing stage on stderr.

    The Rust host reads these lines incrementally and forwards them as
    progress events; stdout keeps the single-response contract untouched.
    """

    print(
        "SVPROGRESS "
        + json.dumps(
            {"stage": stage, "percent": percent},
            separators=(",", ":"),
        ),
        file=sys.stderr,
        flush=True,
    )


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
    parser.add_argument("command", choices=("health", "request"))
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    _configure_logging()
    args = build_parser().parse_args(argv)
    service = AiService()

    if args.command == "health":
        return _health(service)
    return _request(service)
