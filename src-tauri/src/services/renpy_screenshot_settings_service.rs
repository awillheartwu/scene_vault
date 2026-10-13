use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use serde::Serialize;
use sqlx::{SqliteConnection, SqlitePool};
use tauri::{AppHandle, Manager};

use crate::{
    error::AppError,
    models::engine::{RenpyScreenshotPolicyView, RenpyScreenshotSettings},
    services::{log_service, renpy_bridge_service},
};

const SETTINGS_KEY: &str = "renpy.screenshot";
static POLICY_MUTATION_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

// Install/uninstall share this lock when updating the policy's installation map.
pub(crate) async fn lock() -> tokio::sync::MutexGuard<'static, ()> {
    POLICY_MUTATION_LOCK.lock().await
}

pub fn policy_path(app: &AppHandle) -> Result<PathBuf, AppError> {
    // Keep the live policy outside the disposable capture-output directory.
    Ok(app
        .path()
        .app_cache_dir()?
        .join("renpy-screenshot-settings.json"))
}

fn decode_settings(value: Option<String>) -> Result<RenpyScreenshotSettings, AppError> {
    match value {
        Some(value) => serde_json::from_str(&value).map_err(|error| {
            AppError::Validation(format!(
                "stored Ren'Py screenshot settings are invalid: {error}"
            ))
        }),
        None => Ok(RenpyScreenshotSettings::default()),
    }
}

pub async fn get(pool: &SqlitePool) -> Result<RenpyScreenshotSettings, AppError> {
    let value = sqlx::query_scalar("SELECT value_json FROM settings WHERE key = ?")
        .bind(SETTINGS_KEY)
        .fetch_optional(pool)
        .await?;
    decode_settings(value)
}

pub async fn project_policy(
    pool: &SqlitePool,
    project_id: &str,
) -> Result<RenpyScreenshotPolicyView, AppError> {
    let project: Option<Option<bool>> =
        sqlx::query_scalar("SELECT renpy_auto_hide_ui FROM projects WHERE id = ?")
            .bind(project_id.trim())
            .fetch_optional(pool)
            .await?;
    let project_auto_hide_ui = project.ok_or_else(|| AppError::NotFound("project".to_owned()))?;
    let global_auto_hide_ui = get(pool).await?.auto_hide_ui;
    Ok(RenpyScreenshotPolicyView {
        project_auto_hide_ui,
        global_auto_hide_ui,
        resolved_auto_hide_ui: project_auto_hide_ui.unwrap_or(global_auto_hide_ui),
    })
}

