use tauri::{AppHandle, Emitter, State};

use crate::{
    db::AppState,
    error::AppError,
    models::{
        capture::{
            CaptureHistoryPage, CaptureItem, CaptureItemIdInput, CaptureItemListResponse,
            CaptureSession, ClassifyPopupContext, CompleteCaptureProcessingInput,
            DiscoverCapturesInput, DiscoverCapturesResult, EndCaptureSessionInput,
            ImportDirectoryCapturesInput, ImportedRecognitionInput, LabelCaptureInput,
            ListCaptureHistoryInput, ListCategoryItemsInput, MarkCaptureFailedInput,
            ReadCaptureImageInput, RegisterCaptureInput, RelabelCaptureInput, RetryCaptureInput,
            RetryDegradedCapturesInput, StartCaptureSessionInput, StartCaptureSessionResult,
            UnimportedCapture,
        },
        diagnostics::{LogLevel, LogRecord},
    },
    services::{
        capture_archive_service, capture_discovery_service, capture_service, log_service,
        thumbnail_service,
    },
};

#[tauri::command]
pub async fn start_capture_session(
    state: State<'_, AppState>,
    input: StartCaptureSessionInput,
) -> Result<StartCaptureSessionResult, AppError> {
    let result = capture_service::start_session(&state.pool, input).await?;
    session_event("session_started", &result.session);
    Ok(result)
}

#[tauri::command]
pub async fn list_capture_sessions(
    state: State<'_, AppState>,
    project_id: Option<String>,
) -> Result<Vec<CaptureSession>, AppError> {
    capture_service::list_sessions(&state.pool, project_id.as_deref()).await
}

#[tauri::command]
pub async fn end_capture_session(
    state: State<'_, AppState>,
    input: EndCaptureSessionInput,
) -> Result<CaptureSession, AppError> {
    let session = capture_service::end_session(&state.pool, input).await?;
    session_event("session_ended", &session);
    Ok(session)
}

#[tauri::command]
pub async fn list_capture_items(
    state: State<'_, AppState>,
    session_id: String,
    page: Option<u32>,
    page_size: Option<u32>,
) -> Result<CaptureItemListResponse, AppError> {
    match (page, page_size) {
        (None, None) => Ok(CaptureItemListResponse::Legacy(
            capture_service::list_items(&state.pool, &session_id).await?,
        )),
        (Some(page), Some(page_size)) => Ok(CaptureItemListResponse::Paged(
            capture_service::list_items_paged(&state.pool, &session_id, page, page_size).await?,
        )),
        _ => Err(AppError::Validation(
            "page and pageSize must be provided together".to_owned(),
        )),
    }
}

#[tauri::command]
pub async fn list_project_recent_captures(
    state: State<'_, AppState>,
    project_id: String,
    limit: Option<u32>,
) -> Result<Vec<CaptureItem>, AppError> {
    let limit = i64::from(limit.unwrap_or(12).clamp(1, 100));
    capture_service::list_project_recent_items(&state.pool, &project_id, limit).await
}

#[tauri::command]
pub async fn list_capture_history(
    state: State<'_, AppState>,
    input: ListCaptureHistoryInput,
) -> Result<CaptureHistoryPage, AppError> {
    capture_service::list_history(&state.pool, input).await
}

#[tauri::command]
pub async fn discover_captures(
    app: AppHandle,
    state: State<'_, AppState>,
    input: DiscoverCapturesInput,
) -> Result<DiscoverCapturesResult, AppError> {
    let session_id = input.session_id.clone();
    let result = capture_discovery_service::discover(&state.pool, Some(&app), input).await?;
    log_service::record_event(LogRecord {
        level: LogLevel::Info,
        module: "capture.discovery".to_owned(),
        message: format!(
            "discovered {} new captures from {} entries",
            result.discovered_count, result.entries_scanned
        ),
        event: Some("scan_completed".to_owned()),
        session_id: Some(session_id),
        duration_ms: Some(result.scan_duration_ms as f64),
        outcome: Some("succeeded".to_owned()),
        ..Default::default()
    });
    Ok(result)
}

