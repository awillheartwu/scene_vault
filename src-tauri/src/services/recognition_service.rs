use sqlx::SqlitePool;

use crate::{
    error::AppError,
    models::{
        capture::{CaptureItem, RelabelCaptureInput},
        recognition::{
            FaceBankModelCount, FaceBankModelStatus, FaceBankRebuildSummary, FaceRow, FaceSample,
            ListCharacterItemsInput, ReviewRecognitionInput, SetFaceSampleFlaggedInput,
            SetFaceSampleStatusInput, SetRecognitionSuggestionInput, VerificationResult,
            VerifyCaptureIdentityInput, FACE_MODEL_ID, FACE_MODEL_VERSION,
        },
    },
    services::{
        capture_service, processing_settings_service, recognition_settings_service,
        vision_engine_service, vision_settings_service,
    },
};

const RECOGNITION_SOURCES: [&str; 3] = ["face_bank", "vision", "manual"];
const REVIEW_DECISIONS: [&str; 2] = ["accepted", "rejected"];

/// The primary face row of a capture, if any. Phase 1 writes exactly one
/// primary row; the schema allows more for a future multi-face phase.
async fn primary_face(
    pool: &SqlitePool,
    capture_item_id: &str,
) -> Result<Option<FaceRow>, AppError> {
    let face = sqlx::query_as::<_, FaceRow>(
        r#"
        SELECT
            id, capture_item_id, face_index, is_primary, box_json, feature_json,
            feature_model_id, feature_model_version, feature_dim,
            face_sharpness, face_area_ratio,
            confirmed_character_id, created_at, updated_at
        FROM capture_faces
        WHERE capture_item_id = ? AND is_primary = 1
        "#,
    )
    .bind(capture_item_id)
    .fetch_optional(pool)
    .await?;
    Ok(face)
}

