#!/usr/bin/env python3
"""Windows A/B benchmark for Scene Vault one-shot and persistent workers.

The tool uses the production JSON protocol and real configured models without
writing annotated images or avatars. It also verifies deterministic responses
and that a killed worker can be restarted successfully.
"""

from __future__ import annotations

import argparse
import ctypes
import json
import math
import os
import platform
import statistics
import subprocess
import sys
import threading
import time
from collections import deque
from pathlib import Path
from typing import Any

PROTOCOL_VERSION = 1
MIB = 1024 * 1024


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--python", type=Path, required=True)
    parser.add_argument("--module-root", type=Path, required=True)
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--yunet-model", type=Path, required=True)
    parser.add_argument("--sface-model", type=Path)
    parser.add_argument("--arcface-model", type=Path)
    parser.add_argument("--recognizer", choices=("sface", "arcface"), default="sface")
    parser.add_argument("--iterations", type=int, default=10)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.iterations < 3:
        parser.error("--iterations must be at least 3")
    for label, path in (
        ("Python executable", args.python),
        ("module root", args.module_root),
        ("input image", args.input),
        ("YuNet model", args.yunet_model),
    ):
        if not path.exists():
            parser.error(f"{label} does not exist: {path}")
    selected_model = args.arcface_model if args.recognizer == "arcface" else args.sface_model
    if selected_model is None or not selected_model.is_file():
        parser.error(f"{args.recognizer} model does not exist: {selected_model}")
    return args


def request_payload(args: argparse.Namespace) -> dict[str, Any]:
    return {
        "inputPath": str(args.input.resolve()),
        "annotatedOutputPath": None,
        "avatarOutputPath": None,
        "detectFace": True,
        "annotate": False,
        "cropAvatar": False,
        "yunetModelPath": str(args.yunet_model.resolve()),
        "sfaceModelPath": str(args.sface_model.resolve()) if args.sface_model else None,
        "recognizer": args.recognizer,
        "arcfaceModelPath": (
            str(args.arcface_model.resolve()) if args.arcface_model else None
        ),
    }


def environment(args: argparse.Namespace) -> dict[str, str]:
    env = os.environ.copy()
    env["PYTHONPATH"] = str(args.module_root.resolve())
    return env


class MemorySampler:
    def __init__(self, process: subprocess.Popen[bytes]) -> None:
        self.process = process
        self.samples: list[int] = []
        self._stop = threading.Event()
        self._thread = threading.Thread(target=self._run, daemon=True)

    def start(self) -> None:
        self._sample()
        self._thread.start()

    def stop(self) -> int | None:
        self._stop.set()
        self._thread.join(timeout=1)
        self._sample()
        return max(self.samples, default=None)

    def current(self) -> int | None:
        value = working_set_bytes(self.process.pid)
        if value is not None:
            self.samples.append(value)
        return value

    def _sample(self) -> None:
        self.current()

    def _run(self) -> None:
        while not self._stop.wait(0.05):
            self._sample()


def working_set_bytes(pid: int) -> int | None:
    if sys.platform == "win32":
        return windows_working_set_bytes(pid)
    status = Path(f"/proc/{pid}/status")
    try:
        for line in status.read_text(encoding="utf-8").splitlines():
            if line.startswith("VmRSS:"):
                return int(line.split()[1]) * 1024
    except (OSError, ValueError, IndexError):
        return None
    return None


def windows_working_set_bytes(pid: int) -> int | None:
    values = [
        value
        for process_id in windows_process_tree(pid)
        if (value := windows_process_working_set_bytes(process_id)) is not None
    ]
    return sum(values) if values else None


