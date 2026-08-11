use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    error::AppError,
    models::capture::{
        CaptureHistoryEntry, CaptureHistoryPage, CaptureItem, CaptureItemIdInput, CaptureSession,
        ClassifyPopupContext, CompleteCaptureProcessingInput, EndCaptureSessionInput,
        ImportDirectoryCapturesInput, LabelCaptureInput, ListCaptureHistoryInput,
        ListCategoryItemsInput, MarkCaptureFailedInput, RegisterCaptureInput, RelabelCaptureInput,
        RetryCaptureInput, SessionSourceDirectory, StartCaptureSessionInput,
        StartCaptureSessionResult, UnimportedCapture,
    },
    models::character::Character,
};

#[derive(Debug)]
struct BaselineFile {
    source_path: String,
    file_size: i64,
    modified_at_ms: Option<i64>,
}

pub(crate) struct FaceFeatureWrite {
    pub feature_json: Option<String>,
    pub face_box_json: Option<String>,
    pub model_id: Option<String>,
    pub model_version: Option<String>,
    pub face_count: Option<i64>,
    pub face_sharpness: Option<f64>,
    pub face_area_ratio: Option<f64>,
}

pub(crate) const CAPTURE_STATUSES: [&str; 6] = [
    "awaiting_label",
    "queued",
    "processing",
    "archive_pending",
    "completed",
    "failed",
];

const MAX_HISTORY_LIMIT: u32 = 1_000;
const DEFAULT_HISTORY_LIMIT: u32 = 200;
const MAX_ERROR_MESSAGE_CHARS: usize = 4_000;
const REGISTER_STABILITY_DELAY_MS: u64 = 200;

