use std::path::{Path, PathBuf};

use serde_json::json;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    error::AppError,
    models::{
        archive_naming::ArchiveNamingSettings,
        capture::{CaptureItem, CaptureItemIdInput},
        diagnostics::{LogLevel, LogRecord},
    },
    services::{
        archive_naming::{self, ArchiveNameContext},
        archive_naming_settings_service, capture_service, log_service, processing_settings_service,
    },
};

pub async fn archive(
    pool: &SqlitePool,
    input: CaptureItemIdInput,
) -> Result<CaptureItem, AppError> {
    let capture_item_id = input.capture_item_id.trim();
    if capture_item_id.is_empty() {
        return Err(AppError::Validation(
            "capture item id cannot be empty".to_owned(),
        ));
    }

    let current = capture_service::get_item(pool, capture_item_id).await?;
    if current.status == "completed" {
        return Ok(current);
    }
    let item = capture_service::set_archive_pending_for_retry(pool, capture_item_id).await?;

    match archive_pending_item(pool, &item).await {
        Ok(completed) => Ok(completed),
        Err(error) => {
            archive_event(
                &item,
                LogLevel::Error,
                "archive_operation_failed",
                "failed",
                Some("archive_operation_error"),
            );
            // A filesystem or persistence failure must be visible after restart.
            // Keep the generated local files and any verified archive files so a
            // later call can resume safely.
            if let Err(record_error) =
                capture_service::record_failure(pool, capture_item_id, &error.to_string()).await
            {
                return Err(AppError::Archive(format!(
                    "{error}; additionally failed to persist the error: {record_error}"
                )));
            }
            Err(error)
        }
    }
}

async fn archive_pending_item(
    pool: &SqlitePool,
    item: &CaptureItem,
) -> Result<CaptureItem, AppError> {
    let session = capture_service::get_session(pool, &item.session_id).await?;
    let destination: Option<String> =
        sqlx::query_scalar("SELECT destination_directory FROM projects WHERE id = ?")
            .bind(&session.project_id)
            .fetch_one(pool)
            .await?;
    let destination_root = PathBuf::from(
        destination.ok_or_else(|| AppError::Validation("项目还没有配置归档目录".to_owned()))?,
    );
    tokio::fs::create_dir_all(&destination_root).await?;
    let canonical_destination = tokio::fs::canonicalize(&destination_root).await?;
    let session_dirs = capture_service::session_directories(pool, &session).await?;
    for source in &session_dirs {
        if capture_service::path_is_within(&canonical_destination, source) {
            return Err(AppError::Validation(
                "archive destination cannot be inside a capture source directory".to_owned(),
            ));
        }
    }
    let naming = archive_naming_settings_service::get(pool).await?;
    let character_name: Option<String> =
        sqlx::query_scalar("SELECT name FROM characters WHERE id = ?")
            .bind(item.character_id.as_deref())
            .fetch_optional(pool)
            .await?
            .flatten();
    // Only when explicit annotation is disabled is a missing annotated output
    // intentional; with annotation enabled a person capture without one is the
    // degraded fallback (engine not configured or failed).
    let annotation_expected = processing_settings_service::get(pool)
        .await?
        .annotate_person
        .unwrap_or(true);

    match item.classification.as_str() {
        // A person capture without an annotated output was archived through
        // the degraded fallback (no Python engine configured): keep the raw
        // screenshot under a clearly separated directory so a later full
        // reprocess can write the regular annotated output without conflict.
        "person" if item.annotated_path.is_some() => {
            archive_person_item(pool, item, &destination_root, &naming).await
        }
        // Explicit annotation disabled: the capture is fully recognized (face
        // vector + character) but has no labeled copy, so the raw screenshot
        // is archived under the character name.
        "person" if !annotation_expected => {
            archive_source_direct(
                pool,
                item,
                &destination_root,
                "人物图（原图）",
                character_name.as_deref(),
                &naming,
            )
            .await
        }
        // Degraded fallback (no Python engine configured, or engine failed
        // before producing an annotated output): keep the raw screenshot under
        // a clearly separated directory so a later reprocess can write the
        // regular annotated output without conflict.
        "person" => {
            archive_source_direct(
                pool,
                item,
                &destination_root,
                "人物图（未识别）",
                None,
                &naming,
            )
            .await
        }
        "scene" => {
            archive_source_direct(pool, item, &destination_root, "游戏截图", None, &naming).await
        }
        "private" => {
            archive_source_direct(pool, item, &destination_root, "收藏图", None, &naming).await
        }
        _ => Err(AppError::Validation(
            "unclassified capture cannot be archived".to_owned(),
        )),
    }
}

