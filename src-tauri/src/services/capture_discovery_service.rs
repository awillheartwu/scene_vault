use std::{
    path::PathBuf,
    time::{Duration, SystemTime},
};

use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter};

use crate::{
    error::AppError,
    models::{
        capture::{
            CaptureFileReconcileResult, DiscoverCapturesInput, DiscoverCapturesResult,
            ReconcileCaptureFilesInput,
        },
        diagnostics::{LogLevel, LogRecord},
    },
    services::{capture_service, log_service},
};

const DEFAULT_STABILITY_DELAY_MS: u64 = 250;
const MAX_STABILITY_DELAY_MS: u64 = 2_000;
const BACKGROUND_POLL_INTERVAL: Duration = Duration::from_secs(2);
/// A completed source scan slower than this is surfaced as a slow-scan
/// warning. The 1.0 scale commitment (a 10,000-entry source directory, see
/// `scripts/scale-benchmark`) scans in well under a second, so the elapsed
/// duration alone drives the warning: a large-but-fast scan is the expected
/// steady state, not noise.
const SLOW_SCAN_WARNING_THRESHOLD_MS: u64 = 1_000;
/// Filesystems with coarse timestamps (FAT/exFAT, some network shares) round
/// modification times down by up to two seconds. A live capture written just
/// after the session started must not look like a backfill.
const BACKFILL_MTIME_TOLERANCE: Duration = Duration::from_secs(2);

/// Whether a completed scan should be surfaced as a slow-scan warning.
pub(crate) fn should_warn_slow_scan(scan_duration_ms: u64) -> bool {
    scan_duration_ms >= SLOW_SCAN_WARNING_THRESHOLD_MS
}

#[derive(Debug)]
struct Candidate {
    path: PathBuf,
    length: u64,
    modified: Option<SystemTime>,
}

pub async fn discover(
    pool: &SqlitePool,
    app: Option<&AppHandle>,
    input: DiscoverCapturesInput,
) -> Result<DiscoverCapturesResult, AppError> {
    discover_with_options(pool, app, input, false).await
}