#[tauri::command]
pub async fn register_capture(
    state: State<'_, AppState>,
    input: RegisterCaptureInput,
) -> Result<CaptureItem, AppError> {
    let item = capture_service::register_capture(&state.pool, input).await?;
    item_event("registered", &item);
    Ok(item)
}

#[tauri::command]
pub async fn label_capture(
    app: AppHandle,
    state: State<'_, AppState>,
    input: LabelCaptureInput,
) -> Result<CaptureItem, AppError> {
    let item = capture_service::label_capture(&state.pool, input).await?;
    item_event("labeled", &item);
    let _ = app.emit("capture:item-updated", &item);
    Ok(item)
}

#[tauri::command]
pub async fn relabel_capture_item(
    app: AppHandle,
    state: State<'_, AppState>,
    input: RelabelCaptureInput,
) -> Result<CaptureItem, AppError> {
    let item = capture_service::relabel_capture_item(&state.pool, input).await?;
    item_event("relabeled", &item);
    let _ = app.emit("capture:item-updated", &item);
    Ok(item)
}

#[tauri::command]
pub async fn retry_capture(
    app: AppHandle,
    state: State<'_, AppState>,
    input: RetryCaptureInput,
) -> Result<CaptureItem, AppError> {
    let item = capture_service::retry_capture(&state.pool, input).await?;
    item_event("retry_requested", &item);
    let _ = app.emit("capture:item-updated", &item);
    Ok(item)
}

#[tauri::command]
pub async fn retry_degraded_captures(
    state: State<'_, AppState>,
    input: RetryDegradedCapturesInput,
) -> Result<i64, AppError> {
    capture_service::retry_degraded_captures(
        &state.pool,
        &input.project_id,
        input.character_id.as_deref(),
    )
    .await
}

#[tauri::command]
pub async fn list_category_items(
    state: State<'_, AppState>,
    input: ListCategoryItemsInput,
) -> Result<CaptureItemListResponse, AppError> {
    let page = input.page;
    let page_size = input.page_size;
    match (page, page_size) {
        (None, None) => Ok(CaptureItemListResponse::Legacy(
            capture_service::list_category_items(&state.pool, input).await?,
        )),
        (Some(page), Some(page_size)) => Ok(CaptureItemListResponse::Paged(
            capture_service::list_category_items_paged(&state.pool, input, page, page_size).await?,
        )),
        _ => Err(AppError::Validation(
            "page and pageSize must be provided together".to_owned(),
        )),
    }
}

