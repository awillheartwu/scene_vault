-- One project may run several Ren'Py games at once: one row per installation
-- instead of one row per project. `game_directory` is now globally unique, so
-- a single game folder can never be owned by two installs or projects.
CREATE TABLE project_engine_integrations_v2 (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    engine TEXT NOT NULL,
    engine_root TEXT NOT NULL,
    game_directory TEXT NOT NULL COLLATE NOCASE UNIQUE,
    python_major INTEGER,
    bridge_version TEXT NOT NULL,
    metadata_directory TEXT,
    manifest_json TEXT NOT NULL DEFAULT '[]',
    installed_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- Move legacy single-install rows over, minting a UUID for each row. When
-- legacy history left more than one row for one game directory, keep the row
-- with the richest manifest (the best ownership evidence) instead of
-- discarding evidence by insertion order; the others are dropped because the
-- new schema allows one owner per game directory.
INSERT INTO project_engine_integrations_v2 (
    id, project_id, engine, engine_root, game_directory, python_major,
    bridge_version, metadata_directory, manifest_json, installed_at, updated_at
)
SELECT
    lower(
        hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4'
        || substr(hex(randomblob(2)), 2) || '-'
        || substr('89ab', abs(random()) % 4 + 1, 1)
        || substr(hex(randomblob(2)), 2) || '-' || hex(randomblob(6))
    ),
    project_id, engine, engine_root, game_directory, python_major,
    bridge_version, metadata_directory, manifest_json, installed_at, updated_at
FROM project_engine_integrations
WHERE rowid IN (
    SELECT (
        SELECT p2.rowid
        FROM project_engine_integrations p2
        WHERE lower(p2.game_directory) = lower(p1.game_directory)
        ORDER BY length(p2.manifest_json) DESC, p2.rowid ASC
        LIMIT 1
    )
    FROM project_engine_integrations p1
);

DROP TABLE project_engine_integrations;

ALTER TABLE project_engine_integrations_v2 RENAME TO project_engine_integrations;

CREATE INDEX idx_project_engine_integrations_project
    ON project_engine_integrations(project_id);
