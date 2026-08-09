-- Project-level multi-source capture (Option B).
--
-- A project owns a list of source directories and one archive destination.
-- Capture sessions become work periods that snapshot the enabled directories
-- at start time. New capture items carry a content fingerprint
-- (file size + mtime + sha256) with a unique (project, hash) index so the
-- same image appearing in two directories registers only once.
--
-- NOTE: pre-release schema reset. The old session/item tables are dropped
-- because their data is disposable test data (approved by the owner).

DROP TABLE character_face_samples;
DROP TABLE capture_session_baseline_files;
DROP TABLE capture_items;
DROP TABLE capture_sessions;

ALTER TABLE projects ADD COLUMN destination_directory TEXT;

CREATE TABLE capture_sessions (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'active'
        CHECK (status IN ('active', 'completed', 'cancelled')),
    started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ended_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- Snapshot of the directories a work period listened to; later project
-- directory changes do not rewrite finished periods.
CREATE TABLE session_source_directories (
    session_id TEXT NOT NULL REFERENCES capture_sessions(id) ON DELETE CASCADE,
    directory TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    discovery_started_at_ms INTEGER NOT NULL DEFAULT 0
        CHECK (discovery_started_at_ms >= 0),
    baseline_initialized INTEGER NOT NULL DEFAULT 0
        CHECK (baseline_initialized IN (0, 1)),
    PRIMARY KEY (session_id, directory)
);

-- The authoritative source-directory list of a project.
CREATE TABLE project_source_directories (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    directory TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (project_id, directory)
);

CREATE INDEX idx_capture_sessions_project ON capture_sessions(project_id);
CREATE INDEX idx_capture_sessions_status ON capture_sessions(status);
CREATE INDEX idx_project_source_directories_project
    ON project_source_directories(project_id);

CREATE TABLE capture_session_baseline_files (
    session_id TEXT NOT NULL REFERENCES capture_sessions(id) ON DELETE CASCADE,
    source_path TEXT NOT NULL,
    file_size INTEGER NOT NULL CHECK (file_size >= 0),
    modified_at_ms INTEGER,
    PRIMARY KEY (session_id, source_path)
);

CREATE INDEX idx_capture_baseline_session
    ON capture_session_baseline_files(session_id);

CREATE TABLE capture_items (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    session_id TEXT NOT NULL REFERENCES capture_sessions(id) ON DELETE CASCADE,
    asset_id TEXT REFERENCES assets(id) ON DELETE SET NULL,
    character_id TEXT REFERENCES characters(id) ON DELETE SET NULL,
    source_path TEXT NOT NULL,
    classification TEXT NOT NULL DEFAULT 'unclassified'
        CHECK (classification IN ('unclassified', 'person', 'scene', 'private')),
    file_size INTEGER CHECK (file_size IS NULL OR file_size >= 0),
    modified_at_ms INTEGER,
    content_hash TEXT,
    annotated_path TEXT,
    avatar_path TEXT,
    destination_path TEXT,
    destination_avatar_path TEXT,
    status TEXT NOT NULL DEFAULT 'awaiting_label'
        CHECK (status IN (
            'awaiting_label',
            'queued',
            'processing',
            'archive_pending',
            'completed',
            'failed'
        )),
    face_box_json TEXT,
    error_message TEXT,
    failure_stage TEXT
        CHECK (failure_stage IS NULL OR failure_stage IN ('processing', 'archive')),
    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    next_retry_at TEXT,
    processing_warnings_json TEXT NOT NULL DEFAULT '[]',
    suggested_character_id TEXT REFERENCES characters(id) ON DELETE SET NULL,
    recognition_confidence REAL
        CHECK (
            recognition_confidence IS NULL
            OR (recognition_confidence >= 0.0 AND recognition_confidence <= 1.0)
        ),
    recognition_source TEXT
        CHECK (
            recognition_source IS NULL
            OR recognition_source IN ('face_bank', 'vision', 'manual')
        ),
    review_status TEXT NOT NULL DEFAULT 'none'
        CHECK (review_status IN ('none', 'pending', 'accepted', 'rejected')),
    face_feature_json TEXT,
    verification_score REAL,
    verification_status TEXT NOT NULL DEFAULT 'unverified'
        CHECK (verification_status IN ('unverified', 'ok', 'flagged')),
    best_other_score REAL,
    best_other_character_id TEXT REFERENCES characters(id) ON DELETE SET NULL,
    captured_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    processed_at TEXT,
    archived_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (session_id, source_path)
);

-- One registration per unique image per project, regardless of directory.
CREATE UNIQUE INDEX idx_capture_items_project_hash
    ON capture_items(project_id, content_hash)
    WHERE content_hash IS NOT NULL;

CREATE INDEX idx_capture_items_session ON capture_items(session_id);
CREATE INDEX idx_capture_items_status ON capture_items(status);
CREATE INDEX idx_capture_items_character ON capture_items(character_id);
CREATE INDEX idx_capture_items_review_pending
    ON capture_items(review_status) WHERE review_status = 'pending';

CREATE TABLE character_face_samples (
    id TEXT PRIMARY KEY NOT NULL,
    character_id TEXT NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    capture_item_id TEXT NOT NULL REFERENCES capture_items(id) ON DELETE CASCADE,
    face_box_json TEXT,
    feature_json TEXT NOT NULL,
    confidence REAL
        CHECK (confidence IS NULL OR (confidence >= 0.0 AND confidence <= 1.0)),
    status TEXT NOT NULL DEFAULT 'active'
        CHECK (status IN ('active', 'revoked')),
    flagged INTEGER NOT NULL DEFAULT 0 CHECK (flagged IN (0, 1)),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE UNIQUE INDEX idx_character_face_samples_character_capture
    ON character_face_samples(character_id, capture_item_id);

CREATE INDEX idx_character_face_samples_character_status
    ON character_face_samples(character_id, status);
