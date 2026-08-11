use tauri::State;

use crate::{
    db::AppState,
    error::AppError,
    models::{
        character::{
            Character, CharacterSummary, CreateCharacterInput, MergeCharactersInput,
            RenameCharacterInput, SetCharacterAvatarInput,
        },
        diagnostics::{LogLevel, LogRecord},
    },
    services::{character_service, log_service},
};

#[tauri::command]
pub async fn create_character(
    state: State<'_, AppState>,
    input: CreateCharacterInput,
) -> Result<Character, AppError> {
    let character = character_service::create(&state.pool, input).await?;
    character_event("created", &character);
    Ok(character)
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
    let character = character_service::rename(&state.pool, input).await?;
    character_event("renamed", &character);
    Ok(character)
}

#[tauri::command]
pub async fn merge_characters(
    state: State<'_, AppState>,
    input: MergeCharactersInput,
) -> Result<Character, AppError> {
    let character = character_service::merge(&state.pool, input).await?;
    character_event("merged", &character);
    Ok(character)
}

#[tauri::command]
pub async fn set_character_avatar(
    state: State<'_, AppState>,
    input: SetCharacterAvatarInput,
) -> Result<Character, AppError> {
    let character = character_service::set_avatar(&state.pool, input).await?;
    character_event("avatar_updated", &character);
    Ok(character)
}

fn character_event(event: &str, character: &Character) {
    log_service::record_event(LogRecord {
        level: LogLevel::Info,
        module: "character.state".to_owned(),
        message: event.replace('_', " "),
        event: Some(event.to_owned()),
        operation_id: Some(character.id.clone()),
        project_id: Some(character.project_id.clone()),
        outcome: Some("succeeded".to_owned()),
        ..Default::default()
    });
}

#[tauri::command]
pub async fn list_project_character_summaries(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<Vec<CharacterSummary>, AppError> {
    character_service::list_project_summaries(&state.pool, &project_id).await
}
