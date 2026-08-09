-- Face Bank: per-capture SFace feature vector (JSON array of floats) so the
-- feature survives between the pre-label extraction pass and enrollment, and
-- so Rust can match it against character_face_samples without asking Python
-- again. Nullable: captures without a detectable face have no feature.

ALTER TABLE capture_items
ADD COLUMN face_feature_json TEXT;
