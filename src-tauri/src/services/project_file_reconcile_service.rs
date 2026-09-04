use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    time::SystemTime,
};

use sqlx::SqlitePool;

use crate::{
    error::AppError,
    models::capture::{ProjectFileReconcileResult, ReconcileProjectFilesInput},
    services::{capture_discovery_service, capture_service},
};

#[derive(Clone, Debug)]
struct SourceCandidate {
    path: PathBuf,
    normalized_path: String,
    file_name_key: String,
    file_size: i64,
    modified_at_ms: Option<i64>,
}

#[derive(Debug, sqlx::FromRow)]
struct CaptureFileRow {
    id: String,
    source_path: String,
    file_size: Option<i64>,
    content_hash: Option<String>,
    source_file_state: String,
    status: String,
    destination_path: Option<String>,
    destination_avatar_path: Option<String>,
}

#[derive(Debug)]
struct SourceUpdate {
    id: String,
    old_source_path: String,
    new_source_path: Option<String>,
    file_size: Option<i64>,
    modified_at_ms: Option<i64>,
    state: &'static str,
}

/// Audits every capture in a project. Existing files are verified by content,
/// missing paths may be rebound to a same-name/same-hash file in an enabled
/// source directory, and archive targets are checked without importing or
/// enqueueing any source file.
pub async fn reconcile_project_files(
    pool: &SqlitePool,
    input: ReconcileProjectFilesInput,
) -> Result<ProjectFileReconcileResult, AppError> {
    let project_id = input.project_id.trim();
    if project_id.is_empty() {
        return Err(AppError::Validation(
            "project id cannot be empty".to_owned(),
        ));
    }
    let destination_root: Option<String> =
        sqlx::query_scalar("SELECT destination_directory FROM projects WHERE id = ?")
            .bind(project_id)
            .fetch_optional(pool)
            .await?
            .ok_or_else(|| AppError::NotFound("project".to_owned()))?;
    let source_roots: Vec<String> = sqlx::query_scalar(
        "SELECT directory FROM project_source_directories WHERE project_id = ? AND enabled = 1 ORDER BY created_at",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?;
    let rows = sqlx::query_as::<_, CaptureFileRow>(
        r#"
        SELECT id, source_path, file_size, content_hash,
               source_file_state, status, destination_path, destination_avatar_path
        FROM capture_items
        WHERE project_id = ?
        ORDER BY captured_at, id
        "#,
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?;

    let (candidates, unavailable_source_directory_count) = scan_source_roots(&source_roots).await;
    let scanned_directory_count = source_roots
        .len()
        .saturating_sub(unavailable_source_directory_count as usize)
        as u32;
    let scanned_file_count = candidates.len() as u32;
    let candidates_by_path: HashMap<String, usize> = candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| (candidate.normalized_path.clone(), index))
        .collect();
    let occupied_paths: HashSet<String> = rows
        .iter()
        .filter(|row| candidates_by_path.contains_key(&row.source_path))
        .map(|row| row.source_path.clone())
        .collect();

    let mut updates = Vec::with_capacity(rows.len());
    let mut missing_by_identity: HashMap<(String, String), Vec<usize>> = HashMap::new();

    for row in &rows {
        match tokio::fs::metadata(&row.source_path).await {
            Ok(metadata) if metadata.is_file() => {
                let actual_hash = capture_service::sha256_file(Path::new(&row.source_path)).await;
                let state = match (&row.content_hash, actual_hash.as_ref()) {
                    (Some(expected), Ok(actual)) if expected == actual => "available",
                    (Some(_), Ok(_)) => "replaced",
                    (Some(_), Err(_)) => source_state_or_unknown(&row.source_file_state),
                    (None, _)
                        if row.file_size
                            == Some(i64::try_from(metadata.len()).unwrap_or(i64::MAX)) =>
                    {
                        "available"
                    }
                    _ => "replaced",
                };
                updates.push(SourceUpdate {
                    id: row.id.clone(),
                    old_source_path: row.source_path.clone(),
                    new_source_path: Some(row.source_path.clone()),
                    file_size: Some(i64::try_from(metadata.len()).unwrap_or(i64::MAX)),
                    modified_at_ms: metadata.modified().ok().and_then(unix_millis),
                    state,
                });
            }
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                updates.push(SourceUpdate {
                    id: row.id.clone(),
                    old_source_path: row.source_path.clone(),
                    new_source_path: None,
                    file_size: None,
                    modified_at_ms: None,
                    state: source_state_or_unknown(&row.source_file_state),
                });
            }
            _ => {
                let update_index = updates.len();
                updates.push(SourceUpdate {
                    id: row.id.clone(),
                    old_source_path: row.source_path.clone(),
                    new_source_path: None,
                    file_size: None,
                    modified_at_ms: None,
                    state: if row.source_file_state == "replaced" {
                        "replaced"
                    } else if unavailable_source_directory_count > 0 {
                        source_state_or_unknown(&row.source_file_state)
                    } else {
                        "missing"
                    },
                });
                if !matches!(
                    row.status.as_str(),
                    "queued" | "processing" | "archive_pending"
                ) {
                    if let (Some(file_name), Some(hash)) = (
                        Path::new(&row.source_path)
                            .file_name()
                            .and_then(|value| value.to_str()),
                        row.content_hash.as_ref(),
                    ) {
                        missing_by_identity
                            .entry((file_name.to_lowercase(), hash.clone()))
                            .or_default()
                            .push(update_index);
                    }
                }
            }
        }
    }

    let wanted_names: HashSet<&str> = missing_by_identity
        .keys()
        .map(|(name, _)| name.as_str())
        .collect();
    let mut candidates_by_identity: HashMap<(String, String), Vec<usize>> = HashMap::new();
    for (candidate_index, candidate) in candidates.iter().enumerate() {
        if occupied_paths.contains(&candidate.normalized_path)
            || !wanted_names.contains(candidate.file_name_key.as_str())
        {
            continue;
        }
        if let Ok(hash) = capture_service::sha256_file(&candidate.path).await {
            candidates_by_identity
                .entry((candidate.file_name_key.clone(), hash))
                .or_default()
                .push(candidate_index);
        }
    }

    let mut ambiguous_count = 0_u32;
    for (identity, update_indexes) in missing_by_identity {
        let matches = candidates_by_identity
            .get(&identity)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if update_indexes.len() == 1 && matches.len() == 1 {
            let candidate = &candidates[matches[0]];
            let update = &mut updates[update_indexes[0]];
            update.new_source_path = Some(candidate.normalized_path.clone());
            update.file_size = Some(candidate.file_size);
            update.modified_at_ms = candidate.modified_at_ms;
            update.state = "available";
        } else if !matches.is_empty() {
            ambiguous_count = ambiguous_count.saturating_add(update_indexes.len() as u32);
        }
    }

    let mut destination_updates = Vec::with_capacity(rows.len());
    for row in &rows {
        if matches!(
            row.status.as_str(),
            "queued" | "processing" | "archive_pending"
        ) {
            continue;
        }
        let destination_state = capture_discovery_service::check_target_state(
            row.destination_path.as_deref(),
            destination_root.as_deref(),
        )
        .await;
        let avatar_state = capture_discovery_service::check_target_state(
            row.destination_avatar_path.as_deref(),
            destination_root.as_deref(),
        )
        .await;
        destination_updates.push((row.id.clone(), destination_state, avatar_state));
    }

    let mut transaction = pool.begin().await?;
    let mut relocated_count = 0_u32;
    for update in &updates {
        if let Some(path) = &update.new_source_path {
            let applied = sqlx::query(
                "UPDATE capture_items SET source_path = ?, file_size = ?, modified_at_ms = ?, source_file_state = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ? AND source_path = ?",
            )
            .bind(path)
            .bind(update.file_size)
            .bind(update.modified_at_ms)
            .bind(update.state)
            .bind(&update.id)
            .bind(&update.old_source_path)
            .execute(&mut *transaction)
            .await?;
            if path != &update.old_source_path && applied.rows_affected() == 1 {
                relocated_count = relocated_count.saturating_add(1);
            }
        } else {
            sqlx::query(
                "UPDATE capture_items SET source_file_state = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ? AND source_path = ?",
            )
            .bind(update.state)
            .bind(&update.id)
            .bind(&update.old_source_path)
            .execute(&mut *transaction)
            .await?;
        }
    }
    for (id, destination_state, avatar_state) in destination_updates {
        sqlx::query(
            "UPDATE capture_items SET destination_file_state = ?, destination_avatar_file_state = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
        )
        .bind(destination_state)
        .bind(avatar_state)
        .bind(id)
        .execute(&mut *transaction)
        .await?;
    }
    transaction.commit().await?;

    let (source_missing_count, source_replaced_count_db, destination_missing_count, destination_unavailable_count): (i64, i64, i64, i64) = sqlx::query_as(
        r#"
        SELECT
            COALESCE(SUM(source_file_state = 'missing'), 0),
            COALESCE(SUM(source_file_state = 'replaced'), 0),
            COALESCE(SUM(destination_file_state = 'missing') + SUM(destination_avatar_file_state = 'missing'), 0),
            COALESCE(SUM(destination_file_state = 'unavailable') + SUM(destination_avatar_file_state = 'unavailable'), 0)
        FROM capture_items WHERE project_id = ?
        "#,
    )
    .bind(project_id)
    .fetch_one(pool)
    .await?;

    Ok(ProjectFileReconcileResult {
        scanned_directory_count,
        scanned_file_count,
        source_checked_count: rows.len() as u32,
        relocated_count,
        source_missing_count: source_missing_count.max(0) as u32,
        source_replaced_count: source_replaced_count_db.max(0) as u32,
        ambiguous_count,
        unavailable_source_directory_count,
        destination_missing_count: destination_missing_count.max(0) as u32,
        destination_unavailable_count: destination_unavailable_count.max(0) as u32,
    })
}

