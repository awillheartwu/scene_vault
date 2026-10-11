-- Trusted staging root for bridge-extracted assets, recorded at install time
-- so sidecar-provided paths can be validated against the app cache instead
-- of being trusted and later deleted as if they were ours.
ALTER TABLE project_engine_integrations ADD COLUMN assets_directory TEXT;

-- Whether source assets were ingested for this capture. 0 lets the retry
-- pass top up captures whose metadata arrived before their assets were
-- readable (or before the trusted staging root was known).
ALTER TABLE capture_engine_metadata ADD COLUMN assets_ingested INTEGER NOT NULL DEFAULT 0;
