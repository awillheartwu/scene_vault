use std::{
    path::PathBuf,
    time::{Duration, SystemTime},
};

use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter};

use crate::{
    error::AppError,
    models::capture::{DiscoverCapturesInput, DiscoverCapturesResult},
    services::{capture_service, log_service},
};

const DEFAULT_STABILITY_DELAY_MS: u64 = 250;
const MAX_STABILITY_DELAY_MS: u64 = 2_000;
const BACKGROUND_POLL_INTERVAL: Duration = Duration::from_secs(2);
/// Filesystems with coarse timestamps (FAT/exFAT, some network shares) round
/// modification times down by up to two seconds. A live capture written just
/// after the session started must not look like a backfill.
const BACKFILL_MTIME_TOLERANCE: Duration = Duration::from_secs(2);

#[derive(Debug)]
struct Candidate {
    path: PathBuf,
    length: u64,
    modified: Option<SystemTime>,
}

pub async fn discover(
    pool: &SqlitePool,
    input: DiscoverCapturesInput,
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
    let scan_started_at = SystemTime::now();

    let session_dirs = capture_service::session_directories(pool, &session).await?;
    for canonical_root in &session_dirs {
        let Ok(mut directory) = tokio::fs::read_dir(&canonical_root).await else {
            ignored_count = ignored_count.saturating_add(1);
            continue;
        };
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
    if !candidates.is_empty() && delay_ms > 0 {
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    }

    // Ending a session while the stability delay is in progress must prevent
    // new rows from being registered.
    capture_service::require_active_session(pool, session_id).await?;

    let mut discovered_items = Vec::new();
    let mut already_known_count = 0_u32;
    for candidate in candidates {
        let metadata = match tokio::fs::metadata(&candidate.path).await {
            Ok(metadata) if metadata.is_file() => metadata,
            _ => {
                unstable_count = unstable_count.saturating_add(1);
                continue;
            }
        };
        let modified = metadata.modified().ok();
        if metadata.len() == 0
            || metadata.len() != candidate.length
            || (candidate.modified.is_some() && modified != candidate.modified)
        {
            unstable_count = unstable_count.saturating_add(1);
            continue;
        }

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
        if capture_service::matches_discovery_baseline(pool, session_id, &canonical_path, &metadata)
            .await?
        {
            ignored_count = ignored_count.saturating_add(1);
            continue;
        }
        let is_backfill = session_started_at
            .and_then(|started_at| started_at.checked_sub(BACKFILL_MTIME_TOLERANCE))
            .is_some_and(|boundary| {
                candidate
                    .modified
                    .is_some_and(|modified_at| modified_at < boundary)
            });
        let (item, inserted) = capture_service::register_discovered_path(
            pool,
            session_id,
            &canonical_path,
            is_backfill,
        )
        .await?;
        if inserted {
            discovered_items.push(item);
        } else {
            already_known_count = already_known_count.saturating_add(1);
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
        // One unavailable source (for example a sleeping network share) must
        // not stop other active sessions from being reconciled.
        match discover(
            pool,
            DiscoverCapturesInput {
                session_id: session_id.clone(),
                stability_delay_ms: None,
            },
        )
        .await
        {
            Ok(result) => {
                // Observability only: a scan crossing the poll interval or a
                // directory growing large shows up here before polling needs any
                // architectural change (notify/reconciliation is a later option).
                if result.scan_duration_ms >= 1_000 || result.entries_scanned >= 1_000 {
                    log_service::warn(
                        "capture.discovery",
                        format!(
                            "slow scan: session={session_id} duration={}ms entries={} new={} known={} unstable={} ignored={}",
                            result.scan_duration_ms,
                            result.entries_scanned,
                            result.discovered_count,
                            result.already_known_count,
                            result.unstable_count,
                            result.ignored_count,
                        ),
                    );
                }
                if let Some(app) = app {
                    for item in result.discovered_items {
                        let _ = app.emit("capture:item-created", item);
                    }
                }
            }
            Err(error) => log_service::warn(
                "capture.discovery",
                format!("poll failed for session {session_id}: {error}"),
            ),
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
    async fn live_captures_written_during_the_session_are_not_deferred() {
        let pool = db::test_pool().await;
        let (_workspace, session, source) = session_fixture(&pool).await;
        tokio::fs::write(source.join("live.png"), b"live capture")
            .await
            .expect("image");

        let result = discover(
            &pool,
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
