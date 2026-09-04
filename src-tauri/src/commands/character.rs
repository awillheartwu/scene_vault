use tauri::{AppHandle, Emitter, Manager, State};

use crate::{
    db::AppState,
    error::AppError,
    models::{
        capture::{CaptureDeletionPreview, CaptureDeletionResult, DeleteCharacterCapturesInput},
        character::{
            Character, CharacterSummary, CreateCharacterInput, MergeCharactersInput,
            RenameCharacterInput, SetCharacterAvatarInput,
        },
        diagnostics::{LogLevel, LogRecord},
    },
    services::{
        capture_deletion_service, character_service, file_recycle_service::SystemRecycleBin,
        log_service,
    },
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

#[tauri::command]
pub async fn preview_character_deletion(
    state: State<'_, AppState>,
    input: DeleteCharacterCapturesInput,
) -> Result<CaptureDeletionPreview, AppError> {
    capture_deletion_service::preview_character(&state.pool, &input.character_id).await
}

#[tauri::command]
pub async fn delete_character(
    app: AppHandle,
    state: State<'_, AppState>,
    input: DeleteCharacterCapturesInput,
) -> Result<CaptureDeletionResult, AppError> {
    let result = capture_deletion_service::delete_character(
        &state.pool,
        &app.path().app_cache_dir()?,
        &app.path().app_local_data_dir()?,
        &input.character_id,
        input.delete_destination_files,
        input.allow_permanent_network_delete,
        &SystemRecycleBin,
    )
    .await?;
    if result.completed {
        for id in &result.deleted_capture_item_ids {
            let _ = app.emit(
                "capture:item-purged",
                serde_json::json!({ "captureItemId": id }),
            );
        }
        let _ = app.emit(
            "capture:character-deleted",
            serde_json::json!({
                "characterId": input.character_id,
            }),
        );
    }
    log_service::record_event(LogRecord {
        level: if result.completed {
            LogLevel::Info
        } else {
            LogLevel::Warn
        },
        module: "character.deletion".to_owned(),
        message: "character deletion requested".to_owned(),
        event: Some("character_delete_completed".to_owned()),
        operation_id: Some(input.character_id),
        outcome: Some(
            if result.completed {
                "succeeded"
            } else {
                "partial"
            }
            .to_owned(),
        ),
        error_code: (!result.completed).then(|| "target_delete_failed".to_owned()),
        ..Default::default()
    });
    Ok(result)
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
