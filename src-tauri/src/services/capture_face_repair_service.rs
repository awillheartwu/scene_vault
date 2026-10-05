//! Batch repair for close-up captures whose stored face box came from a
//! fragmented detection (the "big face" defect).
//!
//! The job is deliberately conservative: it probes each suspicious capture
//! with the engine without writing anything, re-queues only the ones where
//! the probe found a clearly more complete face, and leaves the actual
//! reprocessing (annotation, avatar, archive replacement) to the existing
//! capture worker.

use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter};

use crate::{
    error::AppError,
    models::vision::{ProcessingSettings, VisionSettings},
    services::{
        capture_roi_service, capture_service, log_service, processing_settings_service,
        vision_engine_service, vision_settings_service,
    },
};

/// Stored boxes at or above this confidence were reliably whole faces across
/// the library sweep; below it a capture is worth probing.
pub const LOW_CONFIDENCE_THRESHOLD: f64 = 0.85;
/// A probed box covering this much more area is a fragmented stored box.
pub const AREA_GROWTH_RATIO: f64 = 1.25;
/// The probed box must also be clearly more confident than the stored one.
pub const CONFIDENCE_GAIN: f64 = 0.08;

pub const PROGRESS_EVENT: &str = "capture:face-repair-progress";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FaceRepairCandidate {
    pub id: String,
    pub source_path: String,
    pub file_name: String,
    pub missing_face_box: bool,
    pub stored_width: i64,
    pub stored_height: i64,
    pub stored_confidence: f64,
}

/// A capture whose earlier reprocess could not replace its own previous
/// archive; the repair re-queues it directly instead of probing.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FaceRepairRetry {
    pub id: String,
    pub file_name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FaceRepairPreview {
    /// Low-confidence or missing boxes that need an engine probe.
    pub candidates: Vec<FaceRepairCandidate>,
    /// Stuck archive replacements that only need a re-queue.
    pub retries: Vec<FaceRepairRetry>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FaceRepairStatus {
    /// `idle` | `running` | `done` | `partial` | `failed`
    pub state: String,
    pub project_id: Option<String>,
    pub task_id: Option<String>,
    pub total: u32,
    pub processed: u32,
    pub requeued: u32,
    pub kept: u32,
    pub failed: u32,
    pub current_file: Option<String>,
    pub message: Option<String>,
}

impl FaceRepairStatus {
    fn idle() -> Self {
        Self {
            state: "idle".to_owned(),
            project_id: None,
            task_id: None,
            total: 0,
            processed: 0,
            requeued: 0,
            kept: 0,
            failed: 0,
            current_file: None,
            message: None,
        }
    }
}

static STATUS: OnceLock<Mutex<FaceRepairStatus>> = OnceLock::new();

fn cell() -> &'static Mutex<FaceRepairStatus> {
    STATUS.get_or_init(|| Mutex::new(FaceRepairStatus::idle()))
}

/// Latest repair snapshot; safe to poll from the UI at any time.
pub fn status() -> FaceRepairStatus {
    cell()
        .lock()
        .map(|value| value.clone())
        .unwrap_or_else(|_| FaceRepairStatus::idle())
}

/// Owns the slot from before the first await until the background job finishes.
/// Dropping a cancelled initialization/job releases it without touching a newer task.
struct Reservation<'a> {
    cell: &'a Mutex<FaceRepairStatus>,
    task_id: String,
}

impl<'a> Reservation<'a> {
    fn acquire(cell: &'a Mutex<FaceRepairStatus>, project_id: &str) -> Result<Self, AppError> {
        if project_id.trim().is_empty() {
            return Err(AppError::Validation(
                "project id cannot be empty".to_owned(),
            ));
        }
        let mut state = cell
            .lock()
            .map_err(|_| AppError::Conflict("repair status unavailable".to_owned()))?;
        if state.state == "running" {
            return Err(AppError::Conflict(
                "a face-box repair is already running".to_owned(),
            ));
        }
        let task_id = uuid::Uuid::new_v4().to_string();
        *state = FaceRepairStatus {
            state: "running".to_owned(),
            project_id: Some(project_id.trim().to_owned()),
            task_id: Some(task_id.clone()),
            ..FaceRepairStatus::idle()
        };
        Ok(Self { cell, task_id })
    }

