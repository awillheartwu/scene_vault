ALTER TABLE capture_sessions
ADD COLUMN discovery_started_at_ms INTEGER NOT NULL DEFAULT 0
    CHECK (discovery_started_at_ms >= 0);

