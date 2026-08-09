CREATE TABLE project_notes (
    id TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL UNIQUE REFERENCES projects(id) ON DELETE CASCADE,
    destination_directory TEXT NOT NULL,
    remote_path TEXT NOT NULL,
    content TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('synced', 'pending', 'failed')),
    attempt_count INTEGER NOT NULL DEFAULT 0,
    next_retry_at TEXT,
    -- Last content that was confirmed written to remote_path, used to detect
    -- external edits (e.g. Obsidian) and back them up instead of overwriting.
    last_synced_content TEXT,
    error_message TEXT,
    synced_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_project_notes_status ON project_notes(status);
