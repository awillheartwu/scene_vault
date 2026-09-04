use std::fs;

use tempfile::tempdir;
use uuid::Uuid;

use super::*;

#[test]
fn scan_counts_files_recursively_and_tolerates_missing_paths() {
    let workspace = tempdir().expect("tempdir");
    let database_path = workspace.path().join("scene-vault.db");
    let database = database_path.clone();
    let logs = workspace.path().join("logs");
    fs::create_dir_all(logs.join("nested")).expect("create logs");
    fs::write(&database, [1_u8; 5]).expect("write database");
    fs::write(format!("{}-wal", database.display()), [1_u8; 3]).expect("write wal");
    fs::write(format!("{}-shm", database.display()), [1_u8; 2]).expect("write shm");
    fs::write(logs.join("current.jsonl"), [2_u8; 7]).expect("write log");
    fs::write(logs.join("nested/archive.jsonl"), [3_u8; 11]).expect("write archive");
    let missing = workspace.path().join("missing");
    let paths = ResourcePaths {
        database_file: database,
        logs,
        thumbnail_cache: missing.clone(),
        capture_output: missing.clone(),
        models: missing.clone(),
        fonts: missing.clone(),
        python_runtime: missing.clone(),
        webview_data: missing,
    };

    let status = scan(&paths);
    let database = status
        .entries
        .iter()
        .find(|entry| entry.kind == "database")
        .unwrap();
    let logs = status
        .entries
        .iter()
        .find(|entry| entry.kind == "logs")
        .unwrap();
    let thumbnails = status
        .entries
        .iter()
        .find(|entry| entry.kind == "thumbnail_cache")
        .unwrap();
    assert_eq!((database.total_bytes, database.file_count), (10, 3));
    assert_eq!((logs.total_bytes, logs.file_count), (18, 2));
    assert_eq!((thumbnails.total_bytes, thumbnails.file_count), (0, 0));
    // Existing paths stay openable; missing paths surface as not configured.
    assert_eq!(
        database.path.as_deref(),
        Some(database_path.to_str().unwrap())
    );
    assert!(logs.path.is_some());
    assert!(thumbnails.path.is_none());
}

#[test]
fn thumbnail_cleanup_removes_contents_and_keeps_root() {
    let workspace = tempdir().expect("tempdir");
    let root = workspace.path().join("thumbnails");
    fs::create_dir_all(root.join("nested")).expect("create cache");
    fs::write(root.join("one.jpg"), [0_u8; 9]).expect("write cache");
    fs::write(root.join("nested/two.jpg"), [0_u8; 4]).expect("write cache");

    let result = cleanup_thumbnail_cache(&root).expect("cleanup thumbnails");
    assert_eq!(result.removed_files, 2);
    assert_eq!(result.reclaimed_bytes, 13);
    assert!(root.is_dir());
    assert_eq!(fs::read_dir(&root).expect("read root").count(), 0);
}

#[cfg(unix)]
#[test]
fn scan_does_not_follow_symlinks() {
    use std::os::unix::fs::symlink;
    let workspace = tempdir().expect("tempdir");
    let root = workspace.path().join("root");
    let outside = workspace.path().join("outside");
    fs::create_dir_all(&root).expect("create root");
    fs::create_dir_all(&outside).expect("create outside");
    fs::write(outside.join("large.bin"), [0_u8; 32]).expect("write outside");
    symlink(&outside, root.join("link")).expect("symlink");
    assert_eq!(disk_usage(&root), DiskUsage::default());
}

#[cfg(unix)]
#[test]
fn thumbnail_cleanup_rejects_a_symlinked_root() {
    use std::os::unix::fs::symlink;
    let workspace = tempdir().expect("tempdir");
    let outside = workspace.path().join("outside");
    let root = workspace.path().join("thumbnails");
    fs::create_dir_all(&outside).expect("create outside");
    fs::write(outside.join("keep.jpg"), [0_u8; 4]).expect("write outside");
    symlink(&outside, &root).expect("symlink");

    let error = cleanup_thumbnail_cache(&root).expect_err("symlink root must be rejected");
    assert!(error.to_string().contains("symlinked cache root"));
    assert!(outside.join("keep.jpg").is_file());
}

#[tokio::test]
async fn capture_cleanup_only_removes_archived_completed_uuid_directories() {
    let pool = crate::db::test_pool().await;
    let workspace = tempdir().expect("tempdir");
    let root = workspace.path().join("capture-output");
    fs::create_dir_all(&root).expect("create output root");
    let project_id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO projects (id, name) VALUES (?, 'Resources')")
        .bind(&project_id)
        .execute(&pool)
        .await
        .expect("project");
    let session_id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO capture_sessions (id, project_id, status) VALUES (?, ?, 'completed')")
        .bind(&session_id)
        .bind(&project_id)
        .execute(&pool)
        .await
        .expect("capture session");

    let safe_id = Uuid::new_v4().to_string();
    let pending_id = Uuid::new_v4().to_string();
    let invalid_id = "../outside";
    let archive = workspace.path().join("archive");
    fs::create_dir_all(&archive).expect("create archive");
    let archived_safe = archive.join("safe.png");
    fs::write(&archived_safe, [1_u8; 10]).expect("write archive");
    let local_safe = root.join(&safe_id).join("image.png");
    for (id, status, annotated, archived, destination) in [
        (
            safe_id.as_str(),
            "completed",
            Some(local_safe.to_str().expect("local path")),
            Some("2026-01-01T00:00:00Z"),
            Some(archived_safe.to_str().expect("archive path")),
        ),
        (pending_id.as_str(), "processing", None, None, None),
        (
            invalid_id,
            "completed",
            None,
            Some("2026-01-01T00:00:00Z"),
            Some("archive.png"),
        ),
    ] {
        sqlx::query(
            "INSERT INTO capture_items (id, session_id, project_id, source_path, classification, status, annotated_path, archived_at, destination_path) VALUES (?, ?, ?, ?, 'scene', ?, ?, ?, ?)",
        )
        .bind(id).bind(&session_id).bind(&project_id).bind(format!("{id}.png")).bind(status).bind(annotated).bind(archived).bind(destination)
        .execute(&pool).await.expect("capture item");
    }
    for id in [&safe_id, &pending_id] {
        fs::create_dir_all(root.join(id)).expect("create item output");
        fs::write(root.join(id).join("image.png"), [0_u8; 10]).expect("write output");
    }

    let result = cleanup_capture_output(&pool, &root)
        .await
        .expect("cleanup output");
    assert_eq!(result.removed_files, 1);
    assert!(!root.join(&safe_id).exists());
    assert!(root.join(&pending_id).exists());
    let stored_annotated: Option<String> =
        sqlx::query_scalar("SELECT annotated_path FROM capture_items WHERE id = ?")
            .bind(&safe_id)
            .fetch_one(&pool)
            .await
            .expect("stored annotated path");
    assert_eq!(stored_annotated.as_deref(), archived_safe.to_str());
}