pub async fn start_session(
    pool: &SqlitePool,
    input: StartCaptureSessionInput,
) -> Result<StartCaptureSessionResult, AppError> {
    let project_id = required(&input.project_id, "capture project id")?;

    let project: (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT destination_directory, last_destination_directory FROM projects WHERE id = ?",
    )
    .bind(project_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound("capture project".to_owned()))?;
    let (destination, fallback_destination) = project;
    let destination_input = destination
        .or(fallback_destination)
        .ok_or_else(|| AppError::Validation("请先配置项目的归档目录".to_owned()))?;

    let source_directories: Vec<String> = sqlx::query_scalar(
        r#"
        SELECT directory
        FROM project_source_directories
        WHERE project_id = ? AND enabled = 1
        ORDER BY created_at ASC
        "#,
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?;
    if source_directories.is_empty() {
        return Err(AppError::Validation(
            "请先为项目添加至少一个截图源目录".to_owned(),
        ));
    }

    let mut canonical_sources = Vec::new();
    for source in &source_directories {
        canonical_sources.push(canonical_existing_directory(Path::new(source)).await?);
    }

    let destination_directory = PathBuf::from(destination_input);
    if !destination_directory.is_absolute() {
        return Err(AppError::Validation(
            "capture destination directory must be an absolute path".to_owned(),
        ));
    }
    if let Ok(metadata) = tokio::fs::metadata(&destination_directory).await {
        if !metadata.is_dir() {
            return Err(AppError::Validation(
                "capture destination path exists but is not a directory".to_owned(),
            ));
        }
    }
    for source in &canonical_sources {
        if path_is_within(&destination_directory, source) {
            return Err(AppError::Validation(
                "capture destination directory cannot be inside a source directory".to_owned(),
            ));
        }
    }

    // Snapshot the baseline of every source directory before the session
    // starts so pre-existing images are not registered as new captures.
    let mut baselines = Vec::new();
    for source in &canonical_sources {
        let baseline_files = snapshot_existing_images(source).await?;
        baselines.push((source.clone(), baseline_files));
    }

    let discovery_started_at_ms = unix_millis(SystemTime::now())?;
    let mut transaction = pool.begin().await?;
    // Only one project listens at a time. Ending any other active session
    // here keeps stale watchers (for example a session left running since
    // yesterday) from accumulating when the user switches projects.
    let stopped_projects: Vec<String> = sqlx::query_scalar(
        r#"
        SELECT project.name
        FROM capture_sessions session
        JOIN projects project ON project.id = session.project_id
        WHERE session.status = 'active'
        "#,
    )
    .fetch_all(&mut *transaction)
    .await?;
    sqlx::query(
        r#"
        UPDATE capture_sessions
        SET
            status = 'cancelled',
            ended_at = COALESCE(ended_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE status = 'active'
        "#,
    )
    .execute(&mut *transaction)
    .await?;
    let session = sqlx::query_as::<_, CaptureSession>(
        r#"
        INSERT INTO capture_sessions (id, project_id)
        VALUES (?, ?)
        RETURNING
            id, project_id, status, started_at, ended_at, created_at, updated_at
        "#,
    )
    .bind(Uuid::new_v4().to_string())
    .bind(project_id)
    .fetch_one(&mut *transaction)
    .await?;

    for (source, baseline_files) in &baselines {
        sqlx::query(
            r#"
            INSERT INTO session_source_directories (
                session_id, directory, enabled, discovery_started_at_ms, baseline_initialized
            )
            VALUES (?, ?, 1, ?, 1)
            "#,
        )
        .bind(&session.id)
        .bind(path_to_string(source))
        .bind(discovery_started_at_ms)
        .execute(&mut *transaction)
        .await?;
        for baseline in baseline_files {
            sqlx::query(
                r#"
                INSERT INTO capture_session_baseline_files (
                    session_id, source_path, file_size, modified_at_ms
                )
                VALUES (?, ?, ?, ?)
                "#,
            )
            .bind(&session.id)
            .bind(&baseline.source_path)
            .bind(baseline.file_size)
            .bind(baseline.modified_at_ms)
            .execute(&mut *transaction)
            .await?;
        }
    }

    // Bump updated_at so the most recently used project surfaces first.
    sqlx::query(
        r#"
        UPDATE projects
        SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ?
        "#,
    )
    .bind(project_id)
    .execute(&mut *transaction)
    .await?;

    transaction.commit().await?;

    Ok(StartCaptureSessionResult {
        session,
        stopped_projects,
    })
}

/// Returns the source directories a session currently watches.
pub async fn session_source_directories(
    pool: &SqlitePool,
    session_id: &str,
) -> Result<Vec<SessionSourceDirectory>, AppError> {
    let directories = sqlx::query_as::<_, SessionSourceDirectory>(
        r#"
        SELECT session_id, directory, enabled, discovery_started_at_ms, baseline_initialized
        FROM session_source_directories
        WHERE session_id = ?
        ORDER BY directory ASC
        "#,
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;
    Ok(directories)
}

/// Recent capture items across all active sessions of a project, for the
/// capture page strip.
pub async fn list_project_recent_items(
    pool: &SqlitePool,
    project_id: &str,
    limit: i64,
) -> Result<Vec<CaptureItem>, AppError> {
    let items = sqlx::query_as::<_, CaptureItem>(
        r#"
        SELECT
            item.id, item.project_id, item.session_id, item.asset_id,
            item.character_id, item.classification, item.source_path, item.file_size, item.modified_at_ms, item.content_hash,
            item.file_size, item.modified_at_ms, item.content_hash,
            item.annotated_path, item.avatar_path, item.destination_path,
            item.destination_avatar_path, item.status, item.face_box_json, item.face_count,
            item.suggested_character_id, item.recognition_confidence,
            item.recognition_source, item.review_status,
            item.error_message, item.failure_stage, item.attempt_count,
            item.next_retry_at, item.processing_warnings_json,
            item.captured_at, item.processed_at, item.archived_at,
            item.created_at, item.updated_at
        FROM capture_items item
        JOIN capture_sessions session ON session.id = item.session_id
        WHERE session.project_id = ? AND session.status = 'active'
        ORDER BY item.captured_at DESC, item.created_at DESC
        LIMIT ?
        "#,
    )
    .bind(project_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(items)
}

pub async fn list_sessions(
    pool: &SqlitePool,
    project_id: Option<&str>,
) -> Result<Vec<CaptureSession>, AppError> {
    let sessions = if let Some(project_id) = project_id {
        let project_id = required(project_id, "capture project id")?;
        sqlx::query_as::<_, CaptureSession>(
            r#"
            SELECT
                id, project_id, status, started_at, ended_at, created_at, updated_at
            FROM capture_sessions
            WHERE project_id = ?
            ORDER BY started_at DESC
            "#,
        )
        .bind(project_id)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, CaptureSession>(
            r#"
            SELECT
                id, project_id, status, started_at, ended_at, created_at, updated_at
            FROM capture_sessions
            ORDER BY started_at DESC
            "#,
        )
        .fetch_all(pool)
        .await?
    };

    Ok(sessions)
}

/// Resolves which stored file a display variant points at.
pub fn image_path(item: &CaptureItem, variant: &str) -> Result<PathBuf, AppError> {
    let path = match variant {
        "source" => Some(item.source_path.clone()),
        "annotated" => item.annotated_path.clone(),
        "avatar" => item.avatar_path.clone(),
        "destination" => item.destination_path.clone(),
        _ => {
            return Err(AppError::Validation(
                "unsupported capture image variant".to_owned(),
            ))
        }
    }
    .ok_or_else(|| AppError::NotFound("capture image".to_owned()))?;
    Ok(PathBuf::from(path))
}

pub async fn end_session(
    pool: &SqlitePool,
    input: EndCaptureSessionInput,
) -> Result<CaptureSession, AppError> {
    let session_id = required(&input.session_id, "capture session id")?;
    let outcome = input
        .status
        .as_deref()
        .unwrap_or("completed")
        .trim()
        .to_ascii_lowercase();
    if !matches!(outcome.as_str(), "completed" | "cancelled") {
        return Err(AppError::Validation(
            "capture session end status must be completed or cancelled".to_owned(),
        ));
    }

    if let Some(session) = sqlx::query_as::<_, CaptureSession>(
        r#"
        UPDATE capture_sessions
        SET
            status = ?,
            ended_at = COALESCE(ended_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ? AND status = 'active'
        RETURNING
            id, project_id, status, started_at, ended_at, created_at, updated_at
        "#,
    )
    .bind(&outcome)
    .bind(session_id)
    .fetch_optional(pool)
    .await?
    {
        return Ok(session);
    }

    let session = get_session(pool, session_id).await?;
    if session.status == outcome {
        return Ok(session);
    }
    Err(AppError::Conflict(format!(
        "capture session already ended as {}",
        session.status
    )))
}

pub async fn list_items(pool: &SqlitePool, session_id: &str) -> Result<Vec<CaptureItem>, AppError> {
    let session_id = required(session_id, "capture session id")?;
    let session_exists: i64 =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM capture_sessions WHERE id = ?)")
            .bind(session_id)
            .fetch_one(pool)
            .await?;
    if session_exists == 0 {
        return Err(AppError::NotFound("capture session".to_owned()));
    }

    let items = sqlx::query_as::<_, CaptureItem>(
        r#"
        SELECT
            id, project_id, session_id, asset_id, character_id, classification, source_path, file_size, modified_at_ms, content_hash,
            annotated_path, avatar_path, destination_path,
            destination_avatar_path, status, face_box_json, face_count,
            suggested_character_id, recognition_confidence, recognition_source,
            review_status, error_message,
            failure_stage, attempt_count, next_retry_at, processing_warnings_json,
            captured_at, processed_at, archived_at, created_at, updated_at
        FROM capture_items
        WHERE session_id = ?
        ORDER BY captured_at DESC, created_at DESC
        "#,
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;
    Ok(items)
}

pub async fn classify_popup_context(pool: &SqlitePool) -> Result<ClassifyPopupContext, AppError> {
    // A shortcut popup belongs to the project currently being monitored.
    // Starting another project ends the previous active session, so this is
    // the single authoritative project context for the global shortcut.
    let project: Option<(String, String)> = sqlx::query_as(
        r#"
        SELECT project.id, project.name
        FROM capture_sessions session
        JOIN projects project ON project.id = session.project_id
        WHERE session.status = 'active'
        ORDER BY session.started_at DESC, session.created_at DESC
        LIMIT 1
        "#,
    )
    .fetch_optional(pool)
    .await?;
    let Some((project_id, project_name)) = project else {
        return Ok(ClassifyPopupContext {
            items: Vec::new(),
            project_id: None,
            project_name: None,
            characters: Vec::new(),
        });
    };

    let items = sqlx::query_as::<_, CaptureItem>(
        r#"
        SELECT
            item.id, item.project_id, item.session_id, item.asset_id, item.character_id,
            item.classification, item.source_path, item.file_size, item.modified_at_ms, item.content_hash,
            item.annotated_path, item.avatar_path, item.destination_path,
            item.destination_avatar_path, item.status, item.face_box_json, item.face_count,
            item.suggested_character_id, item.recognition_confidence,
            item.recognition_source, item.review_status,
            item.error_message, item.failure_stage, item.attempt_count,
            item.next_retry_at, item.processing_warnings_json,
            item.captured_at, item.processed_at, item.archived_at,
            item.created_at, item.updated_at
        FROM capture_items item
        WHERE item.status = 'awaiting_label'
          AND item.project_id = ?
        ORDER BY item.captured_at ASC, item.created_at ASC
        LIMIT 50
        "#,
    )
    .bind(&project_id)
    .fetch_all(pool)
    .await?;

    let characters: Vec<Character> = sqlx::query_as::<_, Character>(
        r#"
            SELECT id, project_id, name, aliases_json, avatar_asset_id, created_at, updated_at
            FROM characters
            WHERE project_id = ?
            ORDER BY name COLLATE NOCASE ASC
            "#,
    )
    .bind(&project_id)
    .fetch_all(pool)
    .await?;

    Ok(ClassifyPopupContext {
        items,
        project_id: Some(project_id),
        project_name: Some(project_name),
        characters,
    })
}

pub async fn list_history(
    pool: &SqlitePool,
    input: ListCaptureHistoryInput,
) -> Result<CaptureHistoryPage, AppError> {
    let project_id = required(&input.project_id, "capture project id")?;
    let status = input
        .status
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());
    if status
        .as_deref()
        .is_some_and(|value| !CAPTURE_STATUSES.contains(&value))
    {
        return Err(AppError::Validation(
            "unsupported capture history status".to_owned(),
        ));
    }
    let session_id = input
        .session_id
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let character_id = input
        .character_id
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let limit = input
        .limit
        .unwrap_or(DEFAULT_HISTORY_LIMIT)
        .clamp(1, MAX_HISTORY_LIMIT) as i64;
    let offset = i64::from(input.offset.unwrap_or(0)).clamp(0, 1_000_000);
    let include_private = input.include_private.unwrap_or(false);

    let total: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM capture_items item
        JOIN capture_sessions session ON session.id = item.session_id
        WHERE session.project_id = ?
          AND (? IS NULL OR item.session_id = ?)
          AND (? IS NULL OR item.character_id = ?)
          AND (? IS NULL OR item.status = ?)
          AND (? = 1 OR item.classification IS NULL OR item.classification != 'private')
        "#,
    )
    .bind(project_id)
    .bind(session_id.as_deref())
    .bind(session_id.as_deref())
    .bind(character_id.as_deref())
    .bind(character_id.as_deref())
    .bind(status.as_deref())
    .bind(status.as_deref())
    .bind(include_private)
    .fetch_one(pool)
    .await?;

    let entries = sqlx::query_as::<_, CaptureHistoryEntry>(
        r#"
        SELECT
            item.id,
            item.session_id,
            session.project_id,
            project.name AS project_name,
            session.status AS session_status,
            item.character_id,
            item.classification,
            character.name AS character_name,
            item.asset_id,
            item.source_path,
            item.annotated_path,
            item.avatar_path,
            item.destination_path,
            item.destination_avatar_path,
            item.status,
            item.face_box_json,
            item.suggested_character_id,
            item.recognition_confidence,
            item.recognition_source,
            item.review_status,
            item.error_message,
            item.failure_stage,
            item.attempt_count,
            item.next_retry_at,
            item.processing_warnings_json,
            item.captured_at,
            item.processed_at,
            item.archived_at
        FROM capture_items item
        JOIN capture_sessions session ON session.id = item.session_id
        JOIN projects project ON project.id = session.project_id
        LEFT JOIN characters character ON character.id = item.character_id
        WHERE session.project_id = ?
          AND (? IS NULL OR item.session_id = ?)
          AND (? IS NULL OR item.character_id = ?)
          AND (? IS NULL OR item.status = ?)
          AND (? = 1 OR item.classification IS NULL OR item.classification != 'private')
        ORDER BY item.captured_at DESC, item.created_at DESC
        LIMIT ?
        OFFSET ?
        "#,
    )
    .bind(project_id)
    .bind(session_id.as_deref())
    .bind(session_id.as_deref())
    .bind(character_id.as_deref())
    .bind(character_id.as_deref())
    .bind(status.as_deref())
    .bind(status.as_deref())
    .bind(include_private)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;
    Ok(CaptureHistoryPage { entries, total })
}

pub async fn register_capture(
    pool: &SqlitePool,
    input: RegisterCaptureInput,
) -> Result<CaptureItem, AppError> {
    let session_id = required(&input.session_id, "capture session id")?;
    let source_path = required(&input.source_path, "capture source path")?;
    let session = require_active_session(pool, session_id).await?;
    let session_dirs = session_directories(pool, &session).await?;
    let source_path = canonical_capture_path(&session_dirs, Path::new(source_path)).await?;
    let metadata = tokio::fs::metadata(&source_path).await?;
    if matches_discovery_baseline(pool, session_id, &source_path, &metadata).await? {
        return Err(AppError::Validation(
            "capture source belongs to the session discovery baseline".to_owned(),
        ));
    }
    verify_stable_file(
        &source_path,
        Duration::from_millis(REGISTER_STABILITY_DELAY_MS),
    )
    .await?;
    // Do not accept a capture after its session was ended during the
    // stability check.
    require_active_session(pool, session_id).await?;
    let (item, _) = register_discovered_path(pool, session_id, &source_path, false).await?;
    Ok(item)
}

pub async fn label_capture(
    pool: &SqlitePool,
    input: LabelCaptureInput,
) -> Result<CaptureItem, AppError> {
    let capture_item_id = required(&input.capture_item_id, "capture item id")?;
    let classification = input
        .classification
        .as_deref()
        .unwrap_or("person")
        .trim()
        .to_ascii_lowercase();
    if !matches!(classification.as_str(), "person" | "scene" | "private") {
        return Err(AppError::Validation(
            "classification must be person, scene, or private".to_owned(),
        ));
    }
    let character_id = if classification == "person" {
        let raw = input.character_id.as_deref().ok_or_else(|| {
            AppError::Validation("character id is required for person captures".to_owned())
        })?;
        Some(required(raw, "character id")?.to_owned())
    } else {
        input
            .character_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };

    let mut transaction = pool.begin().await?;
    if let Some(character_id) = character_id.as_deref() {
        let belongs_to_project: i64 = sqlx::query_scalar(
            r#"
            SELECT EXISTS (
                SELECT 1
                FROM capture_items item
                JOIN capture_sessions session ON session.id = item.session_id
                JOIN characters character
                    ON character.id = ? AND character.project_id = session.project_id
                WHERE item.id = ?
            )
            "#,
        )
        .bind(character_id)
        .bind(capture_item_id)
        .fetch_one(&mut *transaction)
        .await?;

        if belongs_to_project == 0 {
            return Err(AppError::Validation(
                "character and capture item must belong to the same project".to_owned(),
            ));
        }
    }

    let item = sqlx::query_as::<_, CaptureItem>(
        r#"
        UPDATE capture_items
        SET
            character_id = ?,
            classification = ?,
            status = 'queued',
            error_message = NULL,
            failure_stage = NULL,
            attempt_count = 0,
            next_retry_at = NULL,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ? AND status IN ('awaiting_label', 'failed')
        RETURNING
            id, project_id, session_id, asset_id, character_id, classification, source_path, file_size, modified_at_ms, content_hash,
            annotated_path, avatar_path, destination_path,
            destination_avatar_path, status, face_box_json, face_count,
            suggested_character_id, recognition_confidence, recognition_source,
            review_status, error_message,
            failure_stage, attempt_count, next_retry_at, processing_warnings_json,
            captured_at, processed_at, archived_at, created_at, updated_at
        "#,
    )
    .bind(character_id.clone())
    .bind(&classification)
    .bind(capture_item_id)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or_else(|| {
        AppError::Conflict(
            "capture item can only be labelled while awaiting input or after failure".to_owned(),
        )
    })?;

    clear_private_project_cover_tx(&mut transaction, capture_item_id, &classification).await?;

    transaction.commit().await?;
    // Closed-set verification: after a person label, compare the capture's
    // face against the selected character's samples, persist the result, and
    // seed the sample immediately when the feature is already available
    // (UPSERT by character + capture, so a later processing pass only
    // refreshes it). The enrollment step reads verification_status to decide
    // whether the sample participates in matching.
    if classification == "person" {
        let character_id = character_id.as_deref().unwrap_or_default();
        let result =
            crate::services::recognition_service::compute_verification(pool, &item, character_id)
                .await?;
        crate::services::recognition_service::persist_verification(pool, &item.id, &result).await?;
        let _ = crate::services::recognition_service::enroll_face_sample(pool, &item.id).await;
        // Auto-resolve a pending face-bank suggestion when the chosen
        // character matches it: the label already answered the suggestion, so
        // the workbench must not keep it pending. A contradicting suggestion
        // stays pending as a mislabel hint.
        if !character_id.is_empty() {
            sqlx::query(
                r#"
                UPDATE capture_items
                SET review_status = 'accepted'
                WHERE id = ?
                  AND suggested_character_id = ?
                  AND review_status = 'pending'
                "#,
            )
            .bind(capture_item_id)
            .bind(character_id)
            .execute(pool)
            .await?;
        }
    } else {
        // Non-person labels make the suggestion moot.
        sqlx::query(
            r#"
            UPDATE capture_items
            SET suggested_character_id = NULL,
                recognition_confidence = NULL,
                recognition_source = NULL,
                review_status = 'none'
            WHERE id = ?
              AND suggested_character_id IS NOT NULL
            "#,
        )
        .bind(capture_item_id)
        .execute(pool)
        .await?;
    }
    let item = get_item(pool, capture_item_id).await?;
    Ok(item)
}

/// Changes the character and/or classification of an existing capture item.
/// The workbench entry point for corrections: `awaiting_label`/`queued`/
/// `failed` items are updated in place (and re-queued when they were failed),
/// while a `completed` item is re-queued for full reprocessing. Its previous
/// archive paths are kept on the item so the archive step can remove the old
/// NAS files once the replacement archive succeeds. In-flight items
/// (`processing`/`archive_pending`) are rejected.
pub async fn relabel_capture_item(
    pool: &SqlitePool,
    input: RelabelCaptureInput,
) -> Result<CaptureItem, AppError> {
    relabel_capture_item_inner(pool, input, None).await
}

/// Applies the same correction workflow as a manual relabel, but only while
/// the exact pending suggestion the user accepted is still current. This
/// optimistic guard prevents a refreshed/replaced suggestion from being
/// consumed by a stale workbench action.
pub(crate) async fn relabel_capture_item_from_suggestion(
    pool: &SqlitePool,
    input: RelabelCaptureInput,
    expected_suggested_character_id: &str,
) -> Result<CaptureItem, AppError> {
    relabel_capture_item_inner(pool, input, Some(expected_suggested_character_id)).await
}

async fn relabel_capture_item_inner(
    pool: &SqlitePool,
    input: RelabelCaptureInput,
    expected_suggested_character_id: Option<&str>,
) -> Result<CaptureItem, AppError> {
    let capture_item_id = required(&input.capture_item_id, "capture item id")?;
    let classification = input
        .classification
        .as_deref()
        .unwrap_or("person")
        .trim()
        .to_ascii_lowercase();
    if !matches!(classification.as_str(), "person" | "scene" | "private") {
        return Err(AppError::Validation(
            "classification must be person, scene, or private".to_owned(),
        ));
    }
    let character_id = if classification == "person" {
        let raw = input.character_id.as_deref().ok_or_else(|| {
            AppError::Validation("character id is required for person captures".to_owned())
        })?;
        Some(required(raw, "character id")?.to_owned())
    } else {
        // Scene/private captures are outside the person dimension: a relabel
        // away from person always clears the character attribution and its
        // face-bank sample (the workbench only lists person captures).
        None
    };

    let current = get_item(pool, capture_item_id).await?;
    if let Some(character_id) = character_id.as_deref() {
        let belongs_to_project: i64 = sqlx::query_scalar(
            r#"
            SELECT EXISTS (
                SELECT 1
                FROM capture_items item
                JOIN capture_sessions session ON session.id = item.session_id
                JOIN characters character
                    ON character.id = ? AND character.project_id = session.project_id
                WHERE item.id = ?
            )
            "#,
        )
        .bind(character_id)
        .bind(capture_item_id)
        .fetch_one(pool)
        .await?;
        if belongs_to_project == 0 {
            return Err(AppError::Validation(
                "character and capture item must belong to the same project".to_owned(),
            ));
        }
    }

    // Nothing changed: keep the item untouched, including its current status.
    if expected_suggested_character_id.is_none()
        && current.character_id.as_deref() == character_id.as_deref()
        && current.classification == classification
    {
        return Ok(current);
    }

    match current.status.as_str() {
        "processing" | "archive_pending" => Err(AppError::Conflict(
            "cannot relabel a capture that is processing or waiting to archive".to_owned(),
        )),
        "completed" => {
            relabel_completed_capture(
                pool,
                capture_item_id,
                &character_id,
                &classification,
                expected_suggested_character_id,
            )
            .await
        }
        _ => {
            relabel_pending_capture(
                pool,
                capture_item_id,
                &character_id,
                &classification,
                expected_suggested_character_id,
            )
            .await
        }
    }
}

async fn relabel_pending_capture(
    pool: &SqlitePool,
    capture_item_id: &str,
    character_id: &Option<String>,
    classification: &str,
    expected_suggested_character_id: Option<&str>,
) -> Result<CaptureItem, AppError> {
    let mut transaction = pool.begin().await?;
    let item = sqlx::query_as::<_, CaptureItem>(
        r#"
        UPDATE capture_items
        SET
            character_id = ?,
            classification = ?,
            status = 'queued',
            suggested_character_id = NULL,
            recognition_confidence = NULL,
            recognition_source = NULL,
            review_status = 'none',
            verification_score = NULL,
            verification_status = 'unverified',
            best_other_score = NULL,
            best_other_character_id = NULL,
            error_message = NULL,
            failure_stage = NULL,
            attempt_count = 0,
            next_retry_at = NULL,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ? AND status IN ('awaiting_label', 'queued', 'failed')
          AND (
              ? IS NULL OR (
                  suggested_character_id = ? AND review_status = 'pending'
              )
          )
        RETURNING
            id, project_id, session_id, asset_id, character_id, classification, source_path, file_size, modified_at_ms, content_hash,
            annotated_path, avatar_path, destination_path,
            destination_avatar_path, status, face_box_json, face_count,
            suggested_character_id, recognition_confidence, recognition_source,
            review_status, error_message,
            failure_stage, attempt_count, next_retry_at, processing_warnings_json,
            captured_at, processed_at, archived_at, created_at, updated_at
        "#,
    )
    .bind(character_id)
    .bind(classification)
    .bind(capture_item_id)
    .bind(expected_suggested_character_id)
    .bind(expected_suggested_character_id)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or_else(|| AppError::Conflict("capture item state or suggestion changed".to_owned()))?;
    // A sample is the projection of the capture's current primary character.
    // Any relabel, including person A -> person B, invalidates the old row;
    // processing will enroll a fresh row for the new character if eligible.
    sqlx::query("DELETE FROM character_face_samples WHERE capture_item_id = ?")
        .bind(capture_item_id)
        .execute(&mut *transaction)
        .await?;
    clear_private_project_cover_tx(&mut transaction, capture_item_id, classification).await?;
    transaction.commit().await?;
    Ok(item)
}

async fn relabel_completed_capture(
    pool: &SqlitePool,
    capture_item_id: &str,
    character_id: &Option<String>,
    classification: &str,
    expected_suggested_character_id: Option<&str>,
) -> Result<CaptureItem, AppError> {
    let mut transaction = pool.begin().await?;
    let item = sqlx::query_as::<_, CaptureItem>(
        r#"
        UPDATE capture_items
        SET
            character_id = ?,
            classification = ?,
            status = 'queued',
            annotated_path = NULL,
            avatar_path = NULL,
            face_box_json = NULL,
            processing_warnings_json = '[]',
            suggested_character_id = NULL,
            recognition_confidence = NULL,
            recognition_source = NULL,
            review_status = 'none',
            verification_score = NULL,
            verification_status = 'unverified',
            best_other_score = NULL,
            best_other_character_id = NULL,
            error_message = NULL,
            failure_stage = NULL,
            attempt_count = 0,
            next_retry_at = NULL,
            processed_at = NULL,
            archived_at = NULL,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ? AND status = 'completed'
          AND (
              ? IS NULL OR (
                  suggested_character_id = ? AND review_status = 'pending'
              )
          )
        RETURNING
            id, project_id, session_id, asset_id, character_id, classification, source_path, file_size, modified_at_ms, content_hash,
            annotated_path, avatar_path, destination_path,
            destination_avatar_path, status, face_box_json, face_count,
            suggested_character_id, recognition_confidence, recognition_source,
            review_status, error_message,
            failure_stage, attempt_count, next_retry_at, processing_warnings_json,
            captured_at, processed_at, archived_at, created_at, updated_at
        "#,
    )
    .bind(character_id)
    .bind(classification)
    .bind(capture_item_id)
    .bind(expected_suggested_character_id)
    .bind(expected_suggested_character_id)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or_else(|| AppError::Conflict("capture item state or suggestion changed".to_owned()))?;
    sqlx::query("DELETE FROM character_face_samples WHERE capture_item_id = ?")
        .bind(capture_item_id)
        .execute(&mut *transaction)
        .await?;
    clear_private_project_cover_tx(&mut transaction, capture_item_id, classification).await?;
    transaction.commit().await?;
    Ok(item)
}

/// Private captures are hidden from Home by default, so changing a pinned
/// cover to private must clear the project reference in the same transaction.
async fn clear_private_project_cover_tx(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    capture_item_id: &str,
    classification: &str,
) -> Result<(), AppError> {
    if classification != "private" {
        return Ok(());
    }
    sqlx::query(
        r#"
        UPDATE projects
        SET cover_capture_item_id = NULL,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE cover_capture_item_id = ?
        "#,
    )
    .bind(capture_item_id)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

pub async fn mark_processing(
    pool: &SqlitePool,
    input: CaptureItemIdInput,
) -> Result<CaptureItem, AppError> {
    transition_item(pool, &input.capture_item_id, "queued", "processing", true).await
}

pub async fn complete_processing(
    pool: &SqlitePool,
    input: CompleteCaptureProcessingInput,
) -> Result<CaptureItem, AppError> {
    let capture_item_id = required(&input.capture_item_id, "capture item id")?;
    let item = get_item(pool, capture_item_id).await?;
    if item.status != "processing" {
        return Err(AppError::Conflict(
            "capture item must be processing before recording Python output".to_owned(),
        ));
    }

    let annotated_path = validate_local_output(Path::new(required(
        &input.annotated_path,
        "annotated path",
    )?))
    .await?;
    let source_path = tokio::fs::canonicalize(&item.source_path).await?;
    if annotated_path == source_path {
        return Err(AppError::Validation(
            "Python output cannot overwrite the source screenshot".to_owned(),
        ));
    }
    let avatar_path = if let Some(path) = input
        .avatar_path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        let path = validate_local_output(Path::new(path)).await?;
        if path == source_path {
            return Err(AppError::Validation(
                "Python avatar output cannot overwrite the source screenshot".to_owned(),
            ));
        }
        Some(path_to_string(&path))
    } else {
        None
    };

    let face_box_json = input
        .face_box_json
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    if let Some(value) = &face_box_json {
        serde_json::from_str::<serde_json::Value>(value)
            .map_err(|error| AppError::Validation(format!("invalid face box JSON: {error}")))?;
    }
    let warnings_json = input.warnings_json.unwrap_or_else(|| "[]".to_owned());
    let warnings = serde_json::from_str::<serde_json::Value>(&warnings_json).map_err(|error| {
        AppError::Validation(format!("invalid processing warnings JSON: {error}"))
    })?;
    if !warnings.is_array() {
        return Err(AppError::Validation(
            "processing warnings JSON must be an array".to_owned(),
        ));
    }
    let face_feature_json = input
        .face_feature
        .as_ref()
        .map(|feature| {
            if feature.is_empty() || !feature.iter().all(|value| value.is_finite()) {
                return Err(AppError::Validation(
                    "face feature must be a non-empty array of finite numbers".to_owned(),
                ));
            }
            serde_json::to_string(feature).map_err(|error| {
                AppError::Validation(format!("cannot encode face feature: {error}"))
            })
        })
        .transpose()?;
    let face_dim = input
        .face_feature
        .as_ref()
        .map(|feature| feature.len() as i64);

    let mut transaction = pool.begin().await?;
    // The new face row is derived from the processing result that just
    // succeeded; the old rows are replaced atomically so a failed run never
    // leaves the capture without its previous face data.
    let face = FaceFeatureWrite {
        feature_json: face_feature_json,
        face_box_json: face_box_json.clone(),
        model_id: input.face_feature_model_id,
        model_version: input.face_feature_model_version,
        face_count: input.face_count,
        face_sharpness: input.face_sharpness,
        face_area_ratio: input.face_area_ratio,
    };
    write_primary_face_tx(&mut transaction, capture_item_id, &face, face_dim).await?;
    let item = sqlx::query_as::<_, CaptureItem>(
        r#"
        UPDATE capture_items
        SET
            annotated_path = ?,
            avatar_path = ?,
            face_box_json = ?,
            processing_warnings_json = ?,
            status = 'archive_pending',
            error_message = NULL,
            failure_stage = NULL,
            attempt_count = 0,
            next_retry_at = NULL,
            processed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ? AND status = 'processing'
        RETURNING
            id, project_id, session_id, asset_id, character_id, classification, source_path, file_size, modified_at_ms, content_hash,
            annotated_path, avatar_path, destination_path,
            destination_avatar_path, status, face_box_json, face_count,
            suggested_character_id, recognition_confidence, recognition_source,
            review_status, error_message,
            failure_stage, attempt_count, next_retry_at, processing_warnings_json,
            captured_at, processed_at, archived_at, created_at, updated_at
        "#,
    )
    .bind(path_to_string(&annotated_path))
    .bind(avatar_path)
    .bind(face_box_json)
    .bind(warnings_json)
    .bind(capture_item_id)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or_else(|| AppError::Conflict("capture processing state changed".to_owned()))?;
    transaction.commit().await?;
    Ok(item)
}

/// Records a processing-free archive for a `person` capture when the Python
/// vision engine is not configured. The screenshot is archived raw (no
/// annotation or avatar) with a warning; a later retry can re-queue the item
/// for full Python processing once the engine is configured.
pub(crate) async fn complete_processing_without_engine(
    pool: &SqlitePool,
    capture_item_id: &str,
) -> Result<CaptureItem, AppError> {
    let warnings_json =
        serde_json::json!(["AI 引擎未配置：已直接归档原图，配置视觉引擎后可重新处理"]).to_string();
    let item = sqlx::query_as::<_, CaptureItem>(
        r#"
        UPDATE capture_items
        SET
            annotated_path = NULL,
            avatar_path = NULL,
            face_box_json = NULL,
            processing_warnings_json = ?,
            status = 'archive_pending',
            error_message = NULL,
            failure_stage = NULL,
            attempt_count = 0,
            next_retry_at = NULL,
            processed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ? AND status = 'processing'
        RETURNING
            id, project_id, session_id, asset_id, character_id, classification, source_path, file_size, modified_at_ms, content_hash,
            annotated_path, avatar_path, destination_path,
            destination_avatar_path, status, face_box_json, face_count,
            suggested_character_id, recognition_confidence, recognition_source,
            review_status, error_message,
            failure_stage, attempt_count, next_retry_at, processing_warnings_json,
            captured_at, processed_at, archived_at, created_at, updated_at
        "#,
    )
    .bind(warnings_json)
    .bind(capture_item_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::Conflict("capture processing state changed".to_owned()))?;
    Ok(item)
}

pub async fn mark_failed(
    pool: &SqlitePool,
    input: MarkCaptureFailedInput,
) -> Result<CaptureItem, AppError> {
    let capture_item_id = required(&input.capture_item_id, "capture item id")?;
    let error_message = normalized_error_message(&input.error_message)?;
    let item = sqlx::query_as::<_, CaptureItem>(
        r#"
        UPDATE capture_items
        SET
            status = 'failed',
            error_message = ?,
            failure_stage = 'processing',
            next_retry_at = NULL,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ? AND status IN ('queued', 'processing', 'archive_pending', 'failed')
        RETURNING
            id, project_id, session_id, asset_id, character_id, classification, source_path, file_size, modified_at_ms, content_hash,
            annotated_path, avatar_path, destination_path,
            destination_avatar_path, status, face_box_json, face_count,
            suggested_character_id, recognition_confidence, recognition_source,
            review_status, error_message,
            failure_stage, attempt_count, next_retry_at, processing_warnings_json,
            captured_at, processed_at, archived_at, created_at, updated_at
        "#,
    )
    .bind(error_message)
    .bind(capture_item_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| {
        AppError::Conflict(
            "only queued, processing, or archive-pending capture items can be marked failed"
                .to_owned(),
        )
    })?;
    Ok(item)
}

pub async fn retry_capture(
    pool: &SqlitePool,
    input: RetryCaptureInput,
) -> Result<CaptureItem, AppError> {
    let capture_item_id = required(&input.capture_item_id, "capture item id")?;
    let current = get_item(pool, capture_item_id).await?;
    // A completed person capture that was archived without Python (degraded
    // fallback) can be re-queued for full processing once the engine is
    // configured; the old raw archive is replaced when the new one succeeds.
    let degraded = current.status == "completed"
        && current.classification == "person"
        && current.annotated_path.is_none();
    if current.status != "failed" && !degraded {
        return Err(AppError::Conflict(
            "only failed or degraded completed capture items can be retried".to_owned(),
        ));
    }
    let retry_archive = current.failure_stage.as_deref() == Some("archive")
        && current
            .annotated_path
            .as_deref()
            .is_some_and(|path| Path::new(path).is_file());
    let next_status = if retry_archive {
        "archive_pending"
    } else if current.character_id.is_some() {
        "queued"
    } else {
        "awaiting_label"
    };
    let item = if degraded {
        sqlx::query_as::<_, CaptureItem>(
            r#"
            UPDATE capture_items
            SET
                status = 'queued',
                annotated_path = NULL,
                avatar_path = NULL,
                face_box_json = NULL,
                processing_warnings_json = '[]',
                error_message = NULL,
                failure_stage = NULL,
                attempt_count = 0,
                next_retry_at = NULL,
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
            WHERE id = ? AND status = 'completed'
              AND classification = 'person' AND annotated_path IS NULL
            RETURNING
                id, project_id, session_id, asset_id, character_id, classification, source_path, file_size, modified_at_ms, content_hash,
                annotated_path, avatar_path, destination_path,
                destination_avatar_path, status, face_box_json, face_count,
                suggested_character_id, recognition_confidence, recognition_source,
                review_status, error_message,
                failure_stage, attempt_count, next_retry_at, processing_warnings_json,
                captured_at, processed_at, archived_at, created_at, updated_at
            "#,
        )
        .bind(capture_item_id)
        .fetch_one(pool)
        .await?
    } else {
        sqlx::query_as::<_, CaptureItem>(
            r#"
            UPDATE capture_items
            SET
                status = ?,
                annotated_path = CASE WHEN ? = 'queued' THEN NULL ELSE annotated_path END,
                avatar_path = CASE WHEN ? = 'queued' THEN NULL ELSE avatar_path END,
                face_box_json = CASE WHEN ? = 'queued' THEN NULL ELSE face_box_json END,
                processing_warnings_json = CASE WHEN ? = 'queued' THEN '[]' ELSE processing_warnings_json END,
                error_message = NULL,
                failure_stage = NULL,
                attempt_count = 0,
                next_retry_at = NULL,
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
            WHERE id = ? AND status = 'failed'
            RETURNING
                id, project_id, session_id, asset_id, character_id, classification, source_path, file_size, modified_at_ms, content_hash,
                annotated_path, avatar_path, destination_path,
                destination_avatar_path, status, face_box_json, face_count,
                suggested_character_id, recognition_confidence, recognition_source,
                review_status, error_message,
                failure_stage, attempt_count, next_retry_at, processing_warnings_json,
                captured_at, processed_at, archived_at, created_at, updated_at
            "#,
        )
        .bind(next_status)
        .bind(next_status)
        .bind(next_status)
        .bind(next_status)
        .bind(next_status)
        .bind(capture_item_id)
        .fetch_one(pool)
        .await?
    };
    Ok(item)
}

/// Re-queues every degraded person capture (completed without an annotated
/// output) in a project, optionally restricted to one character. Items that
/// changed state concurrently are skipped; the return value is the number of
/// items successfully re-queued.
pub async fn retry_degraded_captures(
    pool: &SqlitePool,
    project_id: &str,
    character_id: Option<&str>,
) -> Result<i64, AppError> {
    let project_id = project_id.trim();
    if project_id.is_empty() {
        return Err(AppError::Validation(
            "project id cannot be empty".to_owned(),
        ));
    }
    let character_id = character_id
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let ids: Vec<String> = sqlx::query_scalar(
        r#"
        SELECT item.id
        FROM capture_items item
        JOIN capture_sessions session ON session.id = item.session_id
        WHERE session.project_id = ?
          AND item.classification = 'person'
          AND item.status = 'completed'
          AND item.annotated_path IS NULL
          AND (? IS NULL OR item.character_id = ?)
        ORDER BY item.captured_at ASC, item.created_at ASC
        "#,
    )
    .bind(project_id)
    .bind(character_id)
    .bind(character_id)
    .fetch_all(pool)
    .await?;

    let mut requeued = 0i64;
    for id in ids {
        let Ok(item) = retry_capture(
            pool,
            RetryCaptureInput {
                capture_item_id: id,
            },
        )
        .await
        else {
            continue;
        };
        if item.status == "queued" {
            requeued += 1;
        }
    }
    Ok(requeued)
}

/// Items of one non-person category for the workbench tabs: unclassified
/// captures waiting for a label, or all scene/private captures of a project.
pub async fn list_category_items(
    pool: &SqlitePool,
    input: ListCategoryItemsInput,
) -> Result<Vec<CaptureItem>, AppError> {
    let project_id = input.project_id.trim();
    let category = input.category.trim();
    if project_id.is_empty() {
        return Err(AppError::Validation(
            "project id cannot be empty".to_owned(),
        ));
    }
    let filter = match category {
        "unclassified" => "item.status = 'awaiting_label'",
        "scene" => "item.classification = 'scene'",
        "private" => "item.classification = 'private'",
        _ => {
            return Err(AppError::Validation(
                "category must be unclassified, scene, or private".to_owned(),
            ))
        }
    };
    let query = format!(
        r#"
        SELECT
            item.id, item.project_id, item.session_id, item.asset_id, item.character_id,
            item.classification, item.source_path, item.file_size, item.modified_at_ms, item.content_hash,
            item.annotated_path, item.avatar_path, item.destination_path,
            item.destination_avatar_path, item.status, item.face_box_json, item.face_count,
            item.suggested_character_id, item.recognition_confidence,
            item.recognition_source, item.review_status,
            item.error_message, item.failure_stage, item.attempt_count,
            item.next_retry_at, item.processing_warnings_json,
            item.captured_at, item.processed_at, item.archived_at,
            item.created_at, item.updated_at
        FROM capture_items item
        JOIN capture_sessions session ON session.id = item.session_id
        WHERE session.project_id = ? AND {filter}
        ORDER BY item.captured_at DESC, item.created_at DESC
        "#,
    );
    let items = sqlx::query_as::<_, CaptureItem>(&query)
        .bind(project_id)
        .fetch_all(pool)
        .await?;
    Ok(items)
}

/// Pre-existing images in the session source directory that are not yet
/// registered as captures. Uses the same non-recursive scan and image filter
/// as the discovery poll so the two views stay consistent.
pub async fn list_unimported_captures(
    pool: &SqlitePool,
    session_id: &str,
) -> Result<Vec<UnimportedCapture>, AppError> {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return Err(AppError::Validation(
            "capture session id cannot be empty".to_owned(),
        ));
    }
    let session = get_session(pool, session_id).await?;
    let mut candidates = Vec::new();
    for dir in session_directories(pool, &session).await? {
        let Ok(mut directory) = tokio::fs::read_dir(&dir).await else {
            continue;
        };
        while let Some(entry) = directory.next_entry().await? {
            let path = entry.path();
            if !is_supported_image(&path) {
                continue;
            }
            let metadata = match entry.metadata().await {
                Ok(metadata) if metadata.is_file() => metadata,
                _ => continue,
            };
            if metadata.len() == 0 {
                continue;
            }
            let source_path = path_to_string(&path);
            let registered: i64 = sqlx::query_scalar(
                // Files registered in any (earlier) session are not import
                // candidates again: re-importing the same directory across
                // sessions would duplicate every record and re-archive the
                // images.
                "SELECT EXISTS (SELECT 1 FROM capture_items WHERE source_path = ?)",
            )
            .bind(&source_path)
            .fetch_one(pool)
            .await?;
            if registered != 0 {
                continue;
            }
            candidates.push(UnimportedCapture {
                file_size: i64::try_from(metadata.len())
                    .map_err(|_| AppError::Validation("capture file is too large".to_owned()))?,
                modified_at_ms: metadata
                    .modified()
                    .ok()
                    .and_then(|value| unix_millis(value).ok()),
                path: source_path,
            });
        }
    }
    candidates.sort_by(|first, second| first.path.cmp(&second.path));
    Ok(candidates)
}

/// Registers pre-existing images as awaiting-label captures. Baseline rows
/// are dropped first so the discovery-baseline rejection does not apply, and
/// the stability wait is skipped because imported files are already static.
/// Paths outside the source directory, non-images and empty files are
/// skipped; already-registered paths are ignored. Returns the number of
/// newly imported captures.
pub async fn import_directory_captures(
    pool: &SqlitePool,
    input: ImportDirectoryCapturesInput,
) -> Result<Vec<CaptureItem>, AppError> {
    let session_id = input.session_id.trim();
    if session_id.is_empty() {
        return Err(AppError::Validation(
            "capture session id cannot be empty".to_owned(),
        ));
    }
    let session = get_session(pool, session_id).await?;
    let session_dirs = session_directories(pool, &session).await?;
    let mut imported = Vec::new();
    for raw in &input.paths {
        let Ok(canonical) = tokio::fs::canonicalize(Path::new(raw.trim())).await else {
            continue;
        };
        if !is_supported_image(&canonical) {
            continue;
        }
        if !session_dirs
            .iter()
            .any(|root| path_is_within(&canonical, root))
        {
            continue;
        }
        let metadata = match tokio::fs::metadata(&canonical).await {
            Ok(metadata) if metadata.is_file() => metadata,
            _ => continue,
        };
        if metadata.len() == 0 {
            continue;
        }
        let source_path = path_to_string(&canonical);
        let already_registered: i64 =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM capture_items WHERE source_path = ?)")
                .bind(&source_path)
                .fetch_one(pool)
                .await?;
        if already_registered != 0 {
            continue;
        }
        sqlx::query(
            "DELETE FROM capture_session_baseline_files WHERE session_id = ? AND source_path = ?",
        )
        .bind(session_id)
        .bind(&source_path)
        .execute(pool)
        .await?;
        let (item, created) = register_discovered_path(pool, session_id, &canonical, true).await?;
        if created {
            imported.push(item);
        }
    }
    Ok(imported)
}

pub async fn deferred_import_recognition_count(
    pool: &SqlitePool,
    session_id: &str,
) -> Result<i64, AppError> {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return Err(AppError::Validation(
            "capture session id cannot be empty".to_owned(),
        ));
    }
    get_session(pool, session_id).await?;
    let count = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM capture_items
        WHERE session_id = ?
          AND status = 'awaiting_label'
          AND recognition_deferred = 1
        "#,
    )
    .bind(session_id)
    .fetch_one(pool)
    .await?;
    Ok(count)
}