/// Writes or clears an automatic character suggestion. A new suggestion always
/// resets `review_status` to `pending`; a cleared suggestion resets it to
/// `none`. The suggestion never changes `character_id` itself.
pub async fn set_suggestion(
    pool: &SqlitePool,
    input: SetRecognitionSuggestionInput,
) -> Result<CaptureItem, AppError> {
    let capture_item_id = input.capture_item_id.trim();
    if capture_item_id.is_empty() {
        return Err(AppError::Validation(
            "capture item id cannot be empty".to_owned(),
        ));
    }
    if let Some(confidence) = input.confidence {
        if !(0.0..=1.0).contains(&confidence) {
            return Err(AppError::Validation(
                "recognition confidence must be between 0.0 and 1.0".to_owned(),
            ));
        }
    }
    if let Some(source) = input.source.as_deref() {
        if !RECOGNITION_SOURCES.contains(&source) {
            return Err(AppError::Validation(format!(
                "recognition source must be one of {}",
                RECOGNITION_SOURCES.join(", ")
            )));
        }
    }

    // Exists check so a missing item surfaces as NotFound, not a DB row error.
    capture_service::get_item(pool, capture_item_id).await?;
    if let Some(suggested_character_id) = input.suggested_character_id.as_deref() {
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
        .bind(suggested_character_id)
        .bind(capture_item_id)
        .fetch_one(pool)
        .await?;
        if belongs_to_project == 0 {
            return Err(AppError::Validation(
                "suggested character and capture item must belong to the same project".to_owned(),
            ));
        }
    }

    let item = sqlx::query_as::<_, CaptureItem>(
        r#"
        UPDATE capture_items
        SET
            suggested_character_id = ?,
            recognition_confidence = ?,
            recognition_source = ?,
            review_status = ?,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ?
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
    .bind(&input.suggested_character_id)
    .bind(input.confidence)
    .bind(&input.source)
    .bind(if input.suggested_character_id.is_some() {
        "pending"
    } else {
        "none"
    })
    .bind(capture_item_id)
    .fetch_one(pool)
    .await?;
    Ok(item)
}

/// Records the human decision on a pending suggestion. `accepted` keeps the
/// suggestion for the workbench history; `rejected` also clears the suggestion
/// fields so a later Face Bank run can propose again.
pub async fn review_suggestion(
    pool: &SqlitePool,
    input: ReviewRecognitionInput,
) -> Result<CaptureItem, AppError> {
    let capture_item_id = input.capture_item_id.trim();
    if capture_item_id.is_empty() {
        return Err(AppError::Validation(
            "capture item id cannot be empty".to_owned(),
        ));
    }
    let decision = input.decision.trim().to_ascii_lowercase();
    if !REVIEW_DECISIONS.contains(&decision.as_str()) {
        return Err(AppError::Validation(
            "review decision must be accepted or rejected".to_owned(),
        ));
    }
    let current = capture_service::get_item(pool, capture_item_id).await?;
    if current.suggested_character_id.is_none() {
        return Err(AppError::Validation(
            "no recognition suggestion to review".to_owned(),
        ));
    }

    let item = sqlx::query_as::<_, CaptureItem>(
        r#"
        UPDATE capture_items
        SET
            review_status = ?,
            suggested_character_id = CASE WHEN ? = 'rejected'
                THEN NULL ELSE suggested_character_id END,
            recognition_confidence = CASE WHEN ? = 'rejected'
                THEN NULL ELSE recognition_confidence END,
            recognition_source = CASE WHEN ? = 'rejected'
                THEN NULL ELSE recognition_source END,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ?
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
    .bind(&decision)
    .bind(&decision)
    .bind(&decision)
    .bind(&decision)
    .bind(capture_item_id)
    .fetch_one(pool)
    .await?;
    Ok(item)
}

/// Accepts the exact pending suggestion currently shown to the user and turns
/// it into a real correction. The existing relabel state machine queues
/// processing and keeps old archive paths until a verified replacement is
/// ready; no database transaction is held across Python or NAS work.
pub async fn accept_suggestion(
    pool: &SqlitePool,
    capture_item_id: &str,
) -> Result<CaptureItem, AppError> {
    let capture_item_id = capture_item_id.trim();
    if capture_item_id.is_empty() {
        return Err(AppError::Validation(
            "capture item id cannot be empty".to_owned(),
        ));
    }
    let current = capture_service::get_item(pool, capture_item_id).await?;
    if current.review_status != "pending" {
        return Err(AppError::Conflict(
            "recognition suggestion is no longer pending".to_owned(),
        ));
    }
    let suggested_character_id = current.suggested_character_id.clone().ok_or_else(|| {
        AppError::Conflict("recognition suggestion is no longer available".to_owned())
    })?;

    if current.character_id.as_deref() == Some(suggested_character_id.as_str())
        && current.classification == "person"
    {
        return review_suggestion(
            pool,
            ReviewRecognitionInput {
                capture_item_id: capture_item_id.to_owned(),
                decision: "accepted".to_owned(),
            },
        )
        .await;
    }

    capture_service::relabel_capture_item_from_suggestion(
        pool,
        RelabelCaptureInput {
            capture_item_id: capture_item_id.to_owned(),
            character_id: Some(suggested_character_id.clone()),
            classification: Some("person".to_owned()),
        },
        &suggested_character_id,
    )
    .await
}

/// Rejects a pending suggestion and re-enrolls the capture's face into its
/// currently labeled character's sample bank. Games that reuse one face
/// model across characters make cross-character matches unavoidable; this
/// anchors the exact face to the character the user labeled so future
/// identical frames match the correct bank first.
pub async fn reject_and_enroll(
    pool: &SqlitePool,
    capture_item_id: &str,
) -> Result<CaptureItem, AppError> {
    let item = review_suggestion(
        pool,
        ReviewRecognitionInput {
            capture_item_id: capture_item_id.to_owned(),
            decision: "rejected".to_owned(),
        },
    )
    .await?;
    enroll_face_sample(pool, capture_item_id).await?;
    Ok(item)
}

/// Rejects every pending suggestion on one character's captures and enrolls
/// each face into that character's sample bank. One click for the common
/// "the game reuses one face model, all suggestions for this character are
/// noise" case: clear the noise and anchor every face to the labeled
/// character at once.
pub async fn batch_reject_and_enroll(
    pool: &SqlitePool,
    project_id: &str,
    character_id: &str,
) -> Result<i64, AppError> {
    let ids: Vec<String> = sqlx::query_scalar(
        r#"
        SELECT item.id
        FROM capture_items item
        JOIN capture_sessions session ON session.id = item.session_id
        WHERE session.project_id = ?
          AND item.character_id = ?
          AND item.classification = 'person'
          AND item.suggested_character_id IS NOT NULL
          AND item.review_status = 'pending'
        "#,
    )
    .bind(project_id)
    .bind(character_id)
    .fetch_all(pool)
    .await?;

    for id in &ids {
        sqlx::query(
            r#"
            UPDATE capture_items
            SET suggested_character_id = NULL,
                recognition_confidence = NULL,
                recognition_source = NULL,
                review_status = 'none',
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
            WHERE id = ?
            "#,
        )
        .bind(id)
        .execute(pool)
        .await?;
        enroll_face_sample(pool, id).await?;
    }
    Ok(ids.len() as i64)
}

/// All person captures labeled to one character, newest first, for the
/// workbench grid. Scene/private captures never belong to the person
/// dimension, even when a `character_id` lingers after a relabel. Suggestion
/// and review fields travel with each item so the UI can show recognition
/// sources and pending decisions without extra queries.
pub async fn list_character_items(
    pool: &SqlitePool,
    input: ListCharacterItemsInput,
) -> Result<Vec<CaptureItem>, AppError> {
    let project_id = input.project_id.trim();
    let character_id = input.character_id.trim();
    if project_id.is_empty() || character_id.is_empty() {
        return Err(AppError::Validation(
            "project id and character id cannot be empty".to_owned(),
        ));
    }
    let belongs_to_project: i64 = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM characters WHERE id = ? AND project_id = ?)",
    )
    .bind(character_id)
    .bind(project_id)
    .fetch_one(pool)
    .await?;
    if belongs_to_project == 0 {
        return Err(AppError::NotFound("character in project".to_owned()));
    }

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
        JOIN capture_sessions session ON session.id = item.session_id
        WHERE session.project_id = ?
          AND item.character_id = ?
          AND item.classification = 'person'
        ORDER BY item.captured_at DESC, item.created_at DESC
        "#,
    )
    .bind(project_id)
    .bind(character_id)
    .fetch_all(pool)
    .await?;
    Ok(items)
}

