#!/usr/bin/env python3
"""Verifies every tracked text file in the repository is valid UTF-8.

Usage (from the repository root):
    python scripts/check-utf8.py

Binary assets (icons, models, fonts, archives, databases) are skipped by
extension. The check reads whole files so multi-byte characters are never
truncated.
"""

from __future__ import annotations

import os
import subprocess
import sys

BINARY_EXTENSIONS = {
    ".png",
    ".ico",
    ".icns",
    ".jpg",
    ".jpeg",
    ".onnx",
    ".zip",
    ".ttf",
    ".otf",
    ".woff2",
    ".ttc",
    ".db",
    ".exe",
    ".pyd",
}


def tracked_files(root: str) -> list[str]:
    result = subprocess.run(
        ["git", "ls-files"],
        cwd=root,
        capture_output=True,
        text=True,
        check=True,
    )
    return [line for line in result.stdout.splitlines() if line]


def main() -> int:
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    failures: list[str] = []
    checked = 0
    for relative in tracked_files(root):
        path = os.path.join(root, relative)
        if not os.path.isfile(path):
            continue
        extension = os.path.splitext(relative)[1].lower()
        if extension in BINARY_EXTENSIONS:
            continue
        with open(path, "rb") as handle:
            data = handle.read()
        checked += 1
        try:
            data.decode("utf-8")
        except UnicodeDecodeError:
            failures.append(relative)

    for failure in failures:
        print(f"NOT UTF-8: {failure}")
    print(f"checked {checked} text files, {len(failures)} invalid")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
