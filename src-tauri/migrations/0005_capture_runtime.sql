CREATE TABLE capture_session_baseline_files (
    session_id TEXT NOT NULL REFERENCES capture_sessions(id) ON DELETE CASCADE,
    source_path TEXT NOT NULL,
    file_size INTEGER NOT NULL CHECK (file_size >= 0),
    modified_at_ms INTEGER,
    PRIMARY KEY (session_id, source_path)
);

CREATE INDEX idx_capture_baseline_session
ON capture_session_baseline_files(session_id);

ALTER TABLE capture_sessions
ADD COLUMN baseline_initialized INTEGER NOT NULL DEFAULT 0
    CHECK (baseline_initialized IN (0, 1));

ALTER TABLE capture_items
ADD COLUMN failure_stage TEXT
    CHECK (failure_stage IS NULL OR failure_stage IN ('processing', 'archive'));

ALTER TABLE capture_items
ADD COLUMN attempt_count INTEGER NOT NULL DEFAULT 0
    CHECK (attempt_count >= 0);

ALTER TABLE capture_items
ADD COLUMN next_retry_at TEXT;

ALTER TABLE capture_items
ADD COLUMN processing_warnings_json TEXT NOT NULL DEFAULT '[]';