pub async fn resolve(
    pool: &SqlitePool,
    project_id: &str,
    installation_override: Option<bool>,
) -> Result<bool, AppError> {
    Ok(installation_override.unwrap_or(
        project_policy(pool, project_id)
            .await?
            .resolved_auto_hide_ui,
    ))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PolicySnapshot {
    schema_version: u8,
    projects: BTreeMap<String, bool>,
    installations: BTreeMap<String, bool>,
}

async fn snapshot(connection: &mut SqliteConnection) -> Result<Vec<u8>, AppError> {
    let value = sqlx::query_scalar("SELECT value_json FROM settings WHERE key = ?")
        .bind(SETTINGS_KEY)
        .fetch_optional(&mut *connection)
        .await?;
    let global = decode_settings(value)?.auto_hide_ui;
    let rows: Vec<(String, Option<bool>)> =
        sqlx::query_as("SELECT id, renpy_auto_hide_ui FROM projects")
            .fetch_all(&mut *connection)
            .await?;
    let projects: BTreeMap<String, bool> = rows
        .into_iter()
        .map(|(id, value)| (id, value.unwrap_or(global)))
        .collect();
    let rows: Vec<(String, String, Option<bool>)> = sqlx::query_as(
        "SELECT id, project_id, auto_hide_ui_override FROM project_engine_integrations",
    )
    .fetch_all(&mut *connection)
    .await?;
    let installations = rows
        .into_iter()
        .map(|(id, project, value)| {
            (
                id,
                value.unwrap_or(*projects.get(&project).unwrap_or(&global)),
            )
        })
        .collect();
    serde_json::to_vec(&PolicySnapshot {
        schema_version: 1,
        projects,
        installations,
    })
    .map_err(|error| AppError::Validation(format!("cannot encode screenshot policy: {error}")))
}

pub(crate) struct PolicyFileBackup {
    path: PathBuf,
    bytes: Option<Vec<u8>>,
}

pub(crate) async fn stage_snapshot(
    connection: &mut SqliteConnection,
    path: &Path,
) -> Result<PolicyFileBackup, AppError> {
    let bytes = snapshot(connection).await?;
    let previous = match tokio::fs::read(path).await {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    renpy_bridge_service::write_atomic(path, &bytes).await?;
    Ok(PolicyFileBackup {
        path: path.to_path_buf(),
        bytes: previous,
    })
}

pub(crate) async fn restore_snapshot(backup: PolicyFileBackup) -> Result<(), AppError> {
    match backup.bytes {
        Some(bytes) => renpy_bridge_service::write_atomic(&backup.path, &bytes).await,
        None => match tokio::fs::remove_file(&backup.path).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        },
    }
}

pub async fn publish(pool: &SqlitePool, app: &AppHandle) -> Result<(), AppError> {
    let _guard = lock().await;
    publish_locked(pool, app).await
}

pub(crate) async fn publish_locked(pool: &SqlitePool, app: &AppHandle) -> Result<(), AppError> {
    let mut connection = pool.acquire().await?;
    let bytes = snapshot(&mut connection).await?;
    let path = policy_path(app)?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    renpy_bridge_service::write_atomic(&path, &bytes).await
}

enum PolicyUpdate<'a> {
    Global(bool),
    Project(&'a str, Option<bool>),
    Installation(&'a str, &'a str, Option<bool>),
}

// A save is successful only once both SQLite and the bridge's local policy
// agree. A file-write failure rolls the transaction back; a commit failure
// restores the old file instead of leaving an unpersisted choice active.
async fn apply(pool: &SqlitePool, path: &Path, update: PolicyUpdate<'_>) -> Result<(), AppError> {
    let _guard = lock().await;
    let mut tx = pool.begin().await?;
    match update {
        PolicyUpdate::Global(auto_hide_ui) => {
            let value = serde_json::to_string(&RenpyScreenshotSettings { auto_hide_ui })
                .map_err(|error| AppError::Validation(error.to_string()))?;
            sqlx::query("INSERT INTO settings (key, value_json) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')")
                .bind(SETTINGS_KEY).bind(value).execute(&mut *tx).await?;
        }
        PolicyUpdate::Project(project, auto_hide_ui) => {
            let changed = sqlx::query("UPDATE projects SET renpy_auto_hide_ui = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?")
                .bind(auto_hide_ui).bind(project.trim()).execute(&mut *tx).await?;
            if changed.rows_affected() == 0 {
                return Err(AppError::NotFound("project".to_owned()));
            }
        }
        PolicyUpdate::Installation(project, installation, auto_hide_ui) => {
            let changed = sqlx::query("UPDATE project_engine_integrations SET auto_hide_ui_override = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ? AND project_id = ?")
                .bind(auto_hide_ui).bind(installation.trim()).bind(project.trim()).execute(&mut *tx).await?;
            if changed.rows_affected() == 0 {
                return Err(AppError::NotFound(
                    "engine integration in project".to_owned(),
                ));
            }
        }
    }
    let backup = stage_snapshot(&mut tx, path).await?;
    if let Err(error) = tx.commit().await {
        let restored = restore_snapshot(backup).await;
        if let Err(restore_error) = restored {
            log_service::error(
                "engine.screenshot",
                format!("screenshot policy rollback failed: {restore_error}"),
            );
            return Err(AppError::Validation(format!(
                "截图设置提交失败（{error}），缓存恢复也失败（{restore_error}）；请重新保存设置"
            )));
        }
        return Err(error.into());
    }
    Ok(())
}

pub async fn update(
    pool: &SqlitePool,
    app: &AppHandle,
    settings: RenpyScreenshotSettings,
) -> Result<RenpyScreenshotSettings, AppError> {
    apply(
        pool,
        &policy_path(app)?,
        PolicyUpdate::Global(settings.auto_hide_ui),
    )
    .await?;
    Ok(settings)
}

pub async fn set_project(
    pool: &SqlitePool,
    app: &AppHandle,
    project: &str,
    value: Option<bool>,
) -> Result<RenpyScreenshotPolicyView, AppError> {
    apply(
        pool,
        &policy_path(app)?,
        PolicyUpdate::Project(project, value),
    )
    .await?;
    project_policy(pool, project).await
}

pub async fn set_installation(
    pool: &SqlitePool,
    app: &AppHandle,
    project: &str,
    installation: &str,
    value: Option<bool>,
) -> Result<(), AppError> {
    apply(
        pool,
        &policy_path(app)?,
        PolicyUpdate::Installation(project, installation, value),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn seed(pool: &SqlitePool) -> (String, String, String) {
        let project = uuid::Uuid::new_v4().to_string();
        let other = uuid::Uuid::new_v4().to_string();
        let installation = uuid::Uuid::new_v4().to_string();
        for id in [&project, &other] {
            sqlx::query("INSERT INTO projects (id, name) VALUES (?, 'game')")
                .bind(id)
                .execute(pool)
                .await
                .unwrap();
        }
        sqlx::query("INSERT INTO project_engine_integrations (id, project_id, engine, engine_root, game_directory, bridge_version, auto_hide_ui, auto_hide_ui_override, installed_at, updated_at) VALUES (?, ?, 'renpy', '/game', '/game/game', '1.1.6', 0, 1, '2026-10-13T00:00:00Z', '2026-10-13T00:00:00Z')")
            .bind(&installation).bind(&project).execute(pool).await.unwrap();
        (project, other, installation)
    }

    #[tokio::test]
    async fn live_snapshot_resolves_overrides_and_reset_to_inherit() {
        let pool = crate::db::test_pool().await;
        let (project, other, installation) = seed(&pool).await;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("policy.json");
        assert!(
            !project_policy(&pool, &project)
                .await
                .unwrap()
                .resolved_auto_hide_ui
        );
        apply(&pool, &path, PolicyUpdate::Global(true))
            .await
            .unwrap();
        assert!(
            project_policy(&pool, &project)
                .await
                .unwrap()
                .resolved_auto_hide_ui
        );
        apply(&pool, &path, PolicyUpdate::Project(&project, Some(false)))
            .await
            .unwrap();
        let view = project_policy(&pool, &project).await.unwrap();
        assert_eq!(view.project_auto_hide_ui, Some(false));
        assert!(view.global_auto_hide_ui);
        assert!(!view.resolved_auto_hide_ui);
        let before = tokio::fs::read(&path).await.unwrap();
        assert!(apply(
            &pool,
            &path,
            PolicyUpdate::Installation(&other, &installation, None)
        )
        .await
        .is_err());
        assert_eq!(tokio::fs::read(&path).await.unwrap(), before);
        let snapshot: serde_json::Value = serde_json::from_slice(&before).unwrap();
        assert_eq!(snapshot["installations"][&installation], true);
        apply(
            &pool,
            &path,
            PolicyUpdate::Installation(&project, &installation, None),
        )
        .await
        .unwrap();
        let snapshot: serde_json::Value =
            serde_json::from_slice(&tokio::fs::read(&path).await.unwrap()).unwrap();
        assert_eq!(snapshot["installations"][&installation], false);
        apply(&pool, &path, PolicyUpdate::Project(&project, None))
            .await
            .unwrap();
        let snapshot: serde_json::Value =
            serde_json::from_slice(&tokio::fs::read(&path).await.unwrap()).unwrap();
        assert_eq!(snapshot["installations"][&installation], true);
    }

    #[tokio::test]
    async fn cache_failure_does_not_commit_an_unpublished_setting() {
        let pool = crate::db::test_pool().await;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("policy.json");
        apply(&pool, &path, PolicyUpdate::Global(false))
            .await
            .unwrap();
        let previous = tokio::fs::read(&path).await.unwrap();
        let blocked = directory.path().join("blocked");
        tokio::fs::write(&blocked, b"regular file").await.unwrap();
        assert!(apply(
            &pool,
            &blocked.join("policy.json"),
            PolicyUpdate::Global(true)
        )
        .await
        .is_err());
        assert!(!get(&pool).await.unwrap().auto_hide_ui);
        assert_eq!(tokio::fs::read(&path).await.unwrap(), previous);
    }
}
