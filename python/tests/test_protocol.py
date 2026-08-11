import json
import subprocess
import sys
import unittest
from pathlib import Path
from unittest.mock import ANY, patch

from scene_vault_ai.errors import InvalidRequestError
from scene_vault_ai.protocol import PROTOCOL_VERSION, Request, Response
from scene_vault_ai.service import AiService
from scene_vault_ai.vision import ProcessingResult, ProcessingTimings


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

    def test_request_id_is_validated_and_echoed(self) -> None:
        request = Request.from_dict(
            {
                "protocolVersion": PROTOCOL_VERSION,
                "requestId": "vision-123",
                "action": "health",
                "payload": {},
            }
        )

        response = AiService().handle(request)

        self.assertEqual(request.request_id, "vision-123")
        self.assertEqual(response.request_id, "vision-123")
        self.assertEqual(response.to_dict()["requestId"], "vision-123")

        for invalid in ("", " padded ", "x" * 129, 123):
            with self.subTest(invalid=invalid):
                with self.assertRaisesRegex(InvalidRequestError, "requestId"):
                    Request.from_dict(
                        {
                            "protocolVersion": PROTOCOL_VERSION,
                            "requestId": invalid,
                            "action": "health",
                            "payload": {},
                        }
                    )

    def test_processing_timings_use_stable_protocol_keys(self) -> None:
        timings = ProcessingTimings(
            processor_init_ms=1.25,
            detect_ms=2.5,
            service_total_ms=4.0,
        ).to_dict()

        self.assertEqual(timings["processorInitMs"], 1.25)
        self.assertEqual(timings["detectMs"], 2.5)
        self.assertEqual(timings["serviceTotalMs"], 4.0)
        self.assertEqual(
            set(timings),
            {
                "processorInitMs",
                "readMs",
                "detectMs",
                "featureMs",
                "annotateMs",
                "cropMs",
                "writeMs",
                "processTotalMs",
                "serviceTotalMs",
            },
        )

    @patch("scene_vault_ai.service.ScreenshotProcessor")
    @patch("scene_vault_ai.service.ProcessingRequest.from_payload")
    def test_process_response_includes_request_id_and_service_timings(
        self,
        parse_payload,
        processor_class,
    ) -> None:
        processing_request = object()
        parse_payload.return_value = processing_request
        processor_class.return_value.process.return_value = ProcessingResult(
            input_path=Path("C:/captures/one.png"),
            image_width=1920,
            image_height=1080,
            annotated_path=None,
            avatar_path=None,
            face_box=None,
        )

        response = AiService().handle(
            Request(
                protocol_version=PROTOCOL_VERSION,
                request_id="process-1",
                action="processScreenshot",
                payload={"inputPath": "C:/captures/one.png"},
            )
        )

        self.assertTrue(response.ok)
        self.assertEqual(response.request_id, "process-1")
        parse_payload.assert_called_once_with({"inputPath": "C:/captures/one.png"})
        processor_class.assert_called_once_with(
            processing_request,
            model_cache=ANY,
            progress=None,
        )
        self.assertGreaterEqual(response.data["timings"]["processorInitMs"], 0.0)
        self.assertGreaterEqual(response.data["timings"]["serviceTotalMs"], 0.0)

    def test_response_without_request_id_remains_backward_compatible(self) -> None:
        response = Response(ok=True, action="health", data={}).to_dict()

        self.assertIsNone(response["requestId"])

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
                    "requestId": "cli-health-1",
                    "action": "health",
                    "payload": {},
                }
            )
        )
        response = json.loads(process.stdout)

        self.assertEqual(process.returncode, 0)
        self.assertIn('"event":"request_succeeded"', process.stderr)
        self.assertIn('"requestId":"cli-health-1"', process.stderr)
        self.assertTrue(response["ok"])
        self.assertEqual(response["action"], "health")
        self.assertEqual(response["requestId"], "cli-health-1")
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
        self.assertIn('"event":"request_failed"', process.stderr)
        self.assertIn('"errorCode":"unsupported_protocol_version"', process.stderr)
        self.assertFalse(response["ok"])
        self.assertEqual(
            response["error"]["code"],
            "unsupported_protocol_version",
        )

    def test_cli_worker_isolates_invalid_requests_and_continues(self) -> None:
        requests = [
            json.dumps(
                {
                    "protocolVersion": PROTOCOL_VERSION,
                    "requestId": "worker-1",
                    "action": "health",
                    "payload": {},
                }
            ),
            "{",
            json.dumps(
                {
                    "protocolVersion": PROTOCOL_VERSION,
                    "requestId": "worker-2",
                    "action": "health",
                    "payload": {},
                }
            ),
        ]
        process = subprocess.run(
            [sys.executable, "-m", "scene_vault_ai", "worker"],
            input="\n".join(requests) + "\n",
            check=False,
            capture_output=True,
            text=True,
        )
        responses = [json.loads(line) for line in process.stdout.splitlines()]

        self.assertEqual(process.returncode, 0)
        self.assertIn('"event":"request_failed"', process.stderr)
        self.assertIn('"event":"request_succeeded"', process.stderr)
        self.assertEqual(len(responses), 3)
        self.assertEqual(responses[0]["requestId"], "worker-1")
        self.assertTrue(responses[0]["ok"])
        self.assertEqual(responses[1]["error"]["code"], "invalid_json")
        self.assertEqual(responses[2]["requestId"], "worker-2")
        self.assertTrue(responses[2]["ok"])

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
