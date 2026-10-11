//! Per-capture image facts: resolution, format, byte size and the archive
//! quality actually used. Facts describe files Scene Vault has seen; they are
//! refreshed when a file is (re)bound and cleared when a replacement is
//! detected, so the UI never shows stale information for a changed file.

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
    let row: Option<(
        Option<String>,
        Option<String>,
        Option<String>,
        String,
        String,
    )> = sqlx::query_as(
        "SELECT facts.source_json, facts.destination_json, item.destination_path, \
         item.destination_file_state, item.source_file_state \
         FROM capture_items item \
         LEFT JOIN capture_image_facts facts ON facts.capture_item_id = item.id \
         WHERE item.id = ?",
    )
    .bind(capture_item_id)
    .fetch_optional(pool)
    .await?;
    let Some((source_json, destination_json, destination_path, destination_state, source_state)) =
        row
    else {
        return Ok(CaptureImageFacts::default());
    };
    let mut source = parse_source_json(source_json.as_deref());
    if source_state == "missing" || source_state == "replaced" {
        source = None;
    }
    let mut destination = parse_destination_json(destination_json.as_deref());
    if destination.is_some() {
        // Facts describe the bytes this app wrote: if the archived file is
        // gone, replaced by a different size, or its state says so, show
        // nothing instead of stale numbers.
        let unchanged = match (
            destination_path.as_deref(),
            destination.as_ref().and_then(|facts| facts.bytes),
        ) {
            (Some(path), Some(bytes)) => tokio::fs::metadata(path)
                .await
                .map(|metadata| metadata.is_file() && metadata.len() == bytes)
                .unwrap_or(false),
            _ => false,
        };
        if destination_state != "available" || !unchanged {
            destination = None;
        }
    }
    Ok(CaptureImageFacts {
        source,
        destination,
    })
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
        "SELECT capture_item_id, source_json, destination_json FROM capture_image_facts \
         WHERE capture_item_id IN (",
    );
    let mut separated = builder.separated(", ");
    for id in capture_item_ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(")");
    let rows: Vec<(String, Option<String>, Option<String>)> =
        builder.build_query_as().fetch_all(pool).await?;
    for (id, source, destination) in rows {
        result.insert(
            id,
            CaptureImageFacts {
                source: parse_source_json(source.as_deref()),
                destination: parse_destination_json(destination.as_deref()),
            },
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