async fn archive_person_item(
    pool: &SqlitePool,
    item: &CaptureItem,
    destination_root: &Path,
    naming: &ArchiveNamingSettings,
) -> Result<CaptureItem, AppError> {
    let annotated_path = item
        .annotated_path
        .as_deref()
        .ok_or_else(|| AppError::Validation("capture has no annotated output".to_owned()))?;
    let annotated_path = validate_local_archive_source(Path::new(annotated_path)).await?;
    let avatar_path = if let Some(path) = item.avatar_path.as_deref() {
        Some(validate_local_archive_source(Path::new(path)).await?)
    } else {
        None
    };

    let character_name: Option<String> =
        sqlx::query_scalar("SELECT name FROM characters WHERE id = ?")
            .bind(item.character_id.as_deref())
            .fetch_optional(pool)
            .await?
            .flatten();
    let source_stem = Path::new(&item.source_path)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("capture");
    let annotated_extension = annotated_path
        .extension()
        .and_then(|value| value.to_str())
        .ok_or_else(|| AppError::Validation("annotated output has no extension".to_owned()))?;
    let annotated_directory = destination_root.join("人物图");
    let annotated_context = ArchiveNameContext {
        source_stem,
        character_name: character_name.as_deref(),
        capture_item_id: &item.id,
        classification: &item.classification,
        captured_at: Some(&item.captured_at),
        avatar: false,
    };
    let annotated_size = tokio::fs::metadata(&annotated_path).await?.len();
    let final_annotated = resolve_archive_destination(
        naming,
        &annotated_context,
        &annotated_directory,
        annotated_extension,
        annotated_size,
    )
    .await?;
    let annotated_size = copy_verified_atomic(&annotated_path, &final_annotated, &item.id).await?;

    let final_avatar = if let Some(avatar_path) = avatar_path {
        let avatar_extension = avatar_path
            .extension()
            .and_then(|value| value.to_str())
            .ok_or_else(|| AppError::Validation("avatar output has no extension".to_owned()))?;
        let avatar_directory = destination_root.join("头像图");
        let avatar_context = ArchiveNameContext {
            avatar: true,
            ..annotated_context
        };
        let avatar_size = tokio::fs::metadata(&avatar_path).await?.len();
        let destination = resolve_archive_destination(
            naming,
            &avatar_context,
            &avatar_directory,
            avatar_extension,
            avatar_size,
        )
        .await?;
        copy_verified_atomic(&avatar_path, &destination, &item.id).await?;
        Some(destination)
    } else {
        None
    };

    persist_completed_archive(
        pool,
        item,
        &final_annotated,
        final_avatar.as_deref(),
        annotated_size,
    )
    .await
}

async fn archive_source_direct(
    pool: &SqlitePool,
    item: &CaptureItem,
    destination_root: &Path,
    subdirectory: &str,
    character_name: Option<&str>,
    naming: &ArchiveNamingSettings,
) -> Result<CaptureItem, AppError> {
    let source = Path::new(&item.source_path);
    let source = validate_local_archive_source(source).await?;
    let source_stem = source
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("capture");
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .ok_or_else(|| AppError::Validation("source capture has no extension".to_owned()))?;
    let directory = destination_root.join(subdirectory);
    let context = ArchiveNameContext {
        source_stem,
        character_name,
        capture_item_id: &item.id,
        classification: &item.classification,
        captured_at: Some(&item.captured_at),
        avatar: false,
    };
    let expected_size = tokio::fs::metadata(&source).await?.len();
    let final_path =
        resolve_archive_destination(naming, &context, &directory, extension, expected_size).await?;
    let archived_size = copy_verified_atomic(&source, &final_path, &item.id).await?;
    persist_completed_archive(pool, item, &final_path, None, archived_size).await
}

/// Resolves the final archive destination. Without `{seq}` the name is
/// deterministic and `copy_verified_atomic` keeps the archive idempotent;
/// with `{seq}` the sequence is bumped until the destination is free (or
/// already holds an identical-length retry of the same capture).
async fn resolve_archive_destination(
    naming: &ArchiveNamingSettings,
    context: &ArchiveNameContext<'_>,
    directory: &Path,
    extension: &str,
    expected_size: u64,
) -> Result<PathBuf, AppError> {
    let uses_seq = archive_naming::uses_seq(naming);
    let mut seq: u32 = 1;
    loop {
        let base_name = if uses_seq {
            archive_naming::render_with_seq(naming, context, seq)
        } else {
            archive_naming::render(naming, context)
        };
        let candidate = directory.join(format!("{base_name}.{extension}"));
        if !uses_seq {
            return Ok(candidate);
        }
        match tokio::fs::metadata(&candidate).await {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(candidate),
            Ok(metadata) if metadata.is_file() && metadata.len() == expected_size => {
                return Ok(candidate);
            }
            Ok(_) => {}
            Err(error) => return Err(AppError::Io(error)),
        }
        seq += 1;
        if seq > 1000 {
            return Err(AppError::Archive(format!(
                "cannot find a unique archive name after 1000 attempts in {}",
                directory.display()
            )));
        }
    }
}