    fn update(&self, update: impl FnOnce(&mut FaceRepairStatus)) -> Option<FaceRepairStatus> {
        let mut state = self.cell.lock().ok()?;
        if state.task_id.as_deref() != Some(&self.task_id) || state.state != "running" {
            return None;
        }
        update(&mut state);
        Some(state.clone())
    }

    fn publish(&self, app: &AppHandle, update: impl FnOnce(&mut FaceRepairStatus)) {
        if let Some(snapshot) = self.update(update) {
            emit(app, &snapshot);
        }
    }
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        self.update(|state| {
            state.state = "failed".to_owned();
            state.current_file = None;
            state.message = Some("修复任务已取消或意外中断，请重新核对。".to_owned());
        });
    }
}

fn emit(app: &AppHandle, value: &FaceRepairStatus) {
    let _ = app.emit(PROGRESS_EVENT, value);
}

/// Selects this project's completed person captures whose stored face box has
/// a low confidence (worth probing with the engine) plus the captures whose
/// earlier reprocess failed to replace their previous archive.
pub async fn preview(pool: &SqlitePool, project_id: &str) -> Result<FaceRepairPreview, AppError> {
    let project_id = project_id.trim();
    if project_id.is_empty() {
        return Err(AppError::Validation(
            "project id cannot be empty".to_owned(),
        ));
    }
    let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
        r#"
        SELECT id, source_path, face_box_json
        FROM capture_items
        WHERE project_id = ?
          AND classification = 'person'
          AND status = 'completed'
          AND source_file_state = 'available'
        ORDER BY captured_at ASC, created_at ASC, id ASC
        "#,
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?;

    let mut candidates = Vec::new();
    for (id, source_path, raw) in rows {
        let missing_face_box = raw.is_none();
        let value = match raw {
            Some(raw) => match serde_json::from_str::<serde_json::Value>(&raw) {
                Ok(value) => value,
                Err(_) => continue,
            },
            None => serde_json::Value::Null,
        };
        let confidence = value
            .get("confidence")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0);
        if confidence >= LOW_CONFIDENCE_THRESHOLD {
            continue;
        }
        let width = value
            .get("width")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);
        let height = value
            .get("height")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);
        if !missing_face_box && (width <= 0 || height <= 0) {
            continue;
        }
        candidates.push(FaceRepairCandidate {
            file_name: std::path::Path::new(&source_path)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| source_path.clone()),
            id,
            source_path,
            missing_face_box,
            stored_width: width,
            stored_height: height,
            stored_confidence: confidence,
        });
    }
    let retry_rows: Vec<(String, String)> = sqlx::query_as(
        r#"
        SELECT id, source_path
        FROM capture_items
        WHERE project_id = ?
          AND status = 'failed'
          AND failure_stage = 'archive'
          AND source_file_state = 'available'
          AND classification = 'person'
          AND (error_message LIKE '%archive destination already exists with unexpected content%'
               OR error_message LIKE '%缺少可信的历史内容指纹%')
        ORDER BY captured_at ASC, created_at ASC, id ASC
        "#,
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?;
    let retries = retry_rows
        .into_iter()
        .map(|(id, source_path)| FaceRepairRetry {
            file_name: std::path::Path::new(&source_path)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or(source_path),
            id,
        })
        .collect();

    Ok(FaceRepairPreview {
        candidates,
        retries,
    })
}

