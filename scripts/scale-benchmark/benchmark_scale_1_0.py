#!/usr/bin/env python3
"""Scene Vault 1.0 scale benchmark harness.

Builds a synthetic SQLite fixture at the 1.0 target size (10,000 capture
items, 10,000 source-directory entries, 100 characters) by replaying the
real migrations from src-tauri/migrations, then measures the query and scan
paths that the ROADMAP 1.0 scale gate (docs/ROADMAP.md, M6) cares about.

Design rules:
  * Standard library only (Python 3.11+), so the harness runs anywhere the
    repo's Python tooling runs.
  * No existing file is modified. The fixture is built in a temporary
    directory (or --fixture-dir) and removed afterwards unless --keep-fixture.
  * Every measured SQL statement is copied verbatim from the production
    service and annotated with its source location.
  * Output is machine-readable JSON on stdout; a human summary goes to
    stderr. The process exits 0 when every gated metric passes, otherwise 1.

Heavy runs must be pinned to CPU 0-3:
    taskset -c 0-3 python3 scripts/scale-benchmark/benchmark_scale_1_0.py

Thresholds (docs/benchmarks/scale-1.0-report.example.json):
  startup_db_init_ms          < 5000 ms  (ROADMAP: startup < 5 s)
  history_backend_ms          < 200 ms   (deep history page, OFFSET 9000)
  face_bank_suggestion_ms     < 150 ms   (sample load + parse + cosine)
  scan_filesystem_ms / full   < 1000 ms  (10k-entry directory poll)
  workbench/category queries  < 1000 ms
  db_main_bytes               <= 100 MB  (10k-row budget)
"""

from __future__ import annotations

import argparse
import json
import math
import operator
import os
import platform
import random
import shutil
import sqlite3
import stat
import sys
import tempfile
import time
from datetime import datetime, timezone
from pathlib import Path

SCHEMA_VERSION = 1
REPO_ROOT = Path(__file__).resolve().parents[2]
MIGRATIONS_DIR = REPO_ROOT / "src-tauri" / "migrations"

# 1.0 target (docs/ROADMAP.md, M6 "1.0 规模守门").
TARGET_ITEMS = 10_000
TARGET_CHARACTERS = 100
TARGET_SCAN_ENTRIES = 10_000

# Recognition profile used by the decision step of the suggestion replica.
# The runtime defaults are still provisional (docs/BENCHMARK.md); these
# constants mirror the current SFace defaults of recognition settings.
CONFIDENCE_THRESHOLD = 0.50
MARGIN = 0.05

MODEL_ID = "opencv-sface"
MODEL_VERSION = "2021dec"

SUPPORTED_IMAGE_EXTS = {"png", "jpg", "jpeg", "webp", "bmp"}


# ---------------------------------------------------------------------------
# Production SQL copied verbatim from services, with source locations.
# ---------------------------------------------------------------------------

# capture_service.rs:379-400 (list_items)
LIST_ITEMS_SQL = """
SELECT
    id, project_id, session_id, asset_id, character_id, classification, source_path, file_size, modified_at_ms, content_hash,
    annotated_path, avatar_path, destination_path,
    destination_avatar_path, status, face_box_json, face_count,
    suggested_character_id, recognition_confidence, recognition_source,
    review_status, error_message,
    failure_stage, attempt_count, next_retry_at, processing_warnings_json,
    captured_at, processed_at, archived_at, created_at, updated_at
FROM capture_items
WHERE session_id = ?
ORDER BY captured_at DESC, created_at DESC
"""

# capture_service.rs:519-535 (list_history count)
HISTORY_COUNT_SQL = """
SELECT COUNT(*)
FROM capture_items item
JOIN capture_sessions session ON session.id = item.session_id
WHERE session.project_id = ?
  AND (? IS NULL OR item.session_id = ?)
  AND (? IS NULL OR item.character_id = ?)
  AND (? IS NULL OR item.status = ?)
  AND (? = 1 OR item.classification IS NULL OR item.classification != 'private')
"""

# capture_service.rs:536-577 (list_history page)
HISTORY_PAGE_SQL = """
SELECT
    item.id,
    item.session_id,
    session.project_id,
    project.name AS project_name,
    session.status AS session_status,
    item.character_id,
    item.classification,
    character.name AS character_name,
    item.asset_id,
    item.source_path,
    item.annotated_path,
    item.avatar_path,
    item.destination_path,
    item.destination_avatar_path,
    item.status,
    item.face_box_json,
    item.suggested_character_id,
    item.recognition_confidence,
    item.recognition_source,
    item.review_status,
    item.error_message,
    item.failure_stage,
    item.attempt_count,
    item.next_retry_at,
    item.processing_warnings_json,
    item.captured_at,
    item.processed_at,
    item.archived_at
FROM capture_items item
JOIN capture_sessions session ON session.id = item.session_id
JOIN projects project ON project.id = session.project_id
LEFT JOIN characters character ON character.id = item.character_id
WHERE session.project_id = ?
  AND (? IS NULL OR item.session_id = ?)
  AND (? IS NULL OR item.character_id = ?)
  AND (? IS NULL OR item.status = ?)
  AND (? = 1 OR item.classification IS NULL OR item.classification != 'private')
ORDER BY item.captured_at DESC, item.created_at DESC
LIMIT ?
OFFSET ?
"""

