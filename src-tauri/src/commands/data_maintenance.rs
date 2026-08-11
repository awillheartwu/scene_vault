use std::path::{Path, PathBuf};

use tauri::{AppHandle, State};

use crate::{
    db::{self, StartupState},
    error::AppError,
    models::{
        data_maintenance::{
            CreateDatabaseBackupInput, DatabaseBackupResult, DatabaseStartupStatus,
            MaintenanceReport, PreflightReport, RestoreRequest, StageDatabaseRestoreInput,
            BACKUP_MANIFEST_EXTENSION,
        },
        diagnostics::{LogLevel, LogRecord},
    },
    services::{data_maintenance_service::DataMaintenanceService, log_service},
};

fn service(state: &StartupState) -> DataMaintenanceService {
    DataMaintenanceService::new(
        state.database_path.clone(),
        env!("CARGO_PKG_VERSION"),
        db::latest_schema_version(),
    )
}

fn require_normal_mode(state: &StartupState) -> Result<(), AppError> {
    if let Some(error) = &state.database_error {
        return Err(AppError::Conflict(format!(
            "database is in recovery mode: {error}"
        )));
    }
    Ok(())
}

fn manifest_path_for(backup: &Path) -> PathBuf {
    PathBuf::from(format!(
        "{}.{}",
        backup.display(),
        BACKUP_MANIFEST_EXTENSION
    ))
}

#[tauri::command]
pub async fn get_database_startup_status(
    state: State<'_, StartupState>,
) -> Result<DatabaseStartupStatus, AppError> {
    let maintenance = service(&state);
    let pending_restore = maintenance
        .pending_restore(&state.recovery_directory)
        .await?
        .is_some();
    Ok(DatabaseStartupStatus {
        mode: if state.database_error.is_some() {
            "recovery".to_owned()
        } else {
            "normal".to_owned()
        },
        database_path: maintenance.database_path().display().to_string(),
        backup_directory: state.backup_directory.display().to_string(),
        recovery_directory: state.recovery_directory.display().to_string(),
        error_message: state.database_error.clone(),
        pending_restore,
        restored_on_startup: state.restored_on_startup,
    })
}

#[tauri::command]
pub async fn cancel_pending_database_restore(
    state: State<'_, StartupState>,
) -> Result<(), AppError> {
    service(&state)
        .clear_pending_restore(&state.recovery_directory)
        .await?;
    Ok(())
}

#[tauri::command]
pub async fn preflight_database(
    state: State<'_, StartupState>,
) -> Result<PreflightReport, AppError> {
    require_normal_mode(&state)?;
    let report = service(&state).preflight().await?;
    log_service::record_event(LogRecord {
        level: if report.ok {
            LogLevel::Info
        } else {
            LogLevel::Warn
        },
        module: "data.maintenance".to_owned(),
        message: if report.ok {
            "database preflight passed".to_owned()
        } else {
            "database preflight found issues".to_owned()
        },
        event: Some("database_preflight_completed".to_owned()),
        outcome: Some(if report.ok { "succeeded" } else { "warning" }.to_owned()),
        ..Default::default()
    });
    Ok(report)
}

#[tauri::command]
pub async fn create_database_backup(
    state: State<'_, StartupState>,
    input: CreateDatabaseBackupInput,
) -> Result<DatabaseBackupResult, AppError> {
    require_normal_mode(&state)?;
    let destination = PathBuf::from(input.destination.trim());
    if input.destination.trim().is_empty() {
        return Err(AppError::Validation(
            "backup destination cannot be empty".to_owned(),
        ));
    }
    let manifest = service(&state).create_backup(&destination).await?;
    let backup_path = if destination.is_dir() {
        destination.join(&manifest.backup_file)
    } else {
        destination
    };
    let result = DatabaseBackupResult {
        manifest_path: manifest_path_for(&backup_path).display().to_string(),
        backup_path: backup_path.display().to_string(),
        manifest,
    };
    log_service::record_event(LogRecord {
        level: LogLevel::Info,
        module: "data.maintenance".to_owned(),
        message: "database backup created".to_owned(),
        event: Some("database_backup_created".to_owned()),
        outcome: Some("succeeded".to_owned()),
        ..Default::default()
    });
    Ok(result)
}

#[tauri::command]
pub async fn stage_database_restore(
    state: State<'_, StartupState>,
    input: StageDatabaseRestoreInput,
) -> Result<RestoreRequest, AppError> {
    let backup_path = input.backup_path.trim();
    if backup_path.is_empty() {
        return Err(AppError::Validation(
            "backup path cannot be empty".to_owned(),
        ));
    }
    let request = service(&state)
        .stage_restore(PathBuf::from(backup_path), &state.recovery_directory)
        .await?;
    log_service::record_event(LogRecord {
        level: LogLevel::Warn,
        module: "data.maintenance".to_owned(),
        message: "database restore staged for next startup".to_owned(),
        event: Some("database_restore_staged".to_owned()),
        outcome: Some("restart_required".to_owned()),
        ..Default::default()
    });
    Ok(request)
}

#[tauri::command]
pub async fn rebuild_database_indexes(
    state: State<'_, StartupState>,
) -> Result<MaintenanceReport, AppError> {
    require_normal_mode(&state)?;
    let report = service(&state).rebuild_indexes().await?;
    log_service::record_event(LogRecord {
        level: if report.integrity_ok {
            LogLevel::Info
        } else {
            LogLevel::Warn
        },
        module: "data.maintenance".to_owned(),
        message: "database indexes rebuilt and statistics refreshed".to_owned(),
        event: Some("database_indexes_rebuilt".to_owned()),
        outcome: Some(
            if report.integrity_ok {
                "succeeded"
            } else {
                "warning"
            }
            .to_owned(),
        ),
        ..Default::default()
    });
    Ok(report)
}

#[tauri::command]
pub fn restart_after_database_restore(app: AppHandle) {
    app.restart();
}