#[tauri::command]
pub async fn list_unimported_captures(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<Vec<UnimportedCapture>, AppError> {
    capture_service::list_unimported_captures(&state.pool, &session_id).await
}

#[tauri::command]
pub async fn import_directory_captures(
    app: AppHandle,
    state: State<'_, AppState>,
    input: ImportDirectoryCapturesInput,
) -> Result<i64, AppError> {
    let items = capture_service::import_directory_captures(&state.pool, input).await?;
    let count = i64::try_from(items.len()).unwrap_or(i64::MAX);
    for item in &items {
        // Mirror live file discovery so the Capture page can show imported
        // screenshots immediately without waiting for a reload.
        let _ = app.emit("capture:item-created", item);
        item_event("imported", item);
    }
    Ok(count)
}

#[tauri::command]
pub async fn deferred_import_recognition_count(
    state: State<'_, AppState>,
    input: ImportedRecognitionInput,
) -> Result<i64, AppError> {
    capture_service::deferred_import_recognition_count(&state.pool, &input.session_id).await
}

#[tauri::command]
pub async fn start_imported_recognition(
    state: State<'_, AppState>,
    input: ImportedRecognitionInput,
) -> Result<i64, AppError> {
    capture_service::start_imported_recognition(&state.pool, &input.session_id).await
}

#[tauri::command]
pub async fn mark_capture_processing(
    state: State<'_, AppState>,
    input: CaptureItemIdInput,
) -> Result<CaptureItem, AppError> {
    capture_service::mark_processing(&state.pool, input).await
}

#[tauri::command]
pub async fn complete_capture_processing(
    state: State<'_, AppState>,
    input: CompleteCaptureProcessingInput,
) -> Result<CaptureItem, AppError> {
    capture_service::complete_processing(&state.pool, input).await
}

#[tauri::command]
pub async fn mark_capture_failed(
    state: State<'_, AppState>,
    input: MarkCaptureFailedInput,
) -> Result<CaptureItem, AppError> {
    capture_service::mark_failed(&state.pool, input).await
}

#[tauri::command]
pub async fn archive_capture(
    state: State<'_, AppState>,
    input: CaptureItemIdInput,
) -> Result<CaptureItem, AppError> {
    capture_archive_service::archive(&state.pool, input).await
}

#[tauri::command]
pub async fn read_capture_image(
    state: State<'_, AppState>,
    input: ReadCaptureImageInput,
) -> Result<tauri::ipc::Response, AppError> {
    let item = capture_service::get_item(&state.pool, input.capture_item_id.trim()).await?;
    let path = capture_service::image_path(&item, input.variant.trim())?;
    let bytes = tokio::fs::read(path).await?;
    Ok(tauri::ipc::Response::new(bytes))
}

#[tauri::command]
pub async fn read_capture_thumbnail(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    input: ReadCaptureImageInput,
) -> Result<tauri::ipc::Response, AppError> {
    use tauri::Manager;

    let item = capture_service::get_item(&state.pool, input.capture_item_id.trim()).await?;
    let variant = input.variant.trim();
    let path = capture_service::image_path(&item, variant)?;
    let cache_root = app.path().app_local_data_dir()?;
    let settings = crate::services::app_settings_service::get(&state.pool).await?;
    let limit_bytes = (settings.thumbnail_cache_size_mb as u64).saturating_mul(1024 * 1024);
    let bytes =
        thumbnail_service::read_thumbnail(&cache_root, &item.id, variant, &path, limit_bytes)
            .await?;
    Ok(tauri::ipc::Response::new(bytes))
}

#[tauri::command]
pub async fn thumbnail_cache_status(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<crate::models::capture::ThumbnailCacheStatus, AppError> {
    use tauri::Manager;

    let cache_root = app.path().app_local_data_dir()?;
    let settings = crate::services::app_settings_service::get(&state.pool).await?;
    let dir = thumbnail_service::thumbnail_cache_dir(&cache_root);
    Ok(crate::models::capture::ThumbnailCacheStatus {
        dir: dir.to_string_lossy().into_owned(),
        total_bytes: thumbnail_service::cache_size(&dir),
        limit_bytes: (settings.thumbnail_cache_size_mb as u64).saturating_mul(1024 * 1024),
    })
}

#[tauri::command]
pub async fn classify_popup_context(
    state: State<'_, AppState>,
) -> Result<ClassifyPopupContext, AppError> {
    capture_service::classify_popup_context(&state.pool).await
}

fn session_event(event: &str, session: &CaptureSession) {
    log_service::record_event(LogRecord {
        level: LogLevel::Info,
        module: "capture.session".to_owned(),
        message: event.replace('_', " "),
        event: Some(event.to_owned()),
        project_id: Some(session.project_id.clone()),
        session_id: Some(session.id.clone()),
        outcome: Some("succeeded".to_owned()),
        ..Default::default()
    });
}

fn item_event(event: &str, item: &CaptureItem) {
    log_service::record_event(LogRecord {
        level: LogLevel::Info,
        module: "capture.state".to_owned(),
        message: event.replace('_', " "),
        event: Some(event.to_owned()),
        project_id: Some(item.project_id.clone()),
        session_id: Some(item.session_id.clone()),
        capture_item_id: Some(item.id.clone()),
        attempt: u32::try_from(item.attempt_count).ok(),
        outcome: Some("succeeded".to_owned()),
        ..Default::default()
    });
}
