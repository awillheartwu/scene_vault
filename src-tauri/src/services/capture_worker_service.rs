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
/// A source image that fails to decode or read this many times is treated as
/// permanently unusable by the pre-label pass (still retried as transient
/// before the budget is exhausted).
const MAX_PRELABEL_RETRY_ATTEMPTS: u32 = 3;
/// Upper bound of unarchived captures checked per missing-source sweep.
const MISSING_SOURCE_SWEEP_LIMIT: u32 = 200;

pub async fn run(pool: SqlitePool, app: AppHandle, cache_root: PathBuf) {
    let mut interval = tokio::time::interval(WORKER_POLL_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut cycle = 0_u32;
    loop {
        interval.tick().await;
        cycle = cycle.wrapping_add(1);
        // Every ~2 seconds, sweep unarchived captures whose source file was
        // deleted before archive and remove them as if they never appeared.
        if cycle.is_multiple_of(4) {
            if let Err(error) = sweep_missing_sources(&pool, &app).await {
                log_service::error(
                    "capture.worker",
                    format!("missing-source sweep failed: {error}"),
                );
            }
        }
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
    let engine_configured = vision_settings_service::is_engine_available(&vision_settings);
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
    // The source may have been deleted while the capture sat in the queue;
    // an unarchived capture with no usable source is removed entirely so it
    // never lingers in the recent list or retry paths. A same-named file
    // discovered later registers as a brand-new capture.
    match tokio::fs::metadata(&item.source_path).await {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => {
            purge_capture_and_emit(app, pool, &item, "source_missing").await?;
            emit_runtime(pool, app).await?;
            return Ok(());
        }
        Err(io_error) if io_error.kind() == std::io::ErrorKind::NotFound => {
            purge_capture_and_emit(app, pool, &item, "source_missing").await?;
            emit_runtime(pool, app).await?;
            return Ok(());
        }
        Err(_) => {}
    }
    if let Err(error) = capture_service::validate_source_identity(pool, &item).await {
        let failed = capture_service::mark_failed(
            pool,
            MarkCaptureFailedInput {
                capture_item_id: item.id.clone(),
                error_message: error.to_string(),
            },
        )
        .await?;
        capture_log(
            &failed,
            LogLevel::Warn,
            "source_identity_changed",
            "failed",
            "capture source no longer matches the registered content",
            Some("source_replaced"),
        );
        emit_item(app, &failed);
        emit_runtime(pool, app).await?;
        return Ok(());
    }
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
    let source = match capture_service::validate_source_identity(pool, &item).await {
        Ok(source) => source,
        Err(AppError::NotFound(_)) => {
            purge_capture_and_emit(app, pool, &item, "source_missing").await?;
            return Ok(false);
        }
        Err(_error) => {
            capture_log(
                &item,
                LogLevel::Warn,
                "prelabel_source_identity_changed",
                "skipped",
                "pre-label face feature extraction skipped because the source no longer matches the registered content",
                Some("source_replaced"),
            );
            schedule_prelabel_retry(pool, &item.id).await?;
            let updated = capture_service::get_item(pool, &item.id).await?;
            emit_item(app, &updated);
            return Ok(false);
        }
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
        &source,
        processing_settings,
        progress,
    )
    .await
    {
        Ok(response) => response,
        Err(error) => {
            // The source image no longer exists: retrying can never
            // succeed, and the capture was never archived, so remove it
            // entirely as if it never appeared. A same-named file
            // discovered later registers as a brand-new capture.
            if let Some(error_code) = vision_engine_service::permanent_source_error_code(&error) {
                // Re-check on disk: the engine may have raced a writer that
                // just created the file. Only purge when it is really gone.
                match tokio::fs::metadata(&item.source_path).await {
                    Err(io_error) if io_error.kind() == std::io::ErrorKind::NotFound => {
                        purge_capture_and_emit(app, pool, &item, error_code).await?;
                        return Ok(false);
                    }
                    _ => {}
                }
                capture_log(
                    &item,
                    LogLevel::Warn,
                    "prelabel_feature_failed",
                    "retrying",
                    "pre-label face feature extraction failed; retry scheduled",
                    Some(error_code),
                );
                schedule_prelabel_retry(pool, &item.id).await?;
                return Ok(false);
            }
            // The image exists but could not be read yet (still being
            // written, locked by the writer, ...). Give it a bounded number
            // of retries, then record the "attempted without a feature"
            // marker so the pre-label pass never re-picks the item.
            if let Some(error_code) = vision_engine_service::bounded_retry_source_error_code(&error)
            {
                let next_attempt = item.attempt_count.saturating_add(1);
                if next_attempt >= i64::from(MAX_PRELABEL_RETRY_ATTEMPTS) {
                    capture_log(
                        &item,
                        LogLevel::Warn,
                        "prelabel_feature_skipped",
                        "skipped",
                        "pre-label face feature extraction skipped: source image unreadable after repeated attempts",
                        Some(error_code),
                    );
                    capture_service::store_face_feature(
                        pool,
                        &item.id,
                        capture_service::FaceFeatureWrite {
                            feature_json: Some("[]".to_owned()),
                            face_box_json: None,
                            model_id: None,
                            model_version: None,
                            face_count: None,
                            face_sharpness: None,
                            face_area_ratio: None,
                        },
                    )
                    .await?;
                    let updated = capture_service::get_item(pool, &item.id).await?;
                    emit_item(app, &updated);
                    return Ok(false);
                }
                capture_log(
                    &item,
                    LogLevel::Error,
                    "prelabel_feature_failed",
                    "retrying",
                    "pre-label face feature extraction failed; retry scheduled",
                    Some(error_code),
                );
                schedule_prelabel_retry_with_attempt(pool, &item.id, next_attempt).await?;
                return Ok(false);
            }
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
            schedule_prelabel_retry(pool, &item.id).await?;
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

    let annotate_person = processing_settings.annotate_person.unwrap_or(true);
    let annotated_path = if annotate_person {
        let returned_annotated = response.annotated_path.as_deref().ok_or_else(|| {
            AppError::Vision("processing response is missing the annotated output path".to_owned())
        })?;
        verify_returned_path(returned_annotated, &annotated, "annotated output").await?;
        Some(capture_service::path_to_string(&annotated))
    } else {
        None
    };
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
            annotated_path,
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
          AND source_file_state NOT IN ('missing', 'replaced')
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
    let sidecar = vision_settings_service::sidecar_executable();
    log_service::info(
        "vision.engine",
        format!(
            "runtime status: configured={} sidecar={sidecar:?} current_exe={:?}",
            vision_settings_service::is_configured(&settings),
            std::env::current_exe(),
        ),
    );
    Ok(CaptureRuntimeStatus {
        engine_status: if vision_settings_service::is_configured(&settings) {
            "configured".to_owned()
        } else if sidecar.is_some() {
            "sidecar".to_owned()
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
    // Validate again immediately before archive. This protects both direct
    // source archives and derived outputs from same-name source replacement.
    match capture_service::validate_source_identity(pool, item).await {
        Ok(_) => {}
        Err(AppError::NotFound(_)) => {
            purge_capture_and_emit(app, pool, item, "source_missing").await?;
            return Ok(());
        }
        Err(error) => {
            let scheduled =
                capture_service::schedule_archive_retry(pool, &item.id, &error.to_string()).await?;
            emit_item(app, &scheduled);
            return Ok(());
        }
    }
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

/// Removes an unarchived capture from the database and notifies every open
/// window so it disappears from recent captures, queues and workbenches.
async fn purge_capture_and_emit(
    app: &AppHandle,
    pool: &SqlitePool,
    item: &CaptureItem,
    error_code: &str,
) -> Result<(), AppError> {
    let removed = capture_service::purge_capture_item(pool, &item.id, &item.status).await?;
    if removed {
        capture_log(
            item,
            LogLevel::Warn,
            "capture_purged",
            "purged",
            "source image was deleted before archive; capture removed",
            Some(error_code),
        );
        let _ = app.emit("capture:item-purged", json!({ "captureItemId": item.id }));
    }
    Ok(())
}

async fn schedule_prelabel_retry(pool: &SqlitePool, capture_item_id: &str) -> Result<(), AppError> {
    sqlx::query(
        r#"
        UPDATE capture_items
        SET next_retry_at = strftime(
            '%Y-%m-%dT%H:%M:%fZ', 'now', '+60 seconds'
        )
        WHERE id = ?
        "#,
    )
    .bind(capture_item_id)
    .execute(pool)
    .await?;
    Ok(())
}

async fn schedule_prelabel_retry_with_attempt(
    pool: &SqlitePool,
    capture_item_id: &str,
    attempt_count: i64,
) -> Result<(), AppError> {
    sqlx::query(
        r#"
        UPDATE capture_items
        SET
            attempt_count = ?,
            next_retry_at = strftime(
                '%Y-%m-%dT%H:%M:%fZ', 'now', '+60 seconds'
            )
        WHERE id = ?
        "#,
    )
    .bind(attempt_count)
    .bind(capture_item_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Bounded sweep that removes unarchived captures whose source file no
/// longer exists (deleted before archive). Newest first so items visible in
/// the recent list are purged promptly; transient IO errors are ignored so
/// a temporarily unreadable disk never destroys rows.
async fn sweep_missing_sources(pool: &SqlitePool, app: &AppHandle) -> Result<(), AppError> {
    let items = sqlx::query_as::<_, (String, String, String, String, String)>(
        r#"
        SELECT id, project_id, session_id, source_path, status
        FROM capture_items
        WHERE archived_at IS NULL
          AND status NOT IN ('processing', 'completed')
        ORDER BY updated_at DESC
        LIMIT ?
        "#,
    )
    .bind(i64::from(MISSING_SOURCE_SWEEP_LIMIT))
    .fetch_all(pool)
    .await?;
    for (id, project_id, session_id, source_path, status) in items {
        let missing = match tokio::fs::metadata(&source_path).await {
            Ok(metadata) => !metadata.is_file(),
            Err(io_error) => io_error.kind() == std::io::ErrorKind::NotFound,
        };
        if !missing {
            continue;
        }
        let removed = capture_service::purge_capture_item(pool, &id, &status).await?;
        if !removed {
            continue;
        }
        log_service::record_event(LogRecord {
            level: LogLevel::Warn,
            module: "capture.worker".to_owned(),
            message: "source image was deleted before archive; capture removed".to_owned(),
            event: Some("capture_purged".to_owned()),
            project_id: Some(project_id),
            session_id: Some(session_id),
            capture_item_id: Some(id.clone()),
            outcome: Some("purged".to_owned()),
            error_code: Some("source_missing".to_owned()),
            ..Default::default()
        });
        let _ = app.emit("capture:item-purged", json!({ "captureItemId": id }));
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
