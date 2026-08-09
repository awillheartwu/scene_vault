use tauri::Manager;
use tauri::State;

use crate::{
    db::AppState,
    error::AppError,
    models::vision::{
        BundledFont, CaptureRuntimeStatus, UpdateVisionSettingsInput, VisionHealth, VisionSettings,
    },
    services::{
        capture_worker_service, font_service, vision_engine_service, vision_settings_service,
    },
};

#[tauri::command]
pub async fn get_vision_settings(state: State<'_, AppState>) -> Result<VisionSettings, AppError> {
    vision_settings_service::get(&state.pool).await
}

#[tauri::command]
pub async fn update_vision_settings(
    state: State<'_, AppState>,
    input: UpdateVisionSettingsInput,
) -> Result<VisionSettings, AppError> {
    vision_settings_service::update(&state.pool, input).await
}

#[tauri::command]
pub async fn check_vision_engine(state: State<'_, AppState>) -> Result<VisionHealth, AppError> {
    let settings = vision_settings_service::get(&state.pool).await?;
    vision_engine_service::health(&settings).await
}

#[tauri::command]
pub async fn get_capture_runtime_status(
    state: State<'_, AppState>,
) -> Result<CaptureRuntimeStatus, AppError> {
    capture_worker_service::runtime_status(&state.pool).await
}

#[tauri::command]
pub async fn list_bundled_fonts(app: tauri::AppHandle) -> Result<Vec<BundledFont>, AppError> {
    let fonts_dir = app
        .path()
        .app_local_data_dir()
        .map_err(|error| {
            AppError::Io(std::io::Error::other(format!(
                "cannot resolve app data directory: {error}"
            )))
        })?
        .join("fonts");
    Ok(font_service::scan_fonts_dir(&fonts_dir))
}
