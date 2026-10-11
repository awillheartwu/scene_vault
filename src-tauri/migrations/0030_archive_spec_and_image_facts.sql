-- Archive output becomes a three-part policy (size/format/quality) with the
-- same global default + per-project inheritance the size switch already uses.
-- 'native' follows the engine-reported screen size of each capture; formats
-- cover 'keep' (never convert) plus explicit png/jpeg/webp.
ALTER TABLE projects ADD COLUMN archive_output_format TEXT NOT NULL DEFAULT 'inherit';
ALTER TABLE projects ADD COLUMN archive_output_quality INTEGER;

-- Per-capture image facts shown by the UI (resolution, format, quality, size).
-- A 1:1 side table, like capture_engine_metadata, so the many existing
-- CaptureItem queries stay untouched. The JSON columns carry the versioned
-- payload written by the app; unknown values stay NULL instead of guessed.
CREATE TABLE capture_image_facts (
    capture_item_id TEXT PRIMARY KEY REFERENCES capture_items(id) ON DELETE CASCADE,
    source_json TEXT,
    destination_json TEXT,
    updated_at TEXT NOT NULL
);
