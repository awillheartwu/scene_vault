use std::path::{Path, PathBuf};

use tauri::{Manager, State};

use crate::{
    db::AppState,
    error::AppError,
    models::diagnostics::{
        ClientLogEventInput, LogCleanupResult, LogPolicySettings, LogQuery, LogQueryResult,
        LogRecord, LogStatus,
    },
    models::resource::{
        CleanupResourceInput, CleanupResourceResult, ProcessResourceStatus, StorageResourceStatus,
    },
    services::{
        log_service, log_settings_service, process_resource_service, resource_storage_service,
        resource_storage_service::ResourcePaths, thumbnail_service, vision_settings_service,
    },
};

#[tauri::command]
pub async fn get_log_settings(state: State<'_, AppState>) -> Result<LogPolicySettings, AppError> {
    log_settings_service::get(&state.pool).await
}

#[tauri::command]
pub async fn update_log_settings(
    state: State<'_, AppState>,
    input: LogPolicySettings,
) -> Result<LogPolicySettings, AppError> {
    let settings = log_settings_service::update(&state.pool, input).await?;
    log_service::configure(&settings)?;
    log_service::record_event(LogRecord {
        level: crate::models::diagnostics::LogLevel::Info,
        module: "settings.logging".to_owned(),
        message: "logging policy updated".to_owned(),
        event: Some("logging_policy_updated".to_owned()),
        outcome: Some("succeeded".to_owned()),
        ..Default::default()
    });
    Ok(settings)
}

const MAX_CLIENT_MESSAGE_CHARS: usize = 500;

#[tauri::command]
pub async fn record_client_event(input: ClientLogEventInput) -> Result<(), AppError> {
    let module = validate_token("module", &input.module, 80)?;
    let event = validate_token("event", &input.event, 80)?;
    let operation_id = input
        .operation_id
        .as_deref()
        .map(|value| validate_token("operationId", value, 128))
        .transpose()?;
    let outcome = input
        .outcome
        .as_deref()
        .map(|value| validate_token("outcome", value, 40))
        .transpose()?;
    let error_code = input
        .error_code
        .as_deref()
        .map(|value| validate_token("errorCode", value, 80))
        .transpose()?;
    if input
        .duration_ms
        .is_some_and(|value| !value.is_finite() || value < 0.0)
    {
        return Err(AppError::Validation(
            "durationMs must be a finite non-negative number".to_owned(),
        ));
    }
    let message = sanitize_client_message(input.message.as_deref().unwrap_or(&event));
    log_service::record_event(LogRecord {
        level: input.level,
        module: format!("ui.{module}"),
        message,
        event: Some(event),
        operation_id,
        duration_ms: input.duration_ms,
        outcome,
        error_code,
        ..Default::default()
    });
    Ok(())
}

fn validate_token(label: &str, value: &str, max_len: usize) -> Result<String, AppError> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > max_len
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._-:".contains(character))
    {
        return Err(AppError::Validation(format!("invalid client log {label}")));
    }
    Ok(value.to_owned())
}

fn sanitize_client_message(value: &str) -> String {
    let single_line = value.lines().collect::<Vec<_>>().join(" | ");
    if contains_local_path(&single_line) {
        return "client event contained a local path; details redacted".to_owned();
    }
    single_line.chars().take(MAX_CLIENT_MESSAGE_CHARS).collect()
}

fn contains_local_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    value.starts_with('/')
        || value.contains(" /")
        || value.contains(r"\\")
        || bytes.windows(3).any(|part| {
            part[0].is_ascii_alphabetic() && part[1] == b':' && matches!(part[2], b'\\' | b'/')
        })
}

#[tauri::command]
pub async fn list_debug_logs(input: LogQuery) -> Result<LogQueryResult, AppError> {
    tauri::async_runtime::spawn_blocking(move || log_service::query(input))
        .await
        .map_err(|error| AppError::Io(std::io::Error::other(error)))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_tokens_are_restricted_and_messages_are_single_line() {
        assert_eq!(
            validate_token("event", "route.error", 20).unwrap(),
            "route.error"
        );
        assert!(validate_token("event", "bad event", 20).is_err());
        assert_eq!(sanitize_client_message("first\r\nsecond"), "first | second");
        assert_eq!(
            sanitize_client_message(r"failed at C:\Users\Example\file.png"),
            "client event contained a local path; details redacted"
        );
    }

    #[test]
    fn python_runtime_path_prefers_configured_python_then_sidecar_then_fallback() {
        let fallback = PathBuf::from("C:/app/ai-runtime");
        let sidecar = PathBuf::from("D:/sv/scene-vault-ai.exe");
        assert_eq!(
            python_runtime_path(
                Some("C:/py/venv/Scripts/python.exe"),
                None,
                fallback.clone()
            ),
            PathBuf::from("C:/py/venv")
        );
        assert_eq!(
            python_runtime_path(Some("C:/py/python.exe"), None, fallback.clone()),
            PathBuf::from("C:/py")
        );
        assert_eq!(
            python_runtime_path(None, Some(&sidecar), fallback.clone()),
            PathBuf::from("D:/sv")
        );
        assert_eq!(python_runtime_path(None, None, fallback.clone()), fallback);
    }
}

#[tauri::command]
pub async fn get_log_status() -> Result<LogStatus, AppError> {
    tauri::async_runtime::spawn_blocking(log_service::status)
        .await
        .map_err(|error| AppError::Io(std::io::Error::other(error)))?
}

