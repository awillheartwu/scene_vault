use tauri::State;

use crate::{
    db::AppState,
    error::AppError,
    models::{
        diagnostics::{LogLevel, LogRecord},
        engine::{
            is_archive_output_size, CapturePolicyView, ProjectIdInput, SetArchiveOutputSizeInput,
            ARCHIVE_OUTPUT_INHERIT,
        },
        project::{
            AddProjectSourceDirectoryInput, CreateProjectInput, Project, ProjectDeletionPreview,
            ProjectOverviewSummary, ProjectSourceDirectory, RemoveProjectSourceDirectoryInput,
            RenameProjectInput, SetProjectCoverInput, SetProjectDestinationInput,
            SetProjectSourceDirectoryEnabledInput,
        },
    },
    services::{log_service, project_service},
};

#[tauri::command]
pub async fn create_project(
    state: State<'_, AppState>,
    input: CreateProjectInput,
) -> Result<Project, AppError> {
    let project = project_service::create(&state.pool, input).await?;
    project_event("created", &project.id);
    Ok(project)
}

#[tauri::command]
pub async fn list_projects(state: State<'_, AppState>) -> Result<Vec<Project>, AppError> {
    project_service::list(&state.pool).await
}

#[tauri::command]
pub async fn rename_project(
    state: State<'_, AppState>,
    input: RenameProjectInput,
) -> Result<Project, AppError> {
    let project = project_service::rename(&state.pool, input).await?;
    project_event("renamed", &project.id);
    Ok(project)
}

#[tauri::command]
pub async fn preview_project_deletion(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<ProjectDeletionPreview, AppError> {
    project_service::preview_deletion(&state.pool, &project_id).await
}

#[tauri::command]
pub async fn delete_project(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<ProjectDeletionPreview, AppError> {
    let result = project_service::delete_project(&state.pool, &project_id).await?;
    project_event("deleted", &project_id);
    Ok(result)
}

#[tauri::command]
pub async fn list_project_overviews(
    state: State<'_, AppState>,
) -> Result<Vec<ProjectOverviewSummary>, AppError> {
    project_service::list_overviews(&state.pool).await
}

#[tauri::command]
pub async fn list_project_source_directories(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<Vec<ProjectSourceDirectory>, AppError> {
    project_service::list_source_directories(&state.pool, &project_id).await
}

#[tauri::command]
pub async fn add_project_source_directory(
    state: State<'_, AppState>,
    input: AddProjectSourceDirectoryInput,
) -> Result<ProjectSourceDirectory, AppError> {
    let directory = project_service::add_source_directory(&state.pool, input).await?;
    project_event("source_directory_added", &directory.project_id);
    Ok(directory)
}

#[tauri::command]
pub async fn remove_project_source_directory(
    state: State<'_, AppState>,
    input: RemoveProjectSourceDirectoryInput,
) -> Result<(), AppError> {
    let operation_id = input.id.clone();
    project_service::remove_source_directory(&state.pool, input).await?;
    operation_event("source_directory_removed", &operation_id);
    Ok(())
}

#[tauri::command]
pub async fn set_project_source_directory_enabled(
    state: State<'_, AppState>,
    input: SetProjectSourceDirectoryEnabledInput,
) -> Result<ProjectSourceDirectory, AppError> {
    let directory = project_service::set_source_directory_enabled(&state.pool, input).await?;
    project_event("source_directory_updated", &directory.project_id);
    Ok(directory)
}

#[tauri::command]
pub async fn set_project_destination_directory(
    state: State<'_, AppState>,
    input: SetProjectDestinationInput,
) -> Result<Project, AppError> {
    let project = project_service::set_destination_directory(&state.pool, input).await?;
    project_event("destination_updated", &project.id);
    Ok(project)
}

#[tauri::command]
pub async fn set_project_cover(
    state: State<'_, AppState>,
    input: SetProjectCoverInput,
) -> Result<Project, AppError> {
    let project = project_service::set_cover(&state.pool, input).await?;
    project_event("cover_updated", &project.id);
    Ok(project)
}

#[tauri::command]
pub async fn get_project_capture_policy(
    state: State<'_, AppState>,
    input: ProjectIdInput,
) -> Result<CapturePolicyView, AppError> {
    let project_id = input.project_id.trim();
    let value: Option<String> =
        sqlx::query_scalar("SELECT archive_output_size FROM projects WHERE id = ?")
            .bind(project_id)
            .fetch_optional(&state.pool)
            .await?;
    let project_size = value.ok_or_else(|| AppError::NotFound("project".to_owned()))?;
    let resolved_size = crate::services::archive_output_settings_service::resolve_for_project(
        &state.pool,
        project_id,
    )
    .await?;
    let global_size = crate::services::archive_output_settings_service::get(&state.pool)
        .await?
        .size;
    Ok(CapturePolicyView {
        project_size,
        resolved_size,
        global_size,
    })
}

#[tauri::command]
pub async fn set_project_archive_output_size(
    state: State<'_, AppState>,
    input: SetArchiveOutputSizeInput,
) -> Result<CapturePolicyView, AppError> {
    let project_id = input.project_id.trim();
    let value = input.archive_output_size.trim().to_lowercase();
    if value != ARCHIVE_OUTPUT_INHERIT && !is_archive_output_size(&value) {
        return Err(AppError::Validation(
            "归档输出尺寸只能是 inherit / keep / 1080p / 1440p / 2160p".to_owned(),
        ));
    }
    let result = sqlx::query(
        "UPDATE projects SET archive_output_size = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
    )
    .bind(&value)
    .bind(project_id)
    .execute(&state.pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("project".to_owned()));
    }
    project_event("archive_output_size_updated", project_id);
    let resolved_size = crate::services::archive_output_settings_service::resolve_for_project(
        &state.pool,
        project_id,
    )
    .await?;
    let global_size = crate::services::archive_output_settings_service::get(&state.pool)
        .await?
        .size;
    Ok(CapturePolicyView {
        project_size: value,
        resolved_size,
        global_size,
    })
}

fn project_event(event: &str, project_id: &str) {
    log_service::record_event(LogRecord {
        level: LogLevel::Info,
        module: "project.state".to_owned(),
        message: event.replace('_', " "),
        event: Some(event.to_owned()),
        project_id: Some(project_id.to_owned()),
        outcome: Some("succeeded".to_owned()),
        ..Default::default()
    });
}

fn operation_event(event: &str, operation_id: &str) {
    log_service::record_event(LogRecord {
        level: LogLevel::Info,
        module: "project.state".to_owned(),
        message: event.replace('_', " "),
        event: Some(event.to_owned()),
        operation_id: Some(operation_id.to_owned()),
        outcome: Some("succeeded".to_owned()),
        ..Default::default()
    });
}
