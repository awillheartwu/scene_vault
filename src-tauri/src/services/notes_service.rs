use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    error::AppError,
    models::{
        diagnostics::{LogLevel, LogRecord},
        project_note::{GetProjectNoteInput, ProjectNote, UpdateProjectNoteInput},
    },
    services::log_service,
};

const NOTE_FILE_NAME: &str = "笔记.md";
const SYNC_POLL_INTERVAL: Duration = Duration::from_millis(2000);

/// Returns the project note, creating it lazily on first access. The note
/// lives in the most recent capture session's destination directory so the
/// user keeps zero-configuration control over where notes land.
pub async fn get_or_create(pool: &SqlitePool, project_id: &str) -> Result<ProjectNote, AppError> {
    let project_id = required(project_id, "project id")?;
    let project_exists: i64 =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM projects WHERE id = ?)")
            .bind(project_id)
            .fetch_one(pool)
            .await?;
    if project_exists == 0 {
        return Err(AppError::NotFound("project".to_owned()));
    }

    if let Some(note) = find_by_project(pool, project_id).await? {
        return Ok(note);
    }

    let destination: Option<String> = sqlx::query_scalar(
        r#"
        SELECT destination_directory
        FROM projects
        WHERE id = ?
        "#,
    )
    .bind(project_id)
    .fetch_optional(pool)
    .await?
    .flatten();
    let Some(destination) = destination else {
        return Err(AppError::Validation(
            "项目还没有截图会话，先开始一次会话以确定笔记位置".to_owned(),
        ));
    };

    let destination_directory = PathBuf::from(&destination);
    let remote_path = destination_directory.join(NOTE_FILE_NAME);
    let note = sqlx::query_as::<_, ProjectNote>(
        r#"
        INSERT INTO project_notes (id, project_id, destination_directory, remote_path)
        VALUES (?, ?, ?, ?)
        RETURNING
            id, project_id, destination_directory, remote_path, content, status,
            attempt_count, next_retry_at, last_synced_content, error_message,
            synced_at, created_at, updated_at
        "#,
    )
    .bind(Uuid::new_v4().to_string())
    .bind(project_id)
    .bind(&destination)
    .bind(path_to_string(&remote_path))
    .fetch_one(pool)
    .await?;
    Ok(note)
}

pub async fn get(pool: &SqlitePool, input: GetProjectNoteInput) -> Result<ProjectNote, AppError> {
    get_or_create(pool, &input.project_id).await
}

/// Saves note content locally and immediately attempts a remote sync so the
/// user sees near-real-time feedback. The background loop handles retries.
pub async fn update_content(
    pool: &SqlitePool,
    input: UpdateProjectNoteInput,
) -> Result<ProjectNote, AppError> {
    let note = get_or_create(pool, &input.project_id).await?;
    let updated = sqlx::query_as::<_, ProjectNote>(
        r#"
        UPDATE project_notes
        SET
            content = ?,
            status = 'pending',
            attempt_count = 0,
            next_retry_at = NULL,
            error_message = NULL,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ?
        RETURNING
            id, project_id, destination_directory, remote_path, content, status,
            attempt_count, next_retry_at, last_synced_content, error_message,
            synced_at, created_at, updated_at
        "#,
    )
    .bind(&input.content)
    .bind(&note.id)
    .fetch_one(pool)
    .await?;

    sync_note(pool, &updated).await
}