async fn discover_with_options(
    pool: &SqlitePool,
    _app: Option<&AppHandle>,
    input: DiscoverCapturesInput,
    force_rehash_known: bool,
) -> Result<DiscoverCapturesResult, AppError> {
    let session_id = input.session_id.trim();
    if session_id.is_empty() {
        return Err(AppError::Validation(
            "capture session id cannot be empty".to_owned(),
        ));
    }
    let delay_ms = input
        .stability_delay_ms
        .unwrap_or(DEFAULT_STABILITY_DELAY_MS);
    if delay_ms > MAX_STABILITY_DELAY_MS {
        return Err(AppError::Validation(format!(
            "capture stability delay cannot exceed {MAX_STABILITY_DELAY_MS} ms"
        )));
    }

    let session = capture_service::require_active_session(pool, session_id).await?;
    // A file that already existed before the session started is a backfill:
    // it must wait for the user to start recognition explicitly, like an
    // imported batch. Files written during the session are live captures
    // that process automatically. This guards sessions whose baseline
    // snapshot was skipped or was taken by an older build.
    let session_started_at_ms: Option<i64> = sqlx::query_scalar(
        r#"
        SELECT CAST((julianday(started_at) - 2440587.5) * 86400000 AS INTEGER)
        FROM capture_sessions
        WHERE id = ?
        "#,
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await?;
    let session_started_at = session_started_at_ms
        .and_then(|millis| u64::try_from(millis).ok())
        .and_then(|millis| SystemTime::UNIX_EPOCH.checked_add(Duration::from_millis(millis)));
    let mut candidates = Vec::new();
    let mut ignored_count = 0_u32;
    let mut unstable_count = 0_u32;
    let mut entries_scanned = 0_u32;
    let mut scanned_roots = Vec::new();
    let scan_started_at = SystemTime::now();

    let session_dirs = capture_service::session_directories(pool, &session).await?;
    for canonical_root in &session_dirs {
        let Ok(mut directory) = tokio::fs::read_dir(&canonical_root).await else {
            ignored_count = ignored_count.saturating_add(1);
            continue;
        };
        scanned_roots.push(canonical_root.clone());
        while let Some(entry) = directory.next_entry().await? {
            entries_scanned = entries_scanned.saturating_add(1);
            let path = entry.path();
            if !capture_service::is_supported_image(&path) {
                ignored_count = ignored_count.saturating_add(1);
                continue;
            }
            let metadata = match entry.metadata().await {
                Ok(metadata) if metadata.is_file() => metadata,
                _ => {
                    ignored_count = ignored_count.saturating_add(1);
                    continue;
                }
            };
            if metadata.len() == 0 {
                unstable_count = unstable_count.saturating_add(1);
                continue;
            }
            candidates.push(Candidate {
                path,
                length: metadata.len(),
                modified: metadata.modified().ok(),
            });
        }
    }

    candidates.sort_by(|left, right| left.path.cmp(&right.path));

    // Registered rows for this project, keyed by normalized path. Sessions
    // are work periods, but file identity is project-wide: a reused filename
    // must also retire an older generation from a previous session.
    #[derive(Clone)]
    struct RegisteredRow {
        id: String,
        file_size: i64,
        modified_at_ms: Option<i64>,
        content_hash: String,
        status: String,
        archived: bool,
    }
    let mut registered: std::collections::HashMap<String, Vec<RegisteredRow>> =
        sqlx::query_as::<_, (String, String, i64, Option<i64>, String, String, bool)>(
            r#"
            SELECT id, source_path, file_size, modified_at_ms, content_hash,
                   status, archived_at IS NOT NULL
            FROM capture_items
            WHERE project_id = ?
            "#,
        )
        .bind(&session.project_id)
        .fetch_all(pool)
        .await?
        .into_iter()
        .fold(std::collections::HashMap::new(), |mut rows, row| {
            let (id, source_path, file_size, modified_at_ms, content_hash, status, archived) = row;
            rows.entry(source_path).or_default().push(RegisteredRow {
                id,
                file_size,
                modified_at_ms,
                content_hash,
                status,
                archived,
            });
            rows
        });

    // Partition candidates: unchanged known files are skipped, files whose
    // recorded stats changed are re-hashed (content may have been replaced),
    // and unseen files go through the content-stability double hash before
    // registration so a half-written screenshot is never indexed.
    let mut already_known_count = 0_u32;
    struct StableCandidate {
        candidate: Candidate,
        canonical_path: PathBuf,
        key: String,
        first_hash: String,
    }
    let mut changed = Vec::new();
    let mut fresh = Vec::new();
    for candidate in candidates {
        let canonical_path = match tokio::fs::canonicalize(&candidate.path).await {
            Ok(path)
                if session_dirs
                    .iter()
                    .any(|root| capture_service::path_is_within(&path, root)) =>
            {
                path
            }
            _ => {
                ignored_count = ignored_count.saturating_add(1);
                continue;
            }
        };
        let key = capture_service::path_to_string(&canonical_path);
        let Some(rows) = registered.get(&key) else {
            fresh.push(StableCandidate {
                candidate,
                canonical_path,
                key,
                first_hash: String::new(),
            });
            continue;
        };
        let candidate_modified = candidate
            .modified
            .and_then(|modified| unix_millis(modified).ok())
            .map(i64::try_from)
            .and_then(Result::ok);
        let stats_match = rows.iter().any(|row| {
            row.file_size == candidate.length as i64
                && row.modified_at_ms.is_some()
                && row.modified_at_ms == candidate_modified
        });
        if stats_match && !force_rehash_known {
            sqlx::query(
                r#"
                UPDATE capture_items
                SET source_file_state = CASE
                        WHEN file_size = ? AND modified_at_ms = ? THEN 'available'
                        ELSE 'replaced'
                    END,
                    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                WHERE project_id = ? AND source_path = ?
                "#,
            )
            .bind(i64::try_from(candidate.length).unwrap_or(i64::MAX))
            .bind(candidate_modified)
            .bind(&session.project_id)
            .bind(&key)
            .execute(pool)
            .await?;
            already_known_count = already_known_count.saturating_add(1);
            continue;
        }
        changed.push(StableCandidate {
            candidate,
            canonical_path,
            key,
            first_hash: String::new(),
        });
    }
    // First content sample for every file that is new or looks replaced,
    // taken before the shared stability delay below.
    for item in fresh.iter_mut().chain(changed.iter_mut()) {
        item.first_hash = capture_service::sha256_file(&item.canonical_path)
            .await
            .unwrap_or_default();
    }

    if (!fresh.is_empty() || !changed.is_empty()) && delay_ms > 0 {
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    }

    // Ending a session while the stability delay is in progress must prevent
    // new rows from being registered.
    capture_service::require_active_session(pool, session_id).await?;

    let mut discovered_items = Vec::new();
    for item in fresh.into_iter().chain(changed) {
        let metadata = match tokio::fs::metadata(&item.candidate.path).await {
            Ok(metadata) if metadata.is_file() => metadata,
            _ => {
                unstable_count = unstable_count.saturating_add(1);
                continue;
            }
        };
        let modified = metadata.modified().ok();
        if metadata.len() == 0
            || metadata.len() != item.candidate.length
            || (item.candidate.modified.is_some() && modified != item.candidate.modified)
        {
            unstable_count = unstable_count.saturating_add(1);
            continue;
        }
        // Second content sample; registration only happens when both samples
        // agree, which a still-growing file can never satisfy.
        let second_hash = capture_service::sha256_file(&item.canonical_path)
            .await
            .unwrap_or_default();
        if item.first_hash.is_empty() || item.first_hash != second_hash {
            unstable_count = unstable_count.saturating_add(1);
            continue;
        }

        let Some(rows) = registered.get(&item.key).cloned() else {
            if capture_service::is_content_ignored(pool, &session.project_id, &second_hash).await? {
                ignored_count = ignored_count.saturating_add(1);
                continue;
            }
            if capture_service::matches_discovery_baseline(
                pool,
                session_id,
                &item.canonical_path,
                &metadata,
            )
            .await?
            {
                ignored_count = ignored_count.saturating_add(1);
                continue;
            }
            let is_backfill = session_started_at
                .and_then(|started_at| started_at.checked_sub(BACKFILL_MTIME_TOLERANCE))
                .is_some_and(|boundary| {
                    item.candidate
                        .modified
                        .is_some_and(|modified_at| modified_at < boundary)
                });
            let (registered_item, inserted) = capture_service::register_discovered_path(
                pool,
                session_id,
                &item.canonical_path,
                is_backfill,
            )
            .await?;
            if inserted {
                discovered_items.push(registered_item);
            } else {
                already_known_count = already_known_count.saturating_add(1);
            }
            continue;
        };

        // A registered file whose content is unchanged only drifted in size
        // or mtime (coarse timestamps, antivirus touches): refresh stats.
        if let Some(row) = rows.iter().find(|row| row.content_hash == second_hash) {
            sqlx::query(
                r#"
                UPDATE capture_items
                SET
                    file_size = ?,
                    modified_at_ms = ?,
                    source_file_state = 'available',
                    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                WHERE id = ?
                "#,
            )
            .bind(i64::try_from(metadata.len()).unwrap_or(i64::MAX))
            .bind(
                modified
                    .and_then(|value| unix_millis(value).ok())
                    .map(i64::try_from)
                    .and_then(Result::ok),
            )
            .bind(&row.id)
            .execute(pool)
            .await?;
            sqlx::query(
                "UPDATE capture_items SET source_file_state = 'replaced', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE project_id = ? AND source_path = ? AND id != ?",
            )
            .bind(&session.project_id)
            .bind(&item.key)
            .bind(&row.id)
            .execute(pool)
            .await?;
            already_known_count = already_known_count.saturating_add(1);
            continue;
        }

        // The path now holds different content. Every older generation keeps
        // its database identity and is marked replaced. Deletion remains an
        // explicit user action; discovery never silently removes history.
        sqlx::query(
            "UPDATE capture_items SET source_file_state = 'replaced', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE project_id = ? AND source_path = ?",
        )
        .bind(&session.project_id)
        .bind(&item.key)
        .execute(pool)
        .await?;
        if rows.iter().any(|row| row.status == "processing") {
            // A processing row owns this path until it finishes or fails.
            already_known_count = already_known_count.saturating_add(1);
            continue;
        }
        if capture_service::is_content_ignored(pool, &session.project_id, &second_hash).await? {
            ignored_count = ignored_count.saturating_add(1);
            continue;
        }
        if capture_service::matches_discovery_baseline(
            pool,
            session_id,
            &item.canonical_path,
            &metadata,
        )
        .await?
        {
            ignored_count = ignored_count.saturating_add(1);
            continue;
        }
        let is_backfill = session_started_at
            .and_then(|started_at| started_at.checked_sub(BACKFILL_MTIME_TOLERANCE))
            .is_some_and(|boundary| {
                item.candidate
                    .modified
                    .is_some_and(|modified_at| modified_at < boundary)
            });
        let (registered_item, inserted) = capture_service::register_discovered_path(
            pool,
            session_id,
            &item.canonical_path,
            is_backfill,
        )
        .await?;
        if inserted {
            discovered_items.push(registered_item);
        } else {
            already_known_count = already_known_count.saturating_add(1);
        }
    }

    // Rows whose paths disappeared are retained only when archived. An
    // unarchived disappearance is handled by the worker's guarded purge.
    for (path, rows) in &mut registered {
        let path = PathBuf::from(path);
        if path.is_file()
            || !scanned_roots
                .iter()
                .any(|root| capture_service::path_is_within(&path, root))
        {
            continue;
        }
        for row in rows.iter().filter(|row| row.archived) {
            sqlx::query(
                "UPDATE capture_items SET source_file_state = 'missing', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
            )
            .bind(&row.id)
            .execute(pool)
            .await?;
        }
    }
    let scan_duration_ms = SystemTime::now()
        .duration_since(scan_started_at)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0);

    Ok(DiscoverCapturesResult {
        discovered_count: discovered_items.len() as u32,
        discovered_items,
        already_known_count,
        unstable_count,
        ignored_count,
        entries_scanned,
        scan_duration_ms,
    })
}

