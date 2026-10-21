use tempfile::tempdir;
use uuid::Uuid;

use super::*;

fn write_image(root: &Path, relative: &str, size: usize) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).expect("create image directory");
    // Usage reads metadata, so these fixtures do not need image decoding.
    fs::write(path, vec![1_u8; size]).expect("write image");
}

async fn add_project(pool: &SqlitePool, directory: Option<&Path>) -> String {
    let id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO projects (id, name, destination_directory) VALUES (?, ?, ?)")
        .bind(&id)
        .bind(&id)
        .bind(directory.map(capture_service::path_to_string))
        .execute(pool)
        .await
        .expect("insert project");
    id
}

fn progress() -> ScanProgress {
    ScanProgress {
        started: Instant::now(),
        last_progress_ms: AtomicU64::new(0),
        opened: AtomicBool::new(false),
        stopped: AtomicBool::new(false),
        request_cancelled: Arc::new(AtomicBool::new(false)),
    }
}

fn assert_no_counts(usage: &ProjectArchiveUsage) {
    assert_eq!(usage.main_image_count, None);
    assert_eq!(usage.avatar_image_count, None);
    assert_eq!(usage.total_bytes, None);
    assert!(usage.message.is_some());
}

#[tokio::test]
async fn lists_current_archive_images_including_avatars_private_and_retained_files() {
    let workspace = tempdir().expect("tempdir");
    let root = workspace.path().join("archive");
    for (path, size) in [
        ("人物图/main-avatar.png", 11),
        ("人物图（原图）/original.jpg", 13),
        ("人物图（未识别）/unrecognized.jpeg", 17),
        ("游戏截图/scene.webp", 19),
        ("收藏图/private.bmp", 23),
        ("old/retained.png", 29),
        ("头像图/character/portrait.png", 7),
        ("old/legacy-AVATAR.PNG", 5),
        ("old/legacy-avatar - repaired-abcdef.png", 3),
    ] {
        write_image(&root, path, size);
    }
    write_image(workspace.path(), "outside.png", 4096);
    let empty = workspace.path().join("empty");
    fs::create_dir(&empty).expect("create empty archive");
    let pool = crate::db::test_pool().await;
    // No capture records exist: old and externally added files still count.
    let project = add_project(&pool, Some(&root)).await;
    let shared = add_project(&pool, Some(&root)).await;
    let unconfigured = add_project(&pool, None).await;
    let missing = add_project(&pool, Some(&workspace.path().join("missing"))).await;
    let empty_project = add_project(&pool, Some(&empty)).await;
    let mut reported = Vec::new();
    let usages = list(&pool, &Uuid::new_v4().to_string(), "scan", |usage| {
        reported.push(usage);
    })
    .await
    .expect("list usage");

    assert_eq!(usages.len(), 5);
    assert_eq!(reported.len(), 5);
    for id in [&project, &shared] {
        let usage = usages.iter().find(|usage| &usage.project_id == id).unwrap();
        assert!(matches!(usage.state, State::Ready));
        assert_eq!(usage.main_image_count, Some(6));
        assert_eq!(usage.avatar_image_count, Some(3));
        assert_eq!(usage.total_bytes, Some(127));
        assert!(chrono::DateTime::parse_from_rfc3339(&usage.scanned_at).is_ok());
    }
    let usage = usages
        .iter()
        .find(|usage| usage.project_id == unconfigured)
        .unwrap();
    assert!(matches!(usage.state, State::Unconfigured));
    assert_no_counts(usage);
    let usage = usages
        .iter()
        .find(|usage| usage.project_id == missing)
        .unwrap();
    assert!(matches!(usage.state, State::Missing));
    assert_no_counts(usage);
    let usage = usages
        .iter()
        .find(|usage| usage.project_id == empty_project)
        .unwrap();
    assert!(matches!(usage.state, State::Ready));
    assert_eq!(usage.main_image_count, Some(0));
    assert_eq!(usage.avatar_image_count, Some(0));
    assert_eq!(usage.total_bytes, Some(0));
}

