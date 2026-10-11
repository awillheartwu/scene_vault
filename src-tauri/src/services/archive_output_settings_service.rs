use sqlx::SqlitePool;

use crate::{
    error::AppError,
    models::engine::{
        is_archive_output_size, ArchiveOutputSettings, UpdateArchiveOutputSettingsInput,
        ARCHIVE_OUTPUT_INHERIT,
    },
};

const SETTINGS_KEY: &str = "archive.output_size";

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
            "归档输出尺寸只能是 keep / 1080p / 1440p / 2160p".to_owned(),
        ));
    }
    Ok(settings)
}

/// Effective size for one project: an explicit project override wins,
/// otherwise the global default applies.
pub async fn resolve_for_project(pool: &SqlitePool, project_id: &str) -> Result<String, AppError> {
    let project_value: Option<String> =
        sqlx::query_scalar("SELECT archive_output_size FROM projects WHERE id = ?")
            .bind(project_id)
            .fetch_optional(pool)
            .await?;
    if let Some(value) = project_value {
        let value = value.trim().to_lowercase();
        if value != ARCHIVE_OUTPUT_INHERIT && is_archive_output_size(&value) {
            return Ok(value);
        }
    }
    Ok(get(pool).await?.size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    #[tokio::test]
    async fn defaults_to_keep_and_round_trips() {
        let pool = db::test_pool().await;
        assert_eq!(get(&pool).await.expect("defaults").size, "keep");
        let saved = update(
            &pool,
            UpdateArchiveOutputSettingsInput {
                settings: ArchiveOutputSettings {
                    size: "1080p".to_owned(),
                },
            },
        )
        .await
        .expect("save");
        assert_eq!(saved.size, "1080p");
        assert_eq!(get(&pool).await.expect("read").size, "1080p");
    }

    #[tokio::test]
    async fn rejects_unknown_sizes() {
        let pool = db::test_pool().await;
        let error = update(
            &pool,
            UpdateArchiveOutputSettingsInput {
                settings: ArchiveOutputSettings {
                    size: "720p".to_owned(),
                },
            },
        )
        .await
        .expect_err("reject");
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
                },
            },
        )
        .await
        .unwrap();
        assert_eq!(
            resolve_for_project(&pool, &fixture.project_id)
                .await
                .unwrap(),
            "1440p"
        );
        sqlx::query("UPDATE projects SET archive_output_size = '1080p' WHERE id = ?")
            .bind(&fixture.project_id)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            resolve_for_project(&pool, &fixture.project_id)
                .await
                .unwrap(),
            "1080p"
        );
    }
}
