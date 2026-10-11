use std::path::Path;

use tauri::{AppHandle, State};

use crate::{
    db::AppState,
    error::AppError,
    models::{
        capture::CaptureItemIdInput,
        engine::{
            BridgeUninstallResult, CaptureEngineMetadata, DetectEngineInput, EngineDetection,
            InstallEngineBridgeInput, LaunchProjectGameInput, ProjectEngineInput,
            ProjectEngineIntegrationView, ProjectGameLaunchOption, UninstallEngineBridgeInput,
        },
    },
    services::{
        engine_detection_service, engine_metadata_service, game_launch_service,
        renpy_bridge_service,
    },
};

#[tauri::command]
pub async fn detect_engine(input: DetectEngineInput) -> Result<EngineDetection, AppError> {
    engine_detection_service::detect(Path::new(input.path.trim())).await
}

#[tauri::command]
pub async fn get_project_engine_integration(
    app: AppHandle,
    state: State<'_, AppState>,
    input: ProjectEngineInput,
) -> Result<Vec<ProjectEngineIntegrationView>, AppError> {
    renpy_bridge_service::status(&state.pool, Some(&app), input).await
}

#[tauri::command]
pub async fn install_project_engine_bridge(
    app: AppHandle,
    state: State<'_, AppState>,
    input: InstallEngineBridgeInput,
) -> Result<ProjectEngineIntegrationView, AppError> {
    renpy_bridge_service::install(&state.pool, &app, input).await
}

#[tauri::command]
pub async fn uninstall_project_engine_bridge(
    app: AppHandle,
    state: State<'_, AppState>,
    input: UninstallEngineBridgeInput,
) -> Result<BridgeUninstallResult, AppError> {
    renpy_bridge_service::uninstall(&state.pool, Some(&app), input).await
}

#[tauri::command]
pub async fn get_capture_engine_metadata(
    state: State<'_, AppState>,
    input: CaptureItemIdInput,
) -> Result<Option<CaptureEngineMetadata>, AppError> {
    engine_metadata_service::get_capture_metadata(&state.pool, input.capture_item_id.trim()).await
}

#[tauri::command]
pub async fn list_project_games(
    state: State<'_, AppState>,
    input: ProjectEngineInput,
) -> Result<Vec<ProjectGameLaunchOption>, AppError> {
    game_launch_service::launch_options(&state.pool, input.project_id.trim()).await
}

#[tauri::command]
pub async fn launch_project_game(
    state: State<'_, AppState>,
    input: LaunchProjectGameInput,
) -> Result<ProjectGameLaunchOption, AppError> {
    game_launch_service::launch(&state.pool, input).await
}
