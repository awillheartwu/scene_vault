-- Capture file lifecycle and same-path generations.
--
-- A capture item identifies immutable content. Screenshot tools commonly
-- recycle names, so the same source path may belong to several historical
-- items as long as their project-wide content hashes differ. File-state
-- columns record availability without overloading processing status.

CREATE TABLE character_face_samples_0020 AS
SELECT * FROM character_face_samples;

CREATE TABLE capture_faces_0020 AS
SELECT * FROM capture_faces;

DROP TABLE character_face_samples;
DROP TABLE capture_faces;

CREATE TABLE capture_items_0020 (
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
            'awaiting_label', 'queued', 'processing',
            'archive_pending', 'completed', 'failed'
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
    face_count INTEGER CHECK (face_count IS NULL OR face_count >= 0),
    recognition_deferred INTEGER NOT NULL DEFAULT 0
        CHECK (recognition_deferred IN (0, 1)),
    source_file_state TEXT NOT NULL DEFAULT 'unknown'
        CHECK (source_file_state IN ('unknown', 'available', 'missing', 'replaced')),
    destination_file_state TEXT NOT NULL DEFAULT 'none'
        CHECK (destination_file_state IN ('none', 'unknown', 'available', 'missing', 'unavailable')),
    destination_avatar_file_state TEXT NOT NULL DEFAULT 'none'
        CHECK (destination_avatar_file_state IN ('none', 'unknown', 'available', 'missing', 'unavailable'))
);

INSERT INTO capture_items_0020 (
    id, project_id, session_id, asset_id, character_id, source_path,
    classification, file_size, modified_at_ms, content_hash,
    annotated_path, avatar_path, destination_path, destination_avatar_path,
    status, face_box_json, error_message, failure_stage, attempt_count,
    next_retry_at, processing_warnings_json, suggested_character_id,
    recognition_confidence, recognition_source, review_status,
    face_feature_json, verification_score, verification_status,
    best_other_score, best_other_character_id, captured_at, processed_at,
    archived_at, created_at, updated_at, face_count, recognition_deferred,
    source_file_state, destination_file_state, destination_avatar_file_state
)
SELECT
    id, project_id, session_id, asset_id, character_id, source_path,
    classification, file_size, modified_at_ms, content_hash,
    annotated_path, avatar_path, destination_path, destination_avatar_path,
    status, face_box_json, error_message, failure_stage, attempt_count,
    next_retry_at, processing_warnings_json, suggested_character_id,
    recognition_confidence, recognition_source, review_status,
    face_feature_json, verification_score, verification_status,
    best_other_score, best_other_character_id, captured_at, processed_at,
    archived_at, created_at, updated_at, face_count, recognition_deferred,
    'unknown',
    CASE WHEN destination_path IS NULL THEN 'none' ELSE 'unknown' END,
    CASE WHEN destination_avatar_path IS NULL THEN 'none' ELSE 'unknown' END
FROM capture_items;

DROP TABLE capture_items;
ALTER TABLE capture_items_0020 RENAME TO capture_items;

CREATE UNIQUE INDEX idx_capture_items_project_hash
    ON capture_items(project_id, content_hash)
    WHERE content_hash IS NOT NULL;
CREATE INDEX idx_capture_items_session ON capture_items(session_id);
CREATE INDEX idx_capture_items_session_source
    ON capture_items(session_id, source_path, captured_at DESC);
CREATE INDEX idx_capture_items_status ON capture_items(status);
CREATE INDEX idx_capture_items_character ON capture_items(character_id);
CREATE INDEX idx_capture_items_classification ON capture_items(classification);
CREATE INDEX idx_capture_items_verification_status
    ON capture_items(verification_status);
CREATE INDEX idx_capture_items_project_status
    ON capture_items(project_id, status);
CREATE INDEX idx_capture_items_project_captured_at
    ON capture_items(project_id, captured_at DESC, id DESC);
CREATE INDEX idx_capture_items_review_pending
    ON capture_items(review_status) WHERE review_status = 'pending';
CREATE INDEX idx_capture_items_deferred_recognition
    ON capture_items(session_id, recognition_deferred, captured_at)
    WHERE status = 'awaiting_label' AND recognition_deferred = 1;

CREATE TABLE character_face_samples (
    id TEXT PRIMARY KEY NOT NULL,
    character_id TEXT NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    capture_item_id TEXT NOT NULL REFERENCES capture_items(id) ON DELETE CASCADE,
    face_box_json TEXT,
    feature_json TEXT NOT NULL,
    confidence REAL CHECK (confidence IS NULL OR (confidence >= 0.0 AND confidence <= 1.0)),
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'revoked')),
    flagged INTEGER NOT NULL DEFAULT 0 CHECK (flagged IN (0, 1)),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    model_id TEXT NOT NULL DEFAULT 'opencv-sface',
    model_version TEXT NOT NULL DEFAULT '2021dec',
    embedding_dim INTEGER NOT NULL DEFAULT 128 CHECK (embedding_dim > 0)
);

INSERT INTO character_face_samples
SELECT * FROM character_face_samples_0020;
DROP TABLE character_face_samples_0020;

CREATE UNIQUE INDEX idx_character_face_samples_character_capture
    ON character_face_samples(character_id, capture_item_id);
CREATE INDEX idx_character_face_samples_character_status
    ON character_face_samples(character_id, status);
CREATE INDEX idx_character_face_samples_model
    ON character_face_samples(model_id, model_version, embedding_dim, status);

CREATE TABLE capture_faces (
    id TEXT PRIMARY KEY NOT NULL,
    capture_item_id TEXT NOT NULL REFERENCES capture_items(id) ON DELETE CASCADE,
    face_index INTEGER NOT NULL CHECK (face_index >= 0),
    is_primary INTEGER NOT NULL DEFAULT 0 CHECK (is_primary IN (0, 1)),
    box_json TEXT,
    feature_json TEXT,
    feature_model_id TEXT,
    feature_model_version TEXT,
    feature_dim INTEGER CHECK (feature_dim IS NULL OR feature_dim > 0),
    confirmed_character_id TEXT REFERENCES characters(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    face_sharpness REAL CHECK (face_sharpness IS NULL OR face_sharpness >= 0),
    face_area_ratio REAL CHECK (
        face_area_ratio IS NULL OR (face_area_ratio >= 0.0 AND face_area_ratio <= 1.0)
    )
);

INSERT INTO capture_faces
SELECT * FROM capture_faces_0020;
DROP TABLE capture_faces_0020;

CREATE UNIQUE INDEX idx_capture_faces_one_primary
    ON capture_faces(capture_item_id) WHERE is_primary = 1;
CREATE UNIQUE INDEX idx_capture_faces_item_face
    ON capture_faces(capture_item_id, face_index);

CREATE TABLE ignored_capture_contents (
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    content_hash TEXT NOT NULL,
    source_path TEXT NOT NULL,
    ignored_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (project_id, content_hash)
);
