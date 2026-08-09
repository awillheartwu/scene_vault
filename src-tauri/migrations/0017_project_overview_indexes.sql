-- Home project library aggregates capture state and resolves each project's
-- latest non-private capture. These indexes keep that read path bounded as
-- project histories grow.
CREATE INDEX idx_capture_items_project_status
    ON capture_items(project_id, status);

CREATE INDEX idx_capture_items_project_captured_at
    ON capture_items(project_id, captured_at DESC, id DESC);
