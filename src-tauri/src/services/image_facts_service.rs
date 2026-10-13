//! Per-capture image facts: resolution, format, byte size and the archive
//! quality actually used. Facts describe files Scene Vault has seen; they are
//! refreshed when a file is (re)bound and cleared when a replacement is
//! detected. Readers hide invalidated facts; explicit identity checks detect
//! same-size replacements that the normal stat-based read cannot distinguish.

use std::{collections::HashMap, path::Path};

use chrono::{SecondsFormat, Utc};
use sqlx::SqlitePool;

use crate::{
    error::AppError,
    models::image_facts::{ArchiveImageFacts, CaptureImageFacts, SourceImageFacts},
};

/// Result of a header-only read of one image file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageProbe {
    pub width: u32,
    pub height: u32,
    /// Canonical lowercase format name (`png`, `jpeg`, `webp`, ...).
    pub format: String,
}

#[derive(sqlx::FromRow)]
struct StoredImageFacts {
    capture_item_id: String,
    source_json: Option<String>,
    destination_json: Option<String>,
    destination_path: Option<String>,
    destination_file_state: String,
    source_file_state: String,
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub fn canonical_format(format: image::ImageFormat) -> String {
    match format {
        image::ImageFormat::Png => "png".to_owned(),
        image::ImageFormat::Jpeg => "jpeg".to_owned(),
        image::ImageFormat::WebP => "webp".to_owned(),
        other => format!("{other:?}").to_lowercase(),
    }
}

/// Reads just the header of an image. Returns None when the format is not
/// supported by this build or the file cannot be read; callers treat that as
/// "unknown" instead of guessing from the extension.
pub async fn probe_file(path: &Path) -> Option<ImageProbe> {
    let owned = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let reader = image::ImageReader::open(&owned)
            .ok()?
            .with_guessed_format()
            .ok()?;
        let format = reader.format()?;
        let (width, height) = reader.into_dimensions().ok()?;
        if width == 0 || height == 0 {
            return None;
        }
        Some(ImageProbe {
            width,
            height,
            format: canonical_format(format),
        })
    })
    .await
    .ok()
    .flatten()
}

pub fn parse_source_json(raw: Option<&str>) -> Option<SourceImageFacts> {
    serde_json::from_str(raw?).ok()
}

pub fn parse_destination_json(raw: Option<&str>) -> Option<ArchiveImageFacts> {
    serde_json::from_str(raw?).ok()
}

/// Both variants of one capture, for the detail views.
pub async fn read(pool: &SqlitePool, capture_item_id: &str) -> Result<CaptureImageFacts, AppError> {
    let row: Option<StoredImageFacts> = sqlx::query_as(
        "SELECT item.id AS capture_item_id, facts.source_json, facts.destination_json, item.destination_path, \
         item.destination_file_state, item.source_file_state \
         FROM capture_items item \
         LEFT JOIN capture_image_facts facts ON facts.capture_item_id = item.id \
         WHERE item.id = ?",
    )
    .bind(capture_item_id)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Ok(CaptureImageFacts::default());
    };
    Ok(valid_facts(
        row.source_json.as_deref(),
        row.destination_json.as_deref(),
        row.destination_path.as_deref(),
        &row.destination_file_state,
        &row.source_file_state,
    )
    .await)
}

async fn valid_facts(
    source_json: Option<&str>,
    destination_json: Option<&str>,
    destination_path: Option<&str>,
    destination_state: &str,
    source_state: &str,
) -> CaptureImageFacts {
    let mut source = parse_source_json(source_json);
    if source_state == "missing" || source_state == "replaced" {
        source = None;
    }
    let mut destination = if destination_state == "available" {
        parse_destination_json(destination_json)
    } else {
        None
    };
    if destination.is_some() {
        // Facts describe the bytes this app wrote: if the archived file is
        // gone, replaced by a different size, or its state says so, show
        // nothing instead of stale numbers.
        let unchanged = match (
            destination_path,
            destination.as_ref().and_then(|facts| facts.bytes),
        ) {
            (Some(path), Some(bytes)) => tokio::fs::metadata(path)
                .await
                .map(|metadata| metadata.is_file() && metadata.len() == bytes)
                .unwrap_or(false),
            _ => false,
        };
        if !unchanged {
            destination = None;
        }
    }
    CaptureImageFacts {
        source,
        destination,
    }
}

/// Bulk read for a page of history rows; missing rows are simply absent.
pub async fn read_many(
    pool: &SqlitePool,
    capture_item_ids: &[String],
) -> Result<HashMap<String, CaptureImageFacts>, AppError> {
    let mut result = HashMap::new();
    if capture_item_ids.is_empty() {
        return Ok(result);
    }
    let mut builder = sqlx::QueryBuilder::new(
        "SELECT facts.capture_item_id, facts.source_json, facts.destination_json, \
         item.destination_path, item.destination_file_state, item.source_file_state \
         FROM capture_image_facts facts \
         JOIN capture_items item ON item.id = facts.capture_item_id \
         WHERE facts.capture_item_id IN (",
    );
    let mut separated = builder.separated(", ");
    for id in capture_item_ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(")");
    let rows: Vec<StoredImageFacts> = builder.build_query_as().fetch_all(pool).await?;
    // Only async file stats remain per row; database reads stay batched and
    // at most one archive target is checked at a time.
    for row in rows {
        result.insert(
            row.capture_item_id,
            valid_facts(
                row.source_json.as_deref(),
                row.destination_json.as_deref(),
                row.destination_path.as_deref(),
                &row.destination_file_state,
                &row.source_file_state,
            )
            .await,
        );
    }
    Ok(result)
}

