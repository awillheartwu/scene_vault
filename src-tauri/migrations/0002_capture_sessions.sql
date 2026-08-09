CREATE TABLE characters (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name TEXT NOT NULL COLLATE NOCASE,
    aliases_json TEXT NOT NULL DEFAULT '[]',
    avatar_asset_id TEXT REFERENCES assets(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (project_id, name)
);

CREATE TABLE capture_sessions (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    source_directory TEXT NOT NULL,
    destination_directory TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active'
        CHECK (status IN ('active', 'completed', 'cancelled')),
    started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ended_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE capture_items (
    id TEXT PRIMARY KEY NOT NULL,
    session_id TEXT NOT NULL REFERENCES capture_sessions(id) ON DELETE CASCADE,
    asset_id TEXT REFERENCES assets(id) ON DELETE SET NULL,
    character_id TEXT REFERENCES characters(id) ON DELETE SET NULL,
    source_path TEXT NOT NULL,
    annotated_path TEXT,
    avatar_path TEXT,
    destination_path TEXT,
    status TEXT NOT NULL DEFAULT 'awaiting_label'
        CHECK (status IN (
            'awaiting_label',
            'queued',
            'processing',
            'archive_pending',
            'completed',
            'failed'
        )),
    face_box_json TEXT,
    error_message TEXT,
    captured_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    processed_at TEXT,
    archived_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (session_id, source_path)
);

CREATE TABLE asset_characters (
    asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
    character_id TEXT NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    face_box_json TEXT,
    confidence REAL CHECK (confidence IS NULL OR (confidence >= 0 AND confidence <= 1)),
    is_primary INTEGER NOT NULL DEFAULT 0 CHECK (is_primary IN (0, 1)),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (asset_id, character_id)
);

CREATE INDEX idx_characters_project ON characters(project_id);
CREATE INDEX idx_capture_sessions_project ON capture_sessions(project_id);
CREATE INDEX idx_capture_sessions_status ON capture_sessions(status);
CREATE INDEX idx_capture_items_session ON capture_items(session_id);
CREATE INDEX idx_capture_items_status ON capture_items(status);
CREATE INDEX idx_capture_items_character ON capture_items(character_id);
CREATE INDEX idx_asset_characters_character ON asset_characters(character_id);
