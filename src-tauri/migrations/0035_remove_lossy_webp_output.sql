-- Lossy WebP output was removed. Preserve the selected container with
-- lossless WebP for future archives; existing files and image facts stay intact.
UPDATE projects
SET archive_output_format = 'webp',
    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
WHERE archive_output_format = 'webp_lossy';

UPDATE settings
SET value_json = json_set(value_json, '$.format', 'webp'),
    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
WHERE key = 'archive.output_size'
  AND CASE WHEN json_valid(value_json)
      THEN json_extract(value_json, '$.format') END = 'webp_lossy';
