use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};

use chrono::{SecondsFormat, Utc};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
    SqlitePool,
};
use tempfile::{tempdir, TempDir};

use super::*;
use crate::models::data_maintenance::{
    BackupManifest, BACKUP_CONTENT_SCOPE, BACKUP_ENGINE, PENDING_RESTORE_FILE,
    PENDING_RESTORE_MANIFEST_FILE, PRE_RESTORE_FILE, RESTORE_REQUEST_FILE,
};

const APP_VERSION: &str = "0.1.0";
const SCHEMA_VERSION: i64 = 19;

async fn open_pool(path: &Path) -> SqlitePool {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(5));
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("open sqlite database")
}

async fn create_database(workspace: &TempDir) -> PathBuf {
    let db_path = workspace.path().join("scene-vault.db");
    let pool = open_pool(&db_path).await;
    seed_database(&pool).await;
    pool.close().await;
    db_path
}

async fn seed_database(pool: &SqlitePool) {
    sqlx::query("CREATE TABLE projects (id TEXT PRIMARY KEY, name TEXT NOT NULL)")
        .execute(pool)
        .await
        .expect("create projects");
    sqlx::query("INSERT INTO projects (id, name) VALUES ('p1', 'Alpha'), ('p2', 'Beta')")
        .execute(pool)
        .await
        .expect("insert projects");
    sqlx::query("CREATE INDEX idx_projects_name ON projects(name)")
        .execute(pool)
        .await
        .expect("create index");
    sqlx::query(
        "CREATE TABLE _sqlx_migrations (
            version BIGINT PRIMARY KEY,
            description TEXT NOT NULL,
            installed_on TEXT NOT NULL,
            success BOOLEAN NOT NULL,
            checksum BLOB NOT NULL,
            execution_time BIGINT NOT NULL
        )",
    )
    .execute(pool)
    .await
    .expect("create migrations");
    sqlx::query(
        "INSERT INTO _sqlx_migrations
            (version, description, installed_on, success, checksum, execution_time)
         VALUES (19, 'fixture', '2026-08-11T00:00:00Z', 1, X'00', 0)",
    )
    .execute(pool)
    .await
    .expect("insert migration row");
}

async fn create_database_without_migrations(workspace: &TempDir) -> PathBuf {
    let db_path = workspace.path().join("scene-vault.db");
    let pool = open_pool(&db_path).await;
    sqlx::query("CREATE TABLE projects (id TEXT PRIMARY KEY, name TEXT NOT NULL)")
        .execute(&pool)
        .await
        .expect("create projects");
    pool.close().await;
    db_path
}

fn service(db_path: &Path) -> DataMaintenanceService {
    DataMaintenanceService::new(db_path.to_path_buf(), APP_VERSION, SCHEMA_VERSION)
}

async fn project_count(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM projects")
        .fetch_one(pool)
        .await
        .expect("count projects")
}

async fn find_backup_file(directory: &Path) -> PathBuf {
    let mut entries = tokio::fs::read_dir(directory)
        .await
        .expect("read backup directory");
    while let Some(entry) = entries.next_entry().await.expect("backup entry") {
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "sqlite")
        {
            return path;
        }
    }
    panic!("no backup file found in {}", directory.display());
}

#[tokio::test]
async fn preflight_reports_healthy_database() {
    let workspace = tempdir().expect("tempdir");
    let db_path = create_database(&workspace).await;

    let report = service(&db_path).preflight().await.expect("preflight");
    assert!(report.ok);
    assert_eq!(report.quick_check, vec!["ok".to_owned()]);
    assert!(report.foreign_key_issues.is_empty());
    assert!(report.migration_issues.is_empty());
    assert_eq!(report.schema_version, Some(SCHEMA_VERSION));
    assert!(!report.sqlite_version.is_empty());
    assert!(!report.journal_mode.is_empty());
    assert!(report.database_size_bytes > 0);
    assert_eq!(report.database_path, db_path.display().to_string());
}

