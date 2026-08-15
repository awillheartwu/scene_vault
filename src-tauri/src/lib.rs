mod commands;
mod db;
mod error;
mod models;
mod services;

use db::AppState;
use services::{
    data_maintenance_service::DataMaintenanceService,
    shortcuts::{CLASSIFY_POPUP_LABEL, NOTE_POPUP_LABEL},
};
use tauri::Manager;

async fn prune_automatic_backups(
    backup_directory: &std::path::Path,
) -> Result<(), error::AppError> {
    let mut entries = tokio::fs::read_dir(backup_directory).await?;
    let mut backups = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("scene-vault-backup-") && name.ends_with(".sqlite") {
            backups.push((name, entry.path()));
        }
    }
    backups.sort_by(|left, right| right.0.cmp(&left.0));
    for (_, path) in backups.into_iter().skip(3) {
        let _ = tokio::fs::remove_file(&path).await;
        let manifest = std::path::PathBuf::from(format!(
            "{}.{}",
            path.display(),
            models::data_maintenance::BACKUP_MANIFEST_EXTENSION
        ));
        let _ = tokio::fs::remove_file(manifest).await;
    }
    Ok(())
}

async fn open_and_prepare_database(
    database_path: &std::path::Path,
    backup_directory: &std::path::Path,
) -> Result<(sqlx::SqlitePool, u64), error::AppError> {
    let database_existed = database_path
        .metadata()
        .is_ok_and(|metadata| metadata.len() > 0);
    let pool = db::connect(database_path, true).await?;
    let latest = db::latest_schema_version();
    let migrations_table_exists: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations'",
    )
    .fetch_one(&pool)
    .await?;
    let applied_version = if migrations_table_exists > 0 {
        sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(version) FROM _sqlx_migrations")
            .fetch_one(&pool)
            .await?
            .unwrap_or_default()
    } else {
        0
    };

    if database_existed && applied_version < latest {
        tokio::fs::create_dir_all(backup_directory).await?;
        let maintenance = DataMaintenanceService::new(
            database_path.to_path_buf(),
            env!("CARGO_PKG_VERSION"),
            latest,
        );
        if let Err(error) = maintenance.create_backup(backup_directory).await {
            pool.close().await;
            return Err(error.into());
        }
        prune_automatic_backups(backup_directory).await?;
    }

    if let Err(error) = db::migrate(&pool).await {
        pool.close().await;
        return Err(error);
    }
    let maintenance = DataMaintenanceService::new(
        database_path.to_path_buf(),
        env!("CARGO_PKG_VERSION"),
        latest,
    );
    let report = maintenance.preflight().await?;
    if !report.ok {
        pool.close().await;
        return Err(error::AppError::Conflict(format!(
            "database preflight failed: {}",
            report
                .quick_check
                .into_iter()
                .chain(report.foreign_key_issues)
                .chain(report.migration_issues)
                .collect::<Vec<_>>()
                .join("; ")
        )));
    }
    let recovered = services::capture_service::recover_interrupted_processing(&pool).await?;
    Ok((pool, recovered))
}

type DatabaseStartupResult = (sqlx::SqlitePool, u64, Option<String>, bool);