async fn persist_completed_archive(
    pool: &SqlitePool,
    item: &CaptureItem,
    final_path: &Path,
    final_avatar: Option<&Path>,
    archived_size: u64,
) -> Result<CaptureItem, AppError> {
    let asset_path = capture_service::path_to_string(final_path);
    let avatar_path = final_avatar.map(capture_service::path_to_string);
    let asset_name = final_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("capture")
        .to_owned();
    let file_size = i64::try_from(archived_size)
        .map_err(|_| AppError::Validation("archived file is too large to index".to_owned()))?;
    let mime_type = image_mime_type(final_path);
    let metadata_json = serde_json::to_string(&json!({
        "source": "captureSession",
        "captureItemId": item.id,
        "sourcePath": item.source_path,
        "classification": item.classification,
        "archivedAvatarPath": avatar_path,
        "processingWarnings": serde_json::from_str::<serde_json::Value>(&item.processing_warnings_json)
            .unwrap_or_else(|_| json!([])),
    }))
    .map_err(|error| AppError::Validation(format!("cannot encode asset metadata: {error}")))?;

    let mut transaction = pool.begin().await?;
    let asset_id: String = sqlx::query_scalar(
        r#"
        INSERT INTO assets (
            id, name, asset_type, path, mime_type, file_size, metadata_json
        )
        VALUES (?, ?, 'image', ?, ?, ?, ?)
        ON CONFLICT(path) DO UPDATE SET
            name = excluded.name,
            mime_type = excluded.mime_type,
            file_size = excluded.file_size,
            metadata_json = excluded.metadata_json,
            status = 'available',
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        RETURNING id
        "#,
    )
    .bind(Uuid::new_v4().to_string())
    .bind(asset_name)
    .bind(&asset_path)
    .bind(mime_type)
    .bind(file_size)
    .bind(metadata_json)
    .fetch_one(&mut *transaction)
    .await?;

    let project_id: String =
        sqlx::query_scalar("SELECT project_id FROM capture_sessions WHERE id = ?")
            .bind(&item.session_id)
            .fetch_one(&mut *transaction)
            .await?;
    sqlx::query(
        r#"
        INSERT INTO project_assets (project_id, asset_id)
        VALUES (?, ?)
        ON CONFLICT(project_id, asset_id) DO NOTHING
        "#,
    )
    .bind(&project_id)
    .bind(&asset_id)
    .execute(&mut *transaction)
    .await?;

    if let Some(character_id) = item.character_id.as_deref() {
        sqlx::query(
            r#"
            INSERT INTO asset_characters (
                asset_id, character_id, face_box_json, is_primary
            )
            VALUES (?, ?, ?, 1)
            ON CONFLICT(asset_id, character_id) DO UPDATE SET
                face_box_json = excluded.face_box_json,
                is_primary = 1
            "#,
        )
        .bind(&asset_id)
        .bind(character_id)
        .bind(item.face_box_json.as_deref())
        .execute(&mut *transaction)
        .await?;
    }

    let completed = sqlx::query_as::<_, CaptureItem>(
        r#"
        UPDATE capture_items
        SET
            asset_id = ?,
            destination_path = ?,
            destination_avatar_path = ?,
            destination_file_state = 'available',
            destination_avatar_file_state = CASE WHEN ? IS NULL THEN 'none' ELSE 'available' END,
            status = 'completed',
            error_message = NULL,
            archived_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ? AND status = 'archive_pending'
        RETURNING
            id, project_id, session_id, asset_id, character_id, classification, source_path, file_size, modified_at_ms, content_hash,
            annotated_path, avatar_path, destination_path,
            destination_avatar_path, status, face_box_json, face_count,
            suggested_character_id, recognition_confidence, recognition_source,
            review_status, error_message,
            failure_stage, attempt_count, next_retry_at, processing_warnings_json,
            captured_at, processed_at, archived_at, created_at, updated_at, source_file_state, destination_file_state, destination_avatar_file_state
        "#,
    )
    .bind(&asset_id)
    .bind(&asset_path)
    .bind(&avatar_path)
    .bind(&avatar_path)
    .bind(&item.id)
    .fetch_optional(&mut *transaction)
    .await?
    .ok_or_else(|| AppError::Conflict("capture archive state changed".to_owned()))?;

    transaction.commit().await?;

    // Reprocessing (degraded fallback or workbench relabel) replaces the
    // previous archive: once the new artifact is verified, move old target
    // files to the Windows recycle bin. There is deliberately no permanent
    // deletion fallback; unsupported targets remain on disk for safety.
    if let Some(previous) = item.destination_path.as_deref() {
        if previous != asset_path {
            let _ = super::file_recycle_service::recycle(Path::new(previous)).await;
            if let Some(previous_asset_id) = item.asset_id.as_deref() {
                let _ = sqlx::query("DELETE FROM assets WHERE id = ?")
                    .bind(previous_asset_id)
                    .execute(pool)
                    .await;
            }
        }
    }
    if let Some(previous_avatar) = item.destination_avatar_path.as_deref() {
        if final_avatar.map(capture_service::path_to_string).as_deref() != Some(previous_avatar) {
            let _ = super::file_recycle_service::recycle(Path::new(previous_avatar)).await;
        }
    }

    Ok(completed)
}