#[tokio::test]
async fn preflight_reports_foreign_key_violations() {
    let workspace = tempdir().expect("tempdir");
    let db_path = create_database(&workspace).await;
    let pool = open_pool(&db_path).await;
    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&pool)
        .await
        .expect("disable foreign keys for fixture");
    sqlx::query(
        "CREATE TABLE captures (
            id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL REFERENCES projects(id)
        )",
    )
    .execute(&pool)
    .await
    .expect("create captures");
    sqlx::query("INSERT INTO captures (id, project_id) VALUES ('c1', 'missing-project')")
        .execute(&pool)
        .await
        .expect("insert capture");
    pool.close().await;

    let report = service(&db_path).preflight().await.expect("preflight");
    assert!(!report.ok);
    assert!(!report.foreign_key_issues.is_empty());
    assert!(report.foreign_key_issues[0].contains("captures"));
}

#[tokio::test]
async fn preflight_reports_dirty_migrations() {
    let workspace = tempdir().expect("tempdir");
    let db_path = create_database(&workspace).await;
    let pool = open_pool(&db_path).await;
    sqlx::query("UPDATE _sqlx_migrations SET success = 0 WHERE version = 19")
        .execute(&pool)
        .await
        .expect("mark migration dirty");
    pool.close().await;

    let report = service(&db_path).preflight().await.expect("preflight");
    assert!(!report.ok);
    assert!(report
        .migration_issues
        .iter()
        .any(|issue| issue.contains("dirty")));
}

#[tokio::test]
async fn preflight_tolerates_missing_migrations_table() {
    let workspace = tempdir().expect("tempdir");
    let db_path = create_database_without_migrations(&workspace).await;

    let report = service(&db_path).preflight().await.expect("preflight");
    assert!(report.ok);
    assert_eq!(report.schema_version, None);
    assert!(report.migration_issues.is_empty());
}

#[tokio::test]
async fn preflight_rejects_missing_database() {
    let workspace = tempdir().expect("tempdir");
    let db_path = workspace.path().join("missing.db");

    let error = service(&db_path)
        .preflight()
        .await
        .expect_err("missing database must fail");
    assert!(error.to_string().contains("does not exist"));
}

#[test]
fn sqlite_version_gating_accepts_3_27_and_rejects_older() {
    assert!(sqlite_version_at_least(
        "3.46.0",
        SQLITE_VACUUM_INTO_MIN_VERSION
    ));
    assert!(sqlite_version_at_least(
        "3.27.0",
        SQLITE_VACUUM_INTO_MIN_VERSION
    ));
    assert!(sqlite_version_at_least(
        "3.27.2",
        SQLITE_VACUUM_INTO_MIN_VERSION
    ));
    assert!(!sqlite_version_at_least(
        "3.26.2",
        SQLITE_VACUUM_INTO_MIN_VERSION
    ));
}

