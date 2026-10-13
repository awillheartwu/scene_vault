use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager};

use super::{perf_log_service, process_resource_service, resource_storage_service};
use crate::{
    error::AppError,
    models::diagnostics::{LogLevel, LogRecord},
};

static STOPPED: AtomicBool = AtomicBool::new(false);
static STOP: tokio::sync::Notify = tokio::sync::Notify::const_new();
const PROCESS_INTERVAL: Duration = Duration::from_secs(60);
const STORAGE_INTERVAL: Duration = Duration::from_secs(5 * 60);

pub fn start(app: &AppHandle) -> Result<(), AppError> {
    let local = app.path().app_local_data_dir()?;
    let database = crate::db::database_path(app)?;
    let paths = vec![
        (
            "capture_output".to_owned(),
            app.path().app_cache_dir()?.join("capture-output"),
        ),
        ("thumbnails".to_owned(), local.join("thumbnails")),
        ("logs".to_owned(), app.path().app_log_dir()?),
        ("webview".to_owned(), local.join("EBWebView")),
        ("database".to_owned(), database.clone()),
        (
            "database_wal".to_owned(),
            database.with_file_name("scene-vault.db-wal"),
        ),
        (
            "database_shm".to_owned(),
            database.with_file_name("scene-vault.db-shm"),
        ),
    ];
    tauri::async_runtime::spawn(async move {
        let mut last_storage = Instant::now();
        loop {
            tokio::select! {
                _ = tokio::time::sleep(PROCESS_INTERVAL) => {},
                _ = STOP.notified() => break,
            }
            if STOPPED.load(Ordering::Relaxed) {
                break;
            }
            if !perf_log_service::is_enabled() {
                continue;
            }
            let storage_due = last_storage.elapsed() >= STORAGE_INTERVAL;
            let paths = paths.clone();
            let _ = tauri::async_runtime::spawn_blocking(move || {
                if STOPPED.load(Ordering::Relaxed) || !perf_log_service::is_enabled() {
                    return;
                }
                let started = Instant::now();
                let processes = process_resource_service::sample_for_monitor();
                let details = process_resource_service::monitor_details();
                if !STOPPED.load(Ordering::Relaxed) {
                    perf_log_service::record_event(LogRecord {
                        level: LogLevel::Debug,
                        module: "app.resources".to_owned(),
                        event: Some("process_sample".to_owned()),
                        message: "application process tree and system memory".to_owned(),
                        duration_ms: Some(started.elapsed().as_secs_f64() * 1000.0),
                        metrics: Some(
                            serde_json::json!({ "processes": processes, "details": details }),
                        ),
                        ..Default::default()
                    });
                }
                if storage_due && !STOPPED.load(Ordering::Relaxed) && perf_log_service::is_enabled()
                {
                    let started = Instant::now();
                    let metrics = resource_storage_service::performance_snapshot(&paths, || {
                        !STOPPED.load(Ordering::Relaxed) && perf_log_service::is_enabled()
                    });
                    if !metrics.is_null() && !STOPPED.load(Ordering::Relaxed) {
                        perf_log_service::record_event(LogRecord {
                            level: LogLevel::Debug,
                            module: "app.resources".to_owned(),
                            event: Some("storage_sample".to_owned()),
                            message: "application cache and database sizes".to_owned(),
                            duration_ms: Some(started.elapsed().as_secs_f64() * 1000.0),
                            metrics: Some(metrics),
                            ..Default::default()
                        });
                    }
                }
            })
            .await;
            if storage_due {
                last_storage = Instant::now();
            }
        }
    });
    Ok(())
}

pub fn shutdown() {
    STOPPED.store(true, Ordering::Relaxed);
    STOP.notify_one();
}