async fn scan_source_roots(roots: &[String]) -> (Vec<SourceCandidate>, u32) {
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();
    let mut unavailable_count = 0_u32;
    for root in roots {
        let Ok(canonical_root) = tokio::fs::canonicalize(root).await else {
            unavailable_count = unavailable_count.saturating_add(1);
            continue;
        };
        let Ok(mut directory) = tokio::fs::read_dir(&canonical_root).await else {
            unavailable_count = unavailable_count.saturating_add(1);
            continue;
        };
        loop {
            let Ok(next) = directory.next_entry().await else {
                break;
            };
            let Some(entry) = next else { break };
            let path = entry.path();
            if !capture_service::is_supported_image(&path) {
                continue;
            }
            let Ok(metadata) = entry.metadata().await else {
                continue;
            };
            if !metadata.is_file() || metadata.len() == 0 {
                continue;
            }
            let Ok(canonical_path) = tokio::fs::canonicalize(&path).await else {
                continue;
            };
            if !capture_service::path_is_within(&canonical_path, &canonical_root) {
                continue;
            }
            let normalized_path = capture_service::normalized_path_string(&canonical_path);
            if !seen.insert(normalized_path.clone()) {
                continue;
            }
            let Some(file_name_key) = canonical_path
                .file_name()
                .and_then(|value| value.to_str())
                .map(str::to_lowercase)
            else {
                continue;
            };
            candidates.push(SourceCandidate {
                path: canonical_path,
                normalized_path,
                file_name_key,
                file_size: i64::try_from(metadata.len()).unwrap_or(i64::MAX),
                modified_at_ms: metadata.modified().ok().and_then(unix_millis),
            });
        }
    }
    candidates.sort_by(|left, right| left.normalized_path.cmp(&right.normalized_path));
    (candidates, unavailable_count)
}