/// Applies a staged restore before opening the live pool, then runs migrations
/// and preflight. If the restored database cannot be prepared, the pre-restore
/// snapshot is put back and opened once more. Keeping this orchestration out of
/// the Tauri setup closure makes the startup ordering directly testable.
async fn initialize_database_startup(
    database_path: &std::path::Path,
    backup_directory: &std::path::Path,
    recovery_directory: &std::path::Path,
    latest_schema_version: i64,
) -> Result<DatabaseStartupResult, error::AppError> {
    let maintenance = DataMaintenanceService::new(
        database_path.to_path_buf(),
        env!("CARGO_PKG_VERSION"),
        latest_schema_version,
    );
    let mut restore_was_applied = false;
    if maintenance.has_pending_restore(recovery_directory).await {
        match maintenance.apply_pending_restore(recovery_directory).await {
            Ok(_) => restore_was_applied = true,
            Err(error) => services::log_service::error(
                "data.maintenance",
                format!("staged database restore failed: {error}"),
            ),
        }
    }

    match open_and_prepare_database(database_path, backup_directory).await {
        Ok((pool, recovered)) => {
            if restore_was_applied {
                maintenance
                    .discard_pre_restore_snapshot(recovery_directory)
                    .await?;
            }
            Ok((pool, recovered, None, restore_was_applied))
        }
        Err(restore_error) if restore_was_applied => {
            maintenance.rollback_restore(recovery_directory).await?;
            match open_and_prepare_database(database_path, backup_directory).await {
                Ok((pool, recovered)) => {
                    services::log_service::error(
                        "data.maintenance",
                        format!(
                            "restored database was rejected; previous database recovered: {restore_error}"
                        ),
                    );
                    Ok((pool, recovered, None, false))
                }
                Err(rollback_error) => {
                    let fallback = db::recovery_pool().await?;
                    Ok((
                        fallback,
                        0,
                        Some(format!(
                            "restored database failed ({restore_error}); previous database could not reopen ({rollback_error})"
                        )),
                        false,
                    ))
                }
            }
        }
        Err(error) => {
            let fallback = db::recovery_pool().await?;
            Ok((fallback, 0, Some(error.to_string()), false))
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .on_window_event(|window, event| {
            // The classify/note popups are separate always-on-top windows;
            // closing the main window must close them too, otherwise the app
            // keeps running invisibly in the tray-less background with a
            // popup left on screen.
            if window.label() == "main"
                && matches!(event, tauri::WindowEvent::CloseRequested { .. })
            {
                let handle = window.app_handle();
                for label in [CLASSIFY_POPUP_LABEL, NOTE_POPUP_LABEL] {
                    if let Some(popup) = handle.get_webview_window(label) {
                        let _ = popup.close();
                    }
                }
            }
        })
        .setup(|app| {
            let log_directory = app.path().app_log_dir()?;
            if let Err(error) = services::log_service::initialize(log_directory) {
                eprintln!("[ERROR] [app.startup] could not initialize logging: {error}");
            }
            services::log_service::info(
                "app.startup",
                format!("Scene Vault {} is starting", env!("CARGO_PKG_VERSION")),
            );
            let app_handle = app.handle().clone();
            let cache_root = app.path().app_cache_dir()?.join("capture-output");
            let database_path = db::database_path(app.handle())?;
            let app_data_directory = app.path().app_data_dir()?;
            let backup_directory = app_data_directory.join("backups");
            let recovery_directory = app_data_directory.join("recovery");
            std::fs::create_dir_all(&backup_directory)?;
            std::fs::create_dir_all(&recovery_directory)?;
            let latest_schema_version = db::latest_schema_version();
            let startup = tauri::async_runtime::block_on(initialize_database_startup(
                &database_path,
                &backup_directory,
                &recovery_directory,
                latest_schema_version,
            ))?;
            let (pool, recovered_items, database_error, restored_on_startup) = startup;
            let normal_mode = database_error.is_none();
            app.manage(AppState { pool: pool.clone() });
            app.manage(db::StartupState {
                database_path,
                backup_directory,
                recovery_directory,
                database_error: database_error.clone(),
                restored_on_startup,
            });
            if !normal_mode {
                services::log_service::error(
                    "data.maintenance",
                    format!(
                        "database unavailable; entering recovery mode: {}",
                        database_error
                            .as_deref()
                            .unwrap_or("unknown database error")
                    ),
                );
                return Ok(());
            }
            {
                let settings =
                    tauri::async_runtime::block_on(services::log_settings_service::get(&pool))?;
                services::log_service::configure(&settings)?;
            }
            if recovered_items > 0 {
                services::log_service::record_event(models::diagnostics::LogRecord {
                    level: models::diagnostics::LogLevel::Warn,
                    module: "capture.recovery".to_owned(),
                    message: format!("marked {recovered_items} interrupted captures as failed"),
                    event: Some("startup_recovery_completed".to_owned()),
                    outcome: Some("recovered".to_owned()),
                    ..Default::default()
                });
            }
            {
                let labels: Vec<String> = app.webview_windows().keys().cloned().collect();
                services::log_service::debug(
                    "window.startup",
                    format!("webview windows at startup: {labels:?}"),
                );
            }
            {
                let app_settings =
                    tauri::async_runtime::block_on(services::app_settings_service::get(&pool))?;
                services::shortcuts::register(app.handle(), &app_settings)?;
                services::vision_worker_service::set_image_processing_core_limit(
                    app_settings.image_processing_core_limit,
                );
            }
            {
                // Caches live in the Local app directory; earlier builds kept
                // thumbnails next to the database, so migrate them once.
                services::thumbnail_service::migrate_legacy_cache(
                    &app.path().app_data_dir()?,
                    &app.path().app_local_data_dir()?,
                );
                let cache_root = app.path().app_local_data_dir()?;
                let app_settings =
                    tauri::async_runtime::block_on(services::app_settings_service::get(&pool))?;
                services::thumbnail_service::enforce_cache_limit(
                    &services::thumbnail_service::thumbnail_cache_dir(&cache_root),
                    (app_settings.thumbnail_cache_size_mb as u64).saturating_mul(1024 * 1024),
                );
            }
            {
                // AI-enabled installs ship a PyInstaller sidecar plus bundled
                // models and the annotation font. Fill the missing settings
                // once so the engine works without any manual configuration.
                if let Err(error) = tauri::async_runtime::block_on(
                    services::vision_settings_service::autoconfigure_bundled(
                        &pool,
                        &app.path().app_local_data_dir()?,
                    ),
                ) {
                    services::log_service::error(
                        "vision.settings",
                        format!("bundled vision autoconfigure failed: {error}"),
                    );
                }
            }
            tauri::async_runtime::spawn(
                services::capture_discovery_service::run_background_polling(
                    pool.clone(),
                    app_handle.clone(),
                ),
            );
            services::notes_service::run_background(pool.clone());
            tauri::async_runtime::spawn(services::capture_worker_service::run(
                pool, app_handle, cache_root,
            ));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::data_maintenance::get_database_startup_status,
            commands::data_maintenance::preflight_database,
            commands::data_maintenance::create_database_backup,
            commands::data_maintenance::stage_database_restore,
            commands::data_maintenance::cancel_pending_database_restore,
            commands::data_maintenance::rebuild_database_indexes,
            commands::data_maintenance::restart_after_database_restore,
            commands::project::create_project,
            commands::project::list_projects,
            commands::project::list_project_overviews,
            commands::project::rename_project,
            commands::project::preview_project_deletion,
            commands::project::delete_project,
            commands::project::list_project_source_directories,
            commands::project::add_project_source_directory,
            commands::project::remove_project_source_directory,
            commands::project::set_project_source_directory_enabled,
            commands::project::set_project_destination_directory,
            commands::project::set_project_cover,
            commands::asset::create_asset,
            commands::character::create_character,
            commands::character::list_characters,
            commands::character::rename_character,
            commands::character::merge_characters,
            commands::character::set_character_avatar,
            commands::character::list_project_character_summaries,
            commands::capture::start_capture_session,
            commands::capture::list_capture_sessions,
            commands::capture::list_project_recent_captures,
            commands::capture::end_capture_session,
            commands::capture::list_capture_items,
            commands::capture::list_capture_history,
            commands::capture::discover_captures,
            commands::capture::register_capture,
            commands::capture::label_capture,
            commands::capture::relabel_capture_item,
            commands::capture::retry_capture,
            commands::capture::retry_degraded_captures,
            commands::capture::list_category_items,
            commands::capture::list_unimported_captures,
            commands::capture::import_directory_captures,
            commands::capture::deferred_import_recognition_count,
            commands::capture::start_imported_recognition,
            commands::capture::mark_capture_processing,
            commands::capture::complete_capture_processing,
            commands::capture::mark_capture_failed,
            commands::capture::archive_capture,
            commands::capture::read_capture_image,
            commands::capture::read_capture_thumbnail,
            commands::capture::thumbnail_cache_status,
            commands::capture::classify_popup_context,
            commands::notes::get_project_note,
            commands::notes::update_project_note,
            commands::notes::open_project_note,
            commands::notes::reveal_project_note,
            commands::recognition::set_recognition_suggestion,
            commands::recognition::review_recognition_suggestion,
            commands::recognition::accept_recognition_suggestion,
            commands::recognition::reject_suggestion_and_enroll,
            commands::recognition::batch_reject_and_enroll,
            commands::recognition::list_character_capture_items,
            commands::recognition::list_character_face_samples,
            commands::recognition::set_face_sample_status,
            commands::recognition::set_face_sample_flagged,
            commands::recognition::verify_capture_identity,
            commands::recognition::suggest_for_capture,
            commands::recognition::rebuild_face_bank,
            commands::recognition::refresh_capture_face_feature,
            commands::recognition::get_face_bank_model_status,
            commands::recognition::get_recognition_defaults,
            commands::vision::get_vision_settings,
            commands::vision::update_vision_settings,
            commands::vision::check_vision_engine,
            commands::vision::get_capture_runtime_status,
            commands::vision::list_bundled_fonts,
            commands::settings::get_app_settings,
            commands::settings::update_app_settings,
            commands::settings::get_archive_naming_settings,
            commands::settings::update_archive_naming_settings,
            commands::settings::get_recognition_settings,
            commands::settings::update_recognition_settings,
            commands::settings::get_processing_settings,
            commands::settings::update_processing_settings,
            commands::diagnostics::list_debug_logs,
            commands::diagnostics::get_log_settings,
            commands::diagnostics::update_log_settings,
            commands::diagnostics::record_client_event,
            commands::diagnostics::get_log_status,
            commands::diagnostics::cleanup_debug_logs,
            commands::diagnostics::get_diagnostic_summary,
            commands::diagnostics::get_process_resource_status,
            commands::diagnostics::get_storage_resource_status,
            commands::diagnostics::cleanup_resource,
            commands::system::open_directory,
            commands::window::set_window_always_on_top,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Scene Vault");
    app.run(|_, event| {
        if matches!(
            event,
            tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
        ) {
            tauri::async_runtime::block_on(services::vision_worker_service::shutdown());
        }
    });
}

#[cfg(test)]
mod startup_tests {
    use super::*;

    async fn create_probe_database(path: &std::path::Path, value: &str) {
        let pool = db::connect(path, true)
            .await
            .expect("connect probe database");
        db::migrate(&pool).await.expect("migrate probe database");
        sqlx::query("CREATE TABLE startup_probe (value TEXT NOT NULL)")
            .execute(&pool)
            .await
            .expect("create startup probe");
        sqlx::query("INSERT INTO startup_probe (value) VALUES (?)")
            .bind(value)
            .execute(&pool)
            .await
            .expect("seed startup probe");
        pool.close().await;
    }

    async fn probe_value(pool: &sqlx::SqlitePool) -> String {
        sqlx::query_scalar("SELECT value FROM startup_probe")
            .fetch_one(pool)
            .await
            .expect("read startup probe")
    }

    async fn stage_database(
        live_path: &std::path::Path,
        source_path: &std::path::Path,
        workspace: &std::path::Path,
    ) -> std::path::PathBuf {
        let backups = workspace.join("source-backups");
        let recovery = workspace.join("recovery");
        tokio::fs::create_dir_all(&backups)
            .await
            .expect("create backups");
        tokio::fs::create_dir_all(&recovery)
            .await
            .expect("create recovery");
        let source = DataMaintenanceService::new(
            source_path.to_path_buf(),
            env!("CARGO_PKG_VERSION"),
            db::latest_schema_version(),
        );
        source
            .create_backup(&backups)
            .await
            .expect("create source backup");
        let mut entries = tokio::fs::read_dir(&backups).await.expect("read backups");
        let mut backup = None;
        while let Some(entry) = entries.next_entry().await.expect("read backup entry") {
            if entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "sqlite")
            {
                backup = Some(entry.path());
                break;
            }
        }
        DataMaintenanceService::new(
            live_path.to_path_buf(),
            env!("CARGO_PKG_VERSION"),
            db::latest_schema_version(),
        )
        .stage_restore(backup.expect("backup file"), &recovery)
        .await
        .expect("stage source backup");
        recovery
    }

    #[tokio::test]
    async fn startup_applies_pending_restore_before_opening_the_live_pool() {
        let workspace = tempfile::tempdir().expect("workspace");
        let live_path = workspace.path().join("live.db");
        let source_path = workspace.path().join("source.db");
        let startup_backups = workspace.path().join("startup-backups");
        create_probe_database(&live_path, "old").await;
        create_probe_database(&source_path, "restored").await;
        let recovery = stage_database(&live_path, &source_path, workspace.path()).await;

        let (pool, _, error, restored) = initialize_database_startup(
            &live_path,
            &startup_backups,
            &recovery,
            db::latest_schema_version(),
        )
        .await
        .expect("initialize restored database");

        assert!(error.is_none());
        assert!(restored);
        assert_eq!(probe_value(&pool).await, "restored");
        assert!(!recovery
            .join(models::data_maintenance::PRE_RESTORE_FILE)
            .exists());
        pool.close().await;
    }

    #[tokio::test]
    async fn startup_rolls_back_when_restored_database_fails_migration_validation() {
        let workspace = tempfile::tempdir().expect("workspace");
        let live_path = workspace.path().join("live.db");
        let source_path = workspace.path().join("source.db");
        let startup_backups = workspace.path().join("startup-backups");
        create_probe_database(&live_path, "old").await;
        create_probe_database(&source_path, "rejected").await;
        let source_pool = db::connect(&source_path, false).await.expect("open source");
        sqlx::query("UPDATE _sqlx_migrations SET checksum = X'01' WHERE version = 1")
            .execute(&source_pool)
            .await
            .expect("make migration checksum incompatible");
        source_pool.close().await;
        let recovery = stage_database(&live_path, &source_path, workspace.path()).await;

        let (pool, _, error, restored) = initialize_database_startup(
            &live_path,
            &startup_backups,
            &recovery,
            db::latest_schema_version(),
        )
        .await
        .expect("initialize rolled-back database");

        assert!(error.is_none());
        assert!(!restored);
        assert_eq!(probe_value(&pool).await, "old");
        pool.close().await;
    }
}