# capture_service.rs:1419-1450 (list_category_items, {filter} substituted)
CATEGORY_ITEM_SELECT = """
SELECT
    item.id, item.project_id, item.session_id, item.asset_id, item.character_id,
    item.classification, item.source_path, item.file_size, item.modified_at_ms, item.content_hash,
    item.annotated_path, item.avatar_path, item.destination_path,
    item.destination_avatar_path, item.status, item.face_box_json, item.face_count,
    item.suggested_character_id, item.recognition_confidence,
    item.recognition_source, item.review_status,
    item.error_message, item.failure_stage, item.attempt_count,
    item.next_retry_at, item.processing_warnings_json,
    item.captured_at, item.processed_at, item.archived_at,
    item.created_at, item.updated_at
FROM capture_items item
JOIN capture_sessions session ON session.id = item.session_id
WHERE session.project_id = ? AND {filter}
ORDER BY item.captured_at DESC, item.created_at DESC
"""

CATEGORY_FILTERS = {
    "unclassified": "item.status = 'awaiting_label'",
    "scene": "item.classification = 'scene'",
    "private": "item.classification = 'private'",
}

# recognition_service.rs:337-376 (list_character_items)
CHARACTER_ITEMS_SQL = """
SELECT
    item.id, item.project_id, item.session_id, item.asset_id, item.character_id,
    item.classification, item.source_path, item.file_size, item.modified_at_ms, item.content_hash,
    item.annotated_path, item.avatar_path, item.destination_path,
    item.destination_avatar_path, item.status, item.face_box_json, item.face_count,
    item.suggested_character_id, item.recognition_confidence,
    item.recognition_source, item.review_status,
    item.error_message, item.failure_stage, item.attempt_count,
    item.next_retry_at, item.processing_warnings_json,
    item.captured_at, item.processed_at, item.archived_at,
    item.created_at, item.updated_at
FROM capture_items item
JOIN capture_sessions session ON session.id = item.session_id
WHERE session.project_id = ?
  AND item.character_id = ?
  AND item.classification = 'person'
ORDER BY item.captured_at DESC, item.created_at DESC
"""

# recognition_service.rs:855-884 (suggest_from_face_bank sample load)
SUGGEST_SAMPLES_SQL = """
SELECT sample.character_id, sample.feature_json
FROM character_face_samples sample
JOIN characters character ON character.id = sample.character_id
JOIN capture_items item ON item.id = sample.capture_item_id
WHERE character.project_id = ?
  AND sample.status = 'active'
  AND sample.flagged = 0
  AND item.classification = 'person'
  AND sample.capture_item_id != ?
  AND sample.character_id != ?
  AND sample.model_id = ?
  AND sample.model_version = ?
"""

# recognition_service.rs:521-538 (compute_verification sample load)
VERIFY_SAMPLES_SQL = """
SELECT sample.character_id, sample.feature_json
FROM character_face_samples sample
JOIN characters character ON character.id = sample.character_id
JOIN capture_items item ON item.id = sample.capture_item_id
WHERE character.project_id = (
    SELECT project_id FROM capture_sessions WHERE id = ?
)
  AND sample.status = 'active'
  AND sample.flagged = 0
  AND item.classification = 'person'
  AND sample.model_id = ?
  AND sample.model_version = ?
"""

# recognition_service.rs:29-35 (primary_face, feature column only)
PRIMARY_FEATURE_SQL = """
SELECT feature_json
FROM capture_faces
WHERE capture_item_id = ? AND is_primary = 1
"""

# capture_service.rs:1790-1818 (next_awaiting_label_without_feature)
PRELABEL_NEXT_SQL = """
SELECT
    id, project_id, session_id, asset_id, character_id, classification, source_path, file_size, modified_at_ms, content_hash,
    annotated_path, avatar_path, destination_path,
    destination_avatar_path, status, face_box_json, face_count,
    suggested_character_id, recognition_confidence, recognition_source,
    review_status, error_message,
    failure_stage, attempt_count, next_retry_at, processing_warnings_json,
    captured_at, processed_at, archived_at, created_at, updated_at
FROM capture_items
WHERE status = 'awaiting_label'
  AND recognition_deferred = 0
  AND NOT EXISTS (
      SELECT 1
      FROM capture_faces face
      WHERE face.capture_item_id = capture_items.id
        AND face.is_primary = 1
        AND face.feature_json IS NOT NULL
  )
  AND (next_retry_at IS NULL OR next_retry_at <= strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
ORDER BY captured_at ASC, created_at ASC
LIMIT 1
"""

# project_service.rs:218-297 (list_overviews)
OVERVIEWS_SQL = """
WITH capture_stats AS (
    SELECT
        project_id,
        COUNT(*) AS capture_count,
        SUM(CASE WHEN status = 'awaiting_label' THEN 1 ELSE 0 END) AS awaiting_count,
        SUM(CASE WHEN status IN ('queued', 'processing', 'archive_pending') THEN 1 ELSE 0 END) AS processing_count,
        SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END) AS completed_count,
        SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END) AS failed_count,
        MAX(captured_at) AS last_capture_at
    FROM capture_items
    WHERE classification <> 'private'
    GROUP BY project_id
),
session_stats AS (
    SELECT
        project_id,
        COUNT(*) AS session_count,
        SUM(CASE WHEN status = 'active' THEN 1 ELSE 0 END) AS active_session_count
    FROM capture_sessions
    GROUP BY project_id
),
source_stats AS (
    SELECT project_id, COUNT(*) AS source_count
    FROM project_source_directories
    WHERE enabled = 1
    GROUP BY project_id
)
SELECT
    p.id AS project_id,
    p.name,
    p.description,
    CASE
        WHEN EXISTS (
            SELECT 1
            FROM capture_items cover
            WHERE cover.id = p.cover_capture_item_id
              AND cover.project_id = p.id
              AND cover.classification <> 'private'
        ) THEN p.cover_capture_item_id
        ELSE NULL
    END AS cover_capture_item_id,
    p.created_at,
    COALESCE(src.source_count, 0) AS source_count,
    (p.destination_directory IS NOT NULL AND p.destination_directory <> '') AS destination_configured,
    COALESCE(ss.session_count, 0) AS session_count,
    COALESCE(ss.active_session_count, 0) AS active_session_count,
    COALESCE(cs.capture_count, 0) AS capture_count,
    COALESCE(cs.awaiting_count, 0) AS awaiting_count,
    COALESCE(cs.processing_count, 0) AS processing_count,
    COALESCE(cs.completed_count, 0) AS completed_count,
    COALESCE(cs.failed_count, 0) AS failed_count,
    COALESCE(cs.last_capture_at, p.updated_at) AS last_activity_at,
    (
        SELECT item.id
        FROM capture_items item
        WHERE item.project_id = p.id
          AND item.classification <> 'private'
        ORDER BY item.captured_at DESC, item.id DESC
        LIMIT 1
    ) AS latest_capture_item_id
FROM projects p
LEFT JOIN capture_stats cs ON cs.project_id = p.id
LEFT JOIN session_stats ss ON ss.project_id = p.id
LEFT JOIN source_stats src ON src.project_id = p.id
ORDER BY last_activity_at DESC, p.name COLLATE NOCASE ASC
"""

