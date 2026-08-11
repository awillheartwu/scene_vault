use std::{path::PathBuf, time::Duration};

use serde_json::json;
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter};

use crate::{
    error::AppError,
    models::{
        capture::{
            CaptureItem, CaptureItemIdInput, CompleteCaptureProcessingInput, MarkCaptureFailedInput,
        },
        diagnostics::{LogLevel, LogRecord},
        vision::CaptureRuntimeStatus,
    },
    services::{
        capture_archive_service, capture_service, log_service, processing_settings_service,
        recognition_service, recognition_settings_service, vision_engine_service,
        vision_settings_service,
    },
};

const WORKER_POLL_INTERVAL: Duration = Duration::from_millis(500);

pub async fn run(pool: SqlitePool, app: AppHandle, cache_root: PathBuf) {
    let mut interval = tokio::time::interval(WORKER_POLL_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        if let Err(error) = process_once(&pool, &app, &cache_root).await {
            log_service::record_event(LogRecord {
                level: LogLevel::Error,
                module: "capture.worker".to_owned(),
                message: "capture worker cycle failed".to_owned(),
                event: Some("cycle_failed".to_owned()),
                outcome: Some("failed".to_owned()),
                error_code: Some("worker_cycle_error".to_owned()),
                ..Default::default()
            });
            let _ = app.emit("capture:runtime-error", error.to_string());
        }
    }
}

pub async fn process_once(
    pool: &SqlitePool,
    app: &AppHandle,
    cache_root: &std::path::Path,
) -> Result<(), AppError> {
    let vision_settings = vision_settings_service::get(pool).await?;
    let processing_settings = processing_settings_service::get(pool).await?;
    let engine_configured = vision_settings_service::is_configured(&vision_settings);
    let recognition = recognition_settings_service::get(pool).await?;
    // Person items are never skipped: without a configured Python engine they
    // are archived raw through the degraded path instead of waiting forever.

    if let Some(item) = capture_service::next_archive_pending(pool).await? {
        archive_and_emit(pool, app, &item).await?;
        emit_runtime(pool, app).await?;
        return Ok(());
    }

    if capture_service::peek_next_queued(pool, false)
        .await?
        .is_none()
    {
        // Idle queue: use the cycle for the pre-label feature pass so the
        // classify popup can show a face-bank recommendation.
        if recognition.extract_at_registration
            && engine_configured
            && process_awaiting_label_feature(pool, &vision_settings, &processing_settings, app)
                .await?
        {
            emit_runtime(pool, app).await?;
            return Ok(());
        }
        return Ok(());
    }
    let Some(item) = capture_service::claim_next_queued(pool, false).await? else {
        return Ok(());
    };
    emit_item(app, &item);
    capture_log(
        &item,
        LogLevel::Info,
        "processing_started",
        "started",
        "capture processing started",
        None,
    );
    emit_runtime(pool, app).await?;

    if item.classification != "person" {
        let pending = capture_service::mark_archive_pending_from_processing(pool, &item.id).await?;
        emit_item(app, &pending);
        archive_and_emit(pool, app, &pending).await?;
        emit_runtime(pool, app).await?;
        return Ok(());
    }

    let result = if engine_configured {
        process_item(
            pool,
            &vision_settings,
            &processing_settings,
            cache_root,
            &item,
            app,
        )
        .await
    } else {
        capture_service::complete_processing_without_engine(pool, &item.id).await
    };
    match result {
        Ok(pending) => {
            capture_log(
                &pending,
                LogLevel::Info,
                "processing_completed",
                if engine_configured {
                    "succeeded"
                } else {
                    "degraded"
                },
                if engine_configured {
                    "capture processing completed"
                } else {
                    "capture processing completed without AI engine"
                },
                None,
            );
            if item.classification == "person" {
                let _ = recognition_service::enroll_face_sample(pool, &pending.id).await;
                let _ = recognition_service::suggest_from_face_bank(pool, &pending.id).await;
            }
            emit_item(app, &pending);
            archive_and_emit(pool, app, &pending).await?;
        }
        Err(error) => {
            capture_log(
                &item,
                LogLevel::Error,
                "processing_failed",
                "failed",
                "capture processing failed",
                Some("vision_processing_error"),
            );
            let failed = capture_service::mark_failed(
                pool,
                MarkCaptureFailedInput {
                    capture_item_id: item.id,
                    error_message: error.to_string(),
                },
            )
            .await?;
            emit_item(app, &failed);
        }
    }
    emit_runtime(pool, app).await?;
    Ok(())
}

