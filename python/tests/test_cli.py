from __future__ import annotations

import json
import sys
import unittest
from types import SimpleNamespace

from scene_vault_ai.cli import _request
from scene_vault_ai.protocol import Response


class _StubService:
    def __init__(self) -> None:
        self.last_payload: dict | None = None

    def handle(self, request, progress=None):  # noqa: ARG002
        self.last_payload = request.payload
        return Response(
            ok=True,
            action="processScreenshot",
            data={"name": request.payload["characterName"]},
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

    def flush(self) -> None:
        pass


class CliEncodingTests(unittest.TestCase):
    def test_chinese_name_round_trips_through_the_utf8_boundary(self) -> None:
        service = _StubService()
        request = {
            "protocolVersion": 1,
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
        self.assertIn("杰德".encode("utf-8"), stdout.data)


if __name__ == "__main__":
    unittest.main()
