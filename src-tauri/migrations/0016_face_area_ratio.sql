-- Share of the image occupied by the primary face (area ratio). The
-- sample-enrollment quality gate uses it to reject background/tiny faces.
-- Nullable: legacy rows or skipped detection.
ALTER TABLE capture_faces
ADD COLUMN face_area_ratio REAL CHECK (
    face_area_ratio IS NULL OR (face_area_ratio >= 0.0 AND face_area_ratio <= 1.0)
);
