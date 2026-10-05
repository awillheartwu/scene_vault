-- Per-capture detection override for close-up ("big face") screenshots.
-- 'auto' keeps the global bigFace policy; 'normalized' forces the downscaled
-- detection pass for this capture.
ALTER TABLE capture_items ADD COLUMN face_detection_mode TEXT NOT NULL DEFAULT 'auto';
