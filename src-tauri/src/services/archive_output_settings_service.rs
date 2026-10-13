use sqlx::SqlitePool;

use crate::{
    error::AppError,
    models::engine::{
        is_archive_output_format, is_archive_output_quality, is_archive_output_size,
        ArchiveOutputSettings, CapturePolicyView, SetProjectCapturePolicyInput,
        UpdateArchiveOutputSettingsInput, ARCHIVE_OUTPUT_INHERIT,
    },
};

const SETTINGS_KEY: &str = "archive.output_size";

/// Fully resolved archive policy for one project: size, format and JPEG
/// quality after inheritance from the global defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedArchiveSpec {
    pub size: String,
    pub format: String,
    pub quality: i64,
}

pub async fn get(pool: &SqlitePool) -> Result<ArchiveOutputSettings, AppError> {
    let value: Option<String> = sqlx::query_scalar("SELECT value_json FROM settings WHERE key = ?")
        .bind(SETTINGS_KEY)
        .fetch_optional(pool)
        .await?;
    match value {
        Some(value) => serde_json::from_str(&value).map_err(|error| {
            AppError::Validation(format!(
                "stored archive output settings are invalid: {error}"
            ))
        }),
        None => Ok(ArchiveOutputSettings::default()),
    }
}

pub async fn update(
    pool: &SqlitePool,
    input: UpdateArchiveOutputSettingsInput,
) -> Result<ArchiveOutputSettings, AppError> {
    let settings = normalize(input.settings)?;
    let value = serde_json::to_string(&settings).map_err(|error| {
        AppError::Validation(format!("cannot encode archive output settings: {error}"))
    })?;
    sqlx::query(
        r#"
        INSERT INTO settings (key, value_json)
        VALUES (?, ?)
        ON CONFLICT(key) DO UPDATE SET
            value_json = excluded.value_json,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        "#,
    )
    .bind(SETTINGS_KEY)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(settings)
}

fn normalize(mut settings: ArchiveOutputSettings) -> Result<ArchiveOutputSettings, AppError> {
    settings.size = settings.size.trim().to_lowercase();
    if !is_archive_output_size(&settings.size) {
        return Err(AppError::Validation(
            "归档输出尺寸只能是 keep / 1080p / 1440p / 2160p / native".to_owned(),
        ));
    }
    settings.format = settings.format.trim().to_lowercase();
    if !is_archive_output_format(&settings.format) {
        return Err(AppError::Validation(
            "归档输出格式只能是 keep / png / jpeg / webp".to_owned(),
        ));
    }
    if !is_archive_output_quality(settings.quality) {
        return Err(AppError::Validation(
            "归档 JPEG 质量需要在 60 到 100 之间".to_owned(),
        ));
    }
    Ok(settings)
}

fn inherited_value(
    project_value: Option<&str>,
    allowed: fn(&str) -> bool,
    fallback: &str,
) -> String {
    match project_value.map(|value| value.trim().to_lowercase()) {
        Some(value) if value != ARCHIVE_OUTPUT_INHERIT && !value.is_empty() && allowed(&value) => {
            value
        }
        _ => fallback.to_owned(),
    }
}

/// Effective policy for one project; a missing project falls back to the
/// global defaults (matching the previous size-only behavior).
pub async fn resolve_spec_for_project(
    pool: &SqlitePool,
    project_id: &str,
) -> Result<ResolvedArchiveSpec, AppError> {
    let global = get(pool).await?;
    let row: Option<(Option<String>, Option<String>, Option<i64>)> = sqlx::query_as(
        "SELECT archive_output_size, archive_output_format, archive_output_quality \
         FROM projects WHERE id = ?",
    )
    .bind(project_id)
    .fetch_optional(pool)
    .await?;
    let (size, format, quality) = row.unwrap_or((None, None, None));
    let quality = match quality {
        Some(value) if is_archive_output_quality(value) => value,
        _ => global.quality,
    };
    Ok(ResolvedArchiveSpec {
        size: inherited_value(size.as_deref(), is_archive_output_size, &global.size),
        format: inherited_value(format.as_deref(), is_archive_output_format, &global.format),
        quality,
    })
}

/// Project values plus resolved values for the capture page policy panel.
pub async fn project_policy_view(
    pool: &SqlitePool,
    project_id: &str,
) -> Result<CapturePolicyView, AppError> {
    let row: Option<(String, String, Option<i64>)> = sqlx::query_as(
        "SELECT archive_output_size, archive_output_format, archive_output_quality \
         FROM projects WHERE id = ?",
    )
    .bind(project_id)
    .fetch_optional(pool)
    .await?;
    let (project_size, project_format, project_quality) =
        row.ok_or_else(|| AppError::NotFound("project".to_owned()))?;
    let global = get(pool).await?;
    let resolved = resolve_spec_for_project(pool, project_id).await?;
    Ok(CapturePolicyView {
        project_size,
        resolved_size: resolved.size,
        global_size: global.size,
        project_format,
        resolved_format: resolved.format,
        global_format: global.format,
        project_quality,
        resolved_quality: resolved.quality,
        global_quality: global.quality,
    })
}

