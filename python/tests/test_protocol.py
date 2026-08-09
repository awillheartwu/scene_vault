import json
import subprocess
import sys
import unittest

from scene_vault_ai.errors import InvalidRequestError
from scene_vault_ai.protocol import PROTOCOL_VERSION, Request
from scene_vault_ai.service import AiService


def run_cli(request_text: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, "-m", "scene_vault_ai", "request"],
        input=request_text,
        check=False,
        capture_output=True,
        text=True,
    )


class ProtocolTests(unittest.TestCase):
    def test_health_request_reports_core_and_vision_state(self) -> None:
        response = AiService().handle(
            Request(
                protocol_version=PROTOCOL_VERSION,
                action="health",
            )
        )

        self.assertTrue(response.ok)
        self.assertEqual(response.action, "health")
        self.assertIn(response.data["status"], {"ready", "degraded"})
        self.assertIn("dependencies", response.data["vision"])
        self.assertEqual(response.data["vision"]["modelProvisioning"], "request")

    def test_rejects_unknown_protocol_version_with_structured_error(self) -> None:
        response = AiService().handle(
            Request(
                protocol_version=PROTOCOL_VERSION + 1,
                action="health",
            )
        )

        self.assertFalse(response.ok)
        self.assertEqual(response.error.code, "unsupported_protocol_version")
        self.assertEqual(response.error.details["expected"], PROTOCOL_VERSION)

    def test_request_parser_rejects_bool_version_and_unknown_fields(self) -> None:
        with self.assertRaisesRegex(InvalidRequestError, "integer"):
            Request.from_dict(
                {
                    "protocolVersion": True,
                    "action": "health",
                    "payload": {},
                }
            )

        with self.assertRaisesRegex(InvalidRequestError, "unknown"):
            Request.from_dict(
                {
                    "protocolVersion": PROTOCOL_VERSION,
                    "action": "health",
                    "payload": {},
                    "typo": True,
                }
            )

    def test_cli_health_command_emits_only_json(self) -> None:
        process = subprocess.run(
            [sys.executable, "-m", "scene_vault_ai", "health"],
            check=True,
            capture_output=True,
            text=True,
        )
        response = json.loads(process.stdout)

        self.assertTrue(response["ok"])
        self.assertEqual(response["protocolVersion"], PROTOCOL_VERSION)
        self.assertEqual(process.stderr, "")
        self.assertEqual(len(process.stdout.strip().splitlines()), 1)

    def test_cli_request_round_trip(self) -> None:
        process = run_cli(
            json.dumps(
                {
                    "protocolVersion": PROTOCOL_VERSION,
                    "action": "health",
                    "payload": {},
                }
            )
        )
        response = json.loads(process.stdout)

        self.assertEqual(process.returncode, 0)
        self.assertEqual(process.stderr, "")
        self.assertTrue(response["ok"])
        self.assertEqual(response["action"], "health")
        self.assertIsNone(response["error"])

    def test_cli_protocol_mismatch_is_json_and_nonzero(self) -> None:
        process = run_cli(
            json.dumps(
                {
                    "protocolVersion": PROTOCOL_VERSION + 1,
                    "action": "health",
                    "payload": {},
                }
            )
        )
        response = json.loads(process.stdout)

        self.assertEqual(process.returncode, 1)
        self.assertEqual(process.stderr, "")
        self.assertFalse(response["ok"])
        self.assertEqual(
            response["error"]["code"],
            "unsupported_protocol_version",
        )

    def test_cli_rejects_duplicate_fields_and_invalid_json(self) -> None:
        duplicate = run_cli(
            '{"protocolVersion":1,"protocolVersion":1,'
            '"action":"health","payload":{}}'
        )
        malformed = run_cli("{")

        duplicate_response = json.loads(duplicate.stdout)
        malformed_response = json.loads(malformed.stdout)
        self.assertEqual(duplicate.returncode, 1)
        self.assertEqual(
            duplicate_response["error"]["code"],
            "invalid_request",
        )
        self.assertEqual(malformed.returncode, 1)
        self.assertEqual(
            malformed_response["error"]["code"],
            "invalid_json",
        )


if __name__ == "__main__":
    unittest.main()