pub async fn start_imported_recognition(
    pool: &SqlitePool,
    session_id: &str,
) -> Result<i64, AppError> {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return Err(AppError::Validation(
            "capture session id cannot be empty".to_owned(),
        ));
    }
    let session = get_session(pool, session_id).await?;
    if session.status != "active" {
        return Err(AppError::Conflict(
            "capture session is no longer active".to_owned(),
        ));
    }
    let updated = sqlx::query(
        r#"
        UPDATE capture_items
        SET
            recognition_deferred = 0,
            next_retry_at = NULL,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE session_id = ?
          AND status = 'awaiting_label'
          AND recognition_deferred = 1
        "#,
    )
    .bind(session_id)
    .execute(pool)
    .await?;
    Ok(i64::try_from(updated.rows_affected()).unwrap_or(i64::MAX))
}

/// Reconciles jobs that could not have a live Python worker after the desktop
/// process restarted. Queued and archive-pending jobs remain retryable in
/// place; only an in-flight processing claim is invalidated.
pub async fn recover_interrupted_processing(pool: &SqlitePool) -> Result<u64, AppError> {
    let result = sqlx::query(
        r#"
        UPDATE capture_items
        SET
            status = 'failed',
            error_message = 'processing was interrupted by an application restart',
            failure_stage = 'processing',
            next_retry_at = NULL,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE status = 'processing'
        "#,
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

pub(crate) async fn get_session(
    pool: &SqlitePool,
    session_id: &str,
) -> Result<CaptureSession, AppError> {
    sqlx::query_as::<_, CaptureSession>(
        r#"
        SELECT
            id, project_id, status, started_at, ended_at, created_at, updated_at
        FROM capture_sessions
        WHERE id = ?
        "#,
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound("capture session".to_owned()))
}

pub(crate) async fn require_active_session(
    pool: &SqlitePool,
    session_id: &str,
) -> Result<CaptureSession, AppError> {
    let session = get_session(pool, session_id).await?;
    if session.status != "active" {
        return Err(AppError::Conflict(
            "capture session is no longer active".to_owned(),
        ));
    }
    Ok(session)
}

pub(crate) async fn get_item(
    pool: &SqlitePool,
    capture_item_id: &str,
) -> Result<CaptureItem, AppError> {
    sqlx::query_as::<_, CaptureItem>(
        r#"
        SELECT
            id, project_id, session_id, asset_id, character_id, classification, source_path, file_size, modified_at_ms, content_hash,
            annotated_path, avatar_path, destination_path,
            destination_avatar_path, status, face_box_json, face_count,
            suggested_character_id, recognition_confidence, recognition_source,
            review_status, error_message,
            failure_stage, attempt_count, next_retry_at, processing_warnings_json,
            captured_at, processed_at, archived_at, created_at, updated_at
        FROM capture_items
        WHERE id = ?
        "#,
    )
    .bind(capture_item_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound("capture item".to_owned()))
}

/// Atomically claims the oldest queued item eligible for processing. The
/// worker passes `skip_person = false` and degrades person items when the
/// vision engine is unconfigured; this flag remains as a low-level option
/// for callers that must never claim a person item.
pub(crate) async fn claim_next_queued(
    pool: &SqlitePool,
    skip_person: bool,
) -> Result<Option<CaptureItem>, AppError> {
    let item = sqlx::query_as::<_, CaptureItem>(
        r#"
        UPDATE capture_items
        SET
            status = 'processing',
            error_message = NULL,
            failure_stage = NULL,
            next_retry_at = NULL,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = (
            SELECT id FROM capture_items
            WHERE status = 'queued'
              AND (? = 0 OR classification != 'person')
            ORDER BY captured_at ASC, created_at ASC
            LIMIT 1
        ) AND status = 'queued'
        RETURNING
            id, project_id, session_id, asset_id, character_id, classification, source_path, file_size, modified_at_ms, content_hash,
            annotated_path, avatar_path, destination_path,
            destination_avatar_path, status, face_box_json, face_count,
            suggested_character_id, recognition_confidence, recognition_source,
            review_status, error_message,
            failure_stage, attempt_count, next_retry_at, processing_warnings_json,
            captured_at, processed_at, archived_at, created_at, updated_at
        "#,
    )
    .bind(skip_person)
    .fetch_optional(pool)
    .await?;
    Ok(item)
}

/// Returns the oldest queued item eligible for processing; see
/// [`claim_next_queued`] for the low-level `skip_person` semantics.
pub(crate) async fn peek_next_queued(
    pool: &SqlitePool,
    skip_person: bool,
) -> Result<Option<CaptureItem>, AppError> {
    let item = sqlx::query_as::<_, CaptureItem>(
        r#"
        SELECT
            id, project_id, session_id, asset_id, character_id, classification, source_path, file_size, modified_at_ms, content_hash,
            annotated_path, avatar_path, destination_path,
            destination_avatar_path, status, face_box_json, face_count,
            suggested_character_id, recognition_confidence, recognition_source,
            review_status, error_message,
            failure_stage, attempt_count, next_retry_at, processing_warnings_json,
            captured_at, processed_at, archived_at, created_at, updated_at
        FROM capture_items
        WHERE status = 'queued'
          AND (? = 0 OR classification != 'person')
        ORDER BY captured_at ASC, created_at ASC
        LIMIT 1
        "#,
    )
    .bind(skip_person)
    .fetch_optional(pool)
    .await?;
    Ok(item)
}

/// Oldest awaiting-label capture without a face-feature attempt. Used by the
/// pre-label feature pass; the item keeps its `awaiting_label` status so the
/// classify popup continues to show it.
pub(crate) async fn next_awaiting_label_without_feature(
    pool: &SqlitePool,
) -> Result<Option<CaptureItem>, AppError> {
    let item = sqlx::query_as::<_, CaptureItem>(
        r#"
        SELECT
            id, project_id, session_id, asset_id, character_id, classification, source_path, file_size, modified_at_ms, content_hash,
            annotated_path, avatar_path, destination_path,
            destination_avatar_path, status, face_box_json, face_count,
            suggested_character_id, recognition_confidence, recognition_source,
            review_status, error_message,
            failure_stage, attempt_count, next_retry_at, processing_warnings_json,
            captured_at, processed_at, archived_at, created_at, updated_at
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
          AND (next_retry_at IS NULL OR next_retry_at <= strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        ORDER BY captured_at ASC, created_at ASC
        LIMIT 1
        "#,
    )
    .fetch_optional(pool)
    .await?;
    Ok(item)
}

/// Persists the result of a pre-label feature pass. `feature_json` is either
/// the encoded SFace vector or `[]` marking "attempted without a feature" so
/// the pass does not re-pick the item.
pub(crate) async fn store_face_feature(
    pool: &SqlitePool,
    capture_item_id: &str,
    face: FaceFeatureWrite,
) -> Result<CaptureItem, AppError> {
    let face_dim = if face.feature_json.as_deref() == Some("[]") {
        None
    } else if let Some(feature_json) = face.feature_json.as_deref() {
        let feature: Vec<f64> = serde_json::from_str(feature_json)
            .map_err(|error| AppError::Validation(format!("invalid face feature: {error}")))?;
        Some(feature.len() as i64)
    } else {
        None
    };
    let mut transaction = pool.begin().await?;
    write_primary_face_tx(&mut transaction, capture_item_id, &face, face_dim).await?;
    transaction.commit().await?;
    get_item(pool, capture_item_id).await
}

/// Atomically replaces a capture's face rows with a new primary face row and
/// mirrors the box on `capture_items`. Face data is derived data: the old
/// rows are deleted only after the new result is available, so a failed
/// extraction leaves the previous rows untouched. Writing a new feature
/// invalidates the stored suggestion (it may have been produced by a
/// different model); verification fields are label-time facts and stay.
async fn write_primary_face_tx(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    capture_item_id: &str,
    face: &FaceFeatureWrite,
    embedding_dim: Option<i64>,
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM capture_faces WHERE capture_item_id = ?")
        .bind(capture_item_id)
        .execute(&mut **transaction)
        .await?;
    sqlx::query(
        r#"
        INSERT INTO capture_faces (
            id, capture_item_id, face_index, is_primary, box_json, feature_json,
            feature_model_id, feature_model_version, feature_dim, face_sharpness,
            face_area_ratio
        )
        VALUES (?, ?, 0, 1, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(Uuid::new_v4().to_string())
    .bind(capture_item_id)
    .bind(&face.face_box_json)
    .bind(&face.feature_json)
    .bind(&face.model_id)
    .bind(&face.model_version)
    .bind(embedding_dim)
    .bind(face.face_sharpness)
    .bind(face.face_area_ratio)
    .execute(&mut **transaction)
    .await?;
    sqlx::query(
        r#"
        UPDATE capture_items
        SET
            face_box_json = COALESCE(?, face_box_json),
            face_count = ?,
            face_feature_json = NULL,
            suggested_character_id = NULL,
            recognition_confidence = NULL,
            recognition_source = NULL,
            review_status = 'none',
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ?
        "#,
    )
    .bind(&face.face_box_json)
    .bind(face.face_count)
    .bind(capture_item_id)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

#[cfg(test)]
#[path = "capture_service_tests.rs"]
mod tests;

pub(crate) async fn mark_archive_pending_from_processing(
    pool: &SqlitePool,
    capture_item_id: &str,
) -> Result<CaptureItem, AppError> {
    let item = sqlx::query_as::<_, CaptureItem>(
        r#"
        UPDATE capture_items
        SET
            status = 'archive_pending',
            error_message = NULL,
            failure_stage = NULL,
            attempt_count = 0,
            next_retry_at = NULL,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ? AND status = 'processing'
        RETURNING
            id, project_id, session_id, asset_id, character_id, classification, source_path, file_size, modified_at_ms, content_hash,
            annotated_path, avatar_path, destination_path,
            destination_avatar_path, status, face_box_json, face_count,
            suggested_character_id, recognition_confidence, recognition_source,
            review_status, error_message,
            failure_stage, attempt_count, next_retry_at, processing_warnings_json,
            captured_at, processed_at, archived_at, created_at, updated_at
        "#,
    )
    .bind(capture_item_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::Conflict("capture item is not processing".to_owned()))?;
    Ok(item)
}

pub(crate) async fn next_archive_pending(
    pool: &SqlitePool,
) -> Result<Option<CaptureItem>, AppError> {
    let item = sqlx::query_as::<_, CaptureItem>(
        r#"
        SELECT
            id, project_id, session_id, asset_id, character_id, classification, source_path, file_size, modified_at_ms, content_hash,
            annotated_path, avatar_path, destination_path,
            destination_avatar_path, status, face_box_json, face_count,
            suggested_character_id, recognition_confidence, recognition_source,
            review_status, error_message,
            failure_stage, attempt_count, next_retry_at, processing_warnings_json,
            captured_at, processed_at, archived_at, created_at, updated_at
        FROM capture_items
        WHERE status = 'archive_pending'
          AND (next_retry_at IS NULL OR next_retry_at <= strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        ORDER BY captured_at ASC, created_at ASC
        LIMIT 1
        "#,
    )
    .fetch_optional(pool)
    .await?;
    Ok(item)
}

pub(crate) async fn schedule_archive_retry(
    pool: &SqlitePool,
    capture_item_id: &str,
    error_message: &str,
) -> Result<CaptureItem, AppError> {
    let current = get_item(pool, capture_item_id).await?;
    let next_attempt = current.attempt_count.saturating_add(1);
    let (status, retry_modifier) = match next_attempt {
        1 => ("archive_pending", Some("+2 seconds")),
        2 => ("archive_pending", Some("+10 seconds")),
        3 => ("archive_pending", Some("+30 seconds")),
        _ => ("failed", None),
    };
    let item = sqlx::query_as::<_, CaptureItem>(
        r#"
        UPDATE capture_items
        SET
            status = ?,
            error_message = ?,
            failure_stage = 'archive',
            attempt_count = ?,
            next_retry_at = CASE
                WHEN ? IS NULL THEN NULL
                ELSE strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?)
            END,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ? AND status = 'failed'
        RETURNING
            id, project_id, session_id, asset_id, character_id, classification, source_path, file_size, modified_at_ms, content_hash,
            annotated_path, avatar_path, destination_path,
            destination_avatar_path, status, face_box_json, face_count,
            suggested_character_id, recognition_confidence, recognition_source,
            review_status, error_message,
            failure_stage, attempt_count, next_retry_at, processing_warnings_json,
            captured_at, processed_at, archived_at, created_at, updated_at
        "#,
    )
    .bind(status)
    .bind(truncate_chars(error_message, MAX_ERROR_MESSAGE_CHARS))
    .bind(next_attempt)
    .bind(retry_modifier)
    .bind(retry_modifier)
    .bind(capture_item_id)
    .fetch_one(pool)
    .await?;
    Ok(item)
}

pub(crate) async fn register_discovered_path(
    pool: &SqlitePool,
    session_id: &str,
    source_path: &Path,
    recognition_deferred: bool,
) -> Result<(CaptureItem, bool), AppError> {
    let session = require_active_session(pool, session_id).await?;
    // Fast path: the path is already registered. Discovery polls active
    // sessions on a fixed interval, and returning the existing row here
    // avoids opening the file and re-reading its contents for a SHA-256 on
    // every poll.
    let existing_id: Option<String> = sqlx::query_scalar(
        r#"
        SELECT id
        FROM capture_items
        WHERE session_id = ? AND source_path = ?
        LIMIT 1
        "#,
    )
    .bind(session_id)
    .bind(path_to_string(source_path))
    .fetch_optional(pool)
    .await?;
    if let Some(existing_id) = existing_id {
        let item = get_item(pool, &existing_id).await?;
        return Ok((item, false));
    }
    let metadata = tokio::fs::metadata(source_path).await?;
    let file_size = i64::try_from(metadata.len())
        .map_err(|_| AppError::Validation("capture source is too large".to_owned()))?;
    let modified_at_ms = metadata
        .modified()
        .ok()
        .and_then(|value| unix_millis(value).ok());
    let content_hash = sha256_file(source_path).await?;

    // Content fingerprint dedup: the same image found in another directory
    // of the same project registers only once.
    let duplicate: Option<String> = sqlx::query_scalar(
        r#"
        SELECT id
        FROM capture_items
        WHERE project_id = ? AND content_hash = ?
        LIMIT 1
        "#,
    )
    .bind(&session.project_id)
    .bind(&content_hash)
    .fetch_optional(pool)
    .await?;

    let source_path = path_to_string(source_path);
    if let Some(duplicate_id) = duplicate {
        // Duplicate content already registered in this project: do not create
        // a second item for the same image.
        let item = sqlx::query_as::<_, CaptureItem>(
            r#"
            SELECT
                id, project_id, session_id, asset_id, character_id,
                classification, source_path, file_size, modified_at_ms, content_hash,
                annotated_path, avatar_path, destination_path,
                destination_avatar_path, status, face_box_json, face_count,
                suggested_character_id, recognition_confidence, recognition_source,
                review_status, error_message,
                failure_stage, attempt_count, next_retry_at, processing_warnings_json,
                captured_at, processed_at, archived_at, created_at, updated_at
            FROM capture_items
            WHERE id = ?
            "#,
        )
        .bind(&duplicate_id)
        .fetch_one(pool)
        .await?;
        return Ok((item, false));
    }

    let result = sqlx::query(
        r#"
        INSERT INTO capture_items (
            id, project_id, session_id, source_path,
            file_size, modified_at_ms, content_hash, recognition_deferred
        )
        SELECT ?, project_id, id, ?, ?, ?, ?, ?
        FROM capture_sessions
        WHERE id = ? AND status = 'active'
        ON CONFLICT(session_id, source_path) DO NOTHING
        "#,
    )
    .bind(Uuid::new_v4().to_string())
    .bind(&source_path)
    .bind(file_size)
    .bind(modified_at_ms)
    .bind(&content_hash)
    .bind(if recognition_deferred { 1 } else { 0 })
    .bind(session_id)
    .execute(pool)
    .await?;

    let item = sqlx::query_as::<_, CaptureItem>(
        r#"
        SELECT
            id, project_id, session_id, asset_id, character_id,
            classification, source_path, file_size, modified_at_ms, content_hash,
            annotated_path, avatar_path, destination_path,
            destination_avatar_path, status, face_box_json, face_count,
            suggested_character_id, recognition_confidence, recognition_source,
            review_status, error_message,
            failure_stage, attempt_count, next_retry_at, processing_warnings_json,
            captured_at, processed_at, archived_at, created_at, updated_at
        FROM capture_items
        WHERE session_id = ? AND source_path = ?
        "#,
    )
    .bind(session_id)
    .bind(source_path)
    .fetch_optional(pool)
    .await?;
    let item = match item {
        Some(item) => item,
        None => {
            require_active_session(pool, session_id).await?;
            return Err(AppError::Conflict(
                "capture could not be registered".to_owned(),
            ));
        }
    };
    Ok((item, result.rows_affected() == 1))
}

pub(crate) async fn matches_discovery_baseline(
    pool: &SqlitePool,
    session_id: &str,
    source_path: &Path,
    metadata: &std::fs::Metadata,
) -> Result<bool, AppError> {
    let current_size = i64::try_from(metadata.len())
        .map_err(|_| AppError::Validation("capture source is too large".to_owned()))?;
    let current_modified = metadata
        .modified()
        .ok()
        .and_then(|value| unix_millis(value).ok());
    let baseline: Option<(i64, Option<i64>)> = sqlx::query_as(
        r#"
        SELECT file_size, modified_at_ms
        FROM capture_session_baseline_files
        WHERE session_id = ? AND source_path = ?
        "#,
    )
    .bind(session_id)
    .bind(path_to_string(source_path))
    .fetch_optional(pool)
    .await?;
    Ok(baseline
        .is_some_and(|(size, modified)| size == current_size && modified == current_modified))
}

pub(crate) async fn set_archive_pending_for_retry(
    pool: &SqlitePool,
    capture_item_id: &str,
) -> Result<CaptureItem, AppError> {
    let current = get_item(pool, capture_item_id).await?;
    if current.status == "archive_pending" {
        return Ok(current);
    }
    if current.status != "failed" || current.annotated_path.is_none() {
        return Err(AppError::Conflict(
            "capture item is not ready for archive".to_owned(),
        ));
    }

    let item = sqlx::query_as::<_, CaptureItem>(
        r#"
        UPDATE capture_items
        SET
            status = 'archive_pending',
            error_message = NULL,
            failure_stage = NULL,
            next_retry_at = NULL,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ? AND status = 'failed'
        RETURNING
            id, project_id, session_id, asset_id, character_id, classification, source_path, file_size, modified_at_ms, content_hash,
            annotated_path, avatar_path, destination_path,
            destination_avatar_path, status, face_box_json, face_count,
            suggested_character_id, recognition_confidence, recognition_source,
            review_status, error_message,
            failure_stage, attempt_count, next_retry_at, processing_warnings_json,
            captured_at, processed_at, archived_at, created_at, updated_at
        "#,
    )
    .bind(capture_item_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::Conflict("capture archive state changed".to_owned()))?;
    Ok(item)
}

pub(crate) async fn record_failure(
    pool: &SqlitePool,
    capture_item_id: &str,
    error_message: &str,
) -> Result<(), AppError> {
    let error_message = truncate_chars(error_message.trim(), MAX_ERROR_MESSAGE_CHARS);
    sqlx::query(
        r#"
        UPDATE capture_items
        SET
            status = 'failed',
            error_message = ?,
            failure_stage = COALESCE(failure_stage, 'archive'),
            next_retry_at = NULL,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ? AND status != 'completed'
        "#,
    )
    .bind(error_message)
    .bind(capture_item_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) fn is_supported_image(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "webp" | "bmp"
            )
        })
}

pub(crate) fn path_to_string(path: &Path) -> String {
    normalized_path_string(path)
}

/// Strips the `\\?\` extended-length prefix that Windows `canonicalize`
/// produces, so canonical paths compare equal to stored plain directory
/// strings. No-op for ordinary paths.
pub(crate) fn normalized_path_string(path: &Path) -> String {
    let value = path.to_string_lossy();
    if let Some(path) = value.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{path}");
    }
    if let Some(path) = value.strip_prefix(r"\\?\") {
        return path.to_owned();
    }
    value.into_owned()
}

/// True when `candidate` is `root` itself or lives underneath it, after
/// normalizing Windows extended-length prefixes on both sides.
pub(crate) fn path_is_within(candidate: &Path, root: &Path) -> bool {
    let candidate = normalized_path_string(candidate);
    let root = normalized_path_string(root);
    if candidate == root {
        return true;
    }
    candidate
        .strip_prefix(&root)
        .is_some_and(|rest| rest.starts_with('\\') || rest.starts_with('/'))
}

async fn transition_item(
    pool: &SqlitePool,
    capture_item_id: &str,
    from: &str,
    to: &str,
    clear_error: bool,
) -> Result<CaptureItem, AppError> {
    let capture_item_id = required(capture_item_id, "capture item id")?;
    let item = sqlx::query_as::<_, CaptureItem>(
        r#"
        UPDATE capture_items
        SET
            status = ?,
            error_message = CASE WHEN ? THEN NULL ELSE error_message END,
            failure_stage = CASE WHEN ? THEN NULL ELSE failure_stage END,
            next_retry_at = CASE WHEN ? THEN NULL ELSE next_retry_at END,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ? AND status = ?
        RETURNING
            id, project_id, session_id, asset_id, character_id, classification, source_path, file_size, modified_at_ms, content_hash,
            annotated_path, avatar_path, destination_path,
            destination_avatar_path, status, face_box_json, face_count,
            suggested_character_id, recognition_confidence, recognition_source,
            review_status, error_message,
            failure_stage, attempt_count, next_retry_at, processing_warnings_json,
            captured_at, processed_at, archived_at, created_at, updated_at
        "#,
    )
    .bind(to)
    .bind(clear_error)
    .bind(clear_error)
    .bind(clear_error)
    .bind(capture_item_id)
    .bind(from)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| {
        AppError::Conflict(format!(
            "capture item can only transition from {from} to {to}"
        ))
    })?;
    Ok(item)
}

async fn canonical_existing_directory(path: &Path) -> Result<PathBuf, AppError> {
    if !path.is_absolute() {
        return Err(AppError::Validation(
            "capture source directory must be an absolute path".to_owned(),
        ));
    }
    let canonical = tokio::fs::canonicalize(path).await?;
    if !tokio::fs::metadata(&canonical).await?.is_dir() {
        return Err(AppError::Validation(
            "capture source path is not a directory".to_owned(),
        ));
    }
    Ok(canonical)
}

pub(crate) async fn session_directories(
    pool: &SqlitePool,
    session: &CaptureSession,
) -> Result<Vec<PathBuf>, AppError> {
    let directories = session_source_directories(pool, &session.id).await?;
    Ok(directories
        .into_iter()
        .filter(|dir| dir.enabled != 0)
        .map(|dir| PathBuf::from(dir.directory))
        .collect())
}

async fn sha256_file(path: &Path) -> Result<String, AppError> {
    let bytes = tokio::fs::read(path).await?;
    tokio::task::spawn_blocking(move || {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        format!("{:x}", hasher.finalize())
    })
    .await
    .map_err(|error| AppError::Io(std::io::Error::other(error.to_string())))
}

async fn canonical_capture_path(
    session_dirs: &[PathBuf],
    path: &Path,
) -> Result<PathBuf, AppError> {
    if !path.is_absolute() {
        return Err(AppError::Validation(
            "capture source path must be absolute".to_owned(),
        ));
    }
    if !is_supported_image(path) {
        return Err(AppError::Validation(
            "capture source must be a supported image".to_owned(),
        ));
    }
    let canonical_path = tokio::fs::canonicalize(path).await?;
    let metadata = tokio::fs::metadata(&canonical_path).await?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(AppError::Validation(
            "capture source must be a non-empty file".to_owned(),
        ));
    }
    if !session_dirs
        .iter()
        .any(|root| path_is_within(&canonical_path, root))
    {
        return Err(AppError::Validation(
            "capture source must be inside one of the session source directories".to_owned(),
        ));
    }
    Ok(canonical_path)
}

async fn snapshot_existing_images(root: &Path) -> Result<Vec<BaselineFile>, AppError> {
    let mut directory = tokio::fs::read_dir(root).await?;
    let mut files = Vec::new();
    while let Some(entry) = directory.next_entry().await? {
        let path = entry.path();
        if !is_supported_image(&path) {
            continue;
        }
        let metadata = match entry.metadata().await {
            Ok(metadata) if metadata.is_file() => metadata,
            _ => continue,
        };
        let canonical = match tokio::fs::canonicalize(&path).await {
            Ok(path) if path_is_within(&path, root) => path,
            _ => continue,
        };
        let file_size = i64::try_from(metadata.len())
            .map_err(|_| AppError::Validation("capture source is too large".to_owned()))?;
        files.push(BaselineFile {
            source_path: path_to_string(&canonical),
            file_size,
            modified_at_ms: metadata
                .modified()
                .ok()
                .and_then(|value| unix_millis(value).ok()),
        });
    }
    Ok(files)
}

async fn validate_local_output(path: &Path) -> Result<PathBuf, AppError> {
    if !path.is_absolute() {
        return Err(AppError::Validation(
            "Python output path must be absolute".to_owned(),
        ));
    }
    if is_unc_path(path) {
        return Err(AppError::Validation(
            "Python output must be local; Rust performs NAS archival".to_owned(),
        ));
    }
    if !is_supported_image(path) {
        return Err(AppError::Validation(
            "Python output must be a supported image".to_owned(),
        ));
    }
    let canonical = tokio::fs::canonicalize(path).await?;
    let metadata = tokio::fs::metadata(&canonical).await?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(AppError::Validation(
            "Python output must be a non-empty file".to_owned(),
        ));
    }
    Ok(canonical)
}