def windows_process_tree(root_pid: int) -> set[int]:
    class ProcessEntry(ctypes.Structure):
        _fields_ = [
            ("dwSize", ctypes.c_ulong),
            ("cntUsage", ctypes.c_ulong),
            ("th32ProcessID", ctypes.c_ulong),
            ("th32DefaultHeapID", ctypes.c_size_t),
            ("th32ModuleID", ctypes.c_ulong),
            ("cntThreads", ctypes.c_ulong),
            ("th32ParentProcessID", ctypes.c_ulong),
            ("pcPriClassBase", ctypes.c_long),
            ("dwFlags", ctypes.c_ulong),
            ("szExeFile", ctypes.c_wchar * 260),
        ]

    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.CreateToolhelp32Snapshot.argtypes = [ctypes.c_ulong, ctypes.c_ulong]
    kernel32.CreateToolhelp32Snapshot.restype = ctypes.c_void_p
    kernel32.Process32FirstW.argtypes = [ctypes.c_void_p, ctypes.POINTER(ProcessEntry)]
    kernel32.Process32FirstW.restype = ctypes.c_int
    kernel32.Process32NextW.argtypes = [ctypes.c_void_p, ctypes.POINTER(ProcessEntry)]
    kernel32.Process32NextW.restype = ctypes.c_int
    kernel32.CloseHandle.argtypes = [ctypes.c_void_p]
    kernel32.CloseHandle.restype = ctypes.c_int
    snapshot = kernel32.CreateToolhelp32Snapshot(0x00000002, 0)
    if not snapshot or snapshot == ctypes.c_void_p(-1).value:
        return {root_pid}
    parent_by_pid: dict[int, int] = {}
    try:
        entry = ProcessEntry()
        entry.dwSize = ctypes.sizeof(entry)
        success = kernel32.Process32FirstW(snapshot, ctypes.byref(entry))
        while success:
            parent_by_pid[int(entry.th32ProcessID)] = int(entry.th32ParentProcessID)
            success = kernel32.Process32NextW(snapshot, ctypes.byref(entry))
    finally:
        kernel32.CloseHandle(snapshot)
    tree = {root_pid}
    changed = True
    while changed:
        changed = False
        for process_id, parent_id in parent_by_pid.items():
            if parent_id in tree and process_id not in tree:
                tree.add(process_id)
                changed = True
    return tree


def windows_process_working_set_bytes(pid: int) -> int | None:
    class ProcessMemoryCounters(ctypes.Structure):
        _fields_ = [
            ("cb", ctypes.c_ulong),
            ("PageFaultCount", ctypes.c_ulong),
            ("PeakWorkingSetSize", ctypes.c_size_t),
            ("WorkingSetSize", ctypes.c_size_t),
            ("QuotaPeakPagedPoolUsage", ctypes.c_size_t),
            ("QuotaPagedPoolUsage", ctypes.c_size_t),
            ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
            ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
            ("PagefileUsage", ctypes.c_size_t),
            ("PeakPagefileUsage", ctypes.c_size_t),
        ]

    process_query_information = 0x0400
    process_vm_read = 0x0010
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    psapi = ctypes.WinDLL("psapi", use_last_error=True)
    kernel32.OpenProcess.argtypes = [ctypes.c_ulong, ctypes.c_int, ctypes.c_ulong]
    kernel32.OpenProcess.restype = ctypes.c_void_p
    kernel32.CloseHandle.argtypes = [ctypes.c_void_p]
    kernel32.CloseHandle.restype = ctypes.c_int
    psapi.GetProcessMemoryInfo.argtypes = [
        ctypes.c_void_p,
        ctypes.POINTER(ProcessMemoryCounters),
        ctypes.c_ulong,
    ]
    psapi.GetProcessMemoryInfo.restype = ctypes.c_int
    handle = kernel32.OpenProcess(
        process_query_information | process_vm_read,
        False,
        pid,
    )
    if not handle:
        return None
    try:
        counters = ProcessMemoryCounters()
        counters.cb = ctypes.sizeof(counters)
        if not psapi.GetProcessMemoryInfo(
            handle,
            ctypes.byref(counters),
            counters.cb,
        ):
            return None
        return int(counters.WorkingSetSize)
    finally:
        kernel32.CloseHandle(handle)