/// Starts a repair run in the background; only one run may be active at once.
pub async fn start(
    app: AppHandle,
    pool: SqlitePool,
    project_id: String,
) -> Result<FaceRepairStatus, AppError> {
    let reservation = Reservation::acquire(cell(), &project_id)?;
    let initialization = async {
        let preview = preview(&pool, &project_id).await?;
        let vision = vision_settings_service::get(&pool).await?;
        if !preview.candidates.is_empty() && !vision_settings_service::is_engine_available(&vision) {
            return Err(AppError::Vision(
                "vision engine is not configured; configure Python and models before repairing face boxes".to_owned(),
            ));
        }
        Ok((preview, vision))
    }.await;
    let (preview, vision) = match initialization {
        Ok(value) => value,
        Err(error) => {
            reservation.publish(&app, |state| {
                state.state = "failed".to_owned();
                state.message = Some(error.to_string());
            });
            return Err(error);
        }
    };
    let initial = reservation
        .update(|state| {
            state.total = (preview.candidates.len() + preview.retries.len()) as u32;
        })
        .ok_or_else(|| AppError::Conflict("repair reservation lost".to_owned()))?;
    emit(&app, &initial);
    tauri::async_runtime::spawn(run(
        app,
        pool,
        preview.candidates,
        preview.retries,
        vision,
        reservation,
    ));
    Ok(initial)
}

async fn run(
    app: AppHandle,
    pool: SqlitePool,
    candidates: Vec<FaceRepairCandidate>,
    retries: Vec<FaceRepairRetry>,
    vision: VisionSettings,
    reservation: Reservation<'static>,
) {
    let processing = match processing_settings_service::get(&pool).await {
        Ok(value) => value,
        Err(error) => {
            reservation.publish(&app, |state| {
                state.state = "failed".to_owned();
                state.message = Some(error.to_string());
            });
            return;
        }
    };
    for (index, candidate) in candidates.iter().enumerate() {
        reservation.publish(&app, |state| {
            state.current_file = Some(candidate.file_name.clone())
        });
        match repair_one(&pool, &vision, &processing, candidate).await {
            Ok(true) => reservation.publish(&app, |state| state.requeued += 1),
            Ok(false) => reservation.publish(&app, |state| state.kept += 1),
            Err(error) => {
                log_service::warn(
                    "capture.face_repair",
                    format!("face-box probe failed for {}: {error}", candidate.id),
                );
                reservation.publish(&app, |state| {
                    state.failed += 1;
                    state.message = Some(error.to_string());
                });
            }
        }
        reservation.publish(&app, |state| state.processed = index as u32 + 1);
    }
    let processed_candidates = candidates.len() as u32;
    for (index, retry) in retries.iter().enumerate() {
        reservation.publish(&app, |state| {
            state.current_file = Some(retry.file_name.clone())
        });
        match capture_service::retry_capture(
            &pool,
            crate::models::capture::RetryCaptureInput {
                capture_item_id: retry.id.clone(),
            },
        )
        .await
        {
            Ok(_) => reservation.publish(&app, |state| state.requeued += 1),
            Err(error) => {
                log_service::warn(
                    "capture.face_repair",
                    format!("archive retry failed for {}: {error}", retry.id),
                );
                reservation.publish(&app, |state| {
                    state.failed += 1;
                    state.message = Some(error.to_string());
                });
            }
        }
        reservation.publish(&app, |state| {
            state.processed = processed_candidates + index as u32 + 1
        });
    }
    reservation.publish(&app, |state| {
        state.state = if state.failed == 0 {
            "done"
        } else if state.failed == state.total {
            "failed"
        } else {
            "partial"
        }
        .to_owned();
        state.current_file = None;
    });
}

async fn repair_one(
    pool: &SqlitePool,
    vision: &VisionSettings,
    processing: &ProcessingSettings,
    candidate: &FaceRepairCandidate,
) -> Result<bool, AppError> {
    let item = capture_service::get_item(pool, &candidate.id).await?;
    if item.status != "completed" || item.classification != "person" {
        return Ok(false);
    }
    let source = capture_service::validate_source_identity(pool, &item).await?;
    let roi = capture_roi_service::get(pool, &candidate.id).await?;
    let mode =
        vision_engine_service::effective_big_face_mode(&item.face_detection_mode, processing);
    let response =
        vision_engine_service::probe_face_box(vision, &source, processing, roi.as_ref(), mode)
            .await?;
    apply_probe(pool, candidate, response.face_box).await
}

