-- Original-asset extraction was removed before shipping: drop its table.
DROP TABLE IF EXISTS capture_source_assets;

-- No more asset ingestion bookkeeping.
ALTER TABLE capture_engine_metadata DROP COLUMN assets_ingested;

-- The bridge now writes screenshot metadata files into this cache directory
-- (nothing is written next to the screenshots anymore).
ALTER TABLE project_engine_integrations RENAME COLUMN assets_directory TO metadata_directory;

-- Archive output size becomes a global default with per-project overrides;
-- the feature never shipped, so every existing project simply inherits.
UPDATE projects SET archive_output_size = 'inherit';
