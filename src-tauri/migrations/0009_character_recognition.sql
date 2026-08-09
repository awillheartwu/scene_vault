-- Milestone 7: character workbench recognition review fields.
--
-- capture_items gains the AI-suggestion columns used by the workbench:
-- the suggested character (never auto-applied), the confidence and source
-- of the suggestion, and the human review decision. Writing a new suggestion
-- resets review_status to 'pending'; accepting or rejecting records the
-- decision without changing character_id itself.

ALTER TABLE capture_items
ADD COLUMN suggested_character_id TEXT REFERENCES characters(id) ON DELETE SET NULL;

ALTER TABLE capture_items
ADD COLUMN recognition_confidence REAL
    CHECK (
        recognition_confidence IS NULL
        OR (recognition_confidence >= 0.0 AND recognition_confidence <= 1.0)
    );

ALTER TABLE capture_items
ADD COLUMN recognition_source TEXT
    CHECK (
        recognition_source IS NULL
        OR recognition_source IN ('face_bank', 'vision', 'manual')
    );

ALTER TABLE capture_items
ADD COLUMN review_status TEXT NOT NULL DEFAULT 'none'
    CHECK (review_status IN ('none', 'pending', 'accepted', 'rejected'));

CREATE INDEX idx_capture_items_review_pending
ON capture_items(review_status) WHERE review_status = 'pending';

-- Face Bank samples: one SFace feature vector per character per capture,
-- collected from human-confirmed captures for M5 automatic suggestions.
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
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE UNIQUE INDEX idx_character_face_samples_character_capture
ON character_face_samples(character_id, capture_item_id);

CREATE INDEX idx_character_face_samples_character_status
ON character_face_samples(character_id, status);