def checked_response(raw: bytes, stderr: str) -> dict[str, Any]:
    try:
        response = json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise RuntimeError(f"invalid engine response: {error}; stderr={stderr[-1000:]}") from error
    if not response.get("ok"):
        engine_error = response.get("error")
        raise RuntimeError(
            f"engine request failed: {engine_error}; stderr={stderr[-1000:]}"
        )
    return response


def response_signature(response: dict[str, Any]) -> str:
    data = response.get("data") or {}
    stable = {
        "faceDetected": data.get("faceDetected"),
        "faceBox": data.get("faceBox"),
        "faceFeature": data.get("faceFeature"),
        "faceFeatureModelId": data.get("faceFeatureModelId"),
        "faceFeatureModelVersion": data.get("faceFeatureModelVersion"),
        "faceCount": data.get("faceCount"),
        "warnings": data.get("warnings"),
    }
    return json.dumps(stable, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def run_oneshot(args: argparse.Namespace) -> tuple[list[float], list[int], list[str]]:
    durations: list[float] = []
    peak_memory: list[int] = []
    signatures: list[str] = []
    request = {
        "protocolVersion": PROTOCOL_VERSION,
        "action": "processScreenshot",
        "payload": request_payload(args),
    }
    encoded = json.dumps(request, ensure_ascii=False).encode("utf-8")
    for _ in range(args.iterations):
        started = time.perf_counter()
        process = subprocess.Popen(
            [str(args.python), "-m", "scene_vault_ai", "request"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=environment(args),
        )
        sampler = MemorySampler(process)
        sampler.start()
        stdout, stderr = process.communicate(encoded)
        peak = sampler.stop()
        durations.append((time.perf_counter() - started) * 1000)
        if peak is not None:
            peak_memory.append(peak)
        if process.returncode != 0:
            stderr_tail = stderr.decode("utf-8", "replace")[-1000:]
            raise RuntimeError(
                f"one-shot exited with {process.returncode}: {stderr_tail}"
            )
        response = checked_response(stdout, stderr.decode("utf-8", "replace"))
        signatures.append(response_signature(response))
    return durations, peak_memory, signatures


class WorkerClient:
    def __init__(self, args: argparse.Namespace) -> None:
        self.args = args
        self.stderr_tail: deque[str] = deque(maxlen=50)
        self.process = subprocess.Popen(
            [str(args.python), "-m", "scene_vault_ai", "worker"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=environment(args),
        )
        self.sampler = MemorySampler(self.process)
        self.sampler.start()
        self._stderr_thread = threading.Thread(target=self._drain_stderr, daemon=True)
        self._stderr_thread.start()

    def _drain_stderr(self) -> None:
        assert self.process.stderr is not None
        for raw_line in self.process.stderr:
            self.stderr_tail.append(raw_line.decode("utf-8", "replace").rstrip())

    def call(self, request: dict[str, Any]) -> dict[str, Any]:
        if self.process.poll() is not None:
            raise RuntimeError(f"worker exited with {self.process.returncode}")
        assert self.process.stdin is not None
        assert self.process.stdout is not None
        self.process.stdin.write(json.dumps(request, ensure_ascii=False).encode("utf-8") + b"\n")
        self.process.stdin.flush()
        line = self.process.stdout.readline()
        if not line:
            raise RuntimeError(
                f"worker closed stdout; stderr={' | '.join(self.stderr_tail)[-1000:]}"
            )
        return checked_response(line, " | ".join(self.stderr_tail))

    def stop(self, *, kill: bool = False) -> int | None:
        if self.process.poll() is None:
            if kill:
                self.process.kill()
            elif self.process.stdin is not None:
                self.process.stdin.close()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait(timeout=5)
        peak = self.sampler.stop()
        self._stderr_thread.join(timeout=1)
        return peak


def worker_request(args: argparse.Namespace, request_id: str) -> dict[str, Any]:
    return {
        "protocolVersion": PROTOCOL_VERSION,
        "requestId": request_id,
        "action": "processScreenshot",
        "payload": request_payload(args),
    }


def health_request(request_id: str) -> dict[str, Any]:
    return {
        "protocolVersion": PROTOCOL_VERSION,
        "requestId": request_id,
        "action": "health",
        "payload": {},
    }


def run_worker(
    args: argparse.Namespace,
) -> tuple[float, list[float], list[int], int | None, list[str], bool]:
    startup_started = time.perf_counter()
    worker = WorkerClient(args)
    worker.call(health_request("benchmark-health"))
    startup_ms = (time.perf_counter() - startup_started) * 1000
    durations: list[float] = []
    memory: list[int] = []
    signatures: list[str] = []
    for index in range(args.iterations):
        started = time.perf_counter()
        response = worker.call(worker_request(args, f"benchmark-{index}"))
        durations.append((time.perf_counter() - started) * 1000)
        current_memory = worker.sampler.current()
        if current_memory is not None:
            memory.append(current_memory)
        signatures.append(response_signature(response))
    peak = worker.stop(kill=True)

    restarted = WorkerClient(args)
    try:
        restarted.call(health_request("recovery-health"))
        recovery_response = restarted.call(worker_request(args, "recovery-request"))
        recovery_ok = response_signature(recovery_response) == signatures[0]
    finally:
        restarted.stop()
    return startup_ms, durations, memory, peak, signatures, recovery_ok


def percentile_95(values: list[float]) -> float:
    ordered = sorted(values)
    return ordered[max(0, math.ceil(len(ordered) * 0.95) - 1)]


def mode_summary(durations: list[float]) -> dict[str, float]:
    warm = durations[1:]
    return {
        "firstMs": round(durations[0], 3),
        "warmMedianMs": round(statistics.median(warm), 3),
        "warmP95Ms": round(percentile_95(warm), 3),
        "totalMs": round(sum(durations), 3),
    }


def mib(value: int | None) -> float | None:
    return round(value / MIB, 3) if value is not None else None


def main() -> int:
    args = parse_args()
    oneshot_durations, oneshot_memory, oneshot_signatures = run_oneshot(args)
    (
        worker_startup_ms,
        worker_durations,
        worker_memory,
        worker_peak,
        worker_signatures,
        recovery_ok,
    ) = run_worker(args)
    consistent = (
        len(set(oneshot_signatures)) == 1
        and len(set(worker_signatures)) == 1
        and oneshot_signatures[0] == worker_signatures[0]
    )
    oneshot = mode_summary(oneshot_durations)
    worker = mode_summary(worker_durations)
    result = {
        "schemaVersion": 1,
        "platform": platform.platform(),
        "pythonVersion": platform.python_version(),
        "recognizer": args.recognizer,
        "iterations": args.iterations,
        "oneShot": {
            **oneshot,
            "maxPeakRssMiB": mib(max(oneshot_memory, default=None)),
        },
        "worker": {
            **worker,
            "startupHealthMs": round(worker_startup_ms, 3),
            "coldTotalMs": round(worker_startup_ms + worker_durations[0], 3),
            "peakRssMiB": mib(worker_peak),
            "rssAfterFirstMiB": mib(worker_memory[0] if worker_memory else None),
            "rssAfterLastMiB": mib(worker_memory[-1] if worker_memory else None),
            "rssGrowthMiB": (
                mib(worker_memory[-1] - worker_memory[0])
                if len(worker_memory) >= 2
                else None
            ),
        },
        "warmSpeedup": round(oneshot["warmMedianMs"] / worker["warmMedianMs"], 3),
        "responsesConsistent": consistent,
        "crashRestartRecovered": recovery_ok,
    }
    encoded = json.dumps(result, ensure_ascii=False, indent=2)
    print(encoded)
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(encoded + "\n", encoding="utf-8")
    return 0 if consistent and recovery_ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