/// Processes the oldest note that is due for a sync attempt. Returns true when
/// a note was handled.
pub async fn sync_once(pool: &SqlitePool) -> Result<bool, AppError> {
    let note = sqlx::query_as::<_, ProjectNote>(
        r#"
        SELECT
            id, project_id, destination_directory, remote_path, content, status,
            attempt_count, next_retry_at, last_synced_content, error_message,
            synced_at, created_at, updated_at
        FROM project_notes
        WHERE status IN ('pending', 'failed')
          AND (next_retry_at IS NULL OR next_retry_at <= strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        ORDER BY updated_at ASC
        LIMIT 1
        "#,
    )
    .fetch_optional(pool)
    .await?;
    let Some(note) = note else {
        return Ok(false);
    };
    sync_note(pool, &note).await?;
    Ok(true)
}

async fn sync_note(pool: &SqlitePool, note: &ProjectNote) -> Result<ProjectNote, AppError> {
    if note.status == "synced" {
        return Ok(note.clone());
    }
    note_log(note, LogLevel::Info, "sync_started", "started", None);
    match write_remote(note).await {
        Ok(conflict_backed_up) => {
            if conflict_backed_up {
                note_log(
                    note,
                    LogLevel::Warn,
                    "external_conflict_backed_up",
                    "recovered",
                    None,
                );
            }
            let synced = mark_synced(pool, &note.id, &note.content).await?;
            note_log(&synced, LogLevel::Info, "sync_completed", "succeeded", None);
            Ok(synced)
        }
        Err(error) => {
            let failed = mark_sync_failure(pool, &note.id, &error.to_string()).await?;
            note_log(
                &failed,
                if failed.status == "failed" {
                    LogLevel::Error
                } else {
                    LogLevel::Warn
                },
                if failed.status == "failed" {
                    "sync_failed"
                } else {
                    "sync_retry_scheduled"
                },
                if failed.status == "failed" {
                    "failed"
                } else {
                    "retrying"
                },
                Some("note_sync_error"),
            );
            Ok(failed)
        }
    }
}

async fn write_remote(note: &ProjectNote) -> Result<bool, AppError> {
    let remote = PathBuf::from(&note.remote_path);
    let parent = remote.parent().ok_or_else(|| {
        AppError::Validation("note remote path has no parent directory".to_owned())
    })?;
    tokio::fs::create_dir_all(parent).await?;

    let existing = match tokio::fs::read_to_string(&remote).await {
        Ok(content) => Some(content),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(AppError::Io(error)),
    };

    let mut conflict_backed_up = false;
    if let Some(existing) = existing {
        let last_synced = note.last_synced_content.as_deref().unwrap_or("");
        let ours = existing == last_synced || existing == note.content;
        if !ours {
            // The file was changed outside the app (e.g. Obsidian). Keep the
            // remote edit as a timestamped copy before writing our content.
            let backup = parent.join(format!("笔记-{}.md", timestamp()));
            tokio::fs::write(&backup, &existing).await?;
            conflict_backed_up = true;
        }
    }

    let temporary = parent.join(format!(".scene-vault-note-{}.partial", Uuid::new_v4()));
    let result = async {
        tokio::fs::write(&temporary, &note.content).await?;
        let written = tokio::fs::read_to_string(&temporary).await?;
        if written != note.content {
            return Err(AppError::Io(std::io::Error::other(
                "note length verification failed",
            )));
        }
        tokio::fs::rename(&temporary, &remote).await?;
        Ok(conflict_backed_up)
    }
    .await;

    if result.is_err() || tokio::fs::try_exists(&temporary).await.unwrap_or(false) {
        let _ = tokio::fs::remove_file(&temporary).await;
    }
    result
}

async fn mark_synced(
    pool: &SqlitePool,
    note_id: &str,
    content: &str,
) -> Result<ProjectNote, AppError> {
    Ok(sqlx::query_as::<_, ProjectNote>(
        r#"
        UPDATE project_notes
        SET
            status = 'synced',
            attempt_count = 0,
            next_retry_at = NULL,
            last_synced_content = ?,
            error_message = NULL,
            synced_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ?
        RETURNING
            id, project_id, destination_directory, remote_path, content, status,
            attempt_count, next_retry_at, last_synced_content, error_message,
            synced_at, created_at, updated_at
        "#,
    )
    .bind(content)
    .bind(note_id)
    .fetch_one(pool)
    .await?)
}

async fn mark_sync_failure(
    pool: &SqlitePool,
    note_id: &str,
    error_message: &str,
) -> Result<ProjectNote, AppError> {
    let current: i64 = sqlx::query_scalar("SELECT attempt_count FROM project_notes WHERE id = ?")
        .bind(note_id)
        .fetch_one(pool)
        .await?;
    let attempt = current.saturating_add(1);
    let (status, retry_modifier) = match attempt {
        1 => ("pending", Some("+2 seconds")),
        2 => ("pending", Some("+10 seconds")),
        3 => ("pending", Some("+30 seconds")),
        _ => ("failed", Some("+60 seconds")),
    };
    Ok(sqlx::query_as::<_, ProjectNote>(
        r#"
        UPDATE project_notes
        SET
            status = ?,
            attempt_count = ?,
            next_retry_at = CASE
                WHEN ? IS NULL THEN NULL
                ELSE strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?)
            END,
            error_message = ?,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ?
        RETURNING
            id, project_id, destination_directory, remote_path, content, status,
            attempt_count, next_retry_at, last_synced_content, error_message,
            synced_at, created_at, updated_at
        "#,
    )
    .bind(status)
    .bind(attempt)
    .bind(retry_modifier)
    .bind(retry_modifier)
    .bind(truncate_chars(error_message, 500))
    .bind(note_id)
    .fetch_one(pool)
    .await?)
}