/// Sets the size only (legacy entry point kept for older callers).
pub async fn set_project_size(
    pool: &SqlitePool,
    project_id: &str,
    size: &str,
) -> Result<CapturePolicyView, AppError> {
    let value = size.trim().to_lowercase();
    if value != ARCHIVE_OUTPUT_INHERIT && !is_archive_output_size(&value) {
        return Err(AppError::Validation(
            "归档输出尺寸只能是 inherit / keep / 1080p / 1440p / 2160p / native".to_owned(),
        ));
    }
    let result = sqlx::query(
        "UPDATE projects SET archive_output_size = ?, \
         updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
    )
    .bind(&value)
    .bind(project_id)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("project".to_owned()));
    }
    project_policy_view(pool, project_id).await
}

/// Saves all three policy fields at once (capture page).
pub async fn set_project_capture_policy(
    pool: &SqlitePool,
    input: SetProjectCapturePolicyInput,
) -> Result<CapturePolicyView, AppError> {
    let project_id = input.project_id.trim();
    let size = input.archive_output_size.trim().to_lowercase();
    if size != ARCHIVE_OUTPUT_INHERIT && !is_archive_output_size(&size) {
        return Err(AppError::Validation(
            "归档输出尺寸只能是 inherit / keep / 1080p / 1440p / 2160p / native".to_owned(),
        ));
    }
    let format = input.archive_output_format.trim().to_lowercase();
    if format != ARCHIVE_OUTPUT_INHERIT && !is_archive_output_format(&format) {
        return Err(AppError::Validation(
            "归档输出格式只能是 inherit / keep / png / jpeg / webp".to_owned(),
        ));
    }
    if let Some(quality) = input.archive_output_quality {
        if !is_archive_output_quality(quality) {
            return Err(AppError::Validation(
                "归档 JPEG 质量需要在 60 到 100 之间".to_owned(),
            ));
        }
    }
    let result = sqlx::query(
        "UPDATE projects SET archive_output_size = ?, archive_output_format = ?, \
         archive_output_quality = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
         WHERE id = ?",
    )
    .bind(&size)
    .bind(&format)
    .bind(input.archive_output_quality)
    .bind(project_id)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("project".to_owned()));
    }
    project_policy_view(pool, project_id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn settings(size: &str) -> ArchiveOutputSettings {
        ArchiveOutputSettings {
            size: size.to_owned(),
            format: "keep".to_owned(),
            quality: 90,
        }
    }

    #[tokio::test]
    async fn defaults_to_keep_and_round_trips() {
        let pool = db::test_pool().await;
        let defaults = get(&pool).await.expect("defaults");
        assert_eq!(defaults.size, "keep");
        assert_eq!(defaults.format, "keep");
        assert_eq!(defaults.quality, 90);

        let saved = update(
            &pool,
            UpdateArchiveOutputSettingsInput {
                settings: ArchiveOutputSettings {
                    size: "1080p".to_owned(),
                    format: "webp".to_owned(),
                    quality: 95,
                },
            },
        )
        .await
        .expect("save");
        assert_eq!(saved.size, "1080p");
        assert_eq!(saved.format, "webp");
        assert_eq!(saved.quality, 95);
        let loaded = get(&pool).await.expect("read");
        assert_eq!(loaded, saved);
    }

    #[tokio::test]
    async fn legacy_settings_json_gets_the_new_defaults() {
        let pool = db::test_pool().await;
        sqlx::query("INSERT INTO settings (key, value_json) VALUES (?, ?)")
            .bind(SETTINGS_KEY)
            .bind("{\"size\":\"1440p\"}")
            .execute(&pool)
            .await
            .expect("legacy row");
        let loaded = get(&pool).await.expect("read legacy");
        assert_eq!(loaded.size, "1440p");
        assert_eq!(loaded.format, "keep");
        assert_eq!(loaded.quality, 90);
    }

    #[tokio::test]
    async fn removal_migration_preserves_other_policy_fields() {
        let pool = db::test_pool().await;
        let legacy_project = uuid::Uuid::new_v4().to_string();
        let jpeg_project = uuid::Uuid::new_v4().to_string();
        for (id, format) in [(&legacy_project, "webp_lossy"), (&jpeg_project, "jpeg")] {
            sqlx::query("INSERT INTO projects (id, name, archive_output_size, archive_output_format, archive_output_quality) VALUES (?, 'Migration', 'native', ?, 100)")
                .bind(id).bind(format).execute(&pool).await.unwrap();
        }
        sqlx::query("INSERT INTO settings (key, value_json) VALUES (?, ?)")
            .bind(SETTINGS_KEY)
            .bind(r#"{"size":"1440p","format":"webp_lossy","quality":88,"extra":"preserved"}"#)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO settings (key, value_json) VALUES ('unrelated', '{')")
            .execute(&pool)
            .await
            .unwrap();

        sqlx::raw_sql(include_str!(
            "../../migrations/0035_remove_lossy_webp_output.sql"
        ))
        .execute(&pool)
        .await
        .unwrap();
        let saved = get(&pool).await.unwrap();
        assert_eq!(saved.size, "1440p");
        assert_eq!(saved.format, "webp");
        assert_eq!(saved.quality, 88);
        let raw: String = sqlx::query_scalar("SELECT value_json FROM settings WHERE key = ?")
            .bind(SETTINGS_KEY)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&raw).unwrap()["extra"],
            "preserved"
        );
        for (id, expected) in [(&legacy_project, "webp"), (&jpeg_project, "jpeg")] {
            let view = project_policy_view(&pool, id).await.unwrap();
            assert_eq!(view.project_format, expected);
            assert_eq!(view.project_size, "native");
            assert_eq!(view.project_quality, Some(100));
        }
        let unrelated: String =
            sqlx::query_scalar("SELECT value_json FROM settings WHERE key = 'unrelated'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(unrelated, "{");

        sqlx::query("UPDATE settings SET value_json = '{' WHERE key = ?")
            .bind(SETTINGS_KEY)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::raw_sql(include_str!(
            "../../migrations/0035_remove_lossy_webp_output.sql"
        ))
        .execute(&pool)
        .await
        .unwrap();
        let malformed: String = sqlx::query_scalar("SELECT value_json FROM settings WHERE key = ?")
            .bind(SETTINGS_KEY)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(malformed, "{");
    }

    #[tokio::test]
    async fn rejects_unknown_values() {
        let pool = db::test_pool().await;
        let error = update(
            &pool,
            UpdateArchiveOutputSettingsInput {
                settings: settings("720p"),
            },
        )
        .await
        .expect_err("reject size");
        assert!(matches!(error, AppError::Validation(_)));

        for format in ["avif", "webp_lossy"] {
            let error = update(
                &pool,
                UpdateArchiveOutputSettingsInput {
                    settings: ArchiveOutputSettings {
                        size: "keep".to_owned(),
                        format: format.to_owned(),
                        quality: 90,
                    },
                },
            )
            .await
            .expect_err("reject format");
            assert!(matches!(error, AppError::Validation(_)));
        }

        let error = update(
            &pool,
            UpdateArchiveOutputSettingsInput {
                settings: ArchiveOutputSettings {
                    size: "keep".to_owned(),
                    format: "jpeg".to_owned(),
                    quality: 40,
                },
            },
        )
        .await
        .expect_err("reject quality");
        assert!(matches!(error, AppError::Validation(_)));
    }

    #[tokio::test]
    async fn project_override_wins_over_global() {
        let pool = db::test_pool().await;
        let fixture = crate::services::test_support::project_with_directories(&pool, "size")
            .await
            .expect("fixture");
        update(
            &pool,
            UpdateArchiveOutputSettingsInput {
                settings: ArchiveOutputSettings {
                    size: "1440p".to_owned(),
                    format: "webp".to_owned(),
                    quality: 92,
                },
            },
        )
        .await
        .unwrap();
        let resolved = resolve_spec_for_project(&pool, &fixture.project_id)
            .await
            .unwrap();
        assert_eq!(resolved.size, "1440p");
        assert_eq!(resolved.format, "webp");
        assert_eq!(resolved.quality, 92);

        let view = set_project_capture_policy(
            &pool,
            SetProjectCapturePolicyInput {
                project_id: fixture.project_id.clone(),
                archive_output_size: "native".to_owned(),
                archive_output_format: "jpeg".to_owned(),
                archive_output_quality: Some(88),
            },
        )
        .await
        .unwrap();
        assert_eq!(view.project_size, "native");
        assert_eq!(view.resolved_size, "native");
        assert_eq!(view.resolved_format, "jpeg");
        assert_eq!(view.resolved_quality, 88);
        assert_eq!(view.global_size, "1440p");
        assert_eq!(view.global_format, "webp");
        assert_eq!(view.global_quality, 92);

        let view = set_project_capture_policy(
            &pool,
            SetProjectCapturePolicyInput {
                project_id: fixture.project_id.clone(),
                archive_output_size: "inherit".to_owned(),
                archive_output_format: "inherit".to_owned(),
                archive_output_quality: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(view.project_size, "inherit");
        assert_eq!(view.project_format, "inherit");
        assert_eq!(view.project_quality, None);
        assert_eq!(view.resolved_size, "1440p");
        assert_eq!(view.resolved_format, "webp");
        assert_eq!(view.resolved_quality, 92);
    }
}
