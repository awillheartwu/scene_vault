use sqlx::SqlitePool;

use crate::{error::AppError, models::diagnostics::LogPolicySettings};

const SETTINGS_KEY: &str = "logging.policy";

pub async fn get(pool: &SqlitePool) -> Result<LogPolicySettings, AppError> {
    let value: Option<String> = sqlx::query_scalar("SELECT value_json FROM settings WHERE key = ?")
        .bind(SETTINGS_KEY)
        .fetch_optional(pool)
        .await?;
    match value {
        Some(value) => normalize(serde_json::from_str(&value).map_err(|error| {
            AppError::Validation(format!("stored log settings are invalid: {error}"))
        })?),
        None => Ok(LogPolicySettings::default()),
    }
}

pub async fn update(
    pool: &SqlitePool,
    settings: LogPolicySettings,
) -> Result<LogPolicySettings, AppError> {
    let settings = normalize(settings)?;
    let value = serde_json::to_string(&settings)
        .map_err(|error| AppError::Validation(format!("cannot encode log settings: {error}")))?;
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

fn normalize(settings: LogPolicySettings) -> Result<LogPolicySettings, AppError> {
    if !(1..=365).contains(&settings.retention_days) {
        return Err(AppError::Validation(
            "log retention days must be between 1 and 365".to_owned(),
        ));
    }
    if !(1..=100).contains(&settings.max_file_size_mb) {
        return Err(AppError::Validation(
            "log file size must be between 1 and 100 MiB".to_owned(),
        ));
    }
    if !(1..=200).contains(&settings.max_archived_files) {
        return Err(AppError::Validation(
            "log archive count must be between 1 and 200".to_owned(),
        ));
    }
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    #[tokio::test]
    async fn defaults_round_trip_and_invalid_values_are_rejected() {
        let pool = db::test_pool().await;
        assert_eq!(get(&pool).await.unwrap(), LogPolicySettings::default());
        let settings = LogPolicySettings {
            retention_days: 30,
            max_file_size_mb: 10,
            max_archived_files: 40,
            automatic_cleanup: false,
        };
        assert_eq!(update(&pool, settings.clone()).await.unwrap(), settings);
        assert_eq!(get(&pool).await.unwrap(), settings);
        assert!(update(
            &pool,
            LogPolicySettings {
                retention_days: 0,
                ..Default::default()
            }
        )
        .await
        .is_err());
    }
}