# capture_service.rs:235-265 (list_project_recent_items)
RECENT_ITEMS_SQL = """
SELECT
    item.id, item.project_id, item.session_id, item.asset_id,
    item.character_id, item.classification, item.source_path, item.file_size, item.modified_at_ms, item.content_hash,
    item.annotated_path, item.avatar_path, item.destination_path,
    item.destination_avatar_path, item.status, item.face_box_json, item.face_count,
    item.suggested_character_id, item.recognition_confidence,
    item.recognition_source, item.review_status,
    item.error_message, item.failure_stage, item.attempt_count,
    item.next_retry_at, item.processing_warnings_json,
    item.captured_at, item.processed_at, item.archived_at,
    item.created_at, item.updated_at
FROM capture_items item
JOIN capture_sessions session ON session.id = item.session_id
WHERE session.project_id = ? AND session.status = 'active'
ORDER BY item.captured_at DESC, item.created_at DESC
LIMIT ?
"""

# capture_service.rs:2130-2149 (matches_discovery_baseline)
BASELINE_SQL = """
SELECT file_size, modified_at_ms
FROM capture_session_baseline_files
WHERE session_id = ? AND source_path = ?
"""


# ---------------------------------------------------------------------------
# Small helpers
# ---------------------------------------------------------------------------

def _pct(sorted_values: list[float], percentile: float) -> float:
    if not sorted_values:
        return 0.0
    index = max(0, min(len(sorted_values) - 1, math.ceil(percentile * len(sorted_values)) - 1))
    return sorted_values[index]