pub fn run_background(pool: SqlitePool) {
    // Must go through the Tauri async runtime: `run_background` is called from
    // the setup hook, which is not inside a Tokio runtime context (plain
    // tokio::spawn panics with "no reactor running").
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(SYNC_POLL_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            if sync_once(&pool).await.is_err() {
                log_service::record_event(LogRecord {
                    level: LogLevel::Error,
                    module: "notes.sync".to_owned(),
                    message: "note background sync cycle failed".to_owned(),
                    event: Some("cycle_failed".to_owned()),
                    outcome: Some("failed".to_owned()),
                    error_code: Some("note_sync_cycle_error".to_owned()),
                    ..Default::default()
                });
            }
        }
    });
}

pub async fn remote_path(
    pool: &SqlitePool,
    input: GetProjectNoteInput,
) -> Result<PathBuf, AppError> {
    let note = get_or_create(pool, &input.project_id).await?;
    Ok(PathBuf::from(&note.remote_path))
}

fn timestamp() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = seconds / 86_400;
    let remaining = seconds % 86_400;
    let (year, month, day) = civil_from_days(days as i64);
    let (hour, minute, second) = (remaining / 3600, (remaining % 3600) / 60, remaining % 60);
    format!("{year:04}{month:02}{day:02}-{hour:02}{minute:02}{second:02}")
}

/// Howard Hinnant's civil_from_days algorithm (UTC).
fn civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn required<'a>(value: &'a str, label: &str) -> Result<&'a str, AppError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(AppError::Validation(format!("{label} cannot be empty")));
    }
    Ok(value)
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn note_log(
    note: &ProjectNote,
    level: LogLevel,
    event: &str,
    outcome: &str,
    error_code: Option<&str>,
) {
    log_service::record_event(LogRecord {
        level,
        module: "notes.sync".to_owned(),
        message: event.replace('_', " "),
        event: Some(event.to_owned()),
        project_id: Some(note.project_id.clone()),
        note_id: Some(note.id.clone()),
        attempt: u32::try_from(note.attempt_count).ok(),
        outcome: Some(outcome.to_owned()),
        error_code: error_code.map(str::to_owned),
        ..Default::default()
    });
}

fn path_to_string(path: &Path) -> String {
    let value = path.to_string_lossy();
    #[cfg(windows)]
    {
        if let Some(path) = value.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{path}");
        }
        if let Some(path) = value.strip_prefix(r"\\?\") {
            return path.to_owned();
        }
    }
    value.into_owned()
}

