use std::path::{Path, PathBuf};

use sqlx::{FromRow, QueryBuilder, Sqlite, SqliteConnection, SqlitePool};

use crate::{
    error::AppError,
    models::capture::{CaptureDeletionPreview, CaptureDeletionResult, FileRecycleFailure},
    services::{
        capture_service,
        file_recycle_service::{is_unc_path, FileRecycler},
        thumbnail_service,
    },
};

#[derive(Debug, Clone, FromRow)]
struct DeletionRow {
    id: String,
    project_id: String,
    source_path: String,
    content_hash: Option<String>,
    annotated_path: Option<String>,
    avatar_path: Option<String>,
    destination_path: Option<String>,
    destination_avatar_path: Option<String>,
    destination_file_state: String,
    destination_avatar_file_state: String,
    asset_id: Option<String>,
    status: String,
}

struct DestinationCandidate {
    path: PathBuf,
    previously_missing: bool,
}

pub async fn preview_capture(
    pool: &SqlitePool,
    capture_item_id: &str,
) -> Result<CaptureDeletionPreview, AppError> {
    let rows = load_rows(pool, Some(capture_item_id), None).await?;
    preview(&rows)
}

pub async fn preview_character(
    pool: &SqlitePool,
    character_id: &str,
) -> Result<CaptureDeletionPreview, AppError> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM characters WHERE id = ?)")
        .bind(character_id.trim())
        .fetch_one(pool)
        .await?;
    if !exists {
        return Err(AppError::NotFound("character".to_owned()));
    }
    let rows = load_rows(pool, None, Some(character_id)).await?;
    preview(&rows)
}

fn preview(rows: &[DeletionRow]) -> Result<CaptureDeletionPreview, AppError> {
    // A preview must remain available even when an older row contains a
    // malformed or no-longer-supported target path. Execution performs the
    // strict safety validation before touching either files or database rows.
    let destination_file_count = rows
        .iter()
        .flat_map(|row| {
            [
                row.destination_path.as_ref(),
                row.destination_avatar_path.as_ref(),
            ]
        })
        .flatten()
        .collect::<std::collections::BTreeSet<_>>()
        .len() as u32;
    let local_derived_file_count = rows
        .iter()
        .flat_map(|row| [row.annotated_path.as_ref(), row.avatar_path.as_ref()])
        .flatten()
        .collect::<std::collections::BTreeSet<_>>()
        .len() as u32;
    let network_destination_file_count = rows
        .iter()
        .flat_map(|row| {
            [
                row.destination_path.as_ref(),
                row.destination_avatar_path.as_ref(),
            ]
        })
        .flatten()
        .filter(|path| is_unc_path(Path::new(path)))
        .collect::<std::collections::BTreeSet<_>>()
        .len() as u32;
    Ok(CaptureDeletionPreview {
        capture_count: rows.len() as u32,
        destination_file_count,
        network_destination_file_count,
        local_derived_file_count,
        source_files_preserved: rows.len() as u32,
    })
}

pub async fn delete_capture<R: FileRecycler>(
    pool: &SqlitePool,
    app_cache_root: &Path,
    app_local_data_root: &Path,
    capture_item_id: &str,
    delete_destination_files: bool,
    allow_permanent_network_delete: bool,
    recycler: &R,
) -> Result<CaptureDeletionResult, AppError> {
    let rows = load_rows(pool, Some(capture_item_id), None).await?;
    execute(
        pool,
        app_cache_root,
        app_local_data_root,
        rows,
        None,
        delete_destination_files,
        allow_permanent_network_delete,
        recycler,
    )
    .await
}

pub async fn delete_character<R: FileRecycler>(
    pool: &SqlitePool,
    app_cache_root: &Path,
    app_local_data_root: &Path,
    character_id: &str,
    delete_destination_files: bool,
    allow_permanent_network_delete: bool,
    recycler: &R,
) -> Result<CaptureDeletionResult, AppError> {
    let character_id = character_id.trim();
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM characters WHERE id = ?)")
        .bind(character_id)
        .fetch_one(pool)
        .await?;
    if !exists {
        return Err(AppError::NotFound("character".to_owned()));
    }
    let rows = load_rows(pool, None, Some(character_id)).await?;
    execute(
        pool,
        app_cache_root,
        app_local_data_root,
        rows,
        Some(character_id),
        delete_destination_files,
        allow_permanent_network_delete,
        recycler,
    )
    .await
}