/// Runs the same source reconciliation as the background poll and also checks
/// archive targets on demand. Target-root failure is reported as unavailable,
/// never as a mass deletion or missing-file conclusion.
pub async fn reconcile_files(
    pool: &SqlitePool,
    app: Option<&AppHandle>,
    input: ReconcileCaptureFilesInput,
) -> Result<CaptureFileReconcileResult, AppError> {
    let session_id = input.session_id.trim().to_owned();
    // A user-triggered reconciliation deliberately hashes known files too.
    // This catches a replacement that happens to preserve both length and
    // filesystem mtime, while background polling keeps its cheap stat path.
    let discovery = discover_with_options(
        pool,
        app,
        DiscoverCapturesInput {
            session_id: session_id.clone(),
            stability_delay_ms: None,
        },
        true,
    )
    .await?;
    let root: Option<String> = sqlx::query_scalar(
        r#"
        SELECT project.destination_directory
        FROM capture_sessions session
        JOIN projects project ON project.id = session.project_id
        WHERE session.id = ?
        "#,
    )
    .bind(&session_id)
    .fetch_optional(pool)
    .await?
    .flatten();
    let targets: Vec<(String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT id, destination_path, destination_avatar_path FROM capture_items WHERE session_id = ?",
    )
    .bind(&session_id)
    .fetch_all(pool)
    .await?;
    for (id, destination, avatar) in targets {
        let destination_state = check_target_state(destination.as_deref(), root.as_deref()).await;
        let avatar_state = check_target_state(avatar.as_deref(), root.as_deref()).await;
        sqlx::query(
            "UPDATE capture_items SET destination_file_state = ?, destination_avatar_file_state = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
        )
        .bind(destination_state)
        .bind(avatar_state)
        .bind(id)
        .execute(pool)
        .await?;
    }
    let (source_missing_count, source_replaced_count, destination_missing_count, destination_unavailable_count): (i64, i64, i64, i64) = sqlx::query_as(
        r#"
        SELECT
            COALESCE(SUM(CASE WHEN source_file_state = 'missing' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN source_file_state = 'replaced' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN destination_file_state = 'missing' OR destination_avatar_file_state = 'missing' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN destination_file_state = 'unavailable' OR destination_avatar_file_state = 'unavailable' THEN 1 ELSE 0 END), 0)
        FROM capture_items WHERE session_id = ?
        "#,
    )
    .bind(&session_id)
    .fetch_one(pool)
    .await?;
    Ok(CaptureFileReconcileResult {
        discovered_count: discovery.discovered_count,
        source_missing_count: source_missing_count.max(0) as u32,
        source_replaced_count: source_replaced_count.max(0) as u32,
        destination_missing_count: destination_missing_count.max(0) as u32,
        destination_unavailable_count: destination_unavailable_count.max(0) as u32,
        unstable_count: discovery.unstable_count,
    })
}