/// Runs the feature-only Python pass for one awaiting-label capture. Any
/// outcome (feature, no face, or engine error) is recorded on the item so the
/// pass never re-picks it; suggestions are best-effort.
async fn process_awaiting_label_feature(
    pool: &SqlitePool,
    settings: &crate::models::vision::VisionSettings,
    processing_settings: &crate::models::vision::ProcessingSettings,
    app: &AppHandle,
) -> Result<bool, AppError> {
    let Some(item) = capture_service::next_awaiting_label_without_feature(pool).await? else {
        return Ok(false);
    };
    let item_id_for_progress = item.id.clone();
    let source_path_for_progress = item.source_path.clone();
    let progress_app = app.clone();
    let progress = Some(Box::new(move |stage: &str, percent: f64| {
        let _ = progress_app.emit(
            "capture:progress",
            json!({
                "captureItemId": item_id_for_progress,
                "sourcePath": source_path_for_progress,
                "stage": stage,
                "percent": percent,
            }),
        );
    }) as vision_engine_service::ProgressCallback);
    let response = match vision_engine_service::extract_face_feature(
        settings,
        &PathBuf::from(&item.source_path),
        processing_settings,
        progress,
    )
    .await
    {
        Ok(response) => response,
        Err(_error) => {
            // Engine failures are transient (broken model/env, missing
            // python, ...). Keep the item retryable instead of permanently
            // marking it "no face", and back off a minute so a broken engine
            // does not spin the worker every poll cycle. Only a successful
            // run that finds no face writes the "[]" marker below.
            capture_log(
                &item,
                LogLevel::Error,
                "prelabel_feature_failed",
                "retrying",
                "pre-label face feature extraction failed; retry scheduled",
                Some("vision_feature_error"),
            );
            sqlx::query(
                r#"
                UPDATE capture_items
                SET next_retry_at = strftime(
                    '%Y-%m-%dT%H:%M:%fZ', 'now', '+60 seconds'
                )
                WHERE id = ?
                "#,
            )
            .bind(&item.id)
            .execute(pool)
            .await?;
            return Ok(false);
        }
    };
    let feature_json = match response.face_feature {
        Some(feature) => serde_json::to_string(&feature)
            .map_err(|error| AppError::Vision(format!("cannot encode face feature: {error}")))?,
        None => "[]".to_owned(),
    };
    let face_box_json = response
        .face_box
        .map(|value| serde_json::to_string(&value))
        .transpose()
        .map_err(|error| AppError::Vision(format!("cannot encode face box: {error}")))?;
    capture_service::store_face_feature(
        pool,
        &item.id,
        capture_service::FaceFeatureWrite {
            feature_json: Some(feature_json),
            face_box_json,
            model_id: response.face_feature_model_id,
            model_version: response.face_feature_model_version,
            face_count: response.face_count,
            face_sharpness: response.face_sharpness,
            face_area_ratio: response.face_area_ratio,
        },
    )
    .await?;
    let _ = recognition_service::suggest_from_face_bank(pool, &item.id).await;
    // Pages only learn about the stored feature/suggestion through
    // item-updated; without it they keep the pre-feature snapshot (no face
    // count, no recommendation) until a reload. Broadcast the fresh item.
    let updated = capture_service::get_item(pool, &item.id).await?;
    emit_item(app, &updated);
    Ok(true)
}

async fn process_item(
    pool: &SqlitePool,
    settings: &crate::models::vision::VisionSettings,
    processing_settings: &crate::models::vision::ProcessingSettings,
    cache_root: &std::path::Path,
    item: &CaptureItem,
    app: &AppHandle,
) -> Result<CaptureItem, AppError> {
    let character_name: String = sqlx::query_scalar("SELECT name FROM characters WHERE id = ?")
        .bind(item.character_id.as_deref())
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::Validation("capture has no valid character".to_owned()))?;

    let output_directory = cache_root.join(&item.id);
    tokio::fs::create_dir_all(&output_directory).await?;
    let annotated = output_directory.join("annotated.png");
    let avatar = output_directory.join("avatar.png");
    let item_id_for_progress = item.id.clone();
    let source_path_for_progress = item.source_path.clone();
    let progress_app = app.clone();
    let progress = Some(Box::new(move |stage: &str, percent: f64| {
        let _ = progress_app.emit(
            "capture:progress",
            json!({
                "captureItemId": item_id_for_progress,
                "sourcePath": source_path_for_progress,
                "stage": stage,
                "percent": percent,
            }),
        );
    }) as vision_engine_service::ProgressCallback);
    let response = vision_engine_service::process_screenshot(
        settings,
        &PathBuf::from(&item.source_path),
        &annotated,
        &avatar,
        &character_name,
        processing_settings,
        progress,
    )
    .await?;

    let returned_annotated = response.annotated_path.as_deref().ok_or_else(|| {
        AppError::Vision("processing response is missing the annotated output path".to_owned())
    })?;
    verify_returned_path(returned_annotated, &annotated, "annotated output").await?;
    let avatar_path = match response.avatar_path.as_deref() {
        Some(path) => {
            verify_returned_path(path, &avatar, "avatar output").await?;
            Some(capture_service::path_to_string(&avatar))
        }
        None => None,
    };
    capture_service::complete_processing(
        pool,
        CompleteCaptureProcessingInput {
            capture_item_id: item.id.clone(),
            annotated_path: capture_service::path_to_string(&annotated),
            avatar_path,
            face_box_json: response
                .face_box
                .map(|value| serde_json::to_string(&value))
                .transpose()
                .map_err(|error| AppError::Vision(format!("cannot encode face box: {error}")))?,
            face_feature: response.face_feature,
            face_feature_model_id: response.face_feature_model_id,
            face_feature_model_version: response.face_feature_model_version,
            face_count: response.face_count,
            face_sharpness: response.face_sharpness,
            face_area_ratio: response.face_area_ratio,
            warnings_json: Some(serde_json::to_string(&response.warnings).map_err(|error| {
                AppError::Vision(format!("cannot encode processing warnings: {error}"))
            })?),
        },
    )
    .await
}

