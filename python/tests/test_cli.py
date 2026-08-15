from __future__ import annotations

import json
import os
import sys
import unittest
from types import SimpleNamespace
from unittest.mock import Mock, patch

from scene_vault_ai.cli import (
    MAX_REQUEST_BYTES,
    _configure_image_processing_threads,
    _request,
    _worker,
)
from scene_vault_ai.protocol import Response


class _StubService:
    def __init__(self) -> None:
        self.last_payload: dict | None = None

    def handle(self, request, progress=None, log_event=None):  # noqa: ARG002
        self.last_payload = request.payload
        return Response(
            ok=True,
            action="processScreenshot",
            data={"name": request.payload["characterName"]},
            request_id=request.request_id,
        )


class _ByteBuffer:
    def __init__(self, data: bytes = b"") -> None:
        self.data = data
        self._position = 0

    def read(self, size: int = -1) -> bytes:
        if size < 0:
            size = len(self.data) - self._position
        chunk = self.data[self._position : self._position + size]
        self._position += len(chunk)
        return chunk

    def write(self, value: bytes) -> int:
        self.data += value
        return len(value)

    def readline(self, size: int = -1) -> bytes:
        if self._position >= len(self.data):
            return b""
        limit = len(self.data) if size < 0 else min(len(self.data), self._position + size)
        newline = self.data.find(b"\n", self._position, limit)
        end = newline + 1 if newline >= 0 else limit
        chunk = self.data[self._position : end]
        self._position = end
        return chunk

    def flush(self) -> None:
        pass


class CliEncodingTests(unittest.TestCase):
    def test_configures_image_processing_thread_limit(self) -> None:
        cv2 = SimpleNamespace(setNumThreads=Mock())
        with (
            patch.dict(
                os.environ,
                {"SCENE_VAULT_IMAGE_PROCESSING_THREADS": "7"},
                clear=False,
            ),
            patch.dict(sys.modules, {"cv2": cv2}),
        ):
            self.assertEqual(_configure_image_processing_threads(), 7)
            self.assertEqual(os.environ["OMP_NUM_THREADS"], "7")
            self.assertEqual(os.environ["OPENBLAS_NUM_THREADS"], "7")
            self.assertEqual(os.environ["MKL_NUM_THREADS"], "7")
            cv2.setNumThreads.assert_called_once_with(7)

    def test_chinese_name_round_trips_through_the_utf8_boundary(self) -> None:
        service = _StubService()
        request = {
            "protocolVersion": 1,
            "requestId": "utf8-1",
            "action": "processScreenshot",
            "payload": {"characterName": "杰德"},
        }
        stdin = _ByteBuffer(json.dumps(request, ensure_ascii=False).encode("utf-8"))
        stdout = _ByteBuffer()
        old_stdin, old_stdout = sys.stdin, sys.stdout
        sys.stdin = SimpleNamespace(buffer=stdin)
        sys.stdout = SimpleNamespace(buffer=stdout)
        try:
            code = _request(service)
        finally:
            sys.stdin, sys.stdout = old_stdin, old_stdout

        self.assertEqual(code, 0)
        self.assertEqual(service.last_payload, request["payload"])
        response = json.loads(stdout.data.decode("utf-8"))
        self.assertEqual(response["data"]["name"], "杰德")
        self.assertEqual(response["requestId"], "utf8-1")
        self.assertIn("杰德".encode("utf-8"), stdout.data)

    def test_worker_rejects_oversized_line_and_reads_the_next_request(self) -> None:
        oversized = b"x" * (MAX_REQUEST_BYTES + 10) + b"\n"
        request = json.dumps(
            {
                "protocolVersion": 1,
                "requestId": "after-large-line",
                "action": "processScreenshot",
                "payload": {"characterName": "星见"},
            },
            ensure_ascii=False,
        ).encode("utf-8")
        service = _StubService()
        stdin = _ByteBuffer(oversized + request + b"\n")
        stdout = _ByteBuffer()
        old_stdin, old_stdout = sys.stdin, sys.stdout
        sys.stdin = SimpleNamespace(buffer=stdin)
        sys.stdout = SimpleNamespace(buffer=stdout)
        try:
            code = _worker(service)
        finally:
            sys.stdin, sys.stdout = old_stdin, old_stdout

        responses = [json.loads(line) for line in stdout.data.decode("utf-8").splitlines()]
        self.assertEqual(code, 0)
        self.assertEqual(len(responses), 2)
        self.assertFalse(responses[0]["ok"])
        self.assertEqual(responses[0]["error"]["code"], "invalid_request")
        self.assertTrue(responses[1]["ok"])
        self.assertEqual(responses[1]["requestId"], "after-large-line")


if __name__ == "__main__":
    unittest.main()