async fn copy_verified_atomic(
    source: &Path,
    destination: &Path,
    capture_item_id: &str,
) -> Result<u64, AppError> {
    let source_metadata = tokio::fs::metadata(source).await?;
    if !source_metadata.is_file() || source_metadata.len() == 0 {
        return Err(AppError::Validation(
            "archive source must be a non-empty file".to_owned(),
        ));
    }
    let expected_length = source_metadata.len();

    if let Ok(existing) = tokio::fs::metadata(destination).await {
        if existing.is_file() && existing.len() == expected_length {
            log_service::record_event(LogRecord {
                level: LogLevel::Debug,
                module: "capture.archive".to_owned(),
                message: "verified and reused existing archive file".to_owned(),
                event: Some("existing_file_reused".to_owned()),
                capture_item_id: Some(capture_item_id.to_owned()),
                outcome: Some("succeeded".to_owned()),
                ..Default::default()
            });
            return Ok(expected_length);
        }
        return Err(AppError::Archive(format!(
            "archive destination already exists with unexpected content: {}",
            destination.display()
        )));
    }

    let parent = destination.parent().ok_or_else(|| {
        AppError::Validation("archive destination has no parent directory".to_owned())
    })?;
    tokio::fs::create_dir_all(parent).await?;
    let temporary = parent.join(format!(
        ".scene-vault-{}-{}.partial",
        archive_naming::short_identifier(capture_item_id),
        Uuid::new_v4()
    ));

    let result = async {
        let copied = tokio::fs::copy(source, &temporary).await?;
        let temporary_metadata = tokio::fs::metadata(&temporary).await?;
        if copied != expected_length
            || !temporary_metadata.is_file()
            || temporary_metadata.len() != expected_length
        {
            return Err(AppError::Archive(format!(
                "archive length verification failed for {}",
                destination.display()
            )));
        }
        log_service::record_event(LogRecord {
            level: LogLevel::Debug,
            module: "capture.archive".to_owned(),
            message: "temporary archive copy passed length verification".to_owned(),
            event: Some("copy_verified".to_owned()),
            capture_item_id: Some(capture_item_id.to_owned()),
            outcome: Some("succeeded".to_owned()),
            ..Default::default()
        });

        if let Err(rename_error) = tokio::fs::rename(&temporary, destination).await {
            // A concurrent retry or a restart after the rename can leave the
            // deterministic final name in place. Length verification makes the
            // operation idempotent without overwriting it.
            if let Ok(existing) = tokio::fs::metadata(destination).await {
                if existing.is_file() && existing.len() == expected_length {
                    return Ok(expected_length);
                }
            }
            return Err(AppError::Io(rename_error));
        }
        log_service::record_event(LogRecord {
            level: LogLevel::Debug,
            module: "capture.archive".to_owned(),
            message: "archive file promoted by atomic rename".to_owned(),
            event: Some("atomic_rename_completed".to_owned()),
            capture_item_id: Some(capture_item_id.to_owned()),
            outcome: Some("succeeded".to_owned()),
            ..Default::default()
        });
        Ok(expected_length)
    }
    .await;

    if result.is_err() || tokio::fs::try_exists(&temporary).await.unwrap_or(false) {
        let _ = tokio::fs::remove_file(&temporary).await;
    }
    result
}