fn encode<T: serde::Serialize>(facts: Option<&T>) -> Result<Option<String>, AppError> {
    facts
        .map(|facts| {
            serde_json::to_string(facts).map_err(|error| {
                AppError::Validation(format!("cannot encode image facts: {error}"))
            })
        })
        .transpose()
}

/// Probes a file and stores the result as the source facts, or clears them
/// when the file cannot be read (a stale record must not survive).
pub async fn refresh_source_facts(
    pool: &SqlitePool,
    capture_item_id: &str,
    source: &Path,
) -> Result<(), AppError> {
    let facts = probe_file(source).await.map(|probe| SourceImageFacts {
        width: Some(probe.width),
        height: Some(probe.height),
        format: Some(probe.format),
    });
    upsert_source(pool, capture_item_id, facts.as_ref()).await
}

pub async fn upsert_source(
    pool: &SqlitePool,
    capture_item_id: &str,
    facts: Option<&SourceImageFacts>,
) -> Result<(), AppError> {
    let json = encode(facts)?;
    sqlx::query(
        r#"
        INSERT INTO capture_image_facts (capture_item_id, source_json, destination_json, updated_at)
        VALUES (?, ?, NULL, ?)
        ON CONFLICT(capture_item_id) DO UPDATE SET
            source_json = excluded.source_json,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(capture_item_id)
    .bind(json)
    .bind(now())
    .execute(pool)
    .await?;
    Ok(())
}

/// Transactional variant used while completing an archive write.
pub async fn upsert_destination_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    capture_item_id: &str,
    facts: &ArchiveImageFacts,
) -> Result<(), AppError> {
    let json = encode(Some(facts))?;
    sqlx::query(
        r#"
        INSERT INTO capture_image_facts (capture_item_id, source_json, destination_json, updated_at)
        VALUES (?, NULL, ?, ?)
        ON CONFLICT(capture_item_id) DO UPDATE SET
            destination_json = excluded.destination_json,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(capture_item_id)
    .bind(json)
    .bind(now())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Drops the source record when the file is known to be missing/replaced.
pub async fn clear_source(pool: &SqlitePool, capture_item_id: &str) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE capture_image_facts SET source_json = NULL, updated_at = ? \
         WHERE capture_item_id = ?",
    )
    .bind(now())
    .bind(capture_item_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Drops the archive record when the archived file was replaced or removed.
pub async fn clear_destination(pool: &SqlitePool, capture_item_id: &str) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE capture_image_facts SET destination_json = NULL, updated_at = ? \
         WHERE capture_item_id = ?",
    )
    .bind(now())
    .bind(capture_item_id)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    #[test]
    fn canonical_format_names_match_the_ui_contract() {
        assert_eq!(canonical_format(image::ImageFormat::Png), "png");
        assert_eq!(canonical_format(image::ImageFormat::Jpeg), "jpeg");
        assert_eq!(canonical_format(image::ImageFormat::WebP), "webp");
    }

    #[test]
    fn invalid_json_is_reported_as_unknown() {
        assert!(parse_source_json(Some("not json")).is_none());
        assert!(parse_destination_json(Some("{")).is_none());
        assert!(parse_source_json(None).is_none());
    }

    #[tokio::test]
    async fn bulk_and_single_reads_filter_the_same_file_states_and_changed_targets() {
        let pool = db::test_pool().await;
        let project = crate::services::test_support::project_with_directories(&pool, "Validity")
            .await
            .unwrap();
        let session = crate::services::test_support::start_session(&pool, &project.project_id)
            .await
            .unwrap();
        let source = SourceImageFacts {
            width: Some(1920),
            height: Some(1080),
            format: Some("png".to_owned()),
        };
        let destination = ArchiveImageFacts {
            width: Some(1280),
            height: Some(720),
            format: Some("jpeg".to_owned()),
            bytes: Some(5),
            quality_kind: Some("quality".to_owned()),
            quality: Some(90),
            size_policy: Some("native".to_owned()),
            size_policy_applied: Some(true),
            format_applied: Some(true),
        };
        let cases = [
            ("available", "available", Some(&b"bytes"[..]), true, true),
            ("missing", "available", Some(&b"bytes"[..]), false, true),
            ("replaced", "available", Some(&b"bytes"[..]), false, true),
            ("unknown", "available", Some(&b"bytes"[..]), true, true),
            ("available", "missing", Some(&b"bytes"[..]), true, false),
            ("available", "unavailable", Some(&b"bytes"[..]), true, false),
            ("available", "unknown", Some(&b"bytes"[..]), true, false),
            ("available", "available", None, true, false),
            ("available", "available", Some(&b"longer"[..]), true, false),
        ];
        let mut expected = Vec::new();
        for (source_state, destination_state, bytes, has_source, has_destination) in cases {
            let id = uuid::Uuid::new_v4().to_string();
            let path = project.destination_directory.join(format!("{id}.jpg"));
            let source_path = project.source_directory.join(format!("{id}.png"));
            if let Some(bytes) = bytes {
                tokio::fs::write(&path, bytes).await.unwrap();
            }
            sqlx::query(
                "INSERT INTO capture_items (id, project_id, session_id, source_path, \
                 source_file_state, destination_path, destination_file_state) VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(&project.project_id)
            .bind(&session.id)
            .bind(source_path.to_string_lossy().to_string())
            .bind(source_state)
            .bind(path.to_string_lossy().to_string())
            .bind(destination_state)
            .execute(&pool)
            .await
            .unwrap();
            upsert_source(&pool, &id, Some(&source)).await.unwrap();
            let mut tx = pool.begin().await.unwrap();
            upsert_destination_tx(&mut tx, &id, &destination)
                .await
                .unwrap();
            tx.commit().await.unwrap();
            expected.push((id, has_source, has_destination));
        }
        let mut ids: Vec<String> = expected.iter().map(|(id, _, _)| id.clone()).collect();
        let absent_id = uuid::Uuid::new_v4().to_string();
        ids.push(absent_id.clone());
        let page = read_many(&pool, &ids).await.unwrap();
        assert!(!page.contains_key(&absent_id));
        for (id, has_source, has_destination) in &expected {
            let single = read(&pool, id).await.unwrap();
            let bulk = &page[id];
            assert_eq!(bulk.source, single.source);
            assert_eq!(bulk.destination, single.destination);
            assert_eq!(bulk.source.is_some(), *has_source, "{id}");
            assert_eq!(bulk.destination.is_some(), *has_destination, "{id}");
        }
        assert!(read_many(&pool, &[]).await.unwrap().is_empty());

        // A later deletion must invalidate facts without waiting for a file
        // reconcile; a directory at that path must also stay invalid.
        let valid_id = &expected[0].0;
        let target = project
            .destination_directory
            .join(format!("{valid_id}.jpg"));
        tokio::fs::remove_file(&target).await.unwrap();
        let page = read_many(&pool, std::slice::from_ref(valid_id))
            .await
            .unwrap();
        assert!(page[valid_id].destination.is_none());
        assert!(read(&pool, valid_id).await.unwrap().destination.is_none());
        tokio::fs::create_dir(&target).await.unwrap();
        let page = read_many(&pool, std::slice::from_ref(valid_id))
            .await
            .unwrap();
        assert!(page[valid_id].destination.is_none());
    }

    #[tokio::test]
    async fn round_trips_facts_and_clears_them_independently() {
        let pool = db::test_pool().await;
        let project = crate::services::test_support::project_with_directories(&pool, "Facts")
            .await
            .unwrap();
        let session = crate::services::test_support::start_session(&pool, &project.project_id)
            .await
            .unwrap();
        let capture_item_id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO capture_items (id, project_id, session_id, source_path) VALUES (?, ?, ?, ?)",
        )
        .bind(&capture_item_id)
        .bind(&project.project_id)
        .bind(&session.id)
        .bind("C:/tmp/shot.png")
        .execute(&pool)
        .await
        .unwrap();
        let source = SourceImageFacts {
            width: Some(1920),
            height: Some(1080),
            format: Some("png".to_owned()),
        };
        upsert_source(&pool, &capture_item_id, Some(&source))
            .await
            .unwrap();
        let destination = ArchiveImageFacts {
            width: Some(1280),
            height: Some(720),
            format: Some("jpeg".to_owned()),
            bytes: Some(5),
            quality_kind: Some("quality".to_owned()),
            quality: Some(90),
            size_policy: Some("native".to_owned()),
            size_policy_applied: Some(true),
            format_applied: Some(true),
        };
        let destination_file = project.destination_directory.join("shot.png");
        tokio::fs::write(&destination_file, b"bytes").await.unwrap();
        sqlx::query(
            "UPDATE capture_items SET destination_path = ?, destination_file_state = 'available' WHERE id = ?",
        )
        .bind(destination_file.to_string_lossy().to_string())
        .bind(&capture_item_id)
        .execute(&pool)
        .await
        .unwrap();
        let mut tx = pool.begin().await.unwrap();
        upsert_destination_tx(&mut tx, &capture_item_id, &destination)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let facts = read(&pool, &capture_item_id).await.unwrap();
        assert_eq!(facts.source, Some(source));
        assert_eq!(facts.destination, Some(destination));

        clear_source(&pool, &capture_item_id).await.unwrap();
        let facts = read(&pool, &capture_item_id).await.unwrap();
        assert!(facts.source.is_none());
        assert!(facts.destination.is_some());

        clear_destination(&pool, &capture_item_id).await.unwrap();
        let facts = read(&pool, &capture_item_id).await.unwrap();
        assert!(facts.destination.is_none());
    }
}
