-- User-picked project cover. NULL means the Home library falls back to the
-- project's latest capture. A capture of any classification (person, scene,
-- private, unclassified) can become the cover.
ALTER TABLE projects ADD COLUMN cover_capture_item_id TEXT;
