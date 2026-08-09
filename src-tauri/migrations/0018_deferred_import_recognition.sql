-- Imported screenshots are registered immediately but must not start the
-- optional pre-label recognition pass until the user explicitly starts the
-- batch from the Capture page. Existing awaiting-label rows are paused once
-- during this migration so an upgrade cannot restart a large pending batch.

ALTER TABLE capture_items
ADD COLUMN recognition_deferred INTEGER NOT NULL DEFAULT 0
    CHECK (recognition_deferred IN (0, 1));

UPDATE capture_items
SET recognition_deferred = 1
WHERE status = 'awaiting_label';

CREATE INDEX idx_capture_items_deferred_recognition
ON capture_items(session_id, recognition_deferred, captured_at)
WHERE status = 'awaiting_label' AND recognition_deferred = 1;