#[tokio::test]
async fn create_backup_writes_consistent_snapshot_and_manifest() {
    let workspace = tempdir().expect("tempdir");
    let db_path = create_database(&workspace).await;
    let backups = workspace.path().join("backups");
    tokio::fs::create_dir_all(&backups)
        .await
        .expect("backup directory");

    let manifest = service(&db_path)
        .create_backup(&backups)
        .await
        .expect("backup");
    assert_eq!(manifest.engine, BACKUP_ENGINE);
    assert_eq!(manifest.content_scope, BACKUP_CONTENT_SCOPE);
    assert!(manifest.excludes_source_images);
    assert_eq!(manifest.schema_version, Some(SCHEMA_VERSION));
    assert_eq!(manifest.table_counts.get("projects"), Some(&2));
    assert_eq!(manifest.table_counts.get("_sqlx_migrations"), Some(&1));
    assert!(manifest.file_size > 0);
    assert_eq!(manifest.app_version, APP_VERSION);

    let backup_file = find_backup_file(&backups).await;
    assert_eq!(
        manifest.backup_file,
        backup_file.file_name().unwrap().to_string_lossy().as_ref()
    );

    let stored: BackupManifest = read_json(&manifest_path_for(&backup_file))
        .await
        .expect("read manifest");
    assert_eq!(stored, manifest);

    let options = SqliteConnectOptions::new()
        .filename(&backup_file)
        .read_only(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("open backup");
    assert_eq!(project_count(&pool).await, 2);
    let migrations: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .expect("migration count");
    assert_eq!(migrations, 1);
    pool.close().await;
}

#[tokio::test]
async fn create_backup_rejects_live_database_paths() {
    let workspace = tempdir().expect("tempdir");
    let db_path = create_database(&workspace).await;
    let service = service(&db_path);

    let error = service
        .create_backup(&db_path)
        .await
        .expect_err("live database must be rejected");
    assert!(matches!(
        error,
        DataMaintenanceError::LivePathDestination(_)
    ));

    let wal_path = PathBuf::from(format!("{}-wal", db_path.display()));
    let error = service
        .create_backup(&wal_path)
        .await
        .expect_err("live WAL must be rejected");
    assert!(matches!(
        error,
        DataMaintenanceError::LivePathDestination(_)
    ));

    let existing_dir = workspace.path().join("existing-dir");
    tokio::fs::create_dir_all(&existing_dir)
        .await
        .expect("existing directory");
    let indirect = existing_dir.join("../scene-vault.db");
    let error = service
        .create_backup(&indirect)
        .await
        .expect_err("indirect live path must be rejected");
    assert!(matches!(
        error,
        DataMaintenanceError::LivePathDestination(_)
    ));
}

#[tokio::test]
async fn create_backup_rejects_existing_destination_file() {
    let workspace = tempdir().expect("tempdir");
    let db_path = create_database(&workspace).await;
    let destination = workspace.path().join("backups").join("taken.sqlite");
    tokio::fs::create_dir_all(destination.parent().unwrap())
        .await
        .expect("backup directory");
    tokio::fs::write(&destination, b"occupied")
        .await
        .expect("write occupied file");

    let error = service(&db_path)
        .create_backup(&destination)
        .await
        .expect_err("existing destination must be rejected");
    assert!(error.to_string().contains("already exists"));
}

#[tokio::test]
async fn validate_backup_rejects_tampered_manifest_checksum() {
    let workspace = tempdir().expect("tempdir");
    let db_path = create_database(&workspace).await;
    let backups = workspace.path().join("backups");
    tokio::fs::create_dir_all(&backups)
        .await
        .expect("backup directory");
    let manifest = service(&db_path)
        .create_backup(&backups)
        .await
        .expect("backup");
    let backup_file = find_backup_file(&backups).await;

    let manifest_path = manifest_path_for(&backup_file);
    let mut tampered = manifest.clone();
    tampered.sha256 = format!("{}0", &tampered.sha256[..63]);
    write_json_atomic(&manifest_path, &tampered)
        .await
        .expect("rewrite manifest");

    let error = service(&db_path)
        .validate_backup(&backup_file)
        .await
        .expect_err("tampered manifest must be rejected");
    assert!(matches!(
        error,
        DataMaintenanceError::ChecksumMismatch { .. }
    ));
}

#[tokio::test]
async fn validate_backup_rejects_tampered_backup_file() {
    let workspace = tempdir().expect("tempdir");
    let db_path = create_database(&workspace).await;
    let backups = workspace.path().join("backups");
    tokio::fs::create_dir_all(&backups)
        .await
        .expect("backup directory");
    service(&db_path)
        .create_backup(&backups)
        .await
        .expect("backup");
    let backup_file = find_backup_file(&backups).await;

    let bytes = tokio::fs::read(&backup_file).await.expect("read backup");
    let mut modified = bytes.clone();
    let last = modified.len() - 1;
    modified[last] ^= 0xFF;
    tokio::fs::write(&backup_file, modified)
        .await
        .expect("write modified backup");

    let error = service(&db_path)
        .validate_backup(&backup_file)
        .await
        .expect_err("tampered backup must be rejected");
    assert!(matches!(
        error,
        DataMaintenanceError::ChecksumMismatch { .. }
    ));
}

#[tokio::test]
async fn validate_backup_rejects_non_sqlite_file() {
    let workspace = tempdir().expect("tempdir");
    let db_path = create_database(&workspace).await;
    let fake = workspace.path().join("fake.sqlite");
    let fake_bytes = b"definitely not a sqlite database".to_vec();
    tokio::fs::write(&fake, &fake_bytes)
        .await
        .expect("write fake backup");
    let manifest = BackupManifest {
        engine: BACKUP_ENGINE.to_owned(),
        content_scope: BACKUP_CONTENT_SCOPE.to_owned(),
        excludes_source_images: true,
        backup_file: "fake.sqlite".to_owned(),
        app_version: APP_VERSION.to_owned(),
        schema_version: None,
        sqlite_version: "3.46.0".to_owned(),
        created_at_utc: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        sha256: sha256_file(&fake).await.expect("fake checksum"),
        file_size: fake_bytes.len() as u64,
        table_counts: BTreeMap::new(),
    };
    write_json_atomic(&manifest_path_for(&fake), &manifest)
        .await
        .expect("write fake manifest");

    let error = service(&db_path)
        .validate_backup(&fake)
        .await
        .expect_err("non-sqlite backup must be rejected");
    assert!(error.to_string().contains("sqlite"));
}

#[tokio::test]
async fn validate_backup_rejects_newer_schema_than_supported() {
    let workspace = tempdir().expect("tempdir");
    let db_path = create_database(&workspace).await;
    let backups = workspace.path().join("backups");
    tokio::fs::create_dir_all(&backups)
        .await
        .expect("backup directory");
    service(&db_path)
        .create_backup(&backups)
        .await
        .expect("backup");
    let backup_file = find_backup_file(&backups).await;

    let older_app = DataMaintenanceService::new(db_path.clone(), APP_VERSION, SCHEMA_VERSION - 1);
    let error = older_app
        .validate_backup(&backup_file)
        .await
        .expect_err("newer schema must be rejected");
    assert!(error.to_string().contains("schema"));
}

#[tokio::test]
async fn validate_backup_rejects_missing_manifest() {
    let workspace = tempdir().expect("tempdir");
    let db_path = create_database(&workspace).await;
    let backups = workspace.path().join("backups");
    tokio::fs::create_dir_all(&backups)
        .await
        .expect("backup directory");
    service(&db_path)
        .create_backup(&backups)
        .await
        .expect("backup");
    let backup_file = find_backup_file(&backups).await;
    tokio::fs::remove_file(manifest_path_for(&backup_file))
        .await
        .expect("remove manifest");

    let error = service(&db_path)
        .validate_backup(&backup_file)
        .await
        .expect_err("missing manifest must be rejected");
    assert!(error.to_string().contains("manifest"));
}

#[tokio::test]
async fn stage_restore_does_not_touch_live_database() {
    let workspace = tempdir().expect("tempdir");
    let db_path = create_database(&workspace).await;
    let service = service(&db_path);
    let backups = workspace.path().join("backups");
    tokio::fs::create_dir_all(&backups)
        .await
        .expect("backup directory");
    let manifest = service.create_backup(&backups).await.expect("backup");
    let backup_file = find_backup_file(&backups).await;
    let live_before = tokio::fs::read(&db_path).await.expect("read live database");

    let recovery = workspace.path().join("recovery");
    let request = service
        .stage_restore(&backup_file, &recovery)
        .await
        .expect("stage restore");

    let live_after = tokio::fs::read(&db_path)
        .await
        .expect("read live database again");
    assert_eq!(live_before, live_after);
    assert!(recovery.join(PENDING_RESTORE_FILE).is_file());
    assert!(recovery.join(PENDING_RESTORE_MANIFEST_FILE).is_file());
    assert!(recovery.join(RESTORE_REQUEST_FILE).is_file());
    assert_eq!(request.sha256, manifest.sha256);
    assert_eq!(request.schema_version, manifest.schema_version);
    assert!(service.has_pending_restore(&recovery).await);
    let pending = service.pending_restore(&recovery).await.expect("pending");
    assert!(pending.is_some());
}

#[tokio::test]
async fn stage_restore_rejects_duplicate_staging() {
    let workspace = tempdir().expect("tempdir");
    let db_path = create_database(&workspace).await;
    let service = service(&db_path);
    let backups = workspace.path().join("backups");
    tokio::fs::create_dir_all(&backups)
        .await
        .expect("backup directory");
    service.create_backup(&backups).await.expect("backup");
    let backup_file = find_backup_file(&backups).await;
    let recovery = workspace.path().join("recovery");

    service
        .stage_restore(&backup_file, &recovery)
        .await
        .expect("first stage");
    let error = service
        .stage_restore(&backup_file, &recovery)
        .await
        .expect_err("duplicate stage must be rejected");
    assert!(matches!(
        error,
        DataMaintenanceError::RestoreAlreadyStaged(_)
    ));
}

#[tokio::test]
async fn apply_pending_restore_swaps_database_and_keeps_rollback_snapshot() {
    let workspace = tempdir().expect("tempdir");
    let db_path = create_database(&workspace).await;
    let service = service(&db_path);
    let backups = workspace.path().join("backups");
    tokio::fs::create_dir_all(&backups)
        .await
        .expect("backup directory");
    service.create_backup(&backups).await.expect("backup");
    let backup_file = find_backup_file(&backups).await;

    let pool = open_pool(&db_path).await;
    sqlx::query("DELETE FROM projects WHERE id = 'p2'")
        .execute(&pool)
        .await
        .expect("delete project");
    sqlx::query("INSERT INTO projects (id, name) VALUES ('p3', 'Gamma'), ('p4', 'Delta')")
        .execute(&pool)
        .await
        .expect("insert projects");
    pool.close().await;
    let live_pool = open_pool(&db_path).await;
    assert_eq!(project_count(&live_pool).await, 3);
    live_pool.close().await;

    let recovery = workspace.path().join("recovery");
    service
        .stage_restore(&backup_file, &recovery)
        .await
        .expect("stage restore");
    let outcome = service
        .apply_pending_restore(&recovery)
        .await
        .expect("apply restore");

    assert!(outcome.validated_after_swap);
    assert_eq!(outcome.schema_version, Some(SCHEMA_VERSION));
    assert!(recovery.join(PRE_RESTORE_FILE).is_file());
    assert!(!recovery.join(RESTORE_REQUEST_FILE).exists());
    assert!(!recovery.join(PENDING_RESTORE_FILE).exists());

    let restored_pool = open_pool(&db_path).await;
    assert_eq!(project_count(&restored_pool).await, 2);
    restored_pool.close().await;

    let snapshot_pool = open_pool(&recovery.join(PRE_RESTORE_FILE)).await;
    assert_eq!(project_count(&snapshot_pool).await, 3);
    snapshot_pool.close().await;
}

#[tokio::test]
async fn apply_pending_restore_without_stage_returns_not_pending() {
    let workspace = tempdir().expect("tempdir");
    let db_path = create_database(&workspace).await;
    let recovery = workspace.path().join("recovery");

    let error = service(&db_path)
        .apply_pending_restore(&recovery)
        .await
        .expect_err("no pending restore must fail");
    assert!(matches!(error, DataMaintenanceError::RestoreNotPending(_)));
}

#[tokio::test]
async fn apply_pending_restore_rejects_corrupted_staged_file() {
    let workspace = tempdir().expect("tempdir");
    let db_path = create_database(&workspace).await;
    let service = service(&db_path);
    let backups = workspace.path().join("backups");
    tokio::fs::create_dir_all(&backups)
        .await
        .expect("backup directory");
    service.create_backup(&backups).await.expect("backup");
    let backup_file = find_backup_file(&backups).await;
    let live_before = tokio::fs::read(&db_path).await.expect("read live database");

    let recovery = workspace.path().join("recovery");
    service
        .stage_restore(&backup_file, &recovery)
        .await
        .expect("stage restore");
    let pending = recovery.join(PENDING_RESTORE_FILE);
    let bytes = tokio::fs::read(&pending).await.expect("read pending");
    let mut modified = bytes.clone();
    modified[0] ^= 0xFF;
    tokio::fs::write(&pending, modified)
        .await
        .expect("corrupt pending restore");

    let error = service
        .apply_pending_restore(&recovery)
        .await
        .expect_err("corrupted pending restore must be rejected");
    assert!(matches!(
        error,
        DataMaintenanceError::ChecksumMismatch { .. }
    ));
    assert_eq!(
        tokio::fs::read(&db_path).await.expect("read live database"),
        live_before
    );
    assert!(recovery.join(RESTORE_REQUEST_FILE).is_file());
}

#[tokio::test]
async fn clear_pending_restore_removes_staging_artifacts() {
    let workspace = tempdir().expect("tempdir");
    let db_path = create_database(&workspace).await;
    let service = service(&db_path);
    let backups = workspace.path().join("backups");
    tokio::fs::create_dir_all(&backups)
        .await
        .expect("backup directory");
    service.create_backup(&backups).await.expect("backup");
    let backup_file = find_backup_file(&backups).await;
    let live_before = tokio::fs::read(&db_path).await.expect("read live database");

    let recovery = workspace.path().join("recovery");
    service
        .stage_restore(&backup_file, &recovery)
        .await
        .expect("stage restore");
    service
        .clear_pending_restore(&recovery)
        .await
        .expect("clear pending restore");

    assert!(!service.has_pending_restore(&recovery).await);
    assert!(!recovery.join(PENDING_RESTORE_FILE).exists());
    assert!(!recovery.join(PENDING_RESTORE_MANIFEST_FILE).exists());
    assert!(!recovery.join(RESTORE_REQUEST_FILE).exists());
    assert_eq!(
        tokio::fs::read(&db_path).await.expect("read live database"),
        live_before
    );
}

#[tokio::test]
async fn rollback_restore_recovers_pre_restore_snapshot() {
    let workspace = tempdir().expect("tempdir");
    let db_path = create_database(&workspace).await;
    let service = service(&db_path);
    let backups = workspace.path().join("backups");
    tokio::fs::create_dir_all(&backups)
        .await
        .expect("backup directory");
    service.create_backup(&backups).await.expect("backup");
    let backup_file = find_backup_file(&backups).await;

    let pool = open_pool(&db_path).await;
    sqlx::query("DELETE FROM projects WHERE id = 'p2'")
        .execute(&pool)
        .await
        .expect("delete project");
    sqlx::query("INSERT INTO projects (id, name) VALUES ('p3', 'Gamma'), ('p4', 'Delta')")
        .execute(&pool)
        .await
        .expect("insert projects");
    pool.close().await;

    let recovery = workspace.path().join("recovery");
    service
        .stage_restore(&backup_file, &recovery)
        .await
        .expect("stage restore");
    service
        .apply_pending_restore(&recovery)
        .await
        .expect("apply restore");
    let pool = open_pool(&db_path).await;
    assert_eq!(project_count(&pool).await, 2);
    pool.close().await;

    let pool = open_pool(&db_path).await;
    sqlx::query("DELETE FROM projects")
        .execute(&pool)
        .await
        .expect("clear projects");
    pool.close().await;

    service
        .rollback_restore(&recovery)
        .await
        .expect("rollback restore");
    let pool = open_pool(&db_path).await;
    assert_eq!(project_count(&pool).await, 3);
    pool.close().await;

    service
        .discard_pre_restore_snapshot(&recovery)
        .await
        .expect("discard snapshot");
    assert!(!recovery.join(PRE_RESTORE_FILE).exists());
}

#[tokio::test]
async fn rebuild_indexes_reports_success() {
    let workspace = tempdir().expect("tempdir");
    let db_path = create_database(&workspace).await;

    let report = service(&db_path)
        .rebuild_indexes()
        .await
        .expect("rebuild indexes");
    assert!(report.reindexed);
    assert!(report.analyzed);
    assert!(report.integrity_ok);
    assert!(!report.sqlite_version.is_empty());
}

#[tokio::test]
async fn rebuild_indexes_tolerates_concurrent_writers() {
    let workspace = tempdir().expect("tempdir");
    let db_path = create_database(&workspace).await;
    let writer_pool = open_pool(&db_path).await;

    let writer = tokio::spawn(async move {
        for index in 0..30 {
            sqlx::query("INSERT INTO projects (id, name) VALUES (?, ?)")
                .bind(format!("w{index}"))
                .bind(format!("Writer {index}"))
                .execute(&writer_pool)
                .await
                .expect("insert during rebuild");
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        writer_pool.close().await;
    });

    let report = service(&db_path)
        .rebuild_indexes()
        .await
        .expect("rebuild indexes");
    assert!(report.integrity_ok);
    writer.await.expect("writer finished");

    let pool = open_pool(&db_path).await;
    assert_eq!(project_count(&pool).await, 32);
    pool.close().await;
}
