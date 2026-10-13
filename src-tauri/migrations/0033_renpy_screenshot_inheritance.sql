-- Global defaults live in settings; projects and installations can override.
ALTER TABLE projects ADD COLUMN renpy_auto_hide_ui INTEGER
    CHECK (renpy_auto_hide_ui IS NULL OR renpy_auto_hide_ui IN (0, 1));
ALTER TABLE project_engine_integrations ADD COLUMN auto_hide_ui_override INTEGER
    CHECK (auto_hide_ui_override IS NULL OR auto_hide_ui_override IN (0, 1));

-- Preserve every existing per-game choice. New installations inherit instead.
UPDATE project_engine_integrations SET auto_hide_ui_override = auto_hide_ui;
