use std::path::Path;

use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::{
    db::AppState,
    error::AppError,
    models::project_note::{
        GetProjectNoteInput, OpenProjectNoteResult, ProjectNote, UpdateProjectNoteInput,
    },
    services::notes_service,
};

#[tauri::command]
pub async fn get_project_note(
    state: State<'_, AppState>,
    input: GetProjectNoteInput,
) -> Result<ProjectNote, AppError> {
    notes_service::get(&state.pool, input).await
}

#[tauri::command]
pub async fn update_project_note(
    state: State<'_, AppState>,
    input: UpdateProjectNoteInput,
) -> Result<ProjectNote, AppError> {
    notes_service::update_content(&state.pool, input).await
}

#[tauri::command]
pub async fn open_project_note(
    app: AppHandle,
    state: State<'_, AppState>,
    input: GetProjectNoteInput,
) -> Result<OpenProjectNoteResult, AppError> {
    let path = notes_service::remote_path(&state.pool, input).await?;
    let exists = tokio::fs::try_exists(&path).await? && Path::new(&path).is_file();
    if exists {
        app.opener()
            .open_path(path.to_string_lossy().into_owned(), None::<&str>)
            .map_err(|error| AppError::Validation(format!("cannot open note: {error}")))?;
    }
    Ok(OpenProjectNoteResult {
        opened: exists,
        path: path.to_string_lossy().into_owned(),
    })
}

#[tauri::command]
pub async fn reveal_project_note(
    app: AppHandle,
    state: State<'_, AppState>,
    input: GetProjectNoteInput,
) -> Result<OpenProjectNoteResult, AppError> {
    let path = notes_service::remote_path(&state.pool, input).await?;
    let exists = tokio::fs::try_exists(&path).await? && Path::new(&path).is_file();
    if exists {
        app.opener()
            .reveal_item_in_dir(&path)
            .map_err(|error| AppError::Validation(format!("cannot reveal note: {error}")))?;
    }
    Ok(OpenProjectNoteResult {
        opened: exists,
        path: path.to_string_lossy().into_owned(),
    })
}
