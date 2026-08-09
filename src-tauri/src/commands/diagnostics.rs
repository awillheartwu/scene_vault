use crate::{
    error::AppError,
    models::diagnostics::{LogCleanupResult, LogQuery, LogQueryResult, LogStatus},
    services::log_service,
};

#[tauri::command]
pub async fn list_debug_logs(input: LogQuery) -> Result<LogQueryResult, AppError> {
    tauri::async_runtime::spawn_blocking(move || log_service::query(input))
        .await
        .map_err(|error| AppError::Io(std::io::Error::other(error)))?
}

#[tauri::command]
pub async fn get_log_status() -> Result<LogStatus, AppError> {
    tauri::async_runtime::spawn_blocking(log_service::status)
        .await
        .map_err(|error| AppError::Io(std::io::Error::other(error)))?
}

#[tauri::command]
pub async fn cleanup_debug_logs() -> Result<LogCleanupResult, AppError> {
    tauri::async_runtime::spawn_blocking(log_service::cleanup)
        .await
        .map_err(|error| AppError::Io(std::io::Error::other(error)))?
}

#[tauri::command]
pub async fn get_diagnostic_summary() -> Result<String, AppError> {
    tauri::async_runtime::spawn_blocking(|| {
        log_service::diagnostic_summary(env!("CARGO_PKG_VERSION"))
    })
    .await
    .map_err(|error| AppError::Io(std::io::Error::other(error)))?
}