pub(crate) async fn check_target_state(path: Option<&str>, root: Option<&str>) -> &'static str {
    let Some(path) = path else {
        return "none";
    };
    match tokio::fs::metadata(path).await {
        Ok(metadata) if metadata.is_file() => "available",
        Ok(_) => "missing",
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => match root {
            Some(root)
                if tokio::fs::metadata(root)
                    .await
                    .is_ok_and(|value| value.is_dir()) =>
            {
                "missing"
            }
            _ => "unavailable",
        },
        Err(_) => "unavailable",
    }
}

fn unix_millis(time: SystemTime) -> Result<i64, std::time::SystemTimeError> {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
}

/// Continuously reconciles active sessions with their source directories.
/// Directory scans are idempotent, so this also backfills screenshots that
/// arrived while Scene Vault was not running.
pub async fn run_background_polling(pool: SqlitePool, app: AppHandle) {
    let mut interval = tokio::time::interval(BACKGROUND_POLL_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        let _ = poll_active_sessions_once(&pool, Some(&app)).await;
    }
}

async fn poll_active_sessions_once(
    pool: &SqlitePool,
    app: Option<&AppHandle>,
) -> Result<(), AppError> {
    let session_ids: Vec<String> =
        sqlx::query_scalar("SELECT id FROM capture_sessions WHERE status = 'active'")
            .fetch_all(pool)
            .await?;
    for session_id in session_ids {
        // Directories added to the project while this session is running are
        // attached here (with a baseline snapshot of their pre-existing
        // images) so scanning and the import dialog pick them up without a
        // session restart.
        if let Ok(session) = capture_service::get_session(pool, &session_id).await {
            if let Err(error) =
                capture_service::sync_session_source_directories(pool, &session).await
            {
                log_service::warn(
                    "capture.discovery",
                    format!("failed to sync session source directories: {error}"),
                );
            }
        }
        // One unavailable source (for example a sleeping network share) must
        // not stop other active sessions from being reconciled.
        match discover(
            pool,
            app,
            DiscoverCapturesInput {
                session_id: session_id.clone(),
                stability_delay_ms: None,
            },
        )
        .await
        {
            Ok(result) => {
                // Observability only: scans that take longer than expected
                // surface here before polling needs any architectural change
                // (notify/reconciliation is a later option). Entry count alone
                // never warns: at the committed 1.0 scale (10k entries) a full
                // scan completes in ~100ms and must not warn on every poll.
                if should_warn_slow_scan(result.scan_duration_ms) {
                    log_service::record_event(LogRecord {
                        level: LogLevel::Warn,
                        module: "capture.discovery".to_owned(),
                        message: format!(
                            "slow scan of {} entries; {} captures discovered",
                            result.entries_scanned, result.discovered_count
                        ),
                        event: Some("slow_scan".to_owned()),
                        session_id: Some(session_id.clone()),
                        duration_ms: Some(result.scan_duration_ms as f64),
                        outcome: Some("succeeded".to_owned()),
                        ..Default::default()
                    });
                }
                if let Some(app) = app {
                    for item in result.discovered_items {
                        let _ = app.emit("capture:item-created", item);
                    }
                }
            }
            Err(_error) => log_service::record_event(LogRecord {
                level: LogLevel::Warn,
                module: "capture.discovery".to_owned(),
                message: "background source scan failed; next poll will retry".to_owned(),
                event: Some("source_scan_failed".to_owned()),
                session_id: Some(session_id),
                outcome: Some("retrying".to_owned()),
                error_code: Some("source_unavailable".to_owned()),
                ..Default::default()
            }),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db,
        models::capture::EndCaptureSessionInput,
        services::{capture_service, test_support},
    };

    #[test]
    fn slow_scan_warning_is_duration_based_and_quiet_at_the_10k_commitment() {
        // The 1.0 fixture scans 10,000 entries in ~100ms; a large-but-fast
        // scan must not warn on every 2s poll.
        assert!(!should_warn_slow_scan(0));
        assert!(!should_warn_slow_scan(100));
        assert!(!should_warn_slow_scan(999));
        assert!(should_warn_slow_scan(1_000));
        assert!(should_warn_slow_scan(2_500));
    }

    async fn session_fixture(
        pool: &SqlitePool,
    ) -> (
        tempfile::TempDir,
        crate::models::capture::CaptureSession,
        PathBuf,
    ) {
        let fixture = test_support::project_with_directories(pool, "Discovery")
            .await
            .expect("project fixture");
        let source = fixture.source_directory.clone();
        let session = test_support::start_session(pool, &fixture.project_id)
            .await
            .expect("start session");
        (fixture._workspace, session, source)
    }

    #[tokio::test]
    async fn discovers_stable_images_idempotently_and_ignores_other_files() {
        let pool = db::test_pool().await;
        let (_workspace, session, source) = session_fixture(&pool).await;
        tokio::fs::write(source.join("001.PNG"), b"image")
            .await
            .expect("image");
        tokio::fs::write(source.join("notes.txt"), b"not an image")
            .await
            .expect("text");
        tokio::fs::write(source.join("empty.jpg"), b"")
            .await
            .expect("empty image");

        let discovered = discover(
            &pool,
            None,
            DiscoverCapturesInput {
                session_id: session.id.clone(),
                stability_delay_ms: Some(0),
            },
        )
        .await
        .expect("discover");
        assert_eq!(discovered.discovered_count, 1);
        assert_eq!(discovered.ignored_count, 1);
        assert_eq!(discovered.unstable_count, 1);
        assert!(
            discovered.entries_scanned >= 3,
            "all directory entries must be counted"
        );
        assert!(discovered.scan_duration_ms > 0);

        let repeated = discover(
            &pool,
            None,
            DiscoverCapturesInput {
                session_id: session.id,
                stability_delay_ms: Some(0),
            },
        )
        .await
        .expect("repeat discover");
        assert_eq!(repeated.discovered_count, 0);
        assert_eq!(repeated.already_known_count, 1);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn does_not_require_reading_already_known_files() {
        use std::os::unix::fs::PermissionsExt;

        let pool = db::test_pool().await;
        let (_workspace, session, source) = session_fixture(&pool).await;
        let image = source.join("001.PNG");
        tokio::fs::write(&image, b"image").await.expect("image");

        let first = discover(
            &pool,
            None,
            DiscoverCapturesInput {
                session_id: session.id.clone(),
                stability_delay_ms: Some(0),
            },
        )
        .await
        .expect("first discover");
        assert_eq!(first.discovered_count, 1);
        assert_eq!(first.already_known_count, 0);

        // Making the file unreadable must not matter: the registered path is
        // resolved from the database, so the fast path never opens it.
        tokio::fs::set_permissions(&image, std::fs::Permissions::from_mode(0o000))
            .await
            .expect("make unreadable");
        let repeated = discover(
            &pool,
            None,
            DiscoverCapturesInput {
                session_id: session.id,
                stability_delay_ms: Some(0),
            },
        )
        .await
        .expect("second discover must not need to read the known file");
        assert_eq!(repeated.discovered_count, 0);
        assert_eq!(repeated.already_known_count, 1);
    }

    #[tokio::test]
    async fn ignores_unchanged_baseline_files_but_discovers_overwritten_paths() {
        let pool = db::test_pool().await;
        let fixture = test_support::project_with_directories(&pool, "Baseline")
            .await
            .expect("project fixture");
        let source = fixture.source_directory.clone();
        let existing = source.join("existing.png");
        tokio::fs::write(&existing, b"old")
            .await
            .expect("old image");
        let session = test_support::start_session(&pool, &fixture.project_id)
            .await
            .expect("session");

        let unchanged = discover(
            &pool,
            None,
            DiscoverCapturesInput {
                session_id: session.id.clone(),
                stability_delay_ms: Some(0),
            },
        )
        .await
        .expect("unchanged baseline");
        assert_eq!(unchanged.discovered_count, 0);
        assert_eq!(unchanged.ignored_count, 1);

        tokio::fs::write(&existing, b"new image with a different size")
            .await
            .expect("overwrite");
        let overwritten = discover(
            &pool,
            None,
            DiscoverCapturesInput {
                session_id: session.id,
                stability_delay_ms: Some(0),
            },
        )
        .await
        .expect("overwritten path");
        assert_eq!(overwritten.discovered_count, 1);
    }

    #[tokio::test]
    async fn archived_path_reuse_keeps_history_and_registers_new_content() {
        let pool = db::test_pool().await;
        let (_workspace, session, source) = session_fixture(&pool).await;
        let image = source.join("reused.png");
        tokio::fs::write(&image, b"first generation")
            .await
            .expect("first");
        let first = discover(
            &pool,
            None,
            DiscoverCapturesInput {
                session_id: session.id.clone(),
                stability_delay_ms: Some(0),
            },
        )
        .await
        .expect("discover first")
        .discovered_items
        .into_iter()
        .next()
        .expect("first item");
        sqlx::query(
            "UPDATE capture_items SET status = 'completed', archived_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
        )
        .bind(&first.id)
        .execute(&pool)
        .await
        .expect("archive first");

        tokio::fs::write(&image, b"second generation with different content")
            .await
            .expect("replace");
        let second = discover(
            &pool,
            None,
            DiscoverCapturesInput {
                session_id: session.id,
                stability_delay_ms: Some(0),
            },
        )
        .await
        .expect("discover replacement");
        assert_eq!(second.discovered_count, 1);
        assert_ne!(second.discovered_items[0].id, first.id);
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT id, source_file_state FROM capture_items WHERE source_path = ? ORDER BY captured_at",
        )
        .bind(capture_service::path_to_string(&image))
        .fetch_all(&pool)
        .await
        .expect("versions");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], (first.id, "replaced".to_owned()));
        assert_eq!(rows[1].1, "available");
    }

    #[tokio::test]
    async fn path_reuse_across_sessions_marks_the_older_generation_replaced() {
        let pool = db::test_pool().await;
        let (_workspace, first_session, source) = session_fixture(&pool).await;
        let image = source.join("cross-session.png");
        tokio::fs::write(&image, b"first generation")
            .await
            .expect("first");
        let first = discover(
            &pool,
            None,
            DiscoverCapturesInput {
                session_id: first_session.id.clone(),
                stability_delay_ms: Some(0),
            },
        )
        .await
        .expect("discover first")
        .discovered_items
        .into_iter()
        .next()
        .expect("first item");
        sqlx::query(
            "UPDATE capture_items SET status = 'completed', archived_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
        )
        .bind(&first.id)
        .execute(&pool)
        .await
        .expect("archive first");
        sqlx::query("UPDATE capture_sessions SET status = 'completed' WHERE id = ?")
            .bind(&first_session.id)
            .execute(&pool)
            .await
            .expect("end first session");
        let second_session = test_support::start_session(&pool, &first_session.project_id)
            .await
            .expect("second session");

        tokio::fs::write(&image, b"second generation with different content")
            .await
            .expect("replace");
        let second = discover(
            &pool,
            None,
            DiscoverCapturesInput {
                session_id: second_session.id.clone(),
                stability_delay_ms: Some(0),
            },
        )
        .await
        .expect("discover second");
        assert_eq!(second.discovered_count, 1);
        assert_ne!(second.discovered_items[0].id, first.id);
        assert_eq!(second.discovered_items[0].session_id, second_session.id);
        let old_state: String =
            sqlx::query_scalar("SELECT source_file_state FROM capture_items WHERE id = ?")
                .bind(&first.id)
                .fetch_one(&pool)
                .await
                .expect("old state");
        assert_eq!(old_state, "replaced");
    }

    #[tokio::test]
    async fn manual_reconcile_rehashes_known_paths_even_when_stats_match() {
        let pool = db::test_pool().await;
        let (_workspace, session, source) = session_fixture(&pool).await;
        let image = source.join("same-stats.png");
        tokio::fs::write(&image, b"first generation")
            .await
            .expect("first");
        let first = discover(
            &pool,
            None,
            DiscoverCapturesInput {
                session_id: session.id.clone(),
                stability_delay_ms: Some(0),
            },
        )
        .await
        .expect("discover first")
        .discovered_items
        .into_iter()
        .next()
        .expect("first item");
        sqlx::query(
            "UPDATE capture_items SET status = 'completed', archived_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
        )
        .bind(&first.id)
        .execute(&pool)
        .await
        .expect("archive first");

        // Simulate a filesystem that reports the same cheap fingerprint for
        // replaced content by updating the recorded stats to the new file.
        tokio::fs::write(&image, b"other generation")
            .await
            .expect("replace with same length");
        let metadata = tokio::fs::metadata(&image).await.expect("metadata");
        let modified_at_ms = metadata
            .modified()
            .ok()
            .and_then(|value| unix_millis(value).ok())
            .and_then(|value| i64::try_from(value).ok());
        sqlx::query("UPDATE capture_items SET file_size = ?, modified_at_ms = ? WHERE id = ?")
            .bind(i64::try_from(metadata.len()).expect("length"))
            .bind(modified_at_ms)
            .bind(&first.id)
            .execute(&pool)
            .await
            .expect("match cheap stats");

        let result = reconcile_files(
            &pool,
            None,
            ReconcileCaptureFilesInput {
                session_id: session.id,
            },
        )
        .await
        .expect("manual reconcile");
        assert_eq!(result.discovered_count, 1);
        assert_eq!(result.source_replaced_count, 1);
        let versions: Vec<(String, String)> =
            sqlx::query_as("SELECT id, source_file_state FROM capture_items WHERE source_path = ?")
                .bind(capture_service::path_to_string(&image))
                .fetch_all(&pool)
                .await
                .expect("versions");
        assert_eq!(versions.len(), 2);
        assert!(versions
            .iter()
            .any(|row| row.0 == first.id && row.1 == "replaced"));
        assert_eq!(
            versions.iter().filter(|row| row.1 == "available").count(),
            1
        );
    }

    #[tokio::test]
    async fn manual_reconcile_tracks_deleted_source_and_target_then_same_name_replacement() {
        let pool = db::test_pool().await;
        let (_workspace, session, source) = session_fixture(&pool).await;
        let image = source.join("recreated.png");
        tokio::fs::write(&image, b"first generation")
            .await
            .expect("first source");
        let first = discover(
            &pool,
            None,
            DiscoverCapturesInput {
                session_id: session.id.clone(),
                stability_delay_ms: Some(0),
            },
        )
        .await
        .expect("discover first")
        .discovered_items
        .into_iter()
        .next()
        .expect("first item");
        let destination_root: String = sqlx::query_scalar(
            r#"
            SELECT project.destination_directory
            FROM capture_sessions session
            JOIN projects project ON project.id = session.project_id
            WHERE session.id = ?
            "#,
        )
        .bind(&session.id)
        .fetch_one(&pool)
        .await
        .expect("destination root");
        let target = PathBuf::from(destination_root).join("recreated-archive.png");
        tokio::fs::write(&target, b"archived first generation")
            .await
            .expect("archive target");
        sqlx::query(
            "UPDATE capture_items SET status = 'completed', archived_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), destination_path = ? WHERE id = ?",
        )
        .bind(capture_service::path_to_string(&target))
        .bind(&first.id)
        .execute(&pool)
        .await
        .expect("archive first");

        tokio::fs::remove_file(&image)
            .await
            .expect("delete source externally");
        tokio::fs::remove_file(&target)
            .await
            .expect("delete target externally");
        let missing = reconcile_files(
            &pool,
            None,
            ReconcileCaptureFilesInput {
                session_id: session.id.clone(),
            },
        )
        .await
        .expect("reconcile missing files");
        assert_eq!(missing.discovered_count, 0);
        assert_eq!(missing.source_missing_count, 1);
        assert_eq!(missing.destination_missing_count, 1);

        tokio::fs::write(&image, b"second generation with different content")
            .await
            .expect("recreate source with the same name");
        let replaced = reconcile_files(
            &pool,
            None,
            ReconcileCaptureFilesInput {
                session_id: session.id,
            },
        )
        .await
        .expect("reconcile replacement");
        assert_eq!(replaced.discovered_count, 1);
        assert_eq!(replaced.source_missing_count, 0);
        assert_eq!(replaced.source_replaced_count, 1);
        assert_eq!(replaced.destination_missing_count, 1);

        let versions: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT id, source_file_state, destination_file_state FROM capture_items WHERE source_path = ? ORDER BY captured_at",
        )
        .bind(capture_service::path_to_string(&image))
        .fetch_all(&pool)
        .await
        .expect("source generations");
        assert_eq!(versions.len(), 2);
        assert_eq!(
            versions[0],
            (first.id, "replaced".to_owned(), "missing".to_owned())
        );
        assert_eq!(versions[1].1, "available");
        assert_eq!(versions[1].2, "none");
    }

    #[tokio::test]
    async fn backfill_that_escaped_baseline_is_registered_deferred() {
        let pool = db::test_pool().await;
        let fixture = test_support::project_with_directories(&pool, "DeferredBackfill")
            .await
            .expect("project fixture");
        let source = fixture.source_directory.clone();
        let image = source.join("old.png");
        tokio::fs::write(&image, b"old image").await.expect("image");
        // Pin the file to a clearly pre-session timestamp instead of relying
        // on wall-clock gaps: coarse filesystems round mtimes down, which
        // would otherwise make this test timing-dependent.
        let old = std::time::UNIX_EPOCH + Duration::from_secs(1);
        std::fs::OpenOptions::new()
            .write(true)
            .open(&image)
            .expect("open image")
            .set_times(std::fs::FileTimes::new().set_modified(old))
            .expect("pin mtime");
        let session = test_support::start_session(&pool, &fixture.project_id)
            .await
            .expect("session");
        // The baseline snapshot recorded the file; drop that row so the scan
        // sees an unregistered file that predates the session start.
        let canonical = tokio::fs::canonicalize(&image).await.expect("canonical");
        sqlx::query(
            "DELETE FROM capture_session_baseline_files WHERE session_id = ? AND source_path = ?",
        )
        .bind(&session.id)
        .bind(capture_service::path_to_string(&canonical))
        .execute(&pool)
        .await
        .expect("clear baseline");

        let result = discover(
            &pool,
            None,
            DiscoverCapturesInput {
                session_id: session.id.clone(),
                stability_delay_ms: Some(0),
            },
        )
        .await
        .expect("discover");
        assert_eq!(result.discovered_count, 1);
        assert_eq!(
            capture_service::deferred_import_recognition_count(&pool, &session.id)
                .await
                .expect("deferred count"),
            1,
            "a file that predates the session must wait for the recognition button"
        );
    }

    #[tokio::test]
    async fn session_picks_up_directories_added_while_running() {
        use crate::{models::project::AddProjectSourceDirectoryInput, services::project_service};
        let pool = db::test_pool().await;
        let fixture = test_support::project_with_directories(&pool, "MidSessionDirs")
            .await
            .expect("fixture");
        let session = test_support::start_session(&pool, &fixture.project_id)
            .await
            .expect("session");

        // A second directory with pre-existing images is added mid-session.
        let added_dir = fixture._workspace.path().join("added");
        tokio::fs::create_dir(&added_dir).await.expect("added dir");
        project_service::add_source_directory(
            &pool,
            AddProjectSourceDirectoryInput {
                project_id: fixture.project_id.clone(),
                directory: capture_service::path_to_string(&added_dir),
            },
        )
        .await
        .expect("add dir");
        let old = added_dir.join("old.png");
        tokio::fs::write(&old, b"old image").await.expect("old");

        poll_active_sessions_once(&pool, None).await.expect("poll");

        let attached: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(*) FROM session_source_directories
            WHERE session_id = ? AND directory = ?
            "#,
        )
        .bind(&session.id)
        .bind(capture_service::path_to_string(&added_dir))
        .fetch_one(&pool)
        .await
        .expect("attached");
        assert_eq!(attached, 1, "mid-session directory must be attached");
        let baselined: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(*) FROM capture_session_baseline_files
            WHERE session_id = ? AND source_path = ?
            "#,
        )
        .bind(&session.id)
        .bind(capture_service::path_to_string(&old))
        .fetch_one(&pool)
        .await
        .expect("baselined");
        assert_eq!(baselined, 1, "pre-existing image must be baselined");

        let discovered = discover(
            &pool,
            None,
            DiscoverCapturesInput {
                session_id: session.id.clone(),
                stability_delay_ms: Some(0),
            },
        )
        .await
        .expect("discover old");
        assert_eq!(
            discovered.discovered_count, 0,
            "baselined image must not auto-register"
        );

        // A live capture written after the attach is discovered normally.
        let fresh = added_dir.join("fresh.png");
        tokio::fs::write(&fresh, b"fresh image")
            .await
            .expect("fresh");
        let discovered = discover(
            &pool,
            None,
            DiscoverCapturesInput {
                session_id: session.id,
                stability_delay_ms: Some(0),
            },
        )
        .await
        .expect("discover fresh");
        assert_eq!(
            discovered.discovered_count, 1,
            "live capture must be discovered"
        );
    }

    #[tokio::test]
    async fn live_captures_written_during_the_session_are_not_deferred() {
        let pool = db::test_pool().await;
        let (_workspace, session, source) = session_fixture(&pool).await;
        tokio::fs::write(source.join("live.png"), b"live capture")
            .await
            .expect("image");

        let result = discover(
            &pool,
            None,
            DiscoverCapturesInput {
                session_id: session.id.clone(),
                stability_delay_ms: Some(0),
            },
        )
        .await
        .expect("discover");
        assert_eq!(result.discovered_count, 1);
        assert_eq!(
            capture_service::deferred_import_recognition_count(&pool, &session.id)
                .await
                .expect("deferred count"),
            0,
            "a capture written after the session started must be live"
        );
    }

    #[tokio::test]
    async fn rejects_a_file_that_changes_during_stability_check() {
        let pool = db::test_pool().await;
        let (_workspace, session, source) = session_fixture(&pool).await;
        let screenshot = source.join("writing.png");
        tokio::fs::write(&screenshot, b"first")
            .await
            .expect("initial bytes");
        let writer_path = screenshot.clone();
        let writer = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(40)).await;
            tokio::fs::write(writer_path, b"second-write-is-longer")
                .await
                .expect("finish write");
        });

        let result = discover(
            &pool,
            None,
            DiscoverCapturesInput {
                session_id: session.id,
                stability_delay_ms: Some(150),
            },
        )
        .await
        .expect("discover");
        writer.await.expect("writer task");
        assert_eq!(result.discovered_count, 0);
        assert_eq!(result.unstable_count, 1);
    }

    #[tokio::test]
    async fn ended_sessions_cannot_discover_more_files() {
        let pool = db::test_pool().await;
        let (_workspace, session, source) = session_fixture(&pool).await;
        tokio::fs::write(source.join("late.png"), b"image")
            .await
            .expect("image");
        capture_service::end_session(
            &pool,
            EndCaptureSessionInput {
                session_id: session.id.clone(),
                status: Some("cancelled".to_owned()),
            },
        )
        .await
        .expect("end session");

        let result = discover(
            &pool,
            None,
            DiscoverCapturesInput {
                session_id: session.id,
                stability_delay_ms: Some(0),
            },
        )
        .await;
        assert!(matches!(result, Err(AppError::Conflict(_))));
    }

    #[tokio::test]
    async fn background_poll_once_backfills_active_sessions_after_restart() {
        let pool = db::test_pool().await;
        let (_workspace, session, source) = session_fixture(&pool).await;
        tokio::fs::write(source.join("offline-arrival.png"), b"image")
            .await
            .expect("image");

        poll_active_sessions_once(&pool, None)
            .await
            .expect("background poll");
        let items = capture_service::list_items(&pool, &session.id)
            .await
            .expect("items");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].status, "awaiting_label");
    }
}