fn archive_event(
    item: &CaptureItem,
    level: LogLevel,
    event: &str,
    outcome: &str,
    error_code: Option<&str>,
) {
    log_service::record_event(LogRecord {
        level,
        module: "capture.archive".to_owned(),
        message: event.replace('_', " "),
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

async fn validate_local_archive_source(path: &Path) -> Result<PathBuf, AppError> {
    if !path.is_absolute() {
        return Err(AppError::Validation(
            "archive source path must be absolute".to_owned(),
        ));
    }
    if path.to_string_lossy().starts_with(r"\\") {
        return Err(AppError::Validation(
            "Python output must be local before Rust archives it".to_owned(),
        ));
    }
    let canonical = tokio::fs::canonicalize(path).await?;
    let metadata = tokio::fs::metadata(&canonical).await?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(AppError::Validation(
            "archive source must be a non-empty file".to_owned(),
        ));
    }
    Ok(canonical)
}

fn image_mime_type(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => Some("image/png"),
        Some("jpg" | "jpeg") => Some("image/jpeg"),
        Some("webp") => Some("image/webp"),
        Some("bmp") => Some("image/bmp"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db,
        models::{
            archive_naming::{ArchiveNamingSettings, UpdateArchiveNamingSettingsInput},
            capture::{
                CaptureItemIdInput, CompleteCaptureProcessingInput, LabelCaptureInput,
                RegisterCaptureInput, RelabelCaptureInput,
            },
            character::CreateCharacterInput,
        },
        services::{
            archive_naming_settings_service, capture_service, character_service, test_support,
        },
    };

    struct Fixture {
        _workspace: tempfile::TempDir,
        item: CaptureItem,
        source: PathBuf,
        annotated: PathBuf,
        avatar: PathBuf,
        destination: PathBuf,
    }

    async fn ready_to_archive(pool: &SqlitePool) -> Fixture {
        let project = test_support::project_with_directories(pool, "Archive")
            .await
            .expect("project fixture");
        let character = character_service::create(
            pool,
            CreateCharacterInput {
                project_id: project.project_id.clone(),
                name: "Ava:Star".to_owned(),
                aliases_json: None,
            },
        )
        .await
        .expect("character");
        let source_directory = project.source_directory.clone();
        let output_directory = project.output_directory.clone();
        let destination = project.destination_directory.clone();
        let source = source_directory.join("shot-001.png");
        let annotated = output_directory.join("shot-001-annotated.png");
        let avatar = output_directory.join("shot-001-avatar.png");
        tokio::fs::write(&annotated, b"annotated result")
            .await
            .expect("annotated");
        tokio::fs::write(&avatar, b"avatar result")
            .await
            .expect("avatar");

        let session = test_support::start_session(pool, &project.project_id)
            .await
            .expect("session");
        tokio::fs::write(&source, b"untouched source")
            .await
            .expect("source");
        let item = capture_service::register_capture(
            pool,
            RegisterCaptureInput {
                session_id: session.id,
                source_path: capture_service::path_to_string(&source),
            },
        )
        .await
        .expect("register");
        let item = capture_service::label_capture(
            pool,
            LabelCaptureInput {
                capture_item_id: item.id,
                character_id: Some(character.id),
                classification: Some("person".to_owned()),
            },
        )
        .await
        .expect("label");
        let item = capture_service::mark_processing(
            pool,
            CaptureItemIdInput {
                capture_item_id: item.id,
            },
        )
        .await
        .expect("processing");
        let item = capture_service::complete_processing(
            pool,
            CompleteCaptureProcessingInput {
                capture_item_id: item.id,
                annotated_path: Some(capture_service::path_to_string(&annotated)),
                avatar_path: Some(capture_service::path_to_string(&avatar)),
                face_box_json: Some(r#"{"x":1,"y":2,"width":3,"height":4}"#.to_owned()),
                face_feature: None,
                face_feature_model_id: None,
                face_feature_model_version: None,
                face_count: None,
                face_sharpness: None,
                face_area_ratio: None,
                warnings_json: None,
            },
        )
        .await
        .expect("processed");

        Fixture {
            _workspace: project._workspace,
            item,
            source,
            annotated,
            avatar,
            destination,
        }
    }

    #[tokio::test]
    async fn archives_outputs_atomically_and_indexes_the_asset() {
        let pool = db::test_pool().await;
        let fixture = ready_to_archive(&pool).await;

        let completed = archive(
            &pool,
            CaptureItemIdInput {
                capture_item_id: fixture.item.id.clone(),
            },
        )
        .await
        .expect("archive");
        assert_eq!(completed.status, "completed");
        assert!(completed.archived_at.is_some());
        let asset_id = completed.asset_id.as_deref().expect("asset id");
        let final_image =
            PathBuf::from(completed.destination_path.as_deref().expect("destination"));
        let final_avatar = PathBuf::from(
            completed
                .destination_avatar_path
                .as_deref()
                .expect("avatar destination"),
        );
        assert_eq!(
            tokio::fs::read(&final_image).await.expect("archived image"),
            b"annotated result"
        );
        assert_eq!(
            tokio::fs::read(&final_avatar)
                .await
                .expect("archived avatar"),
            b"avatar result"
        );
        assert!(final_avatar.starts_with(fixture.destination.join("头像图")));

        // Archival never removes the user's screenshot or Python's recoverable
        // local outputs.
        assert_eq!(
            tokio::fs::read(&fixture.source).await.expect("source"),
            b"untouched source"
        );
        assert!(tokio::fs::try_exists(&fixture.annotated)
            .await
            .expect("annotated exists"));
        assert!(tokio::fs::try_exists(&fixture.avatar)
            .await
            .expect("avatar exists"));

        let project_link_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM project_assets WHERE asset_id = ?")
                .bind(asset_id)
                .fetch_one(&pool)
                .await
                .expect("project link");
        let character_link_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM asset_characters WHERE asset_id = ?")
                .bind(asset_id)
                .fetch_one(&pool)
                .await
                .expect("character link");
        assert_eq!(project_link_count, 1);
        assert_eq!(character_link_count, 1);

        let repeated = archive(
            &pool,
            CaptureItemIdInput {
                capture_item_id: fixture.item.id,
            },
        )
        .await
        .expect("idempotent archive");
        assert_eq!(repeated.asset_id.as_deref(), Some(asset_id));
    }

    #[tokio::test]
    async fn records_archive_failure_and_can_retry_without_losing_outputs() {
        let pool = db::test_pool().await;
        let fixture = ready_to_archive(&pool).await;
        // The destination existed when the session started. Turning it into a
        // regular file simulates an unavailable/invalid NAS target.
        tokio::fs::remove_dir(&fixture.destination)
            .await
            .expect("remove destination directory");
        tokio::fs::write(&fixture.destination, b"not a directory")
            .await
            .expect("blocking file");

        let result = archive(
            &pool,
            CaptureItemIdInput {
                capture_item_id: fixture.item.id.clone(),
            },
        )
        .await;
        assert!(result.is_err());
        let failed = capture_service::get_item(&pool, &fixture.item.id)
            .await
            .expect("failed item");
        assert_eq!(failed.status, "failed");
        assert!(failed.error_message.is_some());
        assert!(tokio::fs::try_exists(&fixture.annotated)
            .await
            .expect("local output remains"));

        tokio::fs::remove_file(&fixture.destination)
            .await
            .expect("remove test blocker");
        let completed = archive(
            &pool,
            CaptureItemIdInput {
                capture_item_id: fixture.item.id,
            },
        )
        .await
        .expect("retry archive");
        assert_eq!(completed.status, "completed");
    }

    #[test]
    fn sanitizes_windows_names_and_reserved_components() {
        assert_eq!(
            archive_naming::sanitize_windows_component("Ava:Star?"),
            "Ava_Star_"
        );
        assert_eq!(archive_naming::sanitize_windows_component("CON"), "_CON");
        assert_eq!(
            archive_naming::sanitize_windows_component("  . "),
            "capture"
        );
    }

    #[tokio::test]
    async fn archives_with_a_custom_naming_template() {
        let pool = db::test_pool().await;
        let fixture = ready_to_archive(&pool).await;
        archive_naming_settings_service::update(
            &pool,
            UpdateArchiveNamingSettingsInput {
                settings: ArchiveNamingSettings {
                    template: "{date}_{source}_{classification}_{id}".to_owned(),
                    separator: "_".to_owned(),
                },
            },
        )
        .await
        .expect("save naming settings");

        let completed = archive(
            &pool,
            CaptureItemIdInput {
                capture_item_id: fixture.item.id.clone(),
            },
        )
        .await
        .expect("archive");
        let date: String = fixture
            .item
            .captured_at
            .chars()
            .filter(|character| character.is_ascii_digit())
            .take(8)
            .collect();
        let short_id = archive_naming::short_identifier(&fixture.item.id);
        let file_name = completed
            .destination_path
            .as_deref()
            .and_then(|path| Path::new(path).file_name())
            .and_then(|name| name.to_str())
            .expect("destination file name");
        assert_eq!(file_name, format!("{date}_shot-001_person_{short_id}.png"));
    }

    #[tokio::test]
    async fn seq_placeholder_bumps_when_the_destination_is_taken() {
        let pool = db::test_pool().await;
        let fixture = ready_to_archive(&pool).await;
        archive_naming_settings_service::update(
            &pool,
            UpdateArchiveNamingSettingsInput {
                settings: ArchiveNamingSettings {
                    template: "{source} {seq}".to_owned(),
                    separator: " - ".to_owned(),
                },
            },
        )
        .await
        .expect("save naming settings");

        // A different capture already claimed `{seq}=1` with different content.
        let taken = fixture.destination.join("人物图").join("shot-001 1.png");
        tokio::fs::create_dir_all(taken.parent().expect("parent"))
            .await
            .expect("directory");
        tokio::fs::write(&taken, b"blocker")
            .await
            .expect("blocking file");

        let completed = archive(
            &pool,
            CaptureItemIdInput {
                capture_item_id: fixture.item.id.clone(),
            },
        )
        .await
        .expect("archive");
        let file_name = completed
            .destination_path
            .as_deref()
            .and_then(|path| Path::new(path).file_name())
            .and_then(|name| name.to_str())
            .expect("destination file name");
        assert_eq!(file_name, "shot-001 2.png");
    }

    #[tokio::test]
    async fn relabel_completed_person_reprocesses_and_replaces_old_archive() {
        let pool = db::test_pool().await;
        let fixture = ready_to_archive(&pool).await;
        let completed = archive(
            &pool,
            CaptureItemIdInput {
                capture_item_id: fixture.item.id.clone(),
            },
        )
        .await
        .expect("archive");
        let old_destination =
            PathBuf::from(completed.destination_path.as_deref().expect("destination"));
        let old_avatar = PathBuf::from(
            completed
                .destination_avatar_path
                .as_deref()
                .expect("avatar destination"),
        );
        let old_asset_id = completed.asset_id.as_deref().expect("asset id").to_owned();
        assert!(old_destination.is_file());
        assert!(old_avatar.is_file());

        let project_id: String =
            sqlx::query_scalar("SELECT project_id FROM capture_sessions WHERE id = ?")
                .bind(&completed.session_id)
                .fetch_one(&pool)
                .await
                .expect("project id");
        let other = character_service::create(
            &pool,
            CreateCharacterInput {
                project_id,
                name: "Bella".to_owned(),
                aliases_json: None,
            },
        )
        .await
        .expect("other character");

        let relabeled = capture_service::relabel_capture_item(
            &pool,
            RelabelCaptureInput {
                capture_item_id: fixture.item.id.clone(),
                character_id: Some(other.id.clone()),
                classification: Some("person".to_owned()),
            },
        )
        .await
        .expect("relabel");
        assert_eq!(relabeled.status, "queued");
        assert!(relabeled.annotated_path.is_none());
        assert!(relabeled.avatar_path.is_none());
        assert!(relabeled.destination_path.is_some());
        assert!(relabeled.destination_avatar_path.is_some());
        assert!(relabeled.archived_at.is_none());

        // Full reprocessing with a configured engine writes fresh outputs.
        let processed = capture_service::mark_processing(
            &pool,
            CaptureItemIdInput {
                capture_item_id: relabeled.id.clone(),
            },
        )
        .await
        .expect("processing");
        assert_eq!(processed.status, "processing");
        let new_annotated = fixture.annotated.with_file_name("shot-001-annotated-2.png");
        let new_avatar = fixture.avatar.with_file_name("shot-001-avatar-2.png");
        tokio::fs::write(&new_annotated, b"annotated result v2")
            .await
            .expect("annotated v2");
        tokio::fs::write(&new_avatar, b"avatar result v2")
            .await
            .expect("avatar v2");
        capture_service::complete_processing(
            &pool,
            CompleteCaptureProcessingInput {
                capture_item_id: relabeled.id.clone(),
                annotated_path: Some(capture_service::path_to_string(&new_annotated)),
                avatar_path: Some(capture_service::path_to_string(&new_avatar)),
                face_box_json: Some(r#"{"x":1,"y":2,"width":3,"height":4}"#.to_owned()),
                face_feature: None,
                face_feature_model_id: None,
                face_feature_model_version: None,
                face_count: None,
                face_sharpness: None,
                face_area_ratio: None,
                warnings_json: None,
            },
        )
        .await
        .expect("complete processing");

        let rearchived = archive(
            &pool,
            CaptureItemIdInput {
                capture_item_id: relabeled.id,
            },
        )
        .await
        .expect("rearchive");
        assert_eq!(rearchived.status, "completed");
        assert_eq!(rearchived.character_id.as_deref(), Some(other.id.as_str()));

        let new_destination = PathBuf::from(
            rearchived
                .destination_path
                .as_deref()
                .expect("new destination"),
        );
        assert_ne!(new_destination, old_destination);
        assert!(new_destination.is_file());
        #[cfg(windows)]
        {
            assert!(!old_destination.exists(), "old archive must be recycled");
            assert!(!old_avatar.exists(), "old avatar archive must be recycled");
        }
        #[cfg(not(windows))]
        {
            assert!(
                old_destination.exists(),
                "unsupported recycle must fail closed"
            );
            assert!(old_avatar.exists(), "unsupported recycle must fail closed");
        }

        let old_asset_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM assets WHERE id = ?")
            .bind(&old_asset_id)
            .fetch_one(&pool)
            .await
            .expect("old asset count");
        assert_eq!(old_asset_rows, 0, "old asset row must be removed");
        let old_project_links: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM project_assets WHERE asset_id = ?")
                .bind(&old_asset_id)
                .fetch_one(&pool)
                .await
                .expect("old link count");
        assert_eq!(old_project_links, 0, "old project link must be removed");

        // The user's source screenshot stays untouched through the whole cycle.
        assert_eq!(
            tokio::fs::read(&fixture.source).await.expect("source"),
            b"untouched source"
        );
    }

    async fn direct_capture_fixture(
        pool: &SqlitePool,
        classification: &str,
    ) -> (tempfile::TempDir, CaptureItem, PathBuf, PathBuf) {
        let project = test_support::project_with_directories(pool, "Direct")
            .await
            .expect("project fixture");
        let source_directory = project.source_directory.clone();
        let destination = project.destination_directory.clone();
        let source = source_directory.join("shot-002.png");
        let session = test_support::start_session(pool, &project.project_id)
            .await
            .expect("session");
        tokio::fs::write(&source, b"raw game frame")
            .await
            .expect("source");
        let item = capture_service::register_capture(
            pool,
            RegisterCaptureInput {
                session_id: session.id,
                source_path: capture_service::path_to_string(&source),
            },
        )
        .await
        .expect("register");
        let item = capture_service::label_capture(
            pool,
            LabelCaptureInput {
                capture_item_id: item.id,
                character_id: None,
                classification: Some(classification.to_owned()),
            },
        )
        .await
        .expect("label");
        assert_eq!(item.classification, classification);
        let item = capture_service::mark_processing(
            pool,
            CaptureItemIdInput {
                capture_item_id: item.id,
            },
        )
        .await
        .expect("processing");
        let item = capture_service::mark_archive_pending_from_processing(pool, &item.id)
            .await
            .expect("archive pending");
        (project._workspace, item, source, destination)
    }

    #[tokio::test]
    async fn archives_scene_source_directly_into_game_directory() {
        let pool = db::test_pool().await;
        let (_workspace, item, source, destination) = direct_capture_fixture(&pool, "scene").await;

        let completed = archive(
            &pool,
            CaptureItemIdInput {
                capture_item_id: item.id,
            },
        )
        .await
        .expect("archive");
        assert_eq!(completed.status, "completed");
        assert!(completed.asset_id.is_some());
        let final_path = PathBuf::from(completed.destination_path.as_deref().expect("destination"));
        assert!(final_path.starts_with(destination.join("游戏截图")));
        assert_eq!(
            tokio::fs::read(&final_path).await.expect("archived source"),
            b"raw game frame"
        );
        assert!(completed.destination_avatar_path.is_none());
        assert_eq!(
            tokio::fs::read(&source).await.expect("source preserved"),
            b"raw game frame"
        );
    }

    #[tokio::test]
    async fn archives_private_source_directly_into_private_directory() {
        let pool = db::test_pool().await;
        let (_workspace, item, source, destination) =
            direct_capture_fixture(&pool, "private").await;

        let completed = archive(
            &pool,
            CaptureItemIdInput {
                capture_item_id: item.id,
            },
        )
        .await
        .expect("archive");
        assert_eq!(completed.status, "completed");
        let final_path = PathBuf::from(completed.destination_path.as_deref().expect("destination"));
        assert!(final_path.starts_with(destination.join("收藏图")));
        assert_eq!(
            tokio::fs::read(&final_path).await.expect("archived source"),
            b"raw game frame"
        );
        assert_eq!(
            tokio::fs::read(&source).await.expect("source preserved"),
            b"raw game frame"
        );
    }

    #[tokio::test]
    async fn scene_label_requires_no_character_and_queues() {
        let pool = db::test_pool().await;
        let project = test_support::project_with_directories(&pool, "SceneOnly")
            .await
            .expect("project fixture");
        let source_directory = project.source_directory.clone();
        let source = source_directory.join("shot-003.png");
        let session = test_support::start_session(&pool, &project.project_id)
            .await
            .expect("session");
        tokio::fs::write(&source, b"pending").await.expect("source");
        let item = capture_service::register_capture(
            &pool,
            RegisterCaptureInput {
                session_id: session.id,
                source_path: capture_service::path_to_string(&source),
            },
        )
        .await
        .expect("register");
        assert_eq!(item.classification, "unclassified");
        assert_eq!(item.status, "awaiting_label");

        let scene = capture_service::label_capture(
            &pool,
            LabelCaptureInput {
                capture_item_id: item.id.clone(),
                character_id: None,
                classification: Some("scene".to_owned()),
            },
        )
        .await
        .expect("label scene");
        assert_eq!(scene.classification, "scene");
        assert_eq!(scene.status, "queued");
    }

    #[tokio::test]
    async fn archives_person_without_annotated_to_unrecognized_directory() {
        let pool = db::test_pool().await;
        let project = test_support::project_with_directories(&pool, "Degraded")
            .await
            .expect("project fixture");
        let source_directory = project.source_directory.clone();
        let source = source_directory.join("shot-004.png");
        let session = test_support::start_session(&pool, &project.project_id)
            .await
            .expect("session");
        // The screenshot must appear after the session baseline snapshot so
        // discovery would treat it as a new capture.
        tokio::fs::write(&source, b"raw-capture")
            .await
            .expect("source file");
        let character = character_service::create(
            &pool,
            CreateCharacterInput {
                project_id: project.project_id.clone(),
                name: "Aurora".to_owned(),
                aliases_json: None,
            },
        )
        .await
        .expect("character");
        let item = capture_service::register_capture(
            &pool,
            RegisterCaptureInput {
                session_id: session.id.clone(),
                source_path: capture_service::path_to_string(&source),
            },
        )
        .await
        .expect("register");
        let labelled = capture_service::label_capture(
            &pool,
            LabelCaptureInput {
                capture_item_id: item.id.clone(),
                character_id: Some(character.id.clone()),
                classification: Some("person".to_owned()),
            },
        )
        .await
        .expect("label person");
        let claimed = capture_service::mark_processing(
            &pool,
            CaptureItemIdInput {
                capture_item_id: labelled.id.clone(),
            },
        )
        .await
        .expect("claim processing");
        assert_eq!(claimed.status, "processing");
        let pending = capture_service::complete_processing_without_engine(&pool, &item.id)
            .await
            .expect("degraded complete");
        assert_eq!(pending.status, "archive_pending");

        let completed = archive(
            &pool,
            CaptureItemIdInput {
                capture_item_id: pending.id.clone(),
            },
        )
        .await
        .expect("archive degraded person");
        assert_eq!(completed.status, "completed");
        assert_eq!(completed.classification, "person");
        let destination = completed.destination_path.as_deref().expect("destination");
        assert!(
            destination.contains("人物图（未识别）"),
            "unexpected destination: {destination}"
        );
        assert!(completed.annotated_path.is_none());

        let linked: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM asset_characters WHERE asset_id = ? AND character_id = ?",
        )
        .bind(&completed.asset_id)
        .bind(character.id)
        .fetch_one(&pool)
        .await
        .expect("asset character link");
        assert_eq!(linked, 1);
        assert!(source.is_file(), "source screenshot must stay untouched");
    }

    #[tokio::test]
    async fn archives_recognized_person_to_original_directory_when_annotation_disabled() {
        let pool = db::test_pool().await;
        processing_settings_service::update(
            &pool,
            crate::models::vision::ProcessingSettings {
                annotate_person: Some(false),
                ..Default::default()
            },
        )
        .await
        .expect("disable annotation");
        let project = test_support::project_with_directories(&pool, "NoAnnotation")
            .await
            .expect("project fixture");
        let source_directory = project.source_directory.clone();
        let source = source_directory.join("shot-005.png");
        let session = test_support::start_session(&pool, &project.project_id)
            .await
            .expect("session");
        tokio::fs::write(&source, b"raw-capture-5")
            .await
            .expect("source file");
        let character = character_service::create(
            &pool,
            CreateCharacterInput {
                project_id: project.project_id.clone(),
                name: "Aurora".to_owned(),
                aliases_json: None,
            },
        )
        .await
        .expect("character");
        let item = capture_service::register_capture(
            &pool,
            RegisterCaptureInput {
                session_id: session.id.clone(),
                source_path: capture_service::path_to_string(&source),
            },
        )
        .await
        .expect("register");
        let labelled = capture_service::label_capture(
            &pool,
            LabelCaptureInput {
                capture_item_id: item.id.clone(),
                character_id: Some(character.id.clone()),
                classification: Some("person".to_owned()),
            },
        )
        .await
        .expect("label person");
        let claimed = capture_service::mark_processing(
            &pool,
            CaptureItemIdInput {
                capture_item_id: labelled.id.clone(),
            },
        )
        .await
        .expect("claim processing");
        assert_eq!(claimed.status, "processing");
        // Simulates a successful processing pass with annotation skipped:
        // feature/avatar data may exist, but no annotated output is written.
        let pending = capture_service::complete_processing(
            &pool,
            CompleteCaptureProcessingInput {
                capture_item_id: item.id.clone(),
                annotated_path: None,
                avatar_path: None,
                face_box_json: None,
                face_feature: None,
                face_feature_model_id: None,
                face_feature_model_version: None,
                face_count: None,
                face_sharpness: None,
                face_area_ratio: None,
                warnings_json: None,
            },
        )
        .await
        .expect("complete without annotation");
        assert_eq!(pending.status, "archive_pending");

        let completed = archive(
            &pool,
            CaptureItemIdInput {
                capture_item_id: pending.id.clone(),
            },
        )
        .await
        .expect("archive person without annotation");
        assert_eq!(completed.status, "completed");
        let destination = completed.destination_path.as_deref().expect("destination");
        assert!(
            destination.contains("人物图（原图）"),
            "unexpected destination: {destination}"
        );
        assert!(
            destination.contains("Aurora"),
            "name missing: {destination}"
        );
        assert!(completed.annotated_path.is_none());
        assert!(source.is_file(), "source screenshot must stay untouched");
    }
}