async fn apply_probe(
    pool: &SqlitePool,
    candidate: &FaceRepairCandidate,
    face_box: Option<serde_json::Value>,
) -> Result<bool, AppError> {
    let Some(probed) = face_box else {
        return Ok(false);
    };
    if !probe_needs_requeue(candidate, &probed) {
        return Ok(false);
    }
    capture_service::requeue_completed_capture(pool, &candidate.id).await?;
    Ok(true)
}

fn probe_needs_requeue(candidate: &FaceRepairCandidate, probed: &serde_json::Value) -> bool {
    // Missing old boxes have no area/confidence baseline. Require a valid,
    // detection before changing the completed capture. The engine has already
    // applied the configured detector threshold; 0.85 only screens OLD boxes.
    if candidate.missing_face_box {
        return ["width", "height"].iter().all(|key| {
            probed
                .get(key)
                .and_then(serde_json::Value::as_f64)
                .is_some_and(|value| value.is_finite() && value > 0.0)
        }) && ["x", "y"].iter().all(|key| {
            probed
                .get(key)
                .and_then(serde_json::Value::as_f64)
                .is_some_and(|value| value.is_finite() && value >= 0.0)
        }) && probed
            .get("confidence")
            .and_then(serde_json::Value::as_f64)
            .is_some_and(|value| value > 0.0 && value <= 1.0);
    }
    box_is_materially_larger(
        candidate.stored_width,
        candidate.stored_height,
        candidate.stored_confidence,
        probed,
    )
}

