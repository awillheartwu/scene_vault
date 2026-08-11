use serde_json::json;
use tauri::{AppHandle, Emitter, State};

use crate::{
    db::AppState,
    error::AppError,
    models::{
        capture::{CaptureItem, CaptureItemIdInput},
        diagnostics::{LogLevel, LogRecord},
        recognition::{
            FaceBankModelStatus, FaceBankRebuildSummary, FaceSample, ListCharacterItemsInput,
            ReviewRecognitionInput, SetFaceSampleFlaggedInput, SetFaceSampleStatusInput,
            SetRecognitionSuggestionInput, VerificationResult, VerifyCaptureIdentityInput,
        },
        recognition_settings::ResolvedRecognitionProfile,
    },
    services::{capture_service, log_service, recognition_service},
};

/// Rebuilds the project's Face Bank with the currently configured model:
/// re-extracts every person capture's feature (feature only, no archive
/// rerun) and re-enrolls the samples. Progress events
/// `capture:face-bank-rebuild-progress` fire per item and a final
/// `capture:face-bank-rebuilt` event carries the summary.
#[tauri::command]
pub async fn rebuild_face_bank(
    app: AppHandle,
    state: State<'_, AppState>,
    project_id: String,
) -> Result<FaceBankRebuildSummary, AppError> {
    let project_id_for_events = project_id.clone();
    let progress_app = app.clone();
    let summary = recognition_service::rebuild_face_bank(
        &state.pool,
        &project_id,
        move |processed, total| {
            let _ = progress_app.emit(
                "capture:face-bank-rebuild-progress",
                json!({
                    "projectId": project_id_for_events,
                    "processed": processed,
                    "total": total,
                }),
            );
        },
    )
    .await?;
    log_service::record_event(LogRecord {
        level: if summary.failed > 0 {
            LogLevel::Warn
        } else {
            LogLevel::Info
        },
        module: "recognition.face_bank".to_owned(),
        message: format!(
            "rebuilt {} of {} samples; {} failed",
            summary.rebuilt, summary.total, summary.failed
        ),
        event: Some("rebuild_completed".to_owned()),
        project_id: Some(project_id.clone()),
        outcome: Some(
            if summary.failed > 0 {
                "degraded"
            } else {
                "succeeded"
            }
            .to_owned(),
        ),
        error_code: (summary.failed > 0).then(|| "partial_rebuild_failure".to_owned()),
        ..Default::default()
    });
    let _ = app.emit("capture:face-bank-rebuilt", json!(summary));
    Ok(summary)
}

/// Re-extracts one capture's primary-face feature with the currently
/// configured engine. Feature-only: no annotation/avatar output, no archive
/// rerun, and the capture's classification/character_id never change. The
/// command mirrors the worker's `capture:progress` events during extraction
/// and emits `capture:item-updated` with the refreshed item when done.
#[tauri::command]
pub async fn refresh_capture_face_feature(
    app: AppHandle,
    state: State<'_, AppState>,
    input: CaptureItemIdInput,
) -> Result<CaptureItem, AppError> {
    let item = capture_service::get_item(&state.pool, &input.capture_item_id).await?;
    let item_id_for_progress = item.id.clone();
    let source_path_for_progress = item.source_path.clone();
    let progress_app = app.clone();
    let updated = recognition_service::refresh_capture_face_feature(
        &state.pool,
        &input.capture_item_id,
        move |stage, percent| {
            let _ = progress_app.emit(
                "capture:progress",
                json!({
                    "captureItemId": item_id_for_progress,
                    "sourcePath": source_path_for_progress,
                    "stage": stage,
                    "percent": percent,
                }),
            );
        },
    )
    .await?;
    let _ = app.emit("capture:item-updated", &updated);
    recognition_item_event("face_feature_refreshed", &updated);
    Ok(updated)
}