async fn verify_returned_path(
    returned: &str,
    expected: &std::path::Path,
    label: &str,
) -> Result<(), AppError> {
    let returned = tokio::fs::canonicalize(returned)
        .await
        .map_err(|error| AppError::Vision(format!("{label} is unavailable: {error}")))?;
    let expected = tokio::fs::canonicalize(expected)
        .await
        .map_err(|error| AppError::Vision(format!("{label} was not created: {error}")))?;
    if returned != expected {
        return Err(AppError::Vision(format!(
            "Python returned an unexpected {label} path"
        )));
    }
    Ok(())
}

pub async fn runtime_status(pool: &SqlitePool) -> Result<CaptureRuntimeStatus, AppError> {
    let settings = vision_settings_service::get(pool).await?;
    let active_capture: Option<(String, String)> = sqlx::query_as(
        "SELECT id, source_path FROM capture_items WHERE status = 'processing' ORDER BY updated_at LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    let (active_capture_item_id, active_capture_source_path) = active_capture
        .map(|(id, source_path)| (Some(id), Some(source_path)))
        .unwrap_or((None, None));
    let queued_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM capture_items WHERE status = 'queued'")
            .fetch_one(pool)
            .await?;
    let archive_pending_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM capture_items WHERE status = 'archive_pending'")
            .fetch_one(pool)
            .await?;
    let prelabel_pending_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM capture_items
        WHERE status = 'awaiting_label'
          AND recognition_deferred = 0
          AND NOT EXISTS (
              SELECT 1
              FROM capture_faces face
              WHERE face.capture_item_id = capture_items.id
                AND face.is_primary = 1
                AND face.feature_json IS NOT NULL
          )
        "#,
    )
    .fetch_one(pool)
    .await?;
    let last_error: Option<String> = sqlx::query_scalar(
        "SELECT error_message FROM capture_items WHERE error_message IS NOT NULL ORDER BY updated_at DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    Ok(CaptureRuntimeStatus {
        engine_status: if vision_settings_service::is_configured(&settings) {
            "configured".to_owned()
        } else {
            "unconfigured".to_owned()
        },
        worker_status: if active_capture_item_id.is_some() {
            "processing".to_owned()
        } else {
            "idle".to_owned()
        },
        active_capture_item_id,
        active_capture_source_path,
        queued_count,
        archive_pending_count,
        prelabel_pending_count,
        last_error,
    })
}

fn emit_item(app: &AppHandle, item: &CaptureItem) {
    let _ = app.emit("capture:item-updated", item);
}

async fn archive_and_emit(
    pool: &SqlitePool,
    app: &AppHandle,
    item: &CaptureItem,
) -> Result<(), AppError> {
    capture_log(
        item,
        LogLevel::Info,
        "archive_started",
        "started",
        "capture archive started",
        None,
    );
    match capture_archive_service::archive(
        pool,
        CaptureItemIdInput {
            capture_item_id: item.id.clone(),
        },
    )
    .await
    {
        Ok(completed) => {
            capture_log(
                &completed,
                LogLevel::Info,
                "archive_completed",
                "succeeded",
                "capture archive completed",
                None,
            );
            emit_item(app, &completed);
        }
        Err(error) => {
            let scheduled =
                capture_service::schedule_archive_retry(pool, &item.id, &error.to_string()).await?;
            capture_log(
                &scheduled,
                LogLevel::Warn,
                "archive_retry_scheduled",
                "retrying",
                "capture archive failed; retry scheduled",
                Some("archive_error"),
            );
            emit_item(app, &scheduled);
        }
    }
    Ok(())
}

fn capture_log(
    item: &CaptureItem,
    level: LogLevel,
    event: &str,
    outcome: &str,
    message: &str,
    error_code: Option<&str>,
) {
    log_service::record_event(LogRecord {
        level,
        module: "capture.worker".to_owned(),
        message: message.to_owned(),
        event: Some(event.to_owned()),
        project_id: Some(item.project_id.clone()),
        session_id: Some(item.session_id.clone()),
        capture_item_id: Some(item.id.clone()),
        attempt: u32::try_from(item.attempt_count).ok(),
        outcome: Some(outcome.to_owned()),
        error_code: error_code.map(str::to_owned),
        ..Default::default()
    });
}

async fn emit_runtime(pool: &SqlitePool, app: &AppHandle) -> Result<(), AppError> {
    let status = runtime_status(pool).await?;
    let _ = app.emit("capture:runtime-status", status);
    Ok(())
}
