-- Optional per-project engine integration (first engine: Ren'Py). The bridge
-- is an explicitly installed, user-visible set of files inside the game
-- folder; non-Ren'Py projects never get rows here.
CREATE TABLE project_engine_integrations (
    project_id TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
    engine TEXT NOT NULL,
    engine_root TEXT NOT NULL,
    game_directory TEXT NOT NULL,
    python_major INTEGER,
    bridge_version TEXT NOT NULL,
    manifest_json TEXT NOT NULL DEFAULT '[]',
    installed_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- Engine metadata attached to one capture (speaker, dialogue, scene tags,
-- resolutions, bridge version). payload_json stays authoritative; promoted
-- columns serve queries and the speaker-to-character suggestion.
CREATE TABLE capture_engine_metadata (
    capture_item_id TEXT PRIMARY KEY REFERENCES capture_items(id) ON DELETE CASCADE,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    engine TEXT NOT NULL,
    schema_version INTEGER NOT NULL,
    bridge_version TEXT NOT NULL,
    speaker_raw TEXT,
    speaker_source TEXT,
    dialogue TEXT,
    suggested_character_id TEXT,
    virtual_width INTEGER,
    virtual_height INTEGER,
    physical_width INTEGER,
    physical_height INTEGER,
    payload_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX idx_capture_engine_metadata_project
    ON capture_engine_metadata(project_id);

-- Original game assets extracted by the bridge. Content-addressed by sha256:
-- the same asset referenced by several captures stores one archived copy.
CREATE TABLE capture_source_assets (
    id TEXT PRIMARY KEY,
    capture_item_id TEXT NOT NULL REFERENCES capture_items(id) ON DELETE CASCADE,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    game_path TEXT NOT NULL,
    sha256 TEXT NOT NULL,
    bytes INTEGER NOT NULL,
    width INTEGER,
    height INTEGER,
    staged_path TEXT,
    archived_path TEXT,
    archive_file_state TEXT NOT NULL DEFAULT 'none',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (capture_item_id, sha256)
);

CREATE INDEX idx_capture_source_assets_project_sha
    ON capture_source_assets(project_id, sha256);
