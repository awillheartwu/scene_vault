-- Remember the last capture source/archive directories per project so a new
-- session can be started without re-picking directories every time.
ALTER TABLE projects ADD COLUMN last_source_directory TEXT;
ALTER TABLE projects ADD COLUMN last_destination_directory TEXT;
