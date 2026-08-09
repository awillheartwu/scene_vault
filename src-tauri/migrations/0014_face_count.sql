-- Face count of the source screenshot, reported by YuNet. Phase 1 keeps the
-- single-primary-face pipeline but the UI warns when several faces were
-- detected (the filename/scene character may differ from the primary face).
ALTER TABLE capture_items
ADD COLUMN face_count INTEGER CHECK (face_count IS NULL OR face_count >= 0);