async fn find_by_project(
    pool: &SqlitePool,
    project_id: &str,
) -> Result<Option<ProjectNote>, AppError> {
    Ok(sqlx::query_as::<_, ProjectNote>(
        r#"
        SELECT
            id, project_id, destination_directory, remote_path, content, status,
            attempt_count, next_retry_at, last_synced_content, error_message,
            synced_at, created_at, updated_at
        FROM project_notes
        WHERE project_id = ?
        "#,
    )
    .bind(project_id)
    .fetch_optional(pool)
    .await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db,
        models::project::CreateProjectInput,
        services::{project_service, test_support},
    };

    async fn project_with_session(
        pool: &SqlitePool,
    ) -> (tempfile::TempDir, String, PathBuf, PathBuf) {
        let fixture = test_support::project_with_directories(pool, "Note Test")
            .await
            .expect("project fixture");
        let source = fixture.source_directory.clone();
        let destination = fixture.destination_directory.clone();
        let _session = test_support::start_session(pool, &fixture.project_id)
            .await
            .expect("session");
        (fixture._workspace, fixture.project_id, source, destination)
    }

    #[tokio::test]
    async fn creates_note_in_latest_session_destination_and_syncs() {
        let pool = db::test_pool().await;
        let (_workspace, project_id, _source, destination) = project_with_session(&pool).await;

        let created = get_or_create(&pool, &project_id)
            .await
            .expect("create note");
        assert_eq!(created.status, "pending");
        assert_eq!(created.content, "");
        assert_eq!(
            created.remote_path,
            destination.join(NOTE_FILE_NAME).to_string_lossy()
        );

        let updated = update_content(
            &pool,
            UpdateProjectNoteInput {
                project_id: project_id.clone(),
                content: "# 今日感想\n\n- 第一条".to_owned(),
            },
        )
        .await
        .expect("update note");
        assert_eq!(updated.status, "synced");

        let written = tokio::fs::read_to_string(&destination.join(NOTE_FILE_NAME))
            .await
            .expect("read remote");
        assert_eq!(written, "# 今日感想\n\n- 第一条");

        // Idempotent: synced notes are returned as-is.
        let again = get_or_create(&pool, &project_id).await.expect("get again");
        assert_eq!(again.status, "synced");
        assert_eq!(
            again.last_synced_content.as_deref(),
            Some("# 今日感想\n\n- 第一条")
        );
    }

    #[tokio::test]
    async fn rejects_project_without_any_session() {
        let pool = db::test_pool().await;
        let project = project_service::create(
            &pool,
            CreateProjectInput {
                name: "No Session".to_owned(),
                description: None,
                cover_asset_id: None,
            },
        )
        .await
        .expect("project");
        let result = get_or_create(&pool, &project.id).await;
        assert!(matches!(result, Err(AppError::Validation(_))));
    }

    #[tokio::test]
    async fn backs_up_external_edits_before_overwriting() {
        let pool = db::test_pool().await;
        let (_workspace, project_id, _source, destination) = project_with_session(&pool).await;
        update_content(
            &pool,
            UpdateProjectNoteInput {
                project_id: project_id.clone(),
                content: "app version".to_owned(),
            },
        )
        .await
        .expect("first sync");

        // Simulate an external edit (e.g. Obsidian).
        tokio::fs::write(
            destination.join(NOTE_FILE_NAME),
            "external edit from Obsidian",
        )
        .await
        .expect("external edit");

        update_content(
            &pool,
            UpdateProjectNoteInput {
                project_id,
                content: "app version 2".to_owned(),
            },
        )
        .await
        .expect("second sync");

        let written = tokio::fs::read_to_string(destination.join(NOTE_FILE_NAME))
            .await
            .expect("read remote");
        assert_eq!(written, "app version 2");

        let mut entries = tokio::fs::read_dir(&destination)
            .await
            .expect("read destination");
        let mut backups = Vec::new();
        while let Some(entry) = entries.next_entry().await.expect("entry") {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("笔记-") && name.ends_with(".md") {
                backups.push(name);
            }
        }
        assert_eq!(backups.len(), 1, "external edit must be backed up");
        let backup_content = tokio::fs::read_to_string(destination.join(&backups[0]))
            .await
            .expect("read backup");
        assert_eq!(backup_content, "external edit from Obsidian");
    }

    #[tokio::test]
    async fn sync_failure_retries_then_marks_failed() {
        let pool = db::test_pool().await;
        let (_workspace, project_id, _source, destination) = project_with_session(&pool).await;
        let note = get_or_create(&pool, &project_id).await.expect("note");

        // Replace the destination with a file so create_dir_all/rename fails.
        tokio::fs::create_dir_all(&destination)
            .await
            .expect("ensure destination");
        tokio::fs::remove_dir(&destination)
            .await
            .expect("remove dir");
        tokio::fs::write(&destination, b"blocker")
            .await
            .expect("blocker");

        let failed = sync_note(&pool, &note).await.expect("sync attempt");
        assert_eq!(failed.status, "pending");
        assert_eq!(failed.attempt_count, 1);
        assert!(failed.next_retry_at.is_some());
        assert!(failed.error_message.is_some());

        // Three more failures push it to failed with a longer retry window.
        let mut current = failed;
        for _ in 0..3 {
            current = sync_note(&pool, &current).await.expect("sync attempt");
        }
        assert_eq!(current.status, "failed");
        assert_eq!(current.attempt_count, 4);
        assert!(current.next_retry_at.is_some());
    }

    #[tokio::test]
    async fn background_sync_once_processes_due_notes() {
        let pool = db::test_pool().await;
        let (_workspace, project_id, _source, destination) = project_with_session(&pool).await;
        let note = get_or_create(&pool, &project_id).await.expect("note");
        sqlx::query(
            "UPDATE project_notes SET content = 'queued content', status = 'pending' WHERE id = ?",
        )
        .bind(&note.id)
        .execute(&pool)
        .await
        .expect("queue content");

        let handled = sync_once(&pool).await.expect("sync once");
        assert!(handled);
        let written = tokio::fs::read_to_string(destination.join(NOTE_FILE_NAME))
            .await
            .expect("read remote");
        assert_eq!(written, "queued content");
    }
}
