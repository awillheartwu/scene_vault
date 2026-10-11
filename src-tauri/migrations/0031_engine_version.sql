-- Exact Ren'Py version: read from renpy/vc_version.py when an installation is
-- detected, and reported by the bridge (1.1.4+) for every screenshot. NULL on
-- installations or captures that predate this or cannot be read.
ALTER TABLE project_engine_integrations ADD COLUMN engine_version TEXT;
ALTER TABLE capture_engine_metadata ADD COLUMN engine_version TEXT;
