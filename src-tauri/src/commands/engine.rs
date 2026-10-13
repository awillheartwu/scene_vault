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
            ProjectEngineIntegrationView, ProjectGameLaunchOption, RenpyScreenshotPolicyView,
            SetInstallationRenpyScreenshotPolicyInput, SetProjectRenpyScreenshotPolicyInput,
            UninstallEngineBridgeInput,
        },
    },
    services::{
        engine_detection_service, engine_metadata_service, game_launch_service,
        renpy_bridge_service, renpy_screenshot_settings_service,
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

#[tauri::command]
pub async fn get_project_renpy_screenshot_policy(
    state: State<'_, AppState>,
    input: ProjectEngineInput,
) -> Result<RenpyScreenshotPolicyView, AppError> {
    renpy_screenshot_settings_service::project_policy(&state.pool, &input.project_id).await
}

#[tauri::command]
pub async fn set_project_renpy_screenshot_policy(
    app: AppHandle,
    state: State<'_, AppState>,
    input: SetProjectRenpyScreenshotPolicyInput,
) -> Result<RenpyScreenshotPolicyView, AppError> {
    renpy_screenshot_settings_service::set_project(
        &state.pool,
        &app,
        &input.project_id,
        input.auto_hide_ui,
    )
    .await
}

#[tauri::command]
pub async fn set_installation_renpy_screenshot_policy(
    app: AppHandle,
    state: State<'_, AppState>,
    input: SetInstallationRenpyScreenshotPolicyInput,
) -> Result<ProjectEngineIntegrationView, AppError> {
    let project_id = input.project_id.trim();
    let integration_id = input.integration_id.trim();
    renpy_screenshot_settings_service::set_installation(
        &state.pool,
        &app,
        project_id,
        integration_id,
        input.auto_hide_ui,
    )
    .await?;
    renpy_bridge_service::status(
        &state.pool,
        Some(&app),
        ProjectEngineInput {
            project_id: project_id.to_owned(),
        },
    )
    .await?
    .into_iter()
    .find(|view| view.integration.id == integration_id)
    .ok_or_else(|| AppError::NotFound("engine integration".to_owned()))
}
