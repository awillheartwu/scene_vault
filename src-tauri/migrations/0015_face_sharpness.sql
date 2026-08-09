-- Sharpness of the primary face (Laplacian variance) reported by Python.
-- The sample-enrollment quality gate uses it: faces whose sharpness is below
-- the configured threshold (e.g. printed/photo faces in scenes) are not
-- enrolled into the Face Bank. Nullable: legacy rows or skipped detection.
ALTER TABLE capture_faces
ADD COLUMN face_sharpness REAL CHECK (
    face_sharpness IS NULL OR face_sharpness >= 0
);
