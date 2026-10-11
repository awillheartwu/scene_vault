-- Per-project archive output bound for raw source copies. 'keep' preserves the
-- captured bytes; '1080p'/'1440p'/'2160p' downscale the image height during
-- archiving. Never upscales, and extracted game assets are never re-encoded.
ALTER TABLE projects ADD COLUMN archive_output_size TEXT NOT NULL DEFAULT 'keep';
