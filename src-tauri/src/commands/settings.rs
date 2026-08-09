use tauri::{AppHandle, State};

use crate::{
    db::AppState,
    error::AppError,
    models::{
        app_settings::{AppSettings, UpdateAppSettingsInput},
        archive_naming::{ArchiveNamingSettings, UpdateArchiveNamingSettingsInput},
        recognition_settings::{RecognitionSettings, UpdateRecognitionSettingsInput},
        vision::ProcessingSettings,
    },
    services::{
        app_settings_service, archive_naming_settings_service, processing_settings_service,
        recognition_settings_service, shortcuts,
    },
};

#[tauri::command]
pub async fn get_app_settings(state: State<'_, AppState>) -> Result<AppSettings, AppError> {
    app_settings_service::get(&state.pool).await
}

#[tauri::command]
pub async fn update_app_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    input: UpdateAppSettingsInput,
) -> Result<AppSettings, AppError> {
    let previous = app_settings_service::get(&state.pool).await?;
    let saved = app_settings_service::update(&state.pool, input).await?;
    shortcuts::reconfigure(&app, &previous, &saved)?;
    Ok(saved)
}

#[tauri::command]
pub async fn get_archive_naming_settings(
    state: State<'_, AppState>,
) -> Result<ArchiveNamingSettings, AppError> {
    archive_naming_settings_service::get(&state.pool).await
}

#[tauri::command]
pub async fn update_archive_naming_settings(
    state: State<'_, AppState>,
    input: UpdateArchiveNamingSettingsInput,
) -> Result<ArchiveNamingSettings, AppError> {
    archive_naming_settings_service::update(&state.pool, input).await
}

#[tauri::command]
pub async fn get_recognition_settings(
    state: State<'_, AppState>,
) -> Result<RecognitionSettings, AppError> {
    recognition_settings_service::get(&state.pool).await
}

#[tauri::command]
pub async fn update_recognition_settings(
    state: State<'_, AppState>,
    input: UpdateRecognitionSettingsInput,
) -> Result<RecognitionSettings, AppError> {
    recognition_settings_service::update(&state.pool, input).await
}

#[tauri::command]
pub async fn get_processing_settings(
    state: State<'_, AppState>,
) -> Result<ProcessingSettings, AppError> {
    processing_settings_service::get(&state.pool).await
}

#[tauri::command]
pub async fn update_processing_settings(
    state: State<'_, AppState>,
    settings: ProcessingSettings,
) -> Result<ProcessingSettings, AppError> {
    processing_settings_service::update(&state.pool, settings).await
}