/// Reports whether the project's Face Bank matches the configured
/// recognizer, so the UI can show a rebuild hint instead of silent
/// "no suggestions".
#[tauri::command]
pub async fn get_face_bank_model_status(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<FaceBankModelStatus, AppError> {
    recognition_service::face_bank_model_status(&state.pool, &project_id).await
}

/// Benchmark-calibrated default profile for a recognizer model id
/// (`opencv-sface` or `arcface-r50`), used by the settings page's
/// "restore defaults" button.
#[tauri::command]
pub async fn get_recognition_defaults(
    model_id: String,
) -> Result<ResolvedRecognitionProfile, AppError> {
    let model_id = model_id.trim();
    if model_id.is_empty() {
        return Err(AppError::Validation("model id cannot be empty".to_owned()));
    }
    Ok(crate::models::recognition_settings::known_defaults(
        model_id,
    ))
}

#[tauri::command]
pub async fn set_recognition_suggestion(
    state: State<'_, AppState>,
    input: SetRecognitionSuggestionInput,
) -> Result<CaptureItem, AppError> {
    recognition_service::set_suggestion(&state.pool, input).await
}

#[tauri::command]
pub async fn review_recognition_suggestion(
    state: State<'_, AppState>,
    input: ReviewRecognitionInput,
) -> Result<CaptureItem, AppError> {
    if input.decision.trim().eq_ignore_ascii_case("accepted") {
        let item =
            recognition_service::accept_suggestion(&state.pool, &input.capture_item_id).await?;
        recognition_item_event("suggestion_accepted", &item);
        return Ok(item);
    }
    let item = recognition_service::review_suggestion(&state.pool, input).await?;
    recognition_item_event("suggestion_reviewed", &item);
    Ok(item)
}

#[tauri::command]
pub async fn accept_recognition_suggestion(
    state: State<'_, AppState>,
    capture_item_id: String,
) -> Result<CaptureItem, AppError> {
    let item = recognition_service::accept_suggestion(&state.pool, &capture_item_id).await?;
    recognition_item_event("suggestion_accepted", &item);
    Ok(item)
}

#[tauri::command]
pub async fn reject_suggestion_and_enroll(
    state: State<'_, AppState>,
    capture_item_id: String,
) -> Result<CaptureItem, AppError> {
    let item = recognition_service::reject_and_enroll(&state.pool, &capture_item_id).await?;
    recognition_item_event("suggestion_rejected_and_enrolled", &item);
    Ok(item)
}

#[tauri::command]
pub async fn batch_reject_and_enroll(
    state: State<'_, AppState>,
    project_id: String,
    character_id: String,
) -> Result<i64, AppError> {
    let count =
        recognition_service::batch_reject_and_enroll(&state.pool, &project_id, &character_id)
            .await?;
    log_service::record_event(LogRecord {
        level: LogLevel::Info,
        module: "recognition.review".to_owned(),
        message: format!("batch reviewed {count} captures"),
        event: Some("batch_rejected_and_enrolled".to_owned()),
        operation_id: Some(character_id),
        project_id: Some(project_id),
        outcome: Some("succeeded".to_owned()),
        ..Default::default()
    });
    Ok(count)
}

#[tauri::command]
pub async fn list_character_capture_items(
    state: State<'_, AppState>,
    input: ListCharacterItemsInput,
) -> Result<Vec<CaptureItem>, AppError> {
    recognition_service::list_character_items(&state.pool, input).await
}

#[tauri::command]
pub async fn list_character_face_samples(
    state: State<'_, AppState>,
    character_id: String,
) -> Result<Vec<FaceSample>, AppError> {
    recognition_service::list_character_face_samples(&state.pool, &character_id).await
}

#[tauri::command]
pub async fn set_face_sample_status(
    state: State<'_, AppState>,
    input: SetFaceSampleStatusInput,
) -> Result<FaceSample, AppError> {
    let sample = recognition_service::set_face_sample_status(&state.pool, input).await?;
    sample_event(&state.pool, "face_sample_status_updated", &sample).await;
    Ok(sample)
}

#[tauri::command]
pub async fn set_face_sample_flagged(
    state: State<'_, AppState>,
    input: SetFaceSampleFlaggedInput,
) -> Result<FaceSample, AppError> {
    let sample = recognition_service::set_face_sample_flagged(&state.pool, input).await?;
    sample_event(&state.pool, "face_sample_flag_updated", &sample).await;
    Ok(sample)
}

#[tauri::command]
pub async fn verify_capture_identity(
    state: State<'_, AppState>,
    input: VerifyCaptureIdentityInput,
) -> Result<VerificationResult, AppError> {
    let capture_item_id = input.capture_item_id.clone();
    let result = recognition_service::verify_capture_identity(&state.pool, input).await?;
    if let Ok(item) = capture_service::get_item(&state.pool, &capture_item_id).await {
        recognition_item_event("identity_verified", &item);
    }
    Ok(result)
}

/// Re-runs the face-bank comparison for one capture with the current sample
/// bank and returns the updated item. The capture page calls this when the
/// user chooses the person classification, so suggestions always use the
/// freshest samples.
#[tauri::command]
pub async fn suggest_for_capture(
    state: State<'_, AppState>,
    input: CaptureItemIdInput,
) -> Result<CaptureItem, AppError> {
    recognition_service::suggest_from_face_bank(&state.pool, &input.capture_item_id).await?;
    let item = capture_service::get_item(&state.pool, &input.capture_item_id).await?;
    recognition_item_event("suggestion_refreshed", &item);
    Ok(item)
}

async fn sample_event(pool: &sqlx::SqlitePool, event: &str, sample: &FaceSample) {
    let Ok(item) = capture_service::get_item(pool, &sample.capture_item_id).await else {
        return;
    };
    log_service::record_event(LogRecord {
        level: LogLevel::Info,
        module: "recognition.face_bank".to_owned(),
        message: event.replace('_', " "),
        event: Some(event.to_owned()),
        operation_id: Some(sample.id.clone()),
        project_id: Some(item.project_id),
        session_id: Some(item.session_id),
        capture_item_id: Some(item.id),
        outcome: Some("succeeded".to_owned()),
        ..Default::default()
    });
}

fn recognition_item_event(event: &str, item: &CaptureItem) {
    log_service::record_event(LogRecord {
        level: LogLevel::Info,
        module: "recognition.review".to_owned(),
        message: event.replace('_', " "),
        event: Some(event.to_owned()),
        project_id: Some(item.project_id.clone()),
        session_id: Some(item.session_id.clone()),
        capture_item_id: Some(item.id.clone()),
        outcome: Some("succeeded".to_owned()),
        ..Default::default()
    });
}
