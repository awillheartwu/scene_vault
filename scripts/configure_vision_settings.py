"""One-click helper: writes the vision/recognition settings into the app DB.

Used by scripts/setup-windows.ps1. The app must not be running while this
writes, because the database is shared with the running application.
"""

from __future__ import annotations

import argparse
import json
import sqlite3


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--db", required=True, help="path to scene-vault.db")
    parser.add_argument("--python-exe", help="venv python executable")
    parser.add_argument("--module-root", help="scene_vault_ai package root")
    parser.add_argument("--yunet", help="YuNet ONNX model path")
    parser.add_argument("--sface", help="SFace ONNX model path")
    parser.add_argument("--font", help="optional annotation font path")
    args = parser.parse_args()

    vision = {
        "pythonExecutablePath": args.python_exe or None,
        "pythonModuleRoot": args.module_root or None,
        "yunetModelPath": args.yunet or None,
        "sfaceModelPath": args.sface or None,
        "fontPath": args.font or None,
    }
    recognition = {
        "extractAtRegistration": True,
        "confidenceThreshold": 0.5,
    }

    connection = sqlite3.connect(args.db)
    try:
        for key, value in (
            ("capture.vision", vision),
            ("capture.recognition", recognition),
        ):
            connection.execute(
                """
                INSERT INTO settings (key, value_json)
                VALUES (?, ?)
                ON CONFLICT(key) DO UPDATE SET
                    value_json = excluded.value_json,
                    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                """,
                (key, json.dumps(value, ensure_ascii=False)),
            )
        connection.commit()
    finally:
        connection.close()
    print(f"settings written into {args.db}")


if __name__ == "__main__":
    main()