async fn execute<R: FileRecycler>(
    pool: &SqlitePool,
    app_cache_root: &Path,
    app_local_data_root: &Path,
    mut rows: Vec<DeletionRow>,
    character_id: Option<&str>,
    delete_destination_files: bool,
    allow_permanent_network_delete: bool,
    recycler: &R,
) -> Result<CaptureDeletionResult, AppError> {
    if rows.iter().any(|row| {
        matches!(
            row.status.as_str(),
            "queued" | "processing" | "archive_pending"
        )
    }) {
        return Err(AppError::Conflict(
            "captures being processed cannot be deleted".to_owned(),
        ));
    }

    let capture_item_id = character_id.is_none().then(|| rows[0].id.clone());
    let mut transaction = pool.begin().await?;
    if let Some(character_id) = character_id {
        let locked = sqlx::query("UPDATE characters SET updated_at = updated_at WHERE id = ?")
            .bind(character_id)
            .execute(&mut *transaction)
            .await?;
        if locked.rows_affected() == 0 {
            return Err(AppError::NotFound("character".to_owned()));
        }
    } else if let Some(capture_item_id) = capture_item_id.as_deref() {
        let locked = sqlx::query("UPDATE capture_items SET updated_at = updated_at WHERE id = ?")
            .bind(capture_item_id)
            .execute(&mut *transaction)
            .await?;
        if locked.rows_affected() == 0 {
            return Err(AppError::NotFound("capture item".to_owned()));
        }
    }
    rows = load_rows_connection(&mut transaction, capture_item_id.as_deref(), character_id).await?;
    if rows.iter().any(|row| {
        matches!(
            row.status.as_str(),
            "queued" | "processing" | "archive_pending"
        )
    }) {
        return Err(AppError::Conflict(
            "captures being processed cannot be deleted".to_owned(),
        ));
    }
    let protected_sources = protected_source_paths(&rows).await;

    let mut result = CaptureDeletionResult {
        completed: false,
        records_deleted: 0,
        deleted_capture_item_ids: Vec::new(),
        destination_files_recycled: 0,
        destination_files_permanently_deleted: 0,
        destination_files_already_missing: 0,
        failures: Vec::new(),
    };
    if delete_destination_files {
        for candidate in destination_paths(&rows)? {
            let path = candidate.path;
            match tokio::fs::metadata(&path).await {
                Ok(metadata) if metadata.is_file() => {
                    if candidate.previously_missing {
                        result.failures.push(FileRecycleFailure {
                            path: capture_service::path_to_string(&path),
                            error: "target path was previously missing and now contains a new file; refusing to recycle it".to_owned(),
                        });
                        continue;
                    }
                    let resolved = tokio::fs::canonicalize(&path)
                        .await
                        .unwrap_or_else(|_| path.clone());
                    if protected_sources.contains(&deletion_path_key(&resolved)) {
                        result.failures.push(FileRecycleFailure {
                            path: capture_service::path_to_string(&path),
                            error: "target path resolves to the protected source image".to_owned(),
                        });
                        continue;
                    }
                    let network_target = is_unc_path(&path);
                    if network_target && !allow_permanent_network_delete {
                        set_destination_state(&mut transaction, &rows, &path, "available").await?;
                        result.failures.push(FileRecycleFailure {
                            path: capture_service::path_to_string(&path),
                            error: "network target requires explicit permanent-delete confirmation"
                                .to_owned(),
                        });
                        continue;
                    }
                    let deletion = if network_target {
                        recycler.delete_permanently(&path).await
                    } else {
                        recycler.recycle(&path).await
                    };
                    match deletion {
                        Ok(()) => {
                            if network_target {
                                result.destination_files_permanently_deleted += 1;
                            } else {
                                result.destination_files_recycled += 1;
                            }
                            set_destination_state(&mut transaction, &rows, &path, "missing")
                                .await?;
                        }
                        Err(error) => {
                            set_destination_state(&mut transaction, &rows, &path, "unavailable")
                                .await?;
                            result.failures.push(FileRecycleFailure {
                                path: capture_service::path_to_string(&path),
                                error: error.to_string(),
                            });
                        }
                    }
                }
                Ok(_) => {
                    set_destination_state(&mut transaction, &rows, &path, "unavailable").await?;
                    result.failures.push(FileRecycleFailure {
                        path: capture_service::path_to_string(&path),
                        error: "target path is not a file".to_owned(),
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    result.destination_files_already_missing += 1;
                    set_destination_state(&mut transaction, &rows, &path, "missing").await?;
                }
                Err(error) => {
                    set_destination_state(&mut transaction, &rows, &path, "unavailable").await?;
                    result.failures.push(FileRecycleFailure {
                        path: capture_service::path_to_string(&path),
                        error: error.to_string(),
                    });
                }
            }
        }
    }
    if !result.failures.is_empty() {
        transaction.commit().await?;
        return Ok(result);
    }

    let ids: Vec<String> = rows.iter().map(|row| row.id.clone()).collect();
    let asset_links: std::collections::BTreeSet<(String, String)> = rows
        .iter()
        .filter_map(|row| {
            row.asset_id
                .as_ref()
                .map(|asset_id| (row.project_id.clone(), asset_id.clone()))
        })
        .collect();
    for row in &rows {
        if let Some(content_hash) = row.content_hash.as_deref() {
            sqlx::query(
                r#"
                INSERT INTO ignored_capture_contents (project_id, content_hash, source_path)
                VALUES (?, ?, ?)
                ON CONFLICT(project_id, content_hash) DO UPDATE SET
                    source_path = excluded.source_path,
                    ignored_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                "#,
            )
            .bind(&row.project_id)
            .bind(content_hash)
            .bind(&row.source_path)
            .execute(&mut *transaction)
            .await?;
        }
    }
    if !ids.is_empty() {
        let mut clear_cover = QueryBuilder::<Sqlite>::new(
            "UPDATE projects SET cover_capture_item_id = NULL WHERE cover_capture_item_id IN (",
        );
        push_values(&mut clear_cover, &ids);
        clear_cover
            .push(")")
            .build()
            .execute(&mut *transaction)
            .await?;

        let mut delete_items =
            QueryBuilder::<Sqlite>::new("DELETE FROM capture_items WHERE id IN (");
        push_values(&mut delete_items, &ids);
        delete_items
            .push(")")
            .build()
            .execute(&mut *transaction)
            .await?;
    }
    if let Some(character_id) = character_id {
        sqlx::query("DELETE FROM characters WHERE id = ?")
            .bind(character_id)
            .execute(&mut *transaction)
            .await?;
    }
    for (project_id, asset_id) in &asset_links {
        sqlx::query(
            r#"
            DELETE FROM project_assets
            WHERE project_id = ? AND asset_id = ?
              AND NOT EXISTS (
                  SELECT 1 FROM capture_items
                  WHERE project_id = ? AND asset_id = ?
              )
            "#,
        )
        .bind(project_id)
        .bind(asset_id)
        .bind(project_id)
        .bind(asset_id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            r#"
            DELETE FROM assets
            WHERE id = ?
              AND NOT EXISTS (SELECT 1 FROM capture_items WHERE asset_id = ?)
              AND NOT EXISTS (SELECT 1 FROM project_assets WHERE asset_id = ?)
              AND NOT EXISTS (SELECT 1 FROM collection_assets WHERE asset_id = ?)
              AND NOT EXISTS (SELECT 1 FROM projects WHERE cover_asset_id = ?)
              AND NOT EXISTS (SELECT 1 FROM characters WHERE avatar_asset_id = ?)
            "#,
        )
        .bind(asset_id)
        .bind(asset_id)
        .bind(asset_id)
        .bind(asset_id)
        .bind(asset_id)
        .bind(asset_id)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;

    for row in &rows {
        for path in [row.annotated_path.as_deref(), row.avatar_path.as_deref()]
            .into_iter()
            .flatten()
        {
            let path = Path::new(path);
            if capture_service::path_is_within(path, app_cache_root)
                || capture_service::path_is_within(path, app_local_data_root)
            {
                let resolved = tokio::fs::canonicalize(path)
                    .await
                    .unwrap_or_else(|_| path.to_path_buf());
                if !protected_sources.contains(&deletion_path_key(&resolved)) {
                    let _ = tokio::fs::remove_file(path).await;
                }
            }
        }
        for variant in ["source", "annotated", "avatar", "destination"] {
            let thumbnail = thumbnail_service::cache_path(app_local_data_root, &row.id, variant);
            let resolved = tokio::fs::canonicalize(&thumbnail)
                .await
                .unwrap_or_else(|_| thumbnail.clone());
            if !protected_sources.contains(&deletion_path_key(&resolved)) {
                let _ = tokio::fs::remove_file(thumbnail).await;
            }
        }
    }
    result.completed = true;
    result.records_deleted = rows.len() as u32;
    result.deleted_capture_item_ids = ids;
    Ok(result)
}

fn destination_paths(rows: &[DeletionRow]) -> Result<Vec<DestinationCandidate>, AppError> {
    let mut paths = std::collections::BTreeMap::<String, DestinationCandidate>::new();
    for row in rows {
        for (value, state) in [
            (
                row.destination_path.as_deref(),
                row.destination_file_state.as_str(),
            ),
            (
                row.destination_avatar_path.as_deref(),
                row.destination_avatar_file_state.as_str(),
            ),
        ]
        .into_iter()
        {
            let Some(value) = value else { continue };
            let path = PathBuf::from(value);
            if path == PathBuf::from(&row.source_path) {
                return Err(AppError::Conflict(
                    "a target path resolves to the protected source image".to_owned(),
                ));
            }
            if !path.is_absolute() || !capture_service::is_supported_image(&path) {
                return Err(AppError::Conflict(
                    "a target path is not an absolute supported image".to_owned(),
                ));
            }
            let key = deletion_path_key(&path);
            paths
                .entry(key)
                .and_modify(|candidate| candidate.previously_missing |= state == "missing")
                .or_insert(DestinationCandidate {
                    path,
                    previously_missing: state == "missing",
                });
        }
    }
    Ok(paths.into_values().collect())
}

async fn set_destination_state(
    connection: &mut SqliteConnection,
    rows: &[DeletionRow],
    path: &Path,
    state: &str,
) -> Result<(), AppError> {
    let path = capture_service::path_to_string(path);
    for row in rows {
        if row.destination_path.as_deref() != Some(path.as_str())
            && row.destination_avatar_path.as_deref() != Some(path.as_str())
        {
            continue;
        }
        sqlx::query(
            r#"
            UPDATE capture_items
            SET destination_file_state = CASE WHEN destination_path = ? THEN ? ELSE destination_file_state END,
                destination_avatar_file_state = CASE WHEN destination_avatar_path = ? THEN ? ELSE destination_avatar_file_state END,
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
            WHERE id = ?
            "#,
        )
        .bind(&path)
        .bind(state)
        .bind(&path)
        .bind(state)
        .bind(&row.id)
        .execute(&mut *connection)
        .await?;
    }
    Ok(())
}

async fn protected_source_paths(rows: &[DeletionRow]) -> std::collections::BTreeSet<String> {
    let mut protected = std::collections::BTreeSet::new();
    for row in rows {
        let source = Path::new(&row.source_path);
        protected.insert(deletion_path_key(source));
        if let Ok(canonical) = tokio::fs::canonicalize(source).await {
            protected.insert(deletion_path_key(&canonical));
        }
    }
    protected
}

fn deletion_path_key(path: &Path) -> String {
    let normalized = capture_service::normalized_path_string(path);
    #[cfg(windows)]
    {
        normalized.replace('/', "\\").to_lowercase()
    }
    #[cfg(not(windows))]
    {
        normalized
    }
}

fn push_values<'a>(builder: &mut QueryBuilder<'a, Sqlite>, values: &'a [String]) {
    let mut separated = builder.separated(", ");
    for value in values {
        separated.push_bind(value);
    }
}

async fn load_rows(
    pool: &SqlitePool,
    capture_item_id: Option<&str>,
    character_id: Option<&str>,
) -> Result<Vec<DeletionRow>, AppError> {
    let rows = if let Some(capture_item_id) = capture_item_id {
        sqlx::query_as::<_, DeletionRow>(
            r#"
            SELECT ci.id, ci.project_id, ci.source_path, ci.content_hash,
                   ci.annotated_path, ci.avatar_path, ci.destination_path,
                   ci.destination_avatar_path, ci.destination_file_state,
                   ci.destination_avatar_file_state, ci.asset_id, ci.status
            FROM capture_items ci
            WHERE ci.id = ?
            "#,
        )
        .bind(capture_item_id.trim())
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as::<_, DeletionRow>(
            r#"
            SELECT ci.id, ci.project_id, ci.source_path, ci.content_hash,
                   ci.annotated_path, ci.avatar_path, ci.destination_path,
                   ci.destination_avatar_path, ci.destination_file_state,
                   ci.destination_avatar_file_state, ci.asset_id, ci.status
            FROM capture_items ci
            WHERE ci.character_id = ?
            ORDER BY ci.captured_at
            "#,
        )
        .bind(character_id.unwrap_or_default().trim())
        .fetch_all(pool)
        .await?
    };
    if capture_item_id.is_some() && rows.is_empty() {
        return Err(AppError::NotFound("capture item".to_owned()));
    }
    Ok(rows)
}

async fn load_rows_connection(
    connection: &mut SqliteConnection,
    capture_item_id: Option<&str>,
    character_id: Option<&str>,
) -> Result<Vec<DeletionRow>, AppError> {
    let rows = if let Some(capture_item_id) = capture_item_id {
        sqlx::query_as::<_, DeletionRow>(
            r#"
            SELECT ci.id, ci.project_id, ci.source_path, ci.content_hash,
                   ci.annotated_path, ci.avatar_path, ci.destination_path,
                   ci.destination_avatar_path, ci.destination_file_state,
                   ci.destination_avatar_file_state, ci.asset_id, ci.status
            FROM capture_items ci
            WHERE ci.id = ?
            "#,
        )
        .bind(capture_item_id)
        .fetch_all(&mut *connection)
        .await?
    } else {
        sqlx::query_as::<_, DeletionRow>(
            r#"
            SELECT ci.id, ci.project_id, ci.source_path, ci.content_hash,
                   ci.annotated_path, ci.avatar_path, ci.destination_path,
                   ci.destination_avatar_path, ci.destination_file_state,
                   ci.destination_avatar_file_state, ci.asset_id, ci.status
            FROM capture_items ci
            WHERE ci.character_id = ?
            ORDER BY ci.captured_at
            "#,
        )
        .bind(character_id.unwrap_or_default())
        .fetch_all(&mut *connection)
        .await?
    };
    if capture_item_id.is_some() && rows.is_empty() {
        return Err(AppError::NotFound("capture item".to_owned()));
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{db, models::capture::RegisterCaptureInput, services::test_support};

    struct RemovingRecycler {
        fail_name: Option<&'static str>,
    }

    impl FileRecycler for RemovingRecycler {
        async fn recycle(&self, path: &Path) -> Result<(), AppError> {
            if self
                .fail_name
                .is_some_and(|name| path.file_name().and_then(|value| value.to_str()) == Some(name))
            {
                return Err(AppError::Conflict("recycle unavailable".to_owned()));
            }
            tokio::fs::remove_file(path).await?;
            Ok(())
        }
    }

    #[test]
    fn deletion_preview_counts_unique_network_targets() {
        let row = DeletionRow {
            id: "capture-1".to_owned(),
            project_id: "project-1".to_owned(),
            source_path: r"C:\shots\one.png".to_owned(),
            content_hash: Some("hash".to_owned()),
            annotated_path: None,
            avatar_path: None,
            destination_path: Some(r"\\nas\archive\one.png".to_owned()),
            destination_avatar_path: Some(r"\\nas\archive\one.png".to_owned()),
            destination_file_state: "available".to_owned(),
            destination_avatar_file_state: "available".to_owned(),
            asset_id: None,
            status: "completed".to_owned(),
        };

        let result = preview(&[row]).expect("preview");
        assert_eq!(result.destination_file_count, 1);
        assert_eq!(result.network_destination_file_count, 1);
    }

    async fn item_fixture(
        pool: &SqlitePool,
    ) -> (
        tempfile::TempDir,
        crate::models::capture::CaptureItem,
        PathBuf,
    ) {
        let fixture = test_support::project_with_directories(pool, "Delete")
            .await
            .expect("project");
        let session = test_support::start_session(pool, &fixture.project_id)
            .await
            .expect("session");
        let source = fixture.source_directory.join("capture.png");
        tokio::fs::write(&source, b"immutable source")
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
        (fixture._workspace, item, source)
    }

    #[tokio::test]
    async fn deleting_a_capture_preserves_source_and_ignores_its_hash() {
        let pool = db::test_pool().await;
        let (workspace, item, source) = item_fixture(&pool).await;
        sqlx::query("UPDATE capture_items SET annotated_path = ? WHERE id = ?")
            .bind(capture_service::path_to_string(&source))
            .bind(&item.id)
            .execute(&pool)
            .await
            .expect("malformed derived source alias");
        sqlx::query("UPDATE projects SET cover_capture_item_id = ? WHERE id = ?")
            .bind(&item.id)
            .bind(&item.project_id)
            .execute(&pool)
            .await
            .expect("cover");

        let result = delete_capture(
            &pool,
            workspace.path(),
            workspace.path(),
            &item.id,
            false,
            false,
            &RemovingRecycler { fail_name: None },
        )
        .await
        .expect("delete");

        assert!(result.completed);
        assert_eq!(result.records_deleted, 1);
        assert!(source.is_file(), "source images are never deleted");
        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM capture_items WHERE id = ?")
            .bind(&item.id)
            .fetch_one(&pool)
            .await
            .expect("count");
        assert_eq!(rows, 0);
        let ignored: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM ignored_capture_contents WHERE project_id = ? AND content_hash = ?",
        )
        .bind(&item.project_id)
        .bind(item.content_hash.as_deref())
        .fetch_one(&pool)
        .await
        .expect("ignored");
        assert_eq!(ignored, 1);
        let cover: Option<String> =
            sqlx::query_scalar("SELECT cover_capture_item_id FROM projects WHERE id = ?")
                .bind(&item.project_id)
                .fetch_one(&pool)
                .await
                .expect("cover");
        assert!(cover.is_none());
    }

    #[tokio::test]
    async fn partial_recycle_keeps_records_and_retry_finishes_idempotently() {
        let pool = db::test_pool().await;
        let (workspace, item, source) = item_fixture(&pool).await;
        let first = workspace.path().join("first.png");
        let second = workspace.path().join("second.png");
        tokio::fs::write(&first, b"first").await.expect("first");
        tokio::fs::write(&second, b"second").await.expect("second");
        sqlx::query(
            "UPDATE capture_items SET destination_path = ?, destination_avatar_path = ?, archived_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), status = 'completed' WHERE id = ?",
        )
        .bind(capture_service::path_to_string(&first))
        .bind(capture_service::path_to_string(&second))
        .bind(&item.id)
        .execute(&pool)
        .await
        .expect("targets");

        let partial = delete_capture(
            &pool,
            workspace.path(),
            workspace.path(),
            &item.id,
            true,
            false,
            &RemovingRecycler {
                fail_name: Some("second.png"),
            },
        )
        .await
        .expect("partial");
        assert!(!partial.completed);
        assert_eq!(partial.destination_files_recycled, 1);
        assert_eq!(partial.failures.len(), 1);
        let retained = capture_service::get_item(&pool, &item.id)
            .await
            .expect("record retained");
        assert_eq!(retained.destination_file_state, "missing");
        assert_eq!(retained.destination_avatar_file_state, "unavailable");
        assert!(!first.exists());
        assert!(second.exists());
        assert!(source.exists());

        tokio::fs::write(&first, b"new occupant")
            .await
            .expect("new occupant");
        let protected_retry = delete_capture(
            &pool,
            workspace.path(),
            workspace.path(),
            &item.id,
            true,
            false,
            &RemovingRecycler {
                fail_name: Some("second.png"),
            },
        )
        .await
        .expect("protected retry");
        assert!(!protected_retry.completed);
        assert!(protected_retry.failures.iter().any(|failure| {
            failure.path.ends_with("first.png") && failure.error.contains("new file")
        }));
        assert!(
            first.exists(),
            "a new occupant must never be recycled on retry"
        );
        tokio::fs::remove_file(&first)
            .await
            .expect("remove test occupant");

        let retried = delete_capture(
            &pool,
            workspace.path(),
            workspace.path(),
            &item.id,
            true,
            false,
            &RemovingRecycler { fail_name: None },
        )
        .await
        .expect("retry");
        assert!(retried.completed);
        assert_eq!(retried.destination_files_already_missing, 1);
        assert_eq!(retried.destination_files_recycled, 1);
    }

    #[tokio::test]
    async fn deleting_capture_keeps_an_asset_shared_with_another_project() {
        let pool = db::test_pool().await;
        let (workspace, item, _source) = item_fixture(&pool).await;
        let other = crate::services::project_service::create(
            &pool,
            crate::models::project::CreateProjectInput {
                name: "Other".to_owned(),
                description: None,
                cover_asset_id: None,
            },
        )
        .await
        .expect("other project");
        sqlx::query(
            "INSERT INTO assets (id, name, asset_type, path) VALUES ('shared-asset', 'shared', 'image', 'D:\\archive\\shared.png')",
        )
        .execute(&pool)
        .await
        .expect("asset");
        for project_id in [&item.project_id, &other.id] {
            sqlx::query(
                "INSERT INTO project_assets (project_id, asset_id) VALUES (?, 'shared-asset')",
            )
            .bind(project_id)
            .execute(&pool)
            .await
            .expect("asset link");
        }
        sqlx::query("UPDATE capture_items SET asset_id = 'shared-asset' WHERE id = ?")
            .bind(&item.id)
            .execute(&pool)
            .await
            .expect("capture asset");

        delete_capture(
            &pool,
            workspace.path(),
            workspace.path(),
            &item.id,
            false,
            false,
            &RemovingRecycler { fail_name: None },
        )
        .await
        .expect("delete capture");

        let asset_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM assets WHERE id = 'shared-asset'")
                .fetch_one(&pool)
                .await
                .expect("asset count");
        let links: Vec<String> = sqlx::query_scalar(
            "SELECT project_id FROM project_assets WHERE asset_id = 'shared-asset' ORDER BY project_id",
        )
        .fetch_all(&pool)
        .await
        .expect("links");
        assert_eq!(asset_count, 1);
        assert_eq!(links, vec![other.id]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn target_alias_cannot_bypass_source_protection() {
        let pool = db::test_pool().await;
        let (workspace, item, source) = item_fixture(&pool).await;
        let alias = workspace.path().join("source-alias.png");
        std::os::unix::fs::symlink(&source, &alias).expect("source alias");
        sqlx::query(
            "UPDATE capture_items SET destination_path = ?, status = 'completed' WHERE id = ?",
        )
        .bind(capture_service::path_to_string(&alias))
        .bind(&item.id)
        .execute(&pool)
        .await
        .expect("target alias");

        let result = delete_capture(
            &pool,
            workspace.path(),
            workspace.path(),
            &item.id,
            true,
            false,
            &RemovingRecycler { fail_name: None },
        )
        .await
        .expect("safe refusal");

        assert!(!result.completed);
        assert_eq!(result.failures.len(), 1);
        assert!(source.exists());
        capture_service::get_item(&pool, &item.id)
            .await
            .expect("record retained");
    }

    #[tokio::test]
    async fn character_delete_cascades_bound_captures_but_keeps_sources() {
        let pool = db::test_pool().await;
        let (workspace, item, source) = item_fixture(&pool).await;
        let target = workspace.path().join("character-target.png");
        tokio::fs::write(&target, b"derived target")
            .await
            .expect("target");
        let character = crate::services::character_service::create(
            &pool,
            crate::models::character::CreateCharacterInput {
                project_id: item.project_id.clone(),
                name: "Delete me".to_owned(),
                aliases_json: None,
            },
        )
        .await
        .expect("character");
        sqlx::query(
            "UPDATE capture_items SET character_id = ?, classification = 'person', status = 'completed', archived_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), destination_path = ? WHERE id = ?",
        )
        .bind(&character.id)
        .bind(capture_service::path_to_string(&target))
        .bind(&item.id)
        .execute(&pool)
        .await
        .expect("bind");

        let result = delete_character(
            &pool,
            workspace.path(),
            workspace.path(),
            &character.id,
            true,
            false,
            &RemovingRecycler { fail_name: None },
        )
        .await
        .expect("delete character");
        assert!(result.completed);
        assert_eq!(result.records_deleted, 1);
        assert!(source.exists());
        assert!(!target.exists());
        assert_eq!(result.destination_files_recycled, 1);
        let characters: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM characters WHERE id = ?")
            .bind(&character.id)
            .fetch_one(&pool)
            .await
            .expect("count");
        assert_eq!(characters, 0);
    }

    #[tokio::test]
    async fn migration_0020_preserves_capture_faces_and_face_samples() {
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(":memory:")
                    .foreign_keys(true),
            )
            .await
            .expect("pre-0020 pool");
        let mut files: Vec<String> = std::fs::read_dir("./migrations")
            .expect("migrations directory")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| {
                name.ends_with(".sql") && name.as_str() < "0020_capture_file_lifecycle.sql"
            })
            .collect();
        files.sort();
        let mut connection = pool.acquire().await.expect("connection");
        for file in files {
            let sql =
                std::fs::read_to_string(format!("./migrations/{file}")).expect("read migration");
            sqlx::raw_sql(&sql)
                .execute(&mut *connection)
                .await
                .unwrap_or_else(|error| panic!("apply {file}: {error}"));
        }
        sqlx::raw_sql(
            r#"
            INSERT INTO projects (id, name) VALUES ('project', 'Migration');
            INSERT INTO capture_sessions (id, project_id) VALUES ('session', 'project');
            INSERT INTO characters (id, project_id, name) VALUES ('character', 'project', 'Ava');
            INSERT INTO capture_items (
                id, project_id, session_id, character_id, source_path,
                content_hash, status
            ) VALUES (
                'capture', 'project', 'session', 'character', 'C:\\shots\\same.png',
                'old-hash', 'completed'
            );
            INSERT INTO character_face_samples (
                id, character_id, capture_item_id, feature_json
            ) VALUES ('sample', 'character', 'capture', '[1.0,0.0]');
            INSERT INTO capture_faces (
                id, capture_item_id, face_index, is_primary, feature_json
            ) VALUES ('face', 'capture', 0, 1, '[1.0,0.0]');
            "#,
        )
        .execute(&mut *connection)
        .await
        .expect("seed pre-0020 data");

        let migration = std::fs::read_to_string("./migrations/0020_capture_file_lifecycle.sql")
            .expect("read 0020");
        sqlx::raw_sql(&migration)
            .execute(&mut *connection)
            .await
            .expect("apply 0020");

        let capture: (String, String, String) = sqlx::query_as(
            "SELECT content_hash, source_file_state, destination_file_state FROM capture_items WHERE id = 'capture'",
        )
        .fetch_one(&mut *connection)
        .await
        .expect("preserved capture");
        assert_eq!(
            capture,
            (
                "old-hash".to_owned(),
                "unknown".to_owned(),
                "none".to_owned()
            )
        );
        let sample_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM character_face_samples WHERE id = 'sample' AND capture_item_id = 'capture'",
        )
        .fetch_one(&mut *connection)
        .await
        .expect("sample count");
        let face_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM capture_faces WHERE id = 'face' AND capture_item_id = 'capture'",
        )
        .fetch_one(&mut *connection)
        .await
        .expect("face count");
        assert_eq!((sample_count, face_count), (1, 1));
        let foreign_key_issues: Vec<(String, i64, String, i64)> =
            sqlx::query_as("PRAGMA foreign_key_check")
                .fetch_all(&mut *connection)
                .await
                .expect("foreign key check");
        assert!(foreign_key_issues.is_empty());
    }
}