/// True when the probed box clearly holds the more complete face: larger by
/// area and clearly more confident than the stored one.
pub fn box_is_materially_larger(
    stored_width: i64,
    stored_height: i64,
    stored_confidence: f64,
    probed: &serde_json::Value,
) -> bool {
    let Some(width) = probed.get("width").and_then(serde_json::Value::as_f64) else {
        return false;
    };
    let Some(height) = probed.get("height").and_then(serde_json::Value::as_f64) else {
        return false;
    };
    let Some(confidence) = probed.get("confidence").and_then(serde_json::Value::as_f64) else {
        return false;
    };
    let stored_area = (stored_width.max(0) as f64) * (stored_height.max(0) as f64);
    if stored_area <= 0.0 || width <= 0.0 || height <= 0.0 {
        return false;
    }
    width * height >= AREA_GROWTH_RATIO * stored_area
        && confidence >= stored_confidence + CONFIDENCE_GAIN
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::models::capture::RegisterCaptureInput;
    use std::future::Future;

    #[test]
    fn concurrent_starts_have_one_owner_and_drop_releases_the_slot() {
        let cell = Mutex::new(FaceRepairStatus::idle());
        let barrier = std::sync::Barrier::new(8);
        let winners = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8)
                .map(|index| {
                    let cell = &cell;
                    let barrier = &barrier;
                    scope.spawn(move || {
                        barrier.wait();
                        let reservation = Reservation::acquire(cell, &format!("project-{index}"));
                        // Hold the winning reservation until all contenders tried.
                        barrier.wait();
                        reservation.is_ok()
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|handle| usize::from(handle.join().unwrap()))
                .sum::<usize>()
        });
        assert_eq!(winners, 1);
        assert_eq!(cell.lock().unwrap().state, "failed");
        assert!(Reservation::acquire(&cell, "next-project").is_ok());
    }

    #[tokio::test]
    async fn cancelled_initialization_releases_reservation() {
        let cell = Mutex::new(FaceRepairStatus::idle());
        let initialization = async {
            let _reservation = Reservation::acquire(&cell, "project-1").unwrap();
            std::future::pending::<()>().await;
        };
        let mut initialization = Box::pin(initialization);
        // Poll once: the reservation must already exist before the pending await.
        assert!(std::future::poll_fn(|cx| std::task::Poll::Ready(
            initialization.as_mut().poll(cx)
        ))
        .await
        .is_pending());
        assert!(Reservation::acquire(&cell, "project-2").is_err());
        drop(initialization);
        assert!(Reservation::acquire(&cell, "project-2").is_ok());
    }

    #[test]
    fn initialization_failure_and_late_updates_cannot_overwrite_next_task() {
        let cell = Mutex::new(FaceRepairStatus::idle());
        let old = Reservation::acquire(&cell, "project-1").unwrap();
        old.update(|state| {
            state.state = "failed".to_owned();
            state.message = Some("initialization error".to_owned());
        });
        let new = Reservation::acquire(&cell, "project-2").unwrap();
        assert!(old.update(|state| state.processed = 99).is_none());
        drop(old);
        let state = cell.lock().unwrap();
        assert_eq!(state.state, "running");
        assert_eq!(state.project_id.as_deref(), Some("project-2"));
        assert_eq!(state.task_id.as_deref(), Some(new.task_id.as_str()));
        assert_eq!(state.processed, 0);
    }

    #[test]
    fn missing_box_requires_a_valid_confident_probe() {
        let candidate = FaceRepairCandidate {
            id: "missing".to_owned(),
            source_path: "source".to_owned(),
            file_name: "source".to_owned(),
            missing_face_box: true,
            stored_width: 0,
            stored_height: 0,
            stored_confidence: 0.0,
        };
        for (width, confidence, expected) in [
            (100, 0.95, true),
            (0, 0.95, false),
            (100, 0.84, true),
            (100, 0.68, true),
            (100, 0.5, true),
            (100, 0.0, false),
            (100, 1.5, false),
        ] {
            assert_eq!(
                probe_needs_requeue(
                    &candidate,
                    &serde_json::json!({"x": 0, "y": 0, "width": width, "height": 120, "confidence": confidence})
                ),
                expected
            );
        }
        assert!(!probe_needs_requeue(&candidate, &serde_json::Value::Null));
    }

    #[test]
    fn requires_both_a_larger_area_and_a_confidence_jump() {
        let probed = |width: i64, height: i64, confidence: f64| serde_json::json!({ "width": width, "height": height, "confidence": confidence });
        // A clearly more complete face wins.
        assert!(box_is_materially_larger(
            500,
            600,
            0.63,
            &probed(1000, 1300, 0.93)
        ));
        // A larger box without the confidence jump is not enough.
        assert!(!box_is_materially_larger(
            500,
            600,
            0.63,
            &probed(1000, 1300, 0.66)
        ));
        // A confidence jump without a material size gain is not enough.
        assert!(!box_is_materially_larger(
            500,
            600,
            0.63,
            &probed(510, 610, 0.93)
        ));
        // Malformed probes are rejected.
        assert!(!box_is_materially_larger(
            500,
            600,
            0.63,
            &serde_json::json!({})
        ));
        assert!(!box_is_materially_larger(
            0,
            0,
            0.63,
            &probed(1000, 1300, 0.93)
        ));
    }

    #[tokio::test]
    async fn preview_keeps_only_low_confidence_completed_person_captures() {
        let pool = db::test_pool().await;
        let fixture = crate::services::test_support::project_with_directories(&pool, "Repair")
            .await
            .expect("project fixture");
        let session = crate::services::test_support::start_session(&pool, &fixture.project_id)
            .await
            .expect("session");

        let mut ids = Vec::new();
        for (name, confidence, classification, status) in [
            ("low.png", 0.63, "person", "completed"),
            ("high.png", 0.93, "person", "completed"),
            ("scene.png", 0.63, "scene", "completed"),
            ("queued.png", 0.63, "person", "queued"),
            ("stuck.png", 0.93, "person", "failed"),
            ("missing.png", 0.0, "person", "completed"),
            ("detection-failed.png", 0.0, "person", "failed"),
            ("unavailable.png", 0.0, "person", "completed"),
        ] {
            let source = fixture.source_directory.join(name);
            // Distinct bytes per file: captures are deduplicated by content hash.
            tokio::fs::write(&source, name.as_bytes())
                .await
                .expect("source");
            let item = capture_service::register_capture(
                &pool,
                RegisterCaptureInput {
                    session_id: session.id.clone(),
                    source_path: capture_service::path_to_string(&source),
                },
            )
            .await
            .expect("register");
            let box_json = serde_json::json!({
                "width": 500,
                "height": 600,
                "confidence": confidence,
            })
            .to_string();
            sqlx::query(
                "UPDATE capture_items SET classification = ?, status = ?, face_box_json = ? WHERE id = ?",
            )
            .bind(classification)
            .bind(status)
            .bind(box_json)
            .bind(&item.id)
            .execute(&pool)
            .await
            .expect("seed box");
            ids.push(item.id);
        }

        // A stuck archive replacement is retried directly, not probed.
        sqlx::query(
            "UPDATE capture_items SET failure_stage = 'archive', error_message = 'archive error: archive destination already exists with unexpected content: X' WHERE id = ?",
        )
        .bind(&ids[4])
        .execute(&pool)
        .await
        .expect("seed failure");

        sqlx::query("UPDATE capture_items SET face_box_json = NULL WHERE id IN (?, ?, ?)")
            .bind(&ids[5])
            .bind(&ids[6])
            .bind(&ids[7])
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE capture_items SET failure_stage = 'processing', error_message = 'no face detected' WHERE id = ?")
            .bind(&ids[6]).execute(&pool).await.unwrap();
        sqlx::query("UPDATE capture_items SET source_file_state = 'missing' WHERE id = ?")
            .bind(&ids[7])
            .execute(&pool)
            .await
            .unwrap();
        let other = crate::services::test_support::project_with_directories(&pool, "Other")
            .await
            .unwrap();
        let empty = preview(&pool, &other.project_id).await.unwrap();
        assert!(empty.candidates.is_empty() && empty.retries.is_empty());

        let candidates = preview(&pool, &fixture.project_id).await.expect("preview");
        assert_eq!(candidates.candidates.len(), 2);
        assert_eq!(candidates.candidates[0].id, ids[0]);
        assert_eq!(candidates.candidates[0].file_name, "low.png");
        assert!((candidates.candidates[0].stored_confidence - 0.63).abs() < f64::EPSILON);
        assert!(!candidates.candidates[0].missing_face_box);
        let missing = candidates
            .candidates
            .iter()
            .find(|entry| entry.id == ids[5])
            .unwrap();
        assert!(missing.missing_face_box);
        assert!(!apply_probe(&pool, missing, None).await.unwrap());
        assert!(
            !apply_probe(&pool, missing, Some(serde_json::json!({"width": 10})))
                .await
                .unwrap()
        );
        assert_eq!(
            capture_service::get_item(&pool, &missing.id)
                .await
                .unwrap()
                .status,
            "completed"
        );

        // Use a real database transition to verify the repair preserves the
        // user's role, manual selection, mode and previous archive references.
        let character_id = uuid::Uuid::new_v4().to_string();
        sqlx::query("INSERT INTO characters (id, project_id, name) VALUES (?, ?, 'Role')")
            .bind(&character_id)
            .bind(&fixture.project_id)
            .execute(&pool)
            .await
            .unwrap();
        let roi = r#"{"x":0.1,"y":0.1,"width":0.5,"height":0.5}"#;
        sqlx::query("UPDATE capture_items SET character_id = ?, manual_face_roi_json = ?, manual_face_roi_ready = 1, face_detection_mode = 'normalized', destination_path = 'old-archive.png' WHERE id = ?")
            .bind(&character_id).bind(roi).bind(&missing.id).execute(&pool).await.unwrap();
        assert!(apply_probe(
            &pool,
            missing,
            Some(
                serde_json::json!({"x": 0, "y": 0, "width": 100, "height": 120, "confidence": 0.95})
            )
        )
        .await
        .unwrap());
        let repaired = capture_service::get_item(&pool, &missing.id).await.unwrap();
        assert_eq!(repaired.status, "queued");
        assert_eq!(repaired.classification, "person");
        assert_eq!(
            repaired.character_id.as_deref(),
            Some(character_id.as_str())
        );
        assert_eq!(repaired.manual_face_roi_json.as_deref(), Some(roi));
        assert_eq!(repaired.face_detection_mode, "normalized");
        assert_eq!(
            repaired.destination_path.as_deref(),
            Some("old-archive.png")
        );
        assert_eq!(candidates.retries.len(), 1);
        assert_eq!(candidates.retries[0].id, ids[4]);
        assert_eq!(candidates.retries[0].file_name, "stuck.png");
    }
}
