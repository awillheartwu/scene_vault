ALTER TABLE capture_items
ADD COLUMN classification TEXT NOT NULL DEFAULT 'unclassified'
    CHECK (classification IN ('unclassified', 'person', 'scene', 'private'));

-- Backfill legacy rows: any item that already has a character is a person shot.
UPDATE capture_items
SET classification = 'person'
WHERE character_id IS NOT NULL;

CREATE INDEX idx_capture_items_classification
ON capture_items(classification);
