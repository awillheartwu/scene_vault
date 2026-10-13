-- Keep migration 0033 unchanged once applied. Promote only untouched,
-- uniform legacy installation choices; later project/game choices stay intact.
CREATE TEMP TABLE scenevault_renpy_uniform_legacy AS
SELECT integrations.project_id, MIN(integrations.auto_hide_ui_override) AS auto_hide_ui
FROM project_engine_integrations AS integrations
JOIN projects ON projects.id = integrations.project_id
WHERE projects.renpy_auto_hide_ui IS NULL
GROUP BY integrations.project_id
HAVING COUNT(*) = COUNT(integrations.auto_hide_ui_override)
    AND MIN(integrations.auto_hide_ui_override) = MAX(integrations.auto_hide_ui_override)
    AND SUM(CASE WHEN integrations.auto_hide_ui_override = integrations.auto_hide_ui THEN 0 ELSE 1 END) = 0;

UPDATE projects SET renpy_auto_hide_ui = (
    SELECT auto_hide_ui FROM scenevault_renpy_uniform_legacy
    WHERE project_id = projects.id
) WHERE id IN (SELECT project_id FROM scenevault_renpy_uniform_legacy);

UPDATE project_engine_integrations SET auto_hide_ui_override = NULL
WHERE project_id IN (SELECT project_id FROM scenevault_renpy_uniform_legacy);

DROP TABLE scenevault_renpy_uniform_legacy;