fn is_unc_path(path: &Path) -> bool {
    path.to_string_lossy().starts_with(r"\\")
}

fn required<'a>(value: &'a str, label: &str) -> Result<&'a str, AppError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(AppError::Validation(format!("{label} cannot be empty")));
    }
    Ok(value)
}

fn normalized_error_message(value: &str) -> Result<String, AppError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(AppError::Validation(
            "capture error message cannot be empty".to_owned(),
        ));
    }
    Ok(truncate_chars(value, MAX_ERROR_MESSAGE_CHARS))
}

#[test]
fn path_is_within_normalizes_windows_extended_prefixes() {
    assert!(path_is_within(
        Path::new(r"\\?\D:\games\0.1\shot.png"),
        Path::new(r"D:\games\0.1")
    ));
    assert!(path_is_within(
        Path::new(r"D:\games\0.1\shot.png"),
        Path::new(r"\\?\D:\games\0.1")
    ));
    assert!(!path_is_within(
        Path::new(r"D:\games\0.2\shot.png"),
        Path::new(r"D:\games\0.1")
    ));
    assert!(path_is_within(
        Path::new(r"\\?\UNC\NAS\share\a.png"),
        Path::new(r"\\NAS\share")
    ));
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

pub(crate) fn unix_millis(value: SystemTime) -> Result<i64, AppError> {
    let millis = value
        .duration_since(UNIX_EPOCH)
        .map_err(|_| AppError::Validation("filesystem timestamp predates Unix epoch".to_owned()))?
        .as_millis();
    i64::try_from(millis)
        .map_err(|_| AppError::Validation("filesystem timestamp is out of range".to_owned()))
}

async fn verify_stable_file(path: &Path, delay: Duration) -> Result<(), AppError> {
    let first = tokio::fs::metadata(path).await?;
    if !first.is_file() || first.len() == 0 {
        return Err(AppError::Validation(
            "capture source must be a non-empty file".to_owned(),
        ));
    }
    tokio::time::sleep(delay).await;
    let second = tokio::fs::metadata(path).await?;
    if !second.is_file()
        || second.len() != first.len()
        || (first.modified().ok().is_some() && first.modified().ok() != second.modified().ok())
    {
        return Err(AppError::Conflict(
            "capture source is still being written".to_owned(),
        ));
    }
    Ok(())
}