fn source_state_or_unknown(state: &str) -> &'static str {
    match state {
        "available" => "available",
        "missing" => "missing",
        "replaced" => "replaced",
        _ => "unknown",
    }
}

fn unix_millis(time: SystemTime) -> Option<i64> {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db,
        models::{
            capture::{DiscoverCapturesInput, EndCaptureSessionInput},
            project::AddProjectSourceDirectoryInput,
        },
        services::{capture_discovery_service, capture_service, project_service, test_support},
    };

    async fn registered_item(
        pool: &SqlitePool,
        fixture: &test_support::ProjectFixture,
        name: &str,
        bytes: &[u8],
    ) -> String {
        let session = test_support::start_session(pool, &fixture.project_id)
            .await
            .expect("start session");
        let path = fixture.source_directory.join(name);
        tokio::fs::write(&path, bytes).await.expect("write source");
        let item = capture_discovery_service::discover(
            pool,
            None,
            DiscoverCapturesInput {
                session_id: session.id.clone(),
                stability_delay_ms: Some(0),
            },
        )
        .await
        .expect("discover")
        .discovered_items
        .into_iter()
        .next()
        .expect("registered item");
        capture_service::end_session(
            pool,
            EndCaptureSessionInput {
                session_id: session.id,
                status: Some("completed".to_owned()),
            },
        )
        .await
        .expect("end session");
        item.id
    }

    #[tokio::test]
    async fn relocates_unique_same_name_and_hash_without_importing_other_files() {
        let pool = db::test_pool().await;
        let fixture = test_support::project_with_directories(&pool, "Relocation")
            .await
            .expect("fixture");
        let item_id = registered_item(&pool, &fixture, "shot.png", b"same content").await;
        let missing_target = fixture.destination_directory.join("missing-archive.png");
        sqlx::query(
            "UPDATE capture_items SET classification = 'scene', status = 'completed', destination_path = ? WHERE id = ?",
        )
        .bind(capture_service::path_to_string(&missing_target))
        .bind(&item_id)
        .execute(&pool)
        .await
        .expect("prepare item");

        let moved = fixture._workspace.path().join("moved");
        tokio::fs::create_dir(&moved).await.expect("moved dir");
        tokio::fs::rename(
            fixture.source_directory.join("shot.png"),
            moved.join("shot.png"),
        )
        .await
        .expect("move source");
        tokio::fs::write(moved.join("unrelated.png"), b"unrelated")
            .await
            .expect("unrelated");
        sqlx::query("UPDATE project_source_directories SET directory = ? WHERE project_id = ?")
            .bind(capture_service::path_to_string(&moved))
            .bind(&fixture.project_id)
            .execute(&pool)
            .await
            .expect("replace source root");

        let result = reconcile_project_files(
            &pool,
            ReconcileProjectFilesInput {
                project_id: fixture.project_id.clone(),
            },
        )
        .await
        .expect("reconcile project");
        assert_eq!(result.relocated_count, 1);
        assert_eq!(result.scanned_file_count, 2);
        assert_eq!(result.source_missing_count, 0);
        assert_eq!(result.destination_missing_count, 1);
        let row: (String, String, String) = sqlx::query_as(
            "SELECT source_path, source_file_state, classification FROM capture_items WHERE id = ?",
        )
        .bind(&item_id)
        .fetch_one(&pool)
        .await
        .expect("updated item");
        assert_eq!(
            row.0,
            capture_service::path_to_string(&moved.join("shot.png"))
        );
        assert_eq!(row.1, "available");
        assert_eq!(row.2, "scene");
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM capture_items WHERE project_id = ?")
                .bind(&fixture.project_id)
                .fetch_one(&pool)
                .await
                .expect("capture count");
        assert_eq!(count, 1, "unrelated source files must not be imported");
    }

    #[tokio::test]
    async fn does_not_guess_between_duplicate_relocation_candidates() {
        let pool = db::test_pool().await;
        let fixture = test_support::project_with_directories(&pool, "Ambiguous")
            .await
            .expect("fixture");
        let item_id = registered_item(&pool, &fixture, "shot.png", b"same content").await;
        tokio::fs::remove_file(fixture.source_directory.join("shot.png"))
            .await
            .expect("remove old source");
        let first = fixture._workspace.path().join("first");
        let second = fixture._workspace.path().join("second");
        tokio::fs::create_dir(&first).await.expect("first dir");
        tokio::fs::create_dir(&second).await.expect("second dir");
        tokio::fs::write(first.join("shot.png"), b"same content")
            .await
            .expect("first candidate");
        tokio::fs::write(second.join("shot.png"), b"same content")
            .await
            .expect("second candidate");
        sqlx::query("UPDATE project_source_directories SET directory = ? WHERE project_id = ?")
            .bind(capture_service::path_to_string(&first))
            .bind(&fixture.project_id)
            .execute(&pool)
            .await
            .expect("first root");
        project_service::add_source_directory(
            &pool,
            AddProjectSourceDirectoryInput {
                project_id: fixture.project_id.clone(),
                directory: capture_service::path_to_string(&second),
            },
        )
        .await
        .expect("second root");

        let result = reconcile_project_files(
            &pool,
            ReconcileProjectFilesInput {
                project_id: fixture.project_id.clone(),
            },
        )
        .await
        .expect("reconcile project");
        assert_eq!(result.relocated_count, 0);
        assert_eq!(result.ambiguous_count, 1);
        assert_eq!(result.source_missing_count, 1);
        let state: String =
            sqlx::query_scalar("SELECT source_file_state FROM capture_items WHERE id = ?")
                .bind(item_id)
                .fetch_one(&pool)
                .await
                .expect("state");
        assert_eq!(state, "missing");
    }

    #[tokio::test]
    async fn preserves_source_state_when_a_configured_root_is_unavailable() {
        let pool = db::test_pool().await;
        let fixture = test_support::project_with_directories(&pool, "Unavailable")
            .await
            .expect("fixture");
        let item_id = registered_item(&pool, &fixture, "shot.png", b"content").await;
        tokio::fs::remove_dir_all(&fixture.source_directory)
            .await
            .expect("disconnect source");

        let result = reconcile_project_files(
            &pool,
            ReconcileProjectFilesInput {
                project_id: fixture.project_id.clone(),
            },
        )
        .await
        .expect("reconcile project");
        assert_eq!(result.unavailable_source_directory_count, 1);
        assert_eq!(result.source_missing_count, 0);
        let state: String =
            sqlx::query_scalar("SELECT source_file_state FROM capture_items WHERE id = ?")
                .bind(item_id)
                .fetch_one(&pool)
                .await
                .expect("state");
        assert_eq!(state, "available");
    }

    #[tokio::test]
    async fn marks_same_path_different_content_as_replaced_without_new_record() {
        let pool = db::test_pool().await;
        let fixture = test_support::project_with_directories(&pool, "Replacement")
            .await
            .expect("fixture");
        let item_id = registered_item(&pool, &fixture, "shot.png", b"first").await;
        tokio::fs::write(fixture.source_directory.join("shot.png"), b"second")
            .await
            .expect("replace source");

        let result = reconcile_project_files(
            &pool,
            ReconcileProjectFilesInput {
                project_id: fixture.project_id.clone(),
            },
        )
        .await
        .expect("reconcile project");
        assert_eq!(result.source_replaced_count, 1);
        let row: (String, i64) = sqlx::query_as(
            "SELECT source_file_state, (SELECT COUNT(*) FROM capture_items WHERE project_id = ?) FROM capture_items WHERE id = ?",
        )
        .bind(&fixture.project_id)
        .bind(item_id)
        .fetch_one(&pool)
        .await
        .expect("state and count");
        assert_eq!(row, ("replaced".to_owned(), 1));
    }
}