def _summary(samples: list[float]) -> tuple[float, float, float]:
    ordered = sorted(samples)
    return ordered[0], ordered[len(ordered) // 2], ordered[-1]


def _metric(
    name: str,
    samples: list[float] | None,
    *,
    unit: str,
    threshold: float | None,
    gated: bool,
    note: str = "",
    skipped: bool = False,
    extra: dict | None = None,
) -> dict:
    if skipped or samples is None:
        return {
            "name": name,
            "value": None,
            "unit": unit,
            "pass": True,
            "gated": gated,
            "skipped": True,
            "threshold": threshold,
            "note": note or "skipped",
            **(extra or {}),
        }
    ordered = sorted(samples)
    value = ordered[len(ordered) // 2]
    passed = threshold is None or value < threshold
    return {
        "name": name,
        "value": round(value, 3),
        "unit": unit,
        "pass": bool(passed),
        "gated": gated,
        "skipped": False,
        "threshold": threshold,
        "p50": round(ordered[len(ordered) // 2], 3),
        "p95": round(_pct(ordered, 0.95), 3),
        "min": round(ordered[0], 3),
        "max": round(ordered[-1], 3),
        "samples": len(ordered),
        "note": note,
        **(extra or {}),
    }


def _timed(fn, reps: int, warmup: bool = True) -> list[float]:
    if warmup:
        fn()
    samples: list[float] = []
    for _ in range(max(1, reps)):
        start = time.perf_counter()
        fn()
        samples.append((time.perf_counter() - start) * 1000.0)
    return samples


def _open_db(path: Path) -> sqlite3.Connection:
    conn = sqlite3.connect(str(path))
    conn.execute("PRAGMA foreign_keys = ON")
    conn.execute("PRAGMA busy_timeout = 5000")
    conn.row_factory = sqlite3.Row
    return conn


def _feature_vector(seed: int) -> str:
    rng = random.Random(seed)
    vector = [rng.gauss(0.0, 1.0) for _ in range(128)]
    norm = math.sqrt(math.fsum(x * x for x in vector))
    vector = [round(x / norm, 9) for x in vector]
    return json.dumps(vector)


def _cosine(first: list[float], second: list[float]) -> float:
    # Mirrors recognition_service.rs:973-988. Uses C-level map/mul/fsum so the
    # proxy measures arithmetic rather than Python loop overhead.
    if not first or len(first) != len(second):
        return 0.0
    dot = math.fsum(map(operator.mul, first, second))
    first_norm = math.sqrt(math.fsum(map(operator.mul, first, first)))
    second_norm = math.sqrt(math.fsum(map(operator.mul, second, second)))
    denominator = first_norm * second_norm
    return dot / denominator if denominator != 0.0 else 0.0


def _format_timestamp(epoch_seconds: int) -> str:
    return (
        datetime.fromtimestamp(epoch_seconds, tz=timezone.utc)
        .strftime("%Y-%m-%dT%H:%M:%fZ")
    )


# ---------------------------------------------------------------------------
# Fixture
# ---------------------------------------------------------------------------

class Fixture:
    def __init__(self, root: Path, items: int, characters: int, scan_entries: int):
        self.root = root
        self.items = items
        self.characters = characters
        self.scan_entries = scan_entries
        self.db_path = root / "scene-vault.db"
        self.scan_dir = root / "source"
        self.project_id = "prj-0001"
        self.session_id = "sess-0001"
        self.character_ids = [f"char-{i:03d}" for i in range(characters)]
        self.person_items = int(items * 0.8)
        self.scene_items = int(items * 0.1)
        self.private_items = int(items * 0.05)
        self.unclassified_items = items - self.person_items - self.scene_items - self.private_items
        self.query_item_id: str | None = None

    def character_for_person(self, person_index: int) -> str:
        if person_index < 1200:
            return self.character_ids[0]
        remaining = self.person_items - 1200
        slot = ((person_index - 1200) * (self.characters - 1)) // max(1, remaining)
        return self.character_ids[1 + slot]


def _run_migrations(conn: sqlite3.Connection) -> int:
    migration_files = sorted(MIGRATIONS_DIR.glob("*.sql"))
    for migration in migration_files:
        conn.executescript(migration.read_text(encoding="utf-8"))
    return len(migration_files)


def _create_scan_directory(fixture: Fixture) -> list[tuple[str, int, int]]:
    fixture.scan_dir.mkdir(parents=True, exist_ok=True)
    entries: list[tuple[str, int, int]] = []
    for i in range(fixture.scan_entries):
        extension = "png" if i < int(fixture.scan_entries * 0.8) else "jpg"
        name = f"shot_{i:05d}.{extension}"
        path = fixture.scan_dir / name
        with open(path, "wb") as handle:
            handle.write(b"1")
        stat_result = os.stat(path)
        entries.append(
            (str(path), stat_result.st_size, stat_result.st_mtime_ns // 1_000_000)
        )
    return entries


def build_fixture(fixture: Fixture) -> dict:
    start = time.perf_counter()
    fixture.root.mkdir(parents=True, exist_ok=True)

    # The harness owns the fixture directory: reset a stale database from a
    # previous run so re-runs are idempotent.
    for suffix in ("", "-wal", "-shm"):
        candidate = fixture.db_path.with_name(fixture.db_path.name + suffix)
        if candidate.exists():
            candidate.unlink()

    # Source directory first: capture_items and baseline rows reference the
    # real scan files so the steady-state poll replica is self-consistent.
    # With --scan-entries 0 the scan benchmark is skipped and items use
    # synthetic placeholder paths.
    scan_entries = _create_scan_directory(fixture) if fixture.scan_entries > 0 else []

    conn = _open_db(fixture.db_path)
    conn.execute("PRAGMA journal_mode = WAL")
    migration_count = _run_migrations(conn)

    feature_cache = {
        char_id: _feature_vector(1000 + index)
        for index, char_id in enumerate(fixture.character_ids)
    }
    generic_feature = _feature_vector(999999)

    now = int(time.time())
    base_epoch = now - fixture.items

    with conn:
        conn.execute(
            "INSERT INTO projects (id, name, destination_directory, created_at, updated_at) VALUES (?, ?, ?, ?, ?)",
            (fixture.project_id, "Benchmark Project", str(fixture.root / "archive"), _format_timestamp(now), _format_timestamp(now)),
        )
        conn.executemany(
            "INSERT INTO characters (id, project_id, name, aliases_json, created_at, updated_at) VALUES (?, ?, ?, '[]', ?, ?)",
            [
                (char_id, fixture.project_id, f"Character_{index:03d}", _format_timestamp(now), _format_timestamp(now))
                for index, char_id in enumerate(fixture.character_ids)
            ],
        )
        conn.execute(
            "INSERT INTO capture_sessions (id, project_id, status, started_at, created_at, updated_at) VALUES (?, ?, 'active', ?, ?, ?)",
            (fixture.session_id, fixture.project_id, _format_timestamp(now - 60), _format_timestamp(now - 60), _format_timestamp(now - 60)),
        )
        conn.execute(
            "INSERT INTO session_source_directories (session_id, directory, enabled, discovery_started_at_ms, baseline_initialized) VALUES (?, ?, 1, ?, 1)",
            (fixture.session_id, str(fixture.scan_dir), int((now - 60) * 1000)),
        )
        conn.execute(
            "INSERT INTO project_source_directories (id, project_id, directory, enabled, created_at) VALUES ('src-0001', ?, ?, 1, ?)",
            (fixture.project_id, str(fixture.scan_dir), _format_timestamp(now)),
        )

        item_rows: list[tuple] = []
        face_rows: list[tuple] = []
        sample_rows: list[tuple] = []
        person_index = 0
        unclassified_ids: list[str] = []

        for i in range(fixture.items):
            item_id = f"item-{i:05d}"
            if scan_entries:
                source_path, file_size, modified_at_ms = scan_entries[i]
            else:
                source_path = f"C:/SceneVaultBench/Source/shot_{i:05d}.png"
                file_size = 1024
                modified_at_ms = (base_epoch + i) * 1000
            if i < fixture.person_items:
                classification = "person"
                status = "completed"
                character_id = fixture.character_for_person(person_index)
                person_index += 1
            elif i < fixture.person_items + fixture.scene_items:
                classification = "scene"
                status = "completed"
                character_id = None
            elif i < fixture.person_items + fixture.scene_items + fixture.private_items:
                classification = "private"
                status = "completed"
                character_id = None
            else:
                classification = "unclassified"
                status = "awaiting_label"
                character_id = None
                unclassified_ids.append(item_id)

            captured_at = _format_timestamp(base_epoch + i)
            content_hash = f"{i:040x}"
            item_rows.append(
                (
                    item_id,
                    fixture.project_id,
                    fixture.session_id,
                    character_id,
                    classification,
                    source_path,
                    file_size,
                    modified_at_ms,
                    content_hash,
                    status,
                    0,
                    captured_at,
                    captured_at,
                    captured_at,
                )
            )

            feature_json = feature_cache.get(character_id or "", generic_feature)
            face_rows.append(
                (
                    f"face-{i:05d}",
                    item_id,
                    0,
                    1,
                    '{"x":10,"y":10,"w":64,"h":64}',
                    feature_json,
                    MODEL_ID,
                    MODEL_VERSION,
                    128,
                    100.0,
                    0.12,
                    character_id,
                    captured_at,
                    captured_at,
                )
            )
            if classification == "person":
                sample_rows.append(
                    (
                        f"sample-{i:05d}",
                        character_id,
                        item_id,
                        feature_json,
                        0.95,
                        "active",
                        0,
                        MODEL_ID,
                        MODEL_VERSION,
                        128,
                        captured_at,
                        captured_at,
                    )
                )

        conn.executemany(
            """INSERT INTO capture_items (
                id, project_id, session_id, character_id, classification, source_path,
                file_size, modified_at_ms, content_hash, status, recognition_deferred,
                captured_at, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)""",
            item_rows,
        )
        conn.executemany(
            """INSERT INTO capture_faces (
                id, capture_item_id, face_index, is_primary, box_json, feature_json,
                feature_model_id, feature_model_version, feature_dim,
                face_sharpness, face_area_ratio, confirmed_character_id, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)""",
            face_rows,
        )
        conn.executemany(
            """INSERT INTO character_face_samples (
                id, character_id, capture_item_id, feature_json, confidence, status,
                flagged, model_id, model_version, embedding_dim, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)""",
            sample_rows,
        )
        if scan_entries:
            conn.executemany(
                "INSERT INTO capture_session_baseline_files (session_id, source_path, file_size, modified_at_ms) VALUES (?, ?, ?, ?)",
                [
                    (fixture.session_id, source_path, file_size, modified_at_ms)
                    for source_path, file_size, modified_at_ms in scan_entries
                ],
            )

    fixture.query_item_id = unclassified_ids[0] if unclassified_ids else f"item-00000"
    conn.close()

    build_ms = (time.perf_counter() - start) * 1000.0
    return {
        "migration_count": migration_count,
        "scan_entries": len(scan_entries),
        "build_ms": build_ms,
    }


# ---------------------------------------------------------------------------
# Metric executors
# ---------------------------------------------------------------------------

def measure_startup(fixture: Fixture, reps: int) -> dict:
    def run_once() -> list[float]:
        start = time.perf_counter()
        conn = _open_db(fixture.db_path)
        conn.execute("PRAGMA journal_mode = WAL")
        quick_check_ms = 0.0
        counts_ms = 0.0
        t0 = time.perf_counter()
        conn.execute("PRAGMA quick_check").fetchall()
        quick_check_ms = (time.perf_counter() - t0) * 1000.0
        t0 = time.perf_counter()
        conn.execute("SELECT COUNT(*) FROM capture_items").fetchone()
        conn.execute("SELECT COUNT(*) FROM characters").fetchone()
        conn.execute("SELECT COUNT(*) FROM character_face_samples").fetchone()
        counts_ms = (time.perf_counter() - t0) * 1000.0
        conn.close()
        return [(time.perf_counter() - start) * 1000.0, quick_check_ms, counts_ms]

    samples: list[float] = []
    quick_check: list[float] = []
    counts: list[float] = []
    for _ in range(max(1, reps)):
        total, check, count = run_once()
        samples.append(total)
        quick_check.append(check)
        counts.append(count)
    return {
        "startup_db_init_ms": _metric("startup_db_init_ms", samples, unit="ms", threshold=5000.0, gated=True, note="fresh connect + WAL + quick_check + counts (Python sqlite3 proxy of db::initialize)"),
        "startup_quick_check_ms": _metric("startup_quick_check_ms", quick_check, unit="ms", threshold=None, gated=False),
        "startup_counts_ms": _metric("startup_counts_ms", counts, unit="ms", threshold=None, gated=False),
    }


def _measure_query(conn: sqlite3.Connection, sql: str, params: tuple, reps: int, warmup: bool = True) -> list[float]:
    def run_once() -> None:
        conn.execute(sql, params).fetchall()
    return _timed(run_once, reps, warmup=warmup)


def measure_read_paths(fixture: Fixture, reps: int) -> dict:
    conn = _open_db(fixture.db_path)
    try:
        overviews = _measure_query(conn, OVERVIEWS_SQL, (), reps)
        recent = _measure_query(conn, RECENT_ITEMS_SQL, (fixture.project_id, 100), reps)
        history_count = _measure_query(
            conn,
            HISTORY_COUNT_SQL,
            (fixture.project_id, None, None, None, None, None, None, 1),
            reps,
        )
        history_page = _measure_query(
            conn,
            HISTORY_PAGE_SQL,
            (fixture.project_id, None, None, None, None, None, None, 1, 100, 9000),
            reps,
        )
        history_backend = [a + b for a, b in zip(history_count, history_page)]
        character_grid = _measure_query(
            conn,
            CHARACTER_ITEMS_SQL,
            (fixture.project_id, fixture.character_ids[0]),
            reps,
        )
        category_metrics: dict = {}
        for category, filter_sql in CATEGORY_FILTERS.items():
            sql = CATEGORY_ITEM_SELECT.format(filter=filter_sql)
            category_metrics[f"category_{category}_ms"] = _metric(
                f"category_{category}_ms",
                _measure_query(conn, sql, (fixture.project_id,), reps),
                unit="ms",
                threshold=1000.0,
                gated=True,
                note=f"list_category_items('{category}') unpaged",
            )
        session_all = _measure_query(conn, LIST_ITEMS_SQL, (fixture.session_id,), reps)
        prelabel = _measure_query(conn, PRELABEL_NEXT_SQL, (), reps)
    finally:
        conn.close()

    return {
        "home_overview_ms": _metric("home_overview_ms", overviews, unit="ms", threshold=None, gated=False, note="list_overviews on 1 project / 10k items"),
        "recent_items_100_ms": _metric("recent_items_100_ms", recent, unit="ms", threshold=None, gated=False),
        "history_count_ms": _metric("history_count_ms", history_count, unit="ms", threshold=None, gated=False),
        "history_page_ms": _metric("history_page_ms", history_page, unit="ms", threshold=None, gated=False),
        "history_backend_ms": _metric("history_backend_ms", history_backend, unit="ms", threshold=200.0, gated=True, note="list_history count + OFFSET 9000 LIMIT 100"),
        "workbench_character_grid_ms": _metric("workbench_character_grid_ms", character_grid, unit="ms", threshold=1000.0, gated=True, note="list_character_items for largest character"),
        **category_metrics,
        "list_session_all_ms": _metric("list_session_all_ms", session_all, unit="ms", threshold=None, gated=False, note="list_items unpaged 10k rows"),
        "prelabel_next_awaiting_ms": _metric("prelabel_next_awaiting_ms", prelabel, unit="ms", threshold=100.0, gated=True, note="soft gate; next_awaiting_label_without_feature"),
    }


def measure_face_bank(fixture: Fixture, reps: int) -> dict:
    conn = _open_db(fixture.db_path)
    try:
        query_feature: list[float] = []
        row = conn.execute(PRIMARY_FEATURE_SQL, (fixture.query_item_id,)).fetchone()
        if row is not None:
            query_feature = json.loads(row[0])

        def suggestion_once() -> None:
            rows = conn.execute(
                SUGGEST_SAMPLES_SQL,
                (fixture.project_id, fixture.query_item_id, "", MODEL_ID, MODEL_VERSION),
            ).fetchall()
            character_scores: dict[str, float] = {}
            for sample_character_id, sample_json in rows:
                sample = json.loads(sample_json)
                similarity = _cosine(query_feature, sample)
                character_scores[sample_character_id] = max(
                    character_scores.get(sample_character_id, 0.0), similarity
                )
            ranked = sorted(character_scores.items(), key=lambda item: item[1], reverse=True)
            runner_up = ranked[1][1] if len(ranked) > 1 else 0.0
            if ranked and ranked[0][1] >= CONFIDENCE_THRESHOLD and ranked[0][1] - runner_up >= MARGIN:
                pass  # decision mirrors suggest_from_face_bank; write is excluded

        verify_item_id = f"item-00000"
        verify_feature: list[float] = []
        row = conn.execute(PRIMARY_FEATURE_SQL, (verify_item_id,)).fetchone()
        if row is not None:
            verify_feature = json.loads(row[0])
        verify_character_id = fixture.character_ids[0]

        def verify_once() -> None:
            rows = conn.execute(
                VERIFY_SAMPLES_SQL,
                (fixture.session_id, MODEL_ID, MODEL_VERSION),
            ).fetchall()
            score: float | None = None
            best_other = 0.0
            for sample_character_id, sample_json in rows:
                sample = json.loads(sample_json)
                similarity = _cosine(verify_feature, sample)
                if sample_character_id == verify_character_id:
                    score = similarity if score is None else max(score, similarity)
                else:
                    best_other = max(best_other, similarity)

        suggestion_samples = _timed(suggestion_once, reps)
        verify_samples = _timed(verify_once, reps)
        suggestion_median = sorted(suggestion_samples)[len(suggestion_samples) // 2]
        rebuild_estimate = suggestion_median * fixture.unclassified_items
    finally:
        conn.close()

    return {
        "face_bank_suggestion_ms": _metric(
            "face_bank_suggestion_ms",
            suggestion_samples,
            unit="ms",
            threshold=150.0,
            gated=True,
            note="10k-sample load + JSON parse + cosine + rank; Python proxy of suggest_from_face_bank (write excluded)",
        ),
        "face_bank_verify_ms": _metric(
            "face_bank_verify_ms",
            verify_samples,
            unit="ms",
            threshold=None,
            gated=False,
            note="compute_verification replica (soft 150 ms expectation)",
        ),
        "rebuild_refresh_estimate_ms": _metric(
            "rebuild_refresh_estimate_ms",
            [rebuild_estimate],
            unit="ms",
            threshold=None,
            gated=False,
            note=f"estimated {fixture.unclassified_items} unclassified items x suggestion median; worker/off-UI in product",
        ),
    }


def measure_scan(fixture: Fixture, reps: int) -> dict:
    if fixture.scan_entries <= 0:
        return {
            "scan_filesystem_ms": _metric("scan_filesystem_ms", None, unit="ms", threshold=1000.0, gated=True, skipped=True, note="--scan-entries 0"),
            "scan_full_ms": _metric("scan_full_ms", None, unit="ms", threshold=1000.0, gated=True, skipped=True, note="--scan-entries 0"),
        }
    conn = _open_db(fixture.db_path)
    scan_dir = fixture.scan_dir

    def filesystem_once() -> int:
        candidates: list[tuple[str, int, int]] = []
        with os.scandir(scan_dir) as iterator:
            for entry in iterator:
                if not entry.name.rsplit(".", 1)[-1].lower() in SUPPORTED_IMAGE_EXTS:
                    continue
                try:
                    info = entry.stat()
                except OSError:
                    continue
                if not stat.S_ISREG(info.st_mode) or info.st_size == 0:
                    continue
                candidates.append((entry.path, info.st_size, info.st_mtime_ns))
        candidates.sort(key=lambda candidate: candidate[0])
        stable = 0
        for path, size, mtime_ns in candidates:
            try:
                info = os.stat(path)
            except OSError:
                continue
            if not stat.S_ISREG(info.st_mode) or info.st_size != size or info.st_mtime_ns != mtime_ns:
                continue
            os.path.realpath(path)
            stable += 1
        return stable

    def full_once() -> None:
        filesystem_once()
        with os.scandir(scan_dir) as iterator:
            for entry in iterator:
                if not entry.name.rsplit(".", 1)[-1].lower() in SUPPORTED_IMAGE_EXTS:
                    continue
                try:
                    info = entry.stat()
                except OSError:
                    continue
                if not stat.S_ISREG(info.st_mode) or info.st_size == 0:
                    continue
                conn.execute(BASELINE_SQL, (fixture.session_id, entry.path)).fetchone()

    fs_samples = _timed(lambda: filesystem_once(), max(1, min(3, reps)))
    full_samples = _timed(full_once, max(1, min(3, reps)))
    scanned = 0
    with os.scandir(scan_dir) as iterator:
        scanned = sum(1 for _ in iterator)
    conn.close()
    return {
        "scan_filesystem_ms": _metric(
            "scan_filesystem_ms",
            fs_samples,
            unit="ms",
            threshold=1000.0,
            gated=True,
            note="read_dir + stat + filter + sort + re-stat + realpath (Python proxy of capture_discovery_service::discover file work)",
            extra={"scan_entries_scanned": scanned},
        ),
        "scan_full_ms": _metric(
            "scan_full_ms",
            full_samples,
            unit="ms",
            threshold=1000.0,
            gated=True,
            note="steady-state poll: file work + per-candidate baseline lookup; Python/sqlite proxy, Rust async expected faster",
            extra={"scan_entries_scanned": scanned},
        ),
    }


def measure_storage(fixture: Fixture) -> dict:
    conn = _open_db(fixture.db_path)
    conn.execute("PRAGMA wal_checkpoint(TRUNCATE)")
    page_size = conn.execute("PRAGMA page_size").fetchone()[0]
    page_count = conn.execute("PRAGMA page_count").fetchone()[0]
    freelist = conn.execute("PRAGMA freelist_count").fetchone()[0]
    sample_bytes = conn.execute(
        "SELECT SUM(LENGTH(feature_json)) FROM character_face_samples"
    ).fetchone()[0]
    sample_rows = conn.execute("SELECT COUNT(*) FROM character_face_samples").fetchone()[0]
    conn.close()

    db_main = fixture.db_path.stat().st_size
    wal_path = fixture.db_path.with_name(fixture.db_path.name + "-wal")
    shm_path = fixture.db_path.with_name(fixture.db_path.name + "-shm")
    wal_size = wal_path.stat().st_size if wal_path.exists() else 0
    shm_size = shm_path.stat().st_size if shm_path.exists() else 0
    return {
        "db_main_bytes": _metric(
            "db_main_bytes",
            [float(db_main)],
            unit="bytes",
            threshold=100.0 * 1024 * 1024,
            gated=True,
            note="10k-row DB main file after WAL checkpoint TRUNCATE",
            extra={"page_size": page_size, "page_count": page_count, "freelist_count": freelist},
        ),
        "db_total_bytes": _metric(
            "db_total_bytes",
            [float(db_main + wal_size + shm_size)],
            unit="bytes",
            threshold=None,
            gated=False,
            extra={"wal_bytes": wal_size, "shm_bytes": shm_size},
        ),
        "sample_bank_json_bytes": _metric(
            "sample_bank_json_bytes",
            [float(sample_bytes or 0.0)],
            unit="bytes",
            threshold=None,
            gated=False,
            extra={"sample_rows": sample_rows},
        ),
    }


# ---------------------------------------------------------------------------
# Report
# ---------------------------------------------------------------------------

def _affinity() -> str:
    try:
        cpus = sorted(os.sched_getaffinity(0))
    except (AttributeError, OSError):
        return "not-restricted"
    if not cpus:
        return "none"
    ranges: list[str] = []
    start = previous = cpus[0]
    for cpu in cpus[1:]:
        if cpu == previous + 1:
            previous = cpu
            continue
        ranges.append(str(start) if start == previous else f"{start}-{previous}")
        start = previous = cpu
    ranges.append(str(start) if start == previous else f"{start}-{previous}")
    return ",".join(ranges)


def _human_summary(metrics: dict, gated_failures: list[dict]) -> None:
    print("=== Scene Vault 1.0 scale benchmark (summary) ===", file=sys.stderr)
    for metric in metrics.values():
        if metric.get("skipped"):
            print(f"  SKIP {metric['name']}", file=sys.stderr)
            continue
        status = "PASS" if metric.get("pass") else "FAIL"
        print(
            f"  {status} {metric['name']}: {metric.get('value')} {metric.get('unit')}"
            + (f" (threshold < {metric['threshold']})" if metric.get("threshold") is not None else ""),
            file=sys.stderr,
        )
    if gated_failures:
        print("Failed gates:", file=sys.stderr)
        for failure in gated_failures:
            print(
                f"  - {failure['name']}: {failure.get('value')} {failure.get('unit')}"
                f" (threshold < {failure.get('threshold')})",
                file=sys.stderr,
            )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, help="write JSON report to this path (stdout always receives it)")
    parser.add_argument("--fixture-dir", type=Path, help="build fixture in this directory (created if missing)")
    parser.add_argument("--keep-fixture", action="store_true", help="do not delete the fixture directory afterwards")
    parser.add_argument("--repetitions", type=int, default=5, help="timing repetitions per metric (default 5)")
    parser.add_argument("--items", type=int, default=TARGET_ITEMS, help=f"capture items (default {TARGET_ITEMS})")
    parser.add_argument("--characters", type=int, default=TARGET_CHARACTERS, help=f"characters (default {TARGET_CHARACTERS})")
    parser.add_argument("--scan-entries", type=int, default=TARGET_SCAN_ENTRIES, help=f"source directory entries (default {TARGET_SCAN_ENTRIES})")
    args = parser.parse_args(argv)

    if args.fixture_dir is not None:
        fixture_root = args.fixture_dir
        fixture_root.mkdir(parents=True, exist_ok=True)
        cleanup = False
    else:
        fixture_root = Path(tempfile.mkdtemp(prefix="sv-scale-bench-"))
        cleanup = True

    fixture = Fixture(fixture_root, args.items, args.characters, args.scan_entries)
    try:
        fixture_info = build_fixture(fixture)
        metrics: dict = {}
        metrics["fixture_build_ms"] = _metric(
            "fixture_build_ms",
            [fixture_info["build_ms"]],
            unit="ms",
            threshold=300_000.0,
            gated=True,
            note="migrations + 10k-file scan directory + fixture inserts; generous environment-dependent gate",
        )
        metrics.update(measure_startup(fixture, max(1, min(3, args.repetitions))))
        metrics.update(measure_read_paths(fixture, args.repetitions))
        metrics.update(measure_face_bank(fixture, args.repetitions))
        metrics.update(measure_scan(fixture, args.repetitions))
        metrics.update(measure_storage(fixture))

        gated = [metric for metric in metrics.values() if metric.get("gated") and not metric.get("skipped")]
        gated_failures = [metric for metric in gated if not metric.get("pass")]
        all_passed = not gated_failures

        report = {
            "tool": "scripts/scale-benchmark/benchmark_scale_1_0.py",
            "schema_version": SCHEMA_VERSION,
            "generated_at_utc": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%fZ"),
            "environment": {
                "platform": platform.platform(),
                "python_version": platform.python_version(),
                "sqlite_version": sqlite3.sqlite_version,
                "cpu_affinity": _affinity(),
                "cpu_count": os.cpu_count(),
                "cwd": str(Path.cwd()),
                "repo_root": str(REPO_ROOT),
            },
            "target": {
                "capture_items": fixture.items,
                "characters": fixture.characters,
                "source_dir_entries": fixture.scan_entries,
                "distribution": {
                    "person": fixture.person_items,
                    "scene": fixture.scene_items,
                    "private": fixture.private_items,
                    "unclassified": fixture.unclassified_items,
                },
            },
            "fixture": {
                "path": str(fixture_root),
                "kept": (not cleanup) or args.keep_fixture,
                "migration_count": fixture_info["migration_count"],
                "scan_entries": fixture_info["scan_entries"],
                "capture_items": fixture.items,
                "characters": fixture.characters,
                "face_rows": fixture.items,
                "face_samples": fixture.person_items,
            },
            "metrics": metrics,
            "summary": {
                "gated_metrics": len(gated),
                "passed_gated": len(gated) - len(gated_failures),
                "failed_gated": len(gated_failures),
                "all_passed": all_passed,
                "failed_names": [metric["name"] for metric in gated_failures],
            },
            "references": {
                "history": "src-tauri/src/services/capture_service.rs:500",
                "category_list": "src-tauri/src/services/capture_service.rs:1419",
                "character_grid": "src-tauri/src/services/recognition_service.rs:337",
                "suggestion": "src-tauri/src/services/recognition_service.rs:855",
                "prelabel": "src-tauri/src/services/capture_service.rs:1790",
                "scan": "src-tauri/src/services/capture_discovery_service.rs:77",
                "migrations": "src-tauri/migrations",
            },
            "limitations": [
                "SQL statements are copied verbatim from services but executed through Python sqlite3, not the Rust/sqlx service code; CPU-bound metrics (face bank, scan) include interpreter overhead and are expected to be slower than the product.",
                "The suggestion write (set_suggestion UPDATE) is excluded; it is a single-row update.",
                "p95 is a sample p95 (small N); median is the pass/fail value.",
                "Fixture is synthetic: deterministic vectors, uniform timestamps, single project/session, 1-byte placeholder images, no thumbnails/archives.",
                "sqlx migration checksums, pool behavior and Windows bundled SQLite version are not replicated.",
                "Scan benchmark runs on local temp storage; NAS/UNC latency is not covered.",
                "Full startup (<5 s) also depends on WebView2/UI, which this harness does not measure.",
            ],
        }

        if args.out is not None:
            args.out.parent.mkdir(parents=True, exist_ok=True)
            args.out.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
        print(json.dumps(report, ensure_ascii=False, indent=2))
        _human_summary(metrics, gated_failures)
        return 0 if all_passed else 1
    finally:
        if cleanup and not args.keep_fixture:
            shutil.rmtree(fixture_root, ignore_errors=True)


if __name__ == "__main__":
    raise SystemExit(main())
