use tauri::{AppHandle, Emitter, State};
use crate::{db::AppState, error::AppError, models::capture::CaptureItem,
    services::{capture_roi_service::{self, FaceRoi, SetFaceRoiInput}, capture_service}};

#[tauri::command]
pub async fn get_capture_face_roi(state: State<'_, AppState>, capture_item_id: String) -> Result<Option<FaceRoi>, AppError> {
    capture_roi_service::get(&state.pool, &capture_item_id).await
}

#[tauri::command]
pub async fn set_capture_face_roi(app: AppHandle, state: State<'_, AppState>, input: SetFaceRoiInput) -> Result<CaptureItem, AppError> {
    let id = input.capture_item_id.clone();
    let result = capture_roi_service::set(&state.pool, input).await;
    if let Ok(item) = capture_service::get_item(&state.pool, &id).await {
        let _ = app.emit("capture:item-updated", &item);
    }
    result
}

#[tauri::command]
pub async fn list_project_pending_captures(state: State<'_, AppState>, project_id: String) -> Result<Vec<CaptureItem>, AppError> {
    Ok(sqlx::query_as::<_, CaptureItem>("SELECT * FROM capture_items WHERE project_id=? AND status='awaiting_label' AND operation_owner IS NULL ORDER BY captured_at, created_at, id")
        .bind(project_id).fetch_all(&state.pool).await?)
}

use crate::{models::capture_reset::{ResetPreviewInput, ResetExecuteInput, ResetJob}, services::{capture_reset_service, file_recycle_service::SystemRecycleBin}};

#[tauri::command]
pub async fn preview_capture_reset(state: State<'_, AppState>, input: ResetPreviewInput) -> Result<ResetJob, AppError> {
    capture_reset_service::preview(&state.pool, input).await
}

#[tauri::command]
pub async fn get_capture_reset(job_id: String) -> Result<ResetJob, AppError> {
    capture_reset_service::get(&job_id)
}

#[tauri::command]
pub fn discard_capture_reset(job_id: String) -> Result<(), AppError> {
    capture_reset_service::discard(&job_id)
}

#[tauri::command]
pub async fn execute_capture_reset(app: AppHandle, state: State<'_, AppState>, input: ResetExecuteInput) -> Result<ResetJob, AppError> {
    use tauri::Manager;
    let result = capture_reset_service::execute(&state.pool, &app.path().app_cache_dir()?, &app.path().app_local_data_dir()?, input, &SystemRecycleBin).await?;
    for entry in &result.items {
        if entry.status == "succeeded" {
            if let Ok(item) = capture_service::get_item(&state.pool, &entry.capture_item_id).await {
                let _ = app.emit("capture:item-updated", &item);
            }
        }
    }
    Ok(result)
}

#[tauri::command]
pub async fn list_capture_reset_candidates(state: State<'_, AppState>, input: ResetPreviewInput) -> Result<Vec<String>, AppError> {
    capture_reset_service::candidates(&state.pool, &input).await
}
