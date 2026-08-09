-- Face-level data: per-face rows and recognition model identity.
--
-- Phase 1 writes only the primary face (face_index = 0, is_primary = 1); the
-- schema allows N rows per capture so a future multi-face phase needs no
-- rewrite. Feature columns are nullable: a row can exist without a usable
-- feature. feature_json = '[]' is the "attempted, no face detected" marker
-- (same convention the pre-label pass used on capture_items).

CREATE TABLE capture_faces (
    id TEXT PRIMARY KEY NOT NULL,
    capture_item_id TEXT NOT NULL REFERENCES capture_items(id) ON DELETE CASCADE,
    face_index INTEGER NOT NULL CHECK (face_index >= 0),
    is_primary INTEGER NOT NULL DEFAULT 0 CHECK (is_primary IN (0, 1)),
    box_json TEXT,
    feature_json TEXT,
    feature_model_id TEXT,
    feature_model_version TEXT,
    feature_dim INTEGER CHECK (
        feature_dim IS NULL OR feature_dim > 0
    ),
    confirmed_character_id TEXT REFERENCES characters(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- A capture has at most one primary face, enforced by the database.
CREATE UNIQUE INDEX idx_capture_faces_one_primary
ON capture_faces(capture_item_id) WHERE is_primary = 1;

-- One row per face per capture (primary index, also serves lookups by item).
CREATE UNIQUE INDEX idx_capture_faces_item_face
ON capture_faces(capture_item_id, face_index);

-- Face Bank samples record which recognition model produced the embedding.
-- Embeddings from different models live in different vector spaces and must
-- never be compared (matching filters on model identity).
ALTER TABLE character_face_samples
ADD COLUMN model_id TEXT NOT NULL DEFAULT 'opencv-sface';

ALTER TABLE character_face_samples
ADD COLUMN model_version TEXT NOT NULL DEFAULT '2021dec';

ALTER TABLE character_face_samples
ADD COLUMN embedding_dim INTEGER NOT NULL DEFAULT 128
    CHECK (embedding_dim > 0);

-- Authorized cleanup (owner, 2026-08-08): every existing face value is test
-- data produced by the polluted OpenCV 5.0.0 pipeline, so it is discarded.
-- The face_feature_json column stays (deprecated, no runtime read/write) for
-- a later DROP; suggestions and verification results derived from polluted
-- features are invalidated here too.
UPDATE capture_items
SET face_feature_json = NULL,
    suggested_character_id = NULL,
    recognition_confidence = NULL,
    recognition_source = NULL,
    review_status = 'none',
    verification_score = NULL,
    verification_status = 'unverified',
    best_other_score = NULL,
    best_other_character_id = NULL;

DELETE FROM character_face_samples;

-- Structural backfill: old items recorded only the primary face box. One
-- primary face row is created per boxed capture with the box preserved and
-- NO feature (polluted embeddings are never copied). Rows without a feature
-- or model stay eligible for re-extraction by the rebuild pass. Historical
-- additional faces are not guessed: capture_faces going live does NOT mean
-- history automatically gains multi-face data.
INSERT INTO capture_faces (
    id, capture_item_id, face_index, is_primary, box_json
)
SELECT
    lower(
        hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-' ||
        hex(randomblob(2)) || '-' || hex(randomblob(2)) || '-' ||
        hex(randomblob(6))
    ),
    id,
    0,
    1,
    face_box_json
FROM capture_items
WHERE face_box_json IS NOT NULL;