#[test]
fn excludes_temporary_system_and_linked_images() {
    let workspace = tempdir().expect("tempdir");
    let root = workspace.path().join("archive");
    write_image(&root, "游戏截图/keep.png", 17);
    for path in [
        "游戏截图/.scenevault-resize-abcdef.png",
        "游戏截图/.scene-vault-abcdef.png",
        "游戏截图/copy.png.partial",
        "游戏截图/empty.png",
        "@eaDir/thumbnail.png",
        "#recycle/deleted.jpg",
        "$RECYCLE.BIN/deleted.png",
        "System Volume Information/system.png",
        ".cache/thumbnail.png",
        "project.json",
        "笔记.md",
    ] {
        write_image(
            &root,
            path,
            if path.ends_with("empty.png") { 0 } else { 99 },
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let outside = workspace.path().join("outside");
        write_image(&outside, "external.png", 1000);
        symlink(&outside, root.join("linked-directory")).expect("link directory");
        symlink(outside.join("external.png"), root.join("linked.png")).expect("link file");
        symlink(root.join("missing.png"), root.join("broken.png")).expect("broken link");
        let linked_root = workspace.path().join("linked-root");
        symlink(&root, &linked_root).expect("link root");
        let usage = scan_directory(&linked_root, &progress()).for_project("p".into(), &timestamp());
        assert!(matches!(usage.state, State::Error));
        assert_no_counts(&usage);
    }
    let usage = scan_directory(&root, &progress()).for_project("p".into(), &timestamp());
    assert!(matches!(usage.state, State::Ready));
    assert_eq!(usage.main_image_count, Some(1));
    assert_eq!(usage.avatar_image_count, Some(0));
    assert_eq!(usage.total_bytes, Some(17));
}

#[tokio::test]
async fn aliases_of_one_root_share_a_snapshot_only_within_the_request() {
    let workspace = tempdir().expect("tempdir");
    let root = workspace.path();
    write_image(root, "游戏截图/current.png", 11);
    let scans = RootScans::default();
    let cancelled = Arc::new(AtomicBool::new(false));
    let first = scan(root.to_str().unwrap(), cancelled.clone(), scans.clone()).await;
    assert!(matches!(first.state, State::Ready));
    assert_eq!(first.bytes, 11);

    write_image(root, "游戏截图/current.png", 23);
    write_image(root, "收藏图/new.png", 7);
    let alias = root.join(".");
    let shared = scan(alias.to_str().unwrap(), cancelled.clone(), scans).await;
    assert!(matches!(shared.state, State::Ready));
    assert_eq!((shared.main_images, shared.bytes), (1, 11));

    let refreshed = scan(root.to_str().unwrap(), cancelled, RootScans::default()).await;
    assert!(matches!(refreshed.state, State::Ready));
    assert_eq!((refreshed.main_images, refreshed.bytes), (2, 30));
}

#[cfg(windows)]
#[test]
fn windows_directory_junctions_are_not_followed_as_children_or_roots() {
    let workspace = tempdir().expect("tempdir");
    let root = workspace.path().join("archive");
    let outside = workspace.path().join("outside");
    write_image(&root, "游戏截图/keep.png", 17);
    write_image(&outside, "external.png", 1000);
    let child_link = root.join("linked-directory");
    let root_link = workspace.path().join("linked-root");
    for link in [&child_link, &root_link] {
        let output = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(link)
            .arg(&outside)
            .output()
            .expect("create temporary junction");
        assert!(
            output.status.success(),
            "temporary junction creation failed"
        );
    }
    let usage = scan_directory(&root, &progress()).for_project("p".into(), &timestamp());
    assert!(matches!(usage.state, State::Ready));
    assert_eq!(usage.main_image_count, Some(1));
    assert_eq!(usage.total_bytes, Some(17));
    let linked_usage =
        scan_directory(&root_link, &progress()).for_project("p".into(), &timestamp());
    assert!(matches!(linked_usage.state, State::Error));
    assert_no_counts(&linked_usage);
    for link in [&child_link, &root_link] {
        fs::remove_dir(link).expect("remove temporary junction without following it");
    }
    assert!(outside.join("external.png").is_file());
}

#[tokio::test]
async fn cancellation_stops_reports_for_shared_roots_and_unconfigured_projects() {
    let workspace = tempdir().expect("tempdir");
    write_image(workspace.path(), "游戏截图/current.png", 11);
    for directory in [Some(workspace.path()), None] {
        let pool = crate::db::test_pool().await;
        add_project(&pool, directory).await;
        add_project(&pool, directory).await;
        let window = Uuid::new_v4().to_string();
        let mut reported = Vec::new();
        let usages = list(&pool, &window, "scan", |usage| {
            reported.push(usage);
            cancel(&window, "scan").expect("cancel during report");
        })
        .await
        .expect("list usage");
        assert_eq!(reported.len(), 1);
        assert_eq!(usages.len(), 1);
    }
}

#[tokio::test]
async fn early_and_stale_cancellation_do_not_publish_or_cancel_a_new_request() {
    let workspace = tempdir().expect("tempdir");
    write_image(workspace.path(), "游戏截图/current.png", 11);
    let pool = crate::db::test_pool().await;
    add_project(&pool, Some(workspace.path())).await;
    add_project(&pool, Some(workspace.path())).await;
    let window = Uuid::new_v4().to_string();
    cancel(&window, "old").expect("cancel before start");
    let usages = list(&pool, &window, "old", |_| panic!("cancelled report"))
        .await
        .expect("cancelled list");
    assert!(usages.is_empty());

    let usages = list(&pool, &window, "new", |_| {
        cancel(&window, "old").expect("stale cancellation");
    })
    .await
    .expect("new list");
    assert_eq!(usages.len(), 2);
    for usage in usages {
        assert!(matches!(usage.state, State::Ready));
        assert_eq!(usage.total_bytes, Some(11));
    }
}

#[tokio::test]
async fn cancellation_at_io_completion_cannot_publish_a_ready_snapshot() {
    let workspace = tempdir().expect("tempdir");
    write_image(workspace.path(), "游戏截图/current.png", 11);
    let root = workspace.path().to_path_buf();
    let cancelled = Arc::new(AtomicBool::new(false));
    let result = limited_io(cancelled.clone(), move |progress| {
        let snapshot = scan_directory(&root, progress);
        cancelled.store(true, Ordering::Relaxed);
        Ok(snapshot)
    })
    .await;
    let snapshot = match result {
        Err(snapshot) => snapshot,
        Ok(_) => panic!("cancelled I/O returned a complete snapshot"),
    };
    assert!(matches!(snapshot.state, State::Partial));
    assert_no_counts(&snapshot.for_project("p".into(), &timestamp()));
}

#[test]
fn incomplete_snapshots_hide_both_zero_and_accumulated_counts() {
    for (main_images, avatars, bytes) in [(0, 0, 0), (2, 1, 37)] {
        for state in [
            State::Partial,
            State::Unavailable,
            State::Missing,
            State::Error,
        ] {
            let snapshot = Snapshot {
                state,
                main_images,
                avatars,
                bytes,
                message: Some("incomplete directory read".into()),
            };
            assert_no_counts(&snapshot.for_project("p".into(), &timestamp()));
        }
    }
}

#[cfg(unix)]
#[test]
fn unreadable_subdirectory_does_not_report_partial_totals_as_complete() {
    use std::os::unix::fs::PermissionsExt;

    let workspace = tempdir().expect("tempdir");
    write_image(workspace.path(), "游戏截图/visible.png", 17);
    write_image(workspace.path(), "收藏图/private.png", 23);
    let blocked = workspace.path().join("收藏图");
    let permissions = fs::metadata(&blocked)
        .expect("directory metadata")
        .permissions();
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0)).expect("block directory");
    let unreadable = fs::read_dir(&blocked).is_err();
    let snapshot = scan_directory(workspace.path(), &progress());
    fs::set_permissions(&blocked, permissions).expect("restore permissions");
    // Elevated users may bypass mode bits; the portable snapshot test above
    // still covers the IPC contract in that environment.
    if !unreadable {
        return;
    }
    assert!(matches!(snapshot.state, State::Partial));
    assert_no_counts(&snapshot.for_project("p".into(), &timestamp()));
}
