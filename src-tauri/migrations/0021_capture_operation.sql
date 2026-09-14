-- User intent is independent of regenerable face features.
ALTER TABLE capture_items ADD COLUMN manual_face_roi_json TEXT;
ALTER TABLE capture_items ADD COLUMN manual_face_roi_ready INTEGER NOT NULL DEFAULT 0;
ALTER TABLE capture_items ADD COLUMN processing_version INTEGER NOT NULL DEFAULT 0;
-- A failed/reset operation retains ownership across restarts until retried.
ALTER TABLE capture_items ADD COLUMN operation_owner TEXT;
