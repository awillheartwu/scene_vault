-- Opt-in per installation: existing games keep their original screenshot action.
ALTER TABLE project_engine_integrations
    ADD COLUMN auto_hide_ui INTEGER NOT NULL DEFAULT 0
    CHECK (auto_hide_ui IN (0, 1));