/// Face-bank samples of one character, newest first, for the workbench
/// sample strip.
pub async fn list_character_face_samples(
    pool: &SqlitePool,
    character_id: &str,
) -> Result<Vec<FaceSample>, AppError> {
    let character_id = character_id.trim();
    if character_id.is_empty() {
        return Err(AppError::Validation(
            "character id cannot be empty".to_owned(),
        ));
    }
    let samples = sqlx::query_as::<_, FaceSample>(
        r#"
        SELECT
            id, character_id, capture_item_id, face_box_json, confidence,
            status, flagged, created_at, updated_at
        FROM character_face_samples
        WHERE character_id = ?
        ORDER BY created_at DESC, id DESC
        "#,
    )
    .bind(character_id)
    .fetch_all(pool)
    .await?;
    Ok(samples)
}

/// Toggles one sample between `active` and `revoked`. Revoked samples are
/// excluded from matching but kept for auditability.
pub async fn set_face_sample_status(
    pool: &SqlitePool,
    input: SetFaceSampleStatusInput,
) -> Result<FaceSample, AppError> {
    let sample_id = input.sample_id.trim();
    if sample_id.is_empty() {
        return Err(AppError::Validation(
            "face sample id cannot be empty".to_owned(),
        ));
    }
    let status = input.status.trim().to_ascii_lowercase();
    if !matches!(status.as_str(), "active" | "revoked") {
        return Err(AppError::Validation(
            "face sample status must be active or revoked".to_owned(),
        ));
    }
    let sample = sqlx::query_as::<_, FaceSample>(
        r#"
        UPDATE character_face_samples
        SET
            status = ?,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ?
        RETURNING
            id, character_id, capture_item_id, face_box_json, confidence,
            status, flagged, created_at, updated_at
        "#,
    )
    .bind(&status)
    .bind(sample_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound("face sample".to_owned()))?;
    Ok(sample)
}

