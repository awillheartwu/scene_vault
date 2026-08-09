use tauri::State;

use crate::{
    db::AppState,
    error::AppError,
    models::character::{
        Character, CharacterSummary, CreateCharacterInput, MergeCharactersInput,
        RenameCharacterInput, SetCharacterAvatarInput,
    },
    services::character_service,
};

#[tauri::command]
pub async fn create_character(
    state: State<'_, AppState>,
    input: CreateCharacterInput,
) -> Result<Character, AppError> {
    character_service::create(&state.pool, input).await
}

#[tauri::command]
pub async fn list_characters(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<Vec<Character>, AppError> {
    character_service::list(&state.pool, &project_id).await
}

#[tauri::command]
pub async fn rename_character(
    state: State<'_, AppState>,
    input: RenameCharacterInput,
) -> Result<Character, AppError> {
    character_service::rename(&state.pool, input).await
}

#[tauri::command]
pub async fn merge_characters(
    state: State<'_, AppState>,
    input: MergeCharactersInput,
) -> Result<Character, AppError> {
    character_service::merge(&state.pool, input).await
}

#[tauri::command]
pub async fn set_character_avatar(
    state: State<'_, AppState>,
    input: SetCharacterAvatarInput,
) -> Result<Character, AppError> {
    character_service::set_avatar(&state.pool, input).await
}

#[tauri::command]
pub async fn list_project_character_summaries(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<Vec<CharacterSummary>, AppError> {
    character_service::list_project_summaries(&state.pool, &project_id).await
}
