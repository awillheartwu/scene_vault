-- Content identities recorded only after a verified archive write/reuse.
-- No backfill from current disk contents: legacy files have unknown identity.
-- Keep each output independently, including an avatar and partial archive retries.
CREATE TABLE capture_archive_fingerprints (
    capture_item_id TEXT NOT NULL REFERENCES capture_items(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    PRIMARY KEY (capture_item_id, path)
);
