mod commands;
mod db;
mod error;
mod models;
mod services;

use db::AppState;
use services::shortcuts::{CLASSIFY_POPUP_LABEL, NOTE_POPUP_LABEL};
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
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
            let pool = tauri::async_runtime::block_on(async {
                let pool = db::initialize(app.handle()).await?;
                services::capture_service::recover_interrupted_processing(&pool).await?;
                Ok::<_, error::AppError>(pool)
            })?;
            app.manage(AppState { pool: pool.clone() });
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
            commands::diagnostics::get_log_status,
            commands::diagnostics::cleanup_debug_logs,
            commands::diagnostics::get_diagnostic_summary,
            commands::window::set_window_always_on_top,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Scene Vault");
}
