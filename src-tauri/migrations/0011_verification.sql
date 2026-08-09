-- Closed-set verification: when a capture is labeled, the selected
-- character's face-bank samples are used to verify "is this face this
-- person". The score and cross-character score are stored so the enrollment
-- step (which runs later, at processing completion) can decide whether the
-- sample participates in matching.

ALTER TABLE capture_items
ADD COLUMN verification_score REAL;

ALTER TABLE capture_items
ADD COLUMN verification_status TEXT NOT NULL DEFAULT 'unverified'
    CHECK (verification_status IN ('unverified', 'ok', 'flagged'));

ALTER TABLE capture_items
ADD COLUMN best_other_score REAL;

ALTER TABLE capture_items
ADD COLUMN best_other_character_id TEXT REFERENCES characters(id) ON DELETE SET NULL;

-- Samples from low-confidence forced confirms are flagged and excluded from
-- matching until the player restores them in the workbench.
ALTER TABLE character_face_samples
ADD COLUMN flagged INTEGER NOT NULL DEFAULT 0 CHECK (flagged IN (0, 1));

CREATE INDEX idx_capture_items_verification_status
ON capture_items(verification_status);