#[tauri::command]
pub async fn cleanup_debug_logs() -> Result<LogCleanupResult, AppError> {
    let result = tauri::async_runtime::spawn_blocking(log_service::cleanup)
        .await
        .map_err(|error| AppError::Io(std::io::Error::other(error)))??;
    log_service::record_event(LogRecord {
        level: crate::models::diagnostics::LogLevel::Info,
        module: "logging.cleanup".to_owned(),
        message: format!("removed {} expired log archives", result.removed_files),
        event: Some("manual_cleanup_completed".to_owned()),
        outcome: Some("succeeded".to_owned()),
        ..Default::default()
    });
    Ok(result)
}

#[tauri::command]
pub async fn get_diagnostic_summary() -> Result<String, AppError> {
    tauri::async_runtime::spawn_blocking(|| {
        log_service::diagnostic_summary(env!("CARGO_PKG_VERSION"))
    })
    .await
    .map_err(|error| AppError::Io(std::io::Error::other(error)))?
}

#[tauri::command]
pub async fn get_process_resource_status() -> Result<ProcessResourceStatus, AppError> {
    tauri::async_runtime::spawn_blocking(process_resource_service::sample)
        .await
        .map_err(|error| AppError::Io(std::io::Error::other(error)))
}

fn configured_parent(value: Option<&str>, fallback: PathBuf) -> PathBuf {
    value
        .map(Path::new)
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or(fallback)
}

fn python_runtime_path(value: Option<&str>, sidecar: Option<&Path>, fallback: PathBuf) -> PathBuf {
    if let Some(executable) = value.map(Path::new) {
        let Some(parent) = executable.parent() else {
            return fallback;
        };
        if parent
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("scripts"))
        {
            return parent.parent().unwrap_or(parent).to_path_buf();
        }
        return parent.to_path_buf();
    }
    if let Some(sidecar) = sidecar {
        if let Some(parent) = sidecar.parent() {
            return parent.to_path_buf();
        }
    }
    fallback
}

async fn resource_paths(
    app: &tauri::AppHandle,
    state: &AppState,
) -> Result<ResourcePaths, AppError> {
    let app_data = app.path().app_data_dir()?;
    let app_local = app.path().app_local_data_dir()?;
    let settings = vision_settings_service::get(&state.pool).await?;
    let model_path = settings
        .yunet_model_path
        .as_deref()
        .or(settings.sface_model_path.as_deref())
        .or(settings.arcface_model_path.as_deref());
    Ok(ResourcePaths {
        database_file: app_data.join("scene-vault.db"),
        logs: app.path().app_log_dir()?,
        thumbnail_cache: thumbnail_service::thumbnail_cache_dir(&app_local),
        capture_output: app.path().app_cache_dir()?.join("capture-output"),
        models: configured_parent(model_path, app_local.join("models")),
        fonts: app_local.join("fonts"),
        python_runtime: python_runtime_path(
            settings.python_executable_path.as_deref(),
            vision_settings_service::sidecar_executable().as_deref(),
            app_local.join("ai-runtime"),
        ),
        webview_data: app_local.join("EBWebView"),
    })
}

#[tauri::command]
pub async fn get_storage_resource_status(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<StorageResourceStatus, AppError> {
    let paths = resource_paths(&app, state.inner()).await?;
    tauri::async_runtime::spawn_blocking(move || resource_storage_service::scan(&paths))
        .await
        .map_err(|error| AppError::Io(std::io::Error::other(error)))
}

#[tauri::command]
pub async fn cleanup_resource(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    input: CleanupResourceInput,
) -> Result<CleanupResourceResult, AppError> {
    let paths = resource_paths(&app, state.inner()).await?;
    let kind = input.kind.trim().to_owned();
    let result = match kind.as_str() {
        "thumbnail_cache" => {
            let root = paths.thumbnail_cache;
            tauri::async_runtime::spawn_blocking(move || {
                resource_storage_service::cleanup_thumbnail_cache(&root)
            })
            .await
            .map_err(|error| AppError::Io(std::io::Error::other(error)))??
        }
        "capture_output" => {
            resource_storage_service::cleanup_capture_output(&state.pool, &paths.capture_output)
                .await?
        }
        "expired_logs" | "logs" => {
            let before = log_service::status()?;
            let cleanup = tauri::async_runtime::spawn_blocking(log_service::cleanup)
                .await
                .map_err(|error| AppError::Io(std::io::Error::other(error)))??;
            CleanupResourceResult {
                kind: "expired_logs".to_owned(),
                removed_files: cleanup.removed_files as u64,
                reclaimed_bytes: before
                    .total_bytes
                    .saturating_sub(cleanup.status.total_bytes),
                message: "已按日志保留策略清理过期归档。当前日志不会被删除。".to_owned(),
            }
        }
        _ => {
            return Err(AppError::Validation(format!(
                "unsupported cleanup resource kind: {kind}"
            )))
        }
    };
    log_service::record_event(LogRecord {
        level: crate::models::diagnostics::LogLevel::Info,
        module: "resource.cleanup".to_owned(),
        message: format!(
            "cleaned {} files and reclaimed {} bytes",
            result.removed_files, result.reclaimed_bytes
        ),
        event: Some("cleanup_completed".to_owned()),
        operation_id: Some(result.kind.clone()),
        outcome: Some("succeeded".to_owned()),
        ..Default::default()
    });
    Ok(result)
}