/// Toggles the flagged marker of one sample. Flagged samples (from
/// low-confidence forced confirms) do not participate in matching; clearing
/// the flag restores the sample and marks the related capture as verified.
pub async fn set_face_sample_flagged(
    pool: &SqlitePool,
    input: SetFaceSampleFlaggedInput,
) -> Result<FaceSample, AppError> {
    let sample_id = input.sample_id.trim();
    if sample_id.is_empty() {
        return Err(AppError::Validation(
            "face sample id cannot be empty".to_owned(),
        ));
    }
    let mut transaction = pool.begin().await?;
    let sample = sqlx::query_as::<_, FaceSample>(
        r#"
        UPDATE character_face_samples
        SET
            flagged = ?,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ?
        RETURNING
            id, character_id, capture_item_id, face_box_json, confidence,
            status, flagged, created_at, updated_at
        "#,
    )
    .bind(if input.flagged { 1 } else { 0 })
    .bind(sample_id)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or_else(|| AppError::NotFound("face sample".to_owned()))?;
    if !input.flagged {
        // The player accepted this sample: the related capture's label is
        // treated as verified so a later reprocess does not re-flag it.
        sqlx::query(
            r#"
            UPDATE capture_items
            SET verification_status = 'ok'
            WHERE id = ? AND verification_status = 'flagged'
            "#,
        )
        .bind(&sample.capture_item_id)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;
    Ok(sample)
}

/// Closed-set verification: the highest similarity of this capture's face
/// against the selected character's active, unflagged samples, plus the best
/// score against every other character (the "more like someone else" signal).
/// `level` is computed from the configured thresholds.
pub async fn verify_capture_identity(
    pool: &SqlitePool,
    input: VerifyCaptureIdentityInput,
) -> Result<VerificationResult, AppError> {
    let item = capture_service::get_item(pool, input.capture_item_id.trim()).await?;
    compute_verification(pool, &item, input.character_id.trim()).await
}

pub(crate) async fn compute_verification(
    pool: &SqlitePool,
    item: &CaptureItem,
    character_id: &str,
) -> Result<VerificationResult, AppError> {
    let face = primary_face(pool, &item.id).await?;
    let has_feature = face.as_ref().is_some_and(FaceRow::has_feature);
    let feature: Option<Vec<f64>> = face
        .as_ref()
        .and_then(|face| face.feature_json.as_deref())
        .filter(|value| *value != "[]")
        .map(serde_json::from_str)
        .transpose()
        .map_err(|error| AppError::Validation(format!("invalid stored face feature: {error}")))?;
    let (model_id, model_version) = match face.as_ref() {
        Some(face) => (
            face.feature_model_id.clone(),
            face.feature_model_version.clone(),
        ),
        None => (None, None),
    };
    let settings = recognition_settings_service::get(pool).await?;
    let profile = recognition_settings_service::resolved(
        pool,
        model_id.as_deref().unwrap_or("unknown-model"),
    )
    .await?;

    let samples: Vec<(String, String)> = sqlx::query_as(
        r#"
        SELECT sample.character_id, sample.feature_json
        FROM character_face_samples sample
        JOIN characters character ON character.id = sample.character_id
        JOIN capture_items item ON item.id = sample.capture_item_id
        WHERE character.project_id = (
            SELECT project_id FROM capture_sessions WHERE id = ?
        )
          AND sample.status = 'active'
          AND sample.flagged = 0
          AND item.classification = 'person'
          AND sample.model_id = ?
          AND sample.model_version = ?
        "#,
    )
    .bind(&item.session_id)
    .bind(model_id)
    .bind(model_version)
    .fetch_all(pool)
    .await?;

    let mut score = None;
    let mut best_other_score = None;
    let mut best_other_character_id = None;
    let mut has_samples = false;
    if let Some(feature) = &feature {
        for (sample_character_id, sample_json) in samples {
            let Ok(sample): Result<Vec<f64>, _> = serde_json::from_str(&sample_json) else {
                continue;
            };
            let similarity = cosine_similarity(feature, &sample);
            if sample_character_id == character_id {
                has_samples = true;
                score = Some(score.map_or(similarity, |current: f64| current.max(similarity)));
            } else if best_other_score.is_none_or(|current| similarity > current) {
                best_other_score = Some(similarity);
                best_other_character_id = Some(sample_character_id);
            }
        }
    }

    let level = if !has_feature || !has_samples || !settings.verification_enabled {
        "unverified".to_owned()
    } else {
        let score = score.unwrap_or(0.0);
        let cross_alarm = best_other_score.is_some_and(|other| {
            other - score > profile.cross_check_delta
                && other >= profile.verification_high_threshold
        });
        if score >= profile.verification_high_threshold {
            "ok".to_owned()
        } else if score < profile.verification_low_threshold || cross_alarm {
            "strong".to_owned()
        } else {
            "low".to_owned()
        }
    };

    Ok(VerificationResult {
        score,
        best_other_score,
        best_other_character_id,
        has_samples,
        has_feature,
        level,
    })
}

#[cfg(test)]
#[path = "recognition_service_tests.rs"]
mod tests;

/// Persists a verification result on the capture item. `ok` keeps the label
/// verified; `low`/`strong` (forced confirms) mark the item as `flagged` so
/// its enrolled sample stays out of matching until restored.
pub(crate) async fn persist_verification(
    pool: &SqlitePool,
    capture_item_id: &str,
    result: &VerificationResult,
) -> Result<(), AppError> {
    let status = match result.level.as_str() {
        "ok" => "ok",
        "low" | "strong" => "flagged",
        _ => "unverified",
    };
    sqlx::query(
        r#"
        UPDATE capture_items
        SET
            verification_score = ?,
            verification_status = ?,
            best_other_score = ?,
            best_other_character_id = ?,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ?
        "#,
    )
    .bind(result.score)
    .bind(status)
    .bind(result.best_other_score)
    .bind(&result.best_other_character_id)
    .bind(capture_item_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Stores (or replaces) one character's face sample for a human-confirmed
/// capture. The feature and face box were produced by the Python engine and
/// persisted on the capture item; a relabel/reprocess updates the sample in
/// place (the row key is character + capture item).
pub async fn enroll_face_sample(
    pool: &SqlitePool,
    capture_item_id: &str,
) -> Result<bool, AppError> {
    let item = capture_service::get_item(pool, capture_item_id).await?;
    let Some(character_id) = item.character_id.as_deref() else {
        remove_face_samples(pool, capture_item_id, None).await?;
        return Ok(false);
    };
    if item.classification != "person" {
        remove_face_samples(pool, capture_item_id, None).await?;
        return Ok(false);
    }
    let Some(face) = primary_face(pool, capture_item_id).await? else {
        remove_face_samples(pool, capture_item_id, None).await?;
        return Ok(false);
    };
    if !face.has_feature() {
        remove_face_samples(pool, capture_item_id, None).await?;
        return Ok(false);
    }
    // Quality gate: printed/photo faces in scenes score near-zero sharpness
    // (e.g. 1.4 vs ~35 for real game faces) and background faces occupy a
    // tiny area share. Such samples would poison the Face Bank, so they are
    // skipped and every failed check is recorded with its value.
    let settings = recognition_settings_service::get(pool).await?;
    let mut gate_reasons: Vec<String> = Vec::new();
    if let Some(min_sharpness) = settings.min_sample_sharpness {
        let sharpness = face.face_sharpness;
        if sharpness.is_none_or(|value| value < min_sharpness) {
            gate_reasons.push(format!(
                "sharpness {} below {min_sharpness}",
                sharpness
                    .map(|value| format!("{value:.1}"))
                    .unwrap_or_else(|| "unknown".to_owned())
            ));
        }
    }
    if let Some(min_area) = settings.min_face_area_ratio {
        let area = face.face_area_ratio;
        if area.is_none_or(|value| value < min_area) {
            gate_reasons.push(format!(
                "area ratio {} below {:.2}%",
                area.map(|value| format!("{:.2}%", value * 100.0))
                    .unwrap_or_else(|| "unknown".to_owned()),
                min_area * 100.0
            ));
        }
    }
    if !gate_reasons.is_empty() {
        remove_face_samples(
            pool,
            capture_item_id,
            Some(&format!(
                "sample_not_enrolled_low_quality: {}",
                gate_reasons.join("; ")
            )),
        )
        .await?;
        return Ok(false);
    }
    let Some(feature_json) = face.feature_json.as_deref() else {
        remove_face_samples(pool, capture_item_id, None).await?;
        return Ok(false);
    };
    // A feature without model identity cannot be matched against other
    // samples; skip enrollment rather than storing an uncomparable vector.
    let (Some(model_id), Some(model_version)) = (
        face.feature_model_id.as_deref(),
        face.feature_model_version.as_deref(),
    ) else {
        remove_face_samples(
            pool,
            capture_item_id,
            Some("sample_not_enrolled_missing_model_identity"),
        )
        .await?;
        return Ok(false);
    };
    let embedding_dim = face.feature_dim;
    let verification_status: String =
        sqlx::query_scalar("SELECT verification_status FROM capture_items WHERE id = ?")
            .bind(capture_item_id)
            .fetch_one(pool)
            .await?;
    let flagged = if verification_status == "flagged" {
        1
    } else {
        0
    };
    let confidence: Option<f64> = item
        .face_box_json
        .as_deref()
        .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
        .and_then(|value| value.get("confidence").and_then(|value| value.as_f64()))
        .filter(|value| (0.0..=1.0).contains(value));

    let mut transaction = pool.begin().await?;
    // A capture can project to only its current character. Remove stale rows
    // left by a person-to-person relabel before the fresh UPSERT below.
    sqlx::query(
        "DELETE FROM character_face_samples WHERE capture_item_id = ? AND character_id != ?",
    )
    .bind(capture_item_id)
    .bind(character_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        r#"
        INSERT INTO character_face_samples (
            id, character_id, capture_item_id, face_box_json, feature_json,
            confidence, status, flagged, model_id, model_version, embedding_dim
        )
        VALUES (?, ?, ?, ?, ?, ?, 'active', ?, ?, ?, ?)
        ON CONFLICT(character_id, capture_item_id) DO UPDATE SET
            face_box_json = excluded.face_box_json,
            feature_json = excluded.feature_json,
            confidence = excluded.confidence,
            status = 'active',
            flagged = excluded.flagged,
            model_id = excluded.model_id,
            model_version = excluded.model_version,
            embedding_dim = excluded.embedding_dim,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        "#,
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(character_id)
    .bind(capture_item_id)
    .bind(face.box_json)
    .bind(feature_json)
    .bind(confidence)
    .bind(flagged)
    .bind(model_id)
    .bind(model_version)
    .bind(embedding_dim)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(true)
}

/// Removes every sample derived from one capture, optionally appending an
/// explanation in the same transaction. Repeated rebuilds do not duplicate an
/// identical warning.
async fn remove_face_samples(
    pool: &SqlitePool,
    capture_item_id: &str,
    warning: Option<&str>,
) -> Result<(), AppError> {
    let mut transaction = pool.begin().await?;
    sqlx::query("DELETE FROM character_face_samples WHERE capture_item_id = ?")
        .bind(capture_item_id)
        .execute(&mut *transaction)
        .await?;
    if let Some(message) = warning {
        let current: String =
            sqlx::query_scalar("SELECT processing_warnings_json FROM capture_items WHERE id = ?")
                .bind(capture_item_id)
                .fetch_one(&mut *transaction)
                .await?;
        let mut warnings: Vec<String> = serde_json::from_str(&current).unwrap_or_default();
        if !warnings.iter().any(|value| value == message) {
            warnings.push(message.to_owned());
        }
        let updated = serde_json::to_string(&warnings)
            .map_err(|error| AppError::Validation(format!("cannot encode warnings: {error}")))?;
        sqlx::query("UPDATE capture_items SET processing_warnings_json = ? WHERE id = ?")
            .bind(updated)
            .bind(capture_item_id)
            .execute(&mut *transaction)
            .await?;
    }
    transaction.commit().await?;
    Ok(())
}

/// Matches one capture's feature against the project's active samples and
/// writes the best suggestion when it clears the configured threshold. The
/// suggestion never changes `character_id`; humans confirm it later.
///
/// The capture's own sample and — for already-labeled captures — the labeled
/// character's samples are excluded, so a capture never suggests its own
/// identity (that would always be a cosine 1.0 self-match right after the
/// worker enrolls the same capture's feature).
pub async fn suggest_from_face_bank(
    pool: &SqlitePool,
    capture_item_id: &str,
) -> Result<(), AppError> {
    let item = capture_service::get_item(pool, capture_item_id).await?;
    // A refresh must not leave an old Face Bank result visible when the
    // current samples or thresholds no longer produce a valid candidate.
    // Preserve manual/other-provider suggestions; this function owns only
    // results previously written by the Face Bank.
    if item.recognition_source.as_deref() == Some("face_bank") {
        set_suggestion(
            pool,
            SetRecognitionSuggestionInput {
                capture_item_id: capture_item_id.to_owned(),
                suggested_character_id: None,
                confidence: None,
                source: None,
            },
        )
        .await?;
    }
    let Some(face) = primary_face(pool, capture_item_id).await? else {
        return Ok(());
    };
    if !face.has_feature() {
        return Ok(());
    }
    // Without model identity the feature cannot be compared against the
    // sample bank; a mismatched model would be a meaningless cosine.
    let (Some(model_id), Some(model_version)) = (
        face.feature_model_id.as_deref(),
        face.feature_model_version.as_deref(),
    ) else {
        return Ok(());
    };
    let project_id: String =
        sqlx::query_scalar("SELECT project_id FROM capture_sessions WHERE id = ?")
            .bind(&item.session_id)
            .fetch_one(pool)
            .await?;
    // "" never matches a real UUID, so unlabeled captures only exclude their
    // own sample while labeled ones also exclude their character's samples.
    let exclude_character = item.character_id.clone().unwrap_or_default();

    let rows: Vec<(String, String)> = sqlx::query_as(
        r#"
        SELECT sample.character_id, sample.feature_json
        FROM character_face_samples sample
        JOIN characters character ON character.id = sample.character_id
        JOIN capture_items item ON item.id = sample.capture_item_id
        WHERE character.project_id = ?
          AND sample.status = 'active'
          AND sample.flagged = 0
          AND item.classification = 'person'
          AND sample.capture_item_id != ?
          AND sample.character_id != ?
          AND sample.model_id = ?
          AND sample.model_version = ?
        "#,
    )
    .bind(&project_id)
    .bind(capture_item_id)
    .bind(exclude_character)
    .bind(model_id)
    .bind(model_version)
    .fetch_all(pool)
    .await?;
    // An empty bank (the first batch of a new project) has nothing to
    // compare against: skip the feature decode and similarity work entirely.
    if rows.is_empty() {
        return Ok(());
    }
    let Some(feature_json) = face.feature_json.as_deref() else {
        return Ok(());
    };
    let feature: Vec<f64> = serde_json::from_str(feature_json)
        .map_err(|error| AppError::Validation(format!("invalid stored face feature: {error}")))?;
    let profile = recognition_settings_service::resolved(pool, model_id).await?;

    // Best similarity per character (max over that character's samples),
    // then a threshold AND margin gate: the top candidate must clear the
    // score floor and beat the runner-up by `margin`, otherwise no
    // suggestion ("宁愿不推荐，也别自信地推荐错人").
    let mut character_scores: std::collections::HashMap<String, f64> =
        std::collections::HashMap::new();
    for (character_id, sample_json) in rows {
        let sample: Vec<f64> = match serde_json::from_str(&sample_json) {
            Ok(sample) => sample,
            Err(_) => continue,
        };
        let similarity = cosine_similarity(&feature, &sample);
        let entry = character_scores.entry(character_id).or_insert(0.0);
        *entry = entry.max(similarity);
    }
    let mut ranked: Vec<(String, f64)> = character_scores.into_iter().collect();
    ranked.sort_by(|left, right| right.1.total_cmp(&left.1));
    if let Some((character_id, similarity)) = ranked.first().cloned() {
        let runner_up = ranked.get(1).map(|(_, score)| *score).unwrap_or(0.0);
        if similarity < profile.confidence_threshold || similarity - runner_up < profile.margin {
            return Ok(());
        }
        set_suggestion(
            pool,
            SetRecognitionSuggestionInput {
                capture_item_id: capture_item_id.to_owned(),
                suggested_character_id: Some(character_id),
                confidence: Some(similarity),
                source: Some("face_bank".to_owned()),
            },
        )
        .await?;
    }
    Ok(())
}

/// Re-evaluates unclassified captures after a Face Bank rebuild. This keeps
/// pending recommendations aligned with the rebuilt features and the current
/// threshold/margin profile instead of leaving pre-rebuild results in place.
async fn refresh_project_suggestions(pool: &SqlitePool, project_id: &str) -> Result<u32, AppError> {
    let item_ids: Vec<String> = sqlx::query_scalar(
        r#"
        SELECT item.id
        FROM capture_items item
        WHERE item.project_id = ?
          AND item.classification = 'unclassified'
          AND (
            item.recognition_source = 'face_bank'
            OR EXISTS (
              SELECT 1
              FROM capture_faces face
              WHERE face.capture_item_id = item.id
                AND face.is_primary = 1
                AND face.feature_json IS NOT NULL
                AND face.feature_json != '[]'
            )
          )
        ORDER BY item.captured_at ASC, item.created_at ASC
        "#,
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?;
    for item_id in &item_ids {
        suggest_from_face_bank(pool, item_id).await?;
    }
    Ok(item_ids.len() as u32)
}

fn cosine_similarity(first: &[f64], second: &[f64]) -> f64 {
    if first.is_empty() || first.len() != second.len() {
        return 0.0;
    }
    let mut dot = 0.0;
    let mut first_norm = 0.0;
    let mut second_norm = 0.0;
    for (a, b) in first.iter().zip(second) {
        dot += a * b;
        first_norm += a * a;
        second_norm += b * b;
    }
    let denominator = first_norm.sqrt() * second_norm.sqrt();
    if denominator == 0.0 {
        0.0
    } else {
        dot / denominator
    }
}

/// Rebuilds the Face Bank of one project: every person capture is re-extracted
/// (feature only, no archive/annotation rerun) with the currently configured
/// model, then re-enrolled. Missing source files drop their samples (they can
/// never be regenerated); per-item extraction failures are counted and do not
/// abort the rest. Progress is reported as `(processed, total)`.
pub async fn rebuild_face_bank(
    pool: &SqlitePool,
    project_id: &str,
    mut on_progress: impl FnMut(u32, u32) + Send,
) -> Result<FaceBankRebuildSummary, AppError> {
    let project_id = project_id.trim();
    if project_id.is_empty() {
        return Err(AppError::Validation(
            "project id cannot be empty".to_owned(),
        ));
    }
    let exists: i64 = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM projects WHERE id = ?)")
        .bind(project_id)
        .fetch_one(pool)
        .await?;
    if exists == 0 {
        return Err(AppError::NotFound("project".to_owned()));
    }
    let vision_settings = vision_settings_service::get(pool).await?;
    if !vision_settings_service::is_configured(&vision_settings) {
        return Err(AppError::Vision(
            "vision engine is not configured; set the Python executable, module root and models before rebuilding the face bank".to_owned(),
        ));
    }
    let processing_settings = processing_settings_service::get(pool).await?;

    let items: Vec<(String, String)> = sqlx::query_as(
        r#"
        SELECT id, source_path
        FROM capture_items
        WHERE project_id = ? AND classification = 'person'
        ORDER BY captured_at ASC, created_at ASC
        "#,
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?;
    let total = items.len() as u32;
    let mut processed = 0_u32;
    let mut rebuilt = 0_u32;
    let mut no_face = 0_u32;
    let mut not_enrolled = 0_u32;
    let mut skipped_missing_source = 0_u32;
    let mut failed = 0_u32;
    let mut stale_preserved = 0_u32;

    for (item_id, source_path) in items {
        processed += 1;
        on_progress(processed, total);
        let source = std::path::PathBuf::from(&source_path);
        if !tokio::fs::metadata(&source)
            .await
            .is_ok_and(|metadata| metadata.is_file())
        {
            skipped_missing_source += 1;
            // A sample whose source is gone can never be regenerated.
            sqlx::query("DELETE FROM character_face_samples WHERE capture_item_id = ?")
                .bind(&item_id)
                .execute(pool)
                .await?;
            continue;
        }
        let response = match vision_engine_service::extract_face_feature(
            &vision_settings,
            &source,
            &processing_settings,
            None,
        )
        .await
        {
            Ok(response) => response,
            Err(_) => {
                failed += 1;
                let has_stale_sample: i64 = sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM character_face_samples WHERE capture_item_id = ?)",
                )
                .bind(&item_id)
                .fetch_one(pool)
                .await?;
                if has_stale_sample != 0 {
                    stale_preserved += 1;
                }
                continue;
            }
        };
        let has_feature = response.face_feature.is_some();
        let feature_json = match response.face_feature {
            Some(feature) => serde_json::to_string(&feature).map_err(|error| {
                AppError::Vision(format!("cannot encode face feature: {error}"))
            })?,
            None => "[]".to_owned(),
        };
        let face_box_json = response
            .face_box
            .map(|value| serde_json::to_string(&value))
            .transpose()
            .map_err(|error| AppError::Vision(format!("cannot encode face box: {error}")))?;
        if capture_service::store_face_feature(
            pool,
            &item_id,
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
        .await
        .is_err()
        {
            failed += 1;
            continue;
        }
        let enrolled = match enroll_face_sample(pool, &item_id).await {
            Ok(enrolled) => enrolled,
            Err(_) => {
                failed += 1;
                let has_stale_sample: i64 = sqlx::query_scalar(
                    "SELECT EXISTS (SELECT 1 FROM character_face_samples WHERE capture_item_id = ?)",
                )
                .bind(&item_id)
                .fetch_one(pool)
                .await?;
                if has_stale_sample != 0 {
                    stale_preserved += 1;
                }
                continue;
            }
        };
        if !has_feature {
            no_face += 1;
        } else if enrolled {
            rebuilt += 1;
        } else {
            not_enrolled += 1;
        }
    }

    let suggestions_refreshed = refresh_project_suggestions(pool, project_id).await?;

    Ok(FaceBankRebuildSummary {
        total,
        rebuilt,
        no_face,
        not_enrolled,
        skipped_missing_source,
        failed,
        stale_preserved,
        suggestions_refreshed,
    })
}

/// Re-extracts one capture's primary-face feature with the currently
/// configured engine. Feature-only: no annotation/avatar output, no archive
/// rerun, and the capture's `classification`/`character_id` never change.
/// Only `person` and `unclassified` captures can be refreshed; queued,
/// processing and archive-pending items are rejected while the worker owns
/// them. On success the new feature replaces the old row atomically, the
/// current character's sample is re-enrolled, and the face-bank suggestion
/// is recomputed from the fresh vector.
pub async fn refresh_capture_face_feature(
    pool: &SqlitePool,
    capture_item_id: &str,
    on_progress: impl FnMut(&str, f64) + Send + 'static,
) -> Result<CaptureItem, AppError> {
    let capture_item_id = capture_item_id.trim();
    if capture_item_id.is_empty() {
        return Err(AppError::Validation(
            "capture item id cannot be empty".to_owned(),
        ));
    }
    let item = capture_service::get_item(pool, capture_item_id).await?;
    match item.classification.as_str() {
        "person" | "unclassified" => {}
        other => {
            return Err(AppError::Validation(format!(
                "only person or unclassified captures can refresh face features, got {other}"
            )));
        }
    }
    if matches!(
        item.status.as_str(),
        "queued" | "processing" | "archive_pending"
    ) {
        return Err(AppError::Conflict(format!(
            "capture is {status}; wait until processing finishes before refreshing its face feature",
            status = item.status
        )));
    }
    let vision_settings = vision_settings_service::get(pool).await?;
    if !vision_settings_service::is_configured(&vision_settings) {
        return Err(AppError::Vision(
            "vision engine is not configured; set the Python executable, module root and models before re-extracting face features"
                .to_owned(),
        ));
    }
    let processing_settings = processing_settings_service::get(pool).await?;
    let source = std::path::PathBuf::from(&item.source_path);
    if !tokio::fs::metadata(&source)
        .await
        .is_ok_and(|metadata| metadata.is_file())
    {
        return Err(AppError::NotFound(format!(
            "source file for capture {capture_item_id}"
        )));
    }
    let response = vision_engine_service::extract_face_feature(
        &vision_settings,
        &source,
        &processing_settings,
        Some(Box::new(on_progress)),
    )
    .await?;
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
        capture_item_id,
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
    // Re-enroll into the capture's current character (unclassified items
    // have no sample to keep) and recompute the suggestion from the fresh
    // vector; store_face_feature already cleared any stale suggestion.
    enroll_face_sample(pool, capture_item_id).await?;
    suggest_from_face_bank(pool, capture_item_id).await?;
    capture_service::get_item(pool, capture_item_id).await
}

/// Dominant model identity of a project's active Face Bank samples versus
/// the recognizer configured in the vision settings. `compatible` is true
/// when the bank is empty or every sample matches the active recognizer.
pub async fn face_bank_model_status(
    pool: &SqlitePool,
    project_id: &str,
) -> Result<FaceBankModelStatus, AppError> {
    let project_id = project_id.trim();
    if project_id.is_empty() {
        return Err(AppError::Validation(
            "project id cannot be empty".to_owned(),
        ));
    }
    let exists: i64 = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM projects WHERE id = ?)")
        .bind(project_id)
        .fetch_one(pool)
        .await?;
    if exists == 0 {
        return Err(AppError::NotFound("project".to_owned()));
    }
    let rows: Vec<(String, String, i64)> = sqlx::query_as(
        r#"
        SELECT sample.model_id, sample.model_version, COUNT(*)
        FROM character_face_samples sample
        JOIN characters character ON character.id = sample.character_id
        WHERE character.project_id = ?
          AND sample.status = 'active'
        GROUP BY sample.model_id, sample.model_version
        ORDER BY COUNT(*) DESC, sample.model_id, sample.model_version
        "#,
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?;
    let vision = vision_settings_service::get(pool).await?;
    let (active_model_id, active_model_version) = match vision.recognizer.as_deref() {
        Some("arcface") => ("arcface-r50".to_owned(), "w600k-r50".to_owned()),
        _ => (FACE_MODEL_ID.to_owned(), FACE_MODEL_VERSION.to_owned()),
    };
    let sample_count = rows.iter().map(|(_, _, count)| *count).sum();
    let incompatible_sample_count = rows
        .iter()
        .filter(|(model_id, model_version, _)| {
            model_id != &active_model_id || model_version != &active_model_version
        })
        .map(|(_, _, count)| *count)
        .sum();
    let model_counts: Vec<FaceBankModelCount> = rows
        .iter()
        .map(|(model_id, model_version, count)| FaceBankModelCount {
            model_id: model_id.clone(),
            model_version: model_version.clone(),
            sample_count: *count,
            compatible: model_id == &active_model_id && model_version == &active_model_version,
        })
        .collect();
    let dominant = rows.first();
    let bank_model_id = dominant.map(|(model_id, _, _)| model_id.clone());
    let bank_model_version = dominant.map(|(_, model_version, _)| model_version.clone());
    let compatible = incompatible_sample_count == 0;
    Ok(FaceBankModelStatus {
        bank_model_id,
        bank_model_version,
        active_model_id: Some(active_model_id),
        active_model_version: Some(active_model_version),
        sample_count,
        incompatible_sample_count,
        compatible,
        model_counts,
    })
}
