use sqlx::SqlitePool;

use crate::{
    error::AppError,
    models::archive_naming::{ArchiveNamingSettings, UpdateArchiveNamingSettingsInput},
    services::archive_naming,
};

const SETTINGS_KEY: &str = "capture.archive_naming";

pub async fn get(pool: &SqlitePool) -> Result<ArchiveNamingSettings, AppError> {
    let value: Option<String> = sqlx::query_scalar("SELECT value_json FROM settings WHERE key = ?")
        .bind(SETTINGS_KEY)
        .fetch_optional(pool)
        .await?;
    match value {
        Some(value) => serde_json::from_str(&value).map_err(|error| {
            AppError::Validation(format!(
                "stored archive naming settings are invalid: {error}"
            ))
        }),
        None => Ok(ArchiveNamingSettings::default()),
    }
}

pub async fn update(
    pool: &SqlitePool,
    input: UpdateArchiveNamingSettingsInput,
) -> Result<ArchiveNamingSettings, AppError> {
    let settings = normalize(input.settings)?;
    let value = serde_json::to_string(&settings).map_err(|error| {
        AppError::Validation(format!("cannot encode archive naming settings: {error}"))
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

fn normalize(mut settings: ArchiveNamingSettings) -> Result<ArchiveNamingSettings, AppError> {
    settings.template = settings.template.trim().to_owned();
    settings.separator = settings.separator.trim().to_owned();
    archive_naming::validate(&settings)?;
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    #[tokio::test]
    async fn defaults_when_unset_and_round_trips_saved_settings() {
        let pool = db::test_pool().await;
        assert_eq!(
            get(&pool).await.expect("defaults"),
            ArchiveNamingSettings::default()
        );

        let settings = ArchiveNamingSettings {
            template: "{date} {source} {id}".to_owned(),
            separator: "_".to_owned(),
        };
        let saved = update(
            &pool,
            UpdateArchiveNamingSettingsInput {
                settings: settings.clone(),
            },
        )
        .await
        .expect("save");
        assert_eq!(saved, settings);
        assert_eq!(get(&pool).await.expect("read"), settings);
    }

    #[tokio::test]
    async fn rejects_unknown_placeholder_and_empty_template() {
        let pool = db::test_pool().await;
        let error = update(
            &pool,
            UpdateArchiveNamingSettingsInput {
                settings: ArchiveNamingSettings {
                    template: "{source} - {owner}".to_owned(),
                    separator: " - ".to_owned(),
                },
            },
        )
        .await
        .expect_err("should reject unknown placeholder");
        assert!(matches!(error, AppError::Validation(_)));

        let error = update(
            &pool,
            UpdateArchiveNamingSettingsInput {
                settings: ArchiveNamingSettings {
                    template: "  ".to_owned(),
                    separator: " - ".to_owned(),
                },
            },
        )
        .await
        .expect_err("should reject empty template");
        assert!(matches!(error, AppError::Validation(_)));
    }
}
