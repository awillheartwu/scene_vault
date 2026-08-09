use sqlx::SqlitePool;

use crate::{
    error::AppError,
    models::app_settings::{AppSettings, UpdateAppSettingsInput},
};

const SETTINGS_KEY: &str = "app.general";

pub async fn get(pool: &SqlitePool) -> Result<AppSettings, AppError> {
    let value: Option<String> = sqlx::query_scalar("SELECT value_json FROM settings WHERE key = ?")
        .bind(SETTINGS_KEY)
        .fetch_optional(pool)
        .await?;
    match value {
        Some(value) => {
            let settings = serde_json::from_str(&value).map_err(|error| {
                AppError::Validation(format!("stored app settings are invalid: {error}"))
            })?;
            normalize(settings)
        }
        None => Ok(AppSettings::default()),
    }
}

pub async fn update(
    pool: &SqlitePool,
    input: UpdateAppSettingsInput,
) -> Result<AppSettings, AppError> {
    let settings = normalize(input.settings)?;
    let value = serde_json::to_string(&settings)
        .map_err(|error| AppError::Validation(format!("cannot encode app settings: {error}")))?;
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

fn normalize(mut settings: AppSettings) -> Result<AppSettings, AppError> {
    settings.classify_shortcut = normalize_shortcut(&settings.classify_shortcut, "classify")?;
    settings.note_shortcut = normalize_shortcut(&settings.note_shortcut, "note")?;
    settings.thumbnail_cache_size_mb = settings.thumbnail_cache_size_mb.clamp(32, 4096);
    // Thumbnail decoding is intentionally serial. Lazy viewport loading keeps
    // the queue small, and one decoder prevents large imported screenshots
    // from creating another CPU/memory spike.
    settings.thumbnail_generation_concurrency = 1;
    Ok(settings)
}

fn normalize_shortcut(value: &str, label: &str) -> Result<String, AppError> {
    let value = value.trim().to_owned();
    if value.is_empty() {
        return Err(AppError::Validation(format!(
            "{label} shortcut cannot be empty"
        )));
    }
    // Validate the string parses as a global shortcut before persisting.
    value
        .parse::<tauri_plugin_global_shortcut::Shortcut>()
        .map_err(|error| AppError::Validation(format!("{label} shortcut is invalid: {error}")))?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    #[tokio::test]
    async fn defaults_when_unset_and_round_trips_saved_settings() {
        let pool = db::test_pool().await;
        assert_eq!(get(&pool).await.expect("defaults"), AppSettings::default());

        let settings = AppSettings {
            classify_shortcut: "Ctrl+Alt+C".to_owned(),
            show_private_by_default: true,
            auto_save_notes: false,
            split_popup_windows: true,
            auto_close_empty_popup: true,
            ..AppSettings::default()
        };
        let saved = update(
            &pool,
            UpdateAppSettingsInput {
                settings: settings.clone(),
            },
        )
        .await
        .expect("save");
        assert_eq!(saved, settings);
        assert_eq!(get(&pool).await.expect("read"), settings);
    }

    #[tokio::test]
    async fn rejects_invalid_shortcut_strings() {
        let pool = db::test_pool().await;
        let settings = AppSettings {
            classify_shortcut: "not a shortcut".to_owned(),
            ..AppSettings::default()
        };
        let error = update(&pool, UpdateAppSettingsInput { settings })
            .await
            .expect_err("should reject");
        assert!(matches!(error, AppError::Validation(_)));
    }

    #[tokio::test]
    async fn forces_serial_thumbnail_generation() {
        let pool = db::test_pool().await;
        let settings = AppSettings {
            thumbnail_generation_concurrency: 16,
            ..AppSettings::default()
        };
        let saved = update(&pool, UpdateAppSettingsInput { settings })
            .await
            .expect("save capped settings");
        assert_eq!(saved.thumbnail_generation_concurrency, 1);
        assert_eq!(
            get(&pool)
                .await
                .expect("read capped settings")
                .thumbnail_generation_concurrency,
            1
        );
    }
}
