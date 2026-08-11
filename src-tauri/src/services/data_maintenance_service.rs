//! Production database maintenance: integrity preflight, consistent backup,
//! two-phase restore and index maintenance.
//!
//! The caller supplies the resolved database path and the recovery directory;
//! this service never resolves the Tauri app data directory itself, which
//! keeps it testable with plain tempfile-backed databases.
//!
//! Safety model:
//! - Backups are always single-file `VACUUM INTO` snapshots. Copying
//!   `scene-vault.db` together with `-wal`/`-shm` is never used, because that
//!   produces an inconsistent snapshot while WAL content is pending.
//! - A backup contains only the SQLite index and metadata. Source screenshots,
//!   NAS archives, models and fonts are never included.
//! - A restore is staged in a recovery directory first, then applied on the
//!   next startup before any connection pool is opened. The live database is
//!   snapshotted as `scene-vault.db.pre-restore*` before the swap, so a failed
//!   post-swap validation can be rolled back.

use std::{
    collections::BTreeMap,
    fs,
    io::ErrorKind,
    path::{Component, Path, PathBuf},
    time::Duration,
};

use chrono::{SecondsFormat, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions},
    ConnectOptions, Row, SqliteConnection,
};
use tokio::io::AsyncReadExt;
use uuid::Uuid;

use crate::models::data_maintenance::{
    BackupManifest, DataMaintenanceError, MaintenanceReport, PreflightReport, RestoreOutcome,
    RestoreRequest, BACKUP_CONTENT_SCOPE, BACKUP_ENGINE, PENDING_RESTORE_FILE,
    PENDING_RESTORE_MANIFEST_FILE, PRE_RESTORE_FILE, PRE_RESTORE_SHM_FILE, PRE_RESTORE_WAL_FILE,
    RESTORE_REQUEST_FILE,
};

const SQLITE_VACUUM_INTO_MIN_VERSION: (u32, u32, u32) = (3, 27, 0);
const COPY_BUFFER_SIZE: usize = 64 * 1024;

#[derive(Debug, Clone)]
pub struct DataMaintenanceService {
    db_path: PathBuf,
    app_version: String,
    max_supported_schema_version: i64,
}

impl DataMaintenanceService {
    pub fn new(
        db_path: PathBuf,
        app_version: impl Into<String>,
        max_supported_schema_version: i64,
    ) -> Self {
        Self {
            db_path,
            app_version: app_version.into(),
            max_supported_schema_version,
        }
    }

    pub fn database_path(&self) -> &Path {
        &self.db_path
    }

    fn wal_path(&self) -> PathBuf {
        PathBuf::from(format!("{}-wal", self.db_path.display()))
    }

    fn shm_path(&self) -> PathBuf {
        PathBuf::from(format!("{}-shm", self.db_path.display()))
    }

    /// Runs `quick_check`, `foreign_key_check`, migration-state and version
    /// checks against the live database. Read-only; never modifies the file.
    pub async fn preflight(&self) -> Result<PreflightReport, DataMaintenanceError> {
        self.require_existing_database()?;
        let mut conn = self.open_connection_at(&self.db_path, true).await?;

        let quick_check = quick_check_rows(&mut conn).await?;
        let foreign_key_issues = foreign_key_issues(&mut conn).await?;
        let schema_version = schema_version(&mut conn).await?;
        let sqlite_version = sqlite_version(&mut conn).await?;
        let journal_mode = journal_mode(&mut conn).await?;
        let mut migration_issues = migration_issues(&mut conn).await?;

        if let Some(schema_version) = schema_version {
            if schema_version > self.max_supported_schema_version {
                migration_issues.push(format!(
                    "database schema version {schema_version} is newer than the supported maximum {}",
                    self.max_supported_schema_version
                ));
            }
        }

        let database_size_bytes = self.db_path.metadata().map(|m| m.len()).unwrap_or(0);
        let wal_size_bytes = self.wal_path().metadata().map(|m| m.len()).unwrap_or(0);
        let ok = quick_check.iter().all(|row| row == "ok")
            && foreign_key_issues.is_empty()
            && migration_issues.is_empty();

        Ok(PreflightReport {
            ok,
            database_path: self.db_path.display().to_string(),
            quick_check,
            foreign_key_issues,
            migration_issues,
            schema_version,
            sqlite_version,
            journal_mode,
            database_size_bytes,
            wal_size_bytes,
            checked_at_utc: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
        })
    }

    /// Creates a WAL-consistent single-file backup with `VACUUM INTO` and
    /// writes a SHA-256 manifest next to it.
    ///
    /// `destination` may be an existing directory (a timestamped file name is
    /// generated) or a full file path that must not exist yet. Live database,
    /// WAL and SHM paths are always rejected.
    pub async fn create_backup(
        &self,
        destination: impl AsRef<Path>,
    ) -> Result<BackupManifest, DataMaintenanceError> {
        self.require_existing_database()?;
        let destination = self.resolve_backup_target(destination.as_ref())?;
        if let Some(parent) = destination.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let sqlite_version = {
            let mut conn = self.open_connection_at(&self.db_path, false).await?;
            sqlite_version(&mut conn).await?
        };
        if !sqlite_version_at_least(&sqlite_version, SQLITE_VACUUM_INTO_MIN_VERSION) {
            return Err(DataMaintenanceError::UnsupportedSqliteVersion(
                sqlite_version,
            ));
        }

        let pool = self.open_pool_at(&self.db_path, 1, false).await?;
        let escaped = destination.to_string_lossy().replace('\'', "''");
        sqlx::query(&format!("VACUUM INTO '{escaped}'"))
            .execute(&pool)
            .await?;
        drop(pool);

        let manifest = match self.collect_manifest(&destination).await {
            Ok(manifest) => manifest,
            Err(error) => {
                let _ = tokio::fs::remove_file(&destination).await;
                let _ = tokio::fs::remove_file(manifest_path_for(&destination)).await;
                return Err(error);
            }
        };
        if let Err(error) = write_json_atomic(&manifest_path_for(&destination), &manifest).await {
            let _ = tokio::fs::remove_file(&destination).await;
            let _ = tokio::fs::remove_file(manifest_path_for(&destination)).await;
            return Err(error);
        }
        Ok(manifest)
    }

    /// Strictly validates a backup and its manifest: engine and content scope,
    /// file name, size, SHA-256, SQLite header, `quick_check`,
    /// `foreign_key_check`, schema compatibility and table counts.
    pub async fn validate_backup(
        &self,
        backup_path: impl AsRef<Path>,
    ) -> Result<BackupManifest, DataMaintenanceError> {
        let backup_path = backup_path.as_ref();
        let manifest_path = manifest_path_for(backup_path);
        if !manifest_path.is_file() {
            return Err(DataMaintenanceError::Manifest(format!(
                "backup manifest not found: {}",
                manifest_path.display()
            )));
        }
        let manifest: BackupManifest = read_json(&manifest_path).await?;

        if manifest.engine != BACKUP_ENGINE {
            return Err(DataMaintenanceError::Manifest(format!(
                "unexpected backup engine {:?}",
                manifest.engine
            )));
        }
        if manifest.content_scope != BACKUP_CONTENT_SCOPE || !manifest.excludes_source_images {
            return Err(DataMaintenanceError::Manifest(format!(
                "backup claims to contain more than the database index and metadata: scope={:?} excludes_source_images={}",
                manifest.content_scope, manifest.excludes_source_images
            )));
        }
        if manifest.backup_file != backup_file_name(backup_path) {
            return Err(DataMaintenanceError::Manifest(format!(
                "manifest backup file {:?} does not match {}",
                manifest.backup_file,
                backup_path.display()
            )));
        }
        if !backup_path.is_file() {
            return Err(DataMaintenanceError::Validation(format!(
                "backup file does not exist: {}",
                backup_path.display()
            )));
        }

        let actual_size = tokio::fs::metadata(backup_path).await?.len();
        if actual_size != manifest.file_size {
            return Err(DataMaintenanceError::Validation(format!(
                "backup size mismatch: manifest={} actual={}",
                manifest.file_size, actual_size
            )));
        }
        let actual_sha = sha256_file(backup_path).await?;
        if actual_sha != manifest.sha256 {
            return Err(DataMaintenanceError::ChecksumMismatch {
                expected: manifest.sha256.clone(),
                actual: actual_sha,
            });
        }
        if !sqlite_header_ok(backup_path).await? {
            return Err(DataMaintenanceError::Validation(format!(
                "backup is not a sqlite database: {}",
                backup_path.display()
            )));
        }

        let mut conn = self
            .open_connection_at(backup_path, true)
            .await
            .map_err(|error| {
                DataMaintenanceError::Validation(format!(
                    "backup is not a readable sqlite database ({}): {}",
                    error,
                    backup_path.display()
                ))
            })?;
        let quick_check = quick_check_rows(&mut conn).await?;
        if !quick_check.iter().all(|row| row == "ok") {
            return Err(DataMaintenanceError::Validation(format!(
                "backup failed quick_check: {quick_check:?}"
            )));
        }
        let foreign_key_issues = foreign_key_issues(&mut conn).await?;
        if !foreign_key_issues.is_empty() {
            return Err(DataMaintenanceError::Validation(format!(
                "backup failed foreign_key_check: {foreign_key_issues:?}"
            )));
        }
        let schema_version = schema_version(&mut conn).await?;
        if schema_version != manifest.schema_version {
            return Err(DataMaintenanceError::Validation(format!(
                "backup schema version mismatch: manifest={:?} database={:?}",
                manifest.schema_version, schema_version
            )));
        }
        if let Some(schema_version) = schema_version {
            if schema_version > self.max_supported_schema_version {
                return Err(DataMaintenanceError::Validation(format!(
                    "backup schema version {schema_version} is newer than the supported maximum {}",
                    self.max_supported_schema_version
                )));
            }
        }
        let table_counts = table_counts(&mut conn).await?;
        if table_counts != manifest.table_counts {
            return Err(DataMaintenanceError::Validation(format!(
                "backup table counts mismatch: {table_counts:?}"
            )));
        }
        Ok(manifest)
    }

    /// Stages a validated backup for the next startup restore. The live
    /// database is not opened, copied or modified.
    pub async fn stage_restore(
        &self,
        backup_path: impl AsRef<Path>,
        recovery_dir: impl AsRef<Path>,
    ) -> Result<RestoreRequest, DataMaintenanceError> {
        let backup_path = backup_path.as_ref();
        let recovery_dir = recovery_dir.as_ref();
        let manifest = self.validate_backup(backup_path).await?;

        tokio::fs::create_dir_all(recovery_dir).await?;
        let pending = recovery_dir.join(PENDING_RESTORE_FILE);
        let pending_manifest = recovery_dir.join(PENDING_RESTORE_MANIFEST_FILE);
        let marker = recovery_dir.join(RESTORE_REQUEST_FILE);
        if pending.exists() || marker.exists() {
            return Err(DataMaintenanceError::RestoreAlreadyStaged(
                recovery_dir.display().to_string(),
            ));
        }

        copy_file(backup_path, &pending).await?;
        copy_file(&manifest_path_for(backup_path), &pending_manifest).await?;
        let mut staged_manifest: BackupManifest = read_json(&pending_manifest).await?;
        staged_manifest.backup_file = backup_file_name(&pending);
        write_json_atomic(&pending_manifest, &staged_manifest).await?;

        let staged_sha = sha256_file(&pending).await?;
        if staged_sha != manifest.sha256 {
            let _ = tokio::fs::remove_file(&pending).await;
            let _ = tokio::fs::remove_file(&pending_manifest).await;
            return Err(DataMaintenanceError::Validation(
                "staged restore copy failed checksum verification".to_owned(),
            ));
        }

        let request = RestoreRequest {
            engine: BACKUP_ENGINE.to_owned(),
            source_backup_path: backup_path.display().to_string(),
            staged_path: pending.display().to_string(),
            manifest_path: pending_manifest.display().to_string(),
            sha256: manifest.sha256.clone(),
            app_version: self.app_version.clone(),
            schema_version: manifest.schema_version,
            requested_at_utc: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        };
        if let Err(error) = write_json_atomic(&marker, &request).await {
            let _ = tokio::fs::remove_file(&pending).await;
            let _ = tokio::fs::remove_file(&pending_manifest).await;
            return Err(error);
        }
        Ok(request)
    }

    pub async fn pending_restore(
        &self,
        recovery_dir: impl AsRef<Path>,
    ) -> Result<Option<RestoreRequest>, DataMaintenanceError> {
        let marker = recovery_dir.as_ref().join(RESTORE_REQUEST_FILE);
        if !marker.is_file() {
            return Ok(None);
        }
        let request: RestoreRequest = read_json(&marker).await?;
        Ok(Some(request))
    }

    pub async fn has_pending_restore(&self, recovery_dir: impl AsRef<Path>) -> bool {
        recovery_dir.as_ref().join(RESTORE_REQUEST_FILE).is_file()
    }

    /// Removes a staged restore without touching the live database.
    pub async fn clear_pending_restore(
        &self,
        recovery_dir: impl AsRef<Path>,
    ) -> Result<(), DataMaintenanceError> {
        let recovery_dir = recovery_dir.as_ref();
        for name in [
            RESTORE_REQUEST_FILE,
            PENDING_RESTORE_FILE,
            PENDING_RESTORE_MANIFEST_FILE,
        ] {
            let _ = tokio::fs::remove_file(recovery_dir.join(name)).await;
        }
        Ok(())
    }

    /// Applies a staged restore. Must run before any connection pool is opened
    /// on the live database. The live `scene-vault.db`/`-wal`/`-shm` are first
    /// snapshotted into the recovery directory; the pending file is then
    /// swapped in and revalidated. If the post-swap check fails, the previous
    /// database is restored automatically.
    pub async fn apply_pending_restore(
        &self,
        recovery_dir: impl AsRef<Path>,
    ) -> Result<RestoreOutcome, DataMaintenanceError> {
        let recovery_dir = recovery_dir.as_ref();
        let marker_path = recovery_dir.join(RESTORE_REQUEST_FILE);
        let pending = recovery_dir.join(PENDING_RESTORE_FILE);
        let pending_manifest = recovery_dir.join(PENDING_RESTORE_MANIFEST_FILE);

        if !marker_path.is_file() {
            return Err(DataMaintenanceError::RestoreNotPending(
                recovery_dir.display().to_string(),
            ));
        }
        let request: RestoreRequest = read_json(&marker_path).await?;
        if !pending.is_file() {
            return Err(DataMaintenanceError::Validation(format!(
                "staged restore file is missing: {}",
                pending.display()
            )));
        }
        let actual_sha = sha256_file(&pending).await?;
        if actual_sha != request.sha256 {
            return Err(DataMaintenanceError::ChecksumMismatch {
                expected: request.sha256,
                actual: actual_sha,
            });
        }
        self.validate_backup(&pending).await?;

        self.snapshot_live_database(recovery_dir).await?;
        remove_if_exists(&self.wal_path()).await;
        remove_if_exists(&self.shm_path()).await;
        if let Err(error) = replace_file(&pending, &self.db_path).await {
            let rollback_message = match self.rollback(recovery_dir).await {
                Ok(()) => "previous database restored".to_owned(),
                Err(rollback_error) => format!("rollback also failed: {rollback_error}"),
            };
            return Err(DataMaintenanceError::RestoreRolledBack(format!(
                "database swap failed ({error}); {rollback_message}"
            )));
        }

        let validated_after_swap = match self.validate_swapped_database().await {
            Ok(valid) => valid,
            Err(error) => {
                let rollback_message = match self.rollback(recovery_dir).await {
                    Ok(()) => "previous database restored".to_owned(),
                    Err(rollback_error) => format!("rollback also failed: {rollback_error}"),
                };
                return Err(DataMaintenanceError::RestoreRolledBack(format!(
                    "post-swap validation failed ({error}); {rollback_message}"
                )));
            }
        };
        if !validated_after_swap {
            let rollback_message = match self.rollback(recovery_dir).await {
                Ok(()) => "previous database restored".to_owned(),
                Err(rollback_error) => format!("rollback also failed: {rollback_error}"),
            };
            return Err(DataMaintenanceError::RestoreRolledBack(format!(
                "post-swap integrity check failed; {rollback_message}"
            )));
        }

        let _ = tokio::fs::remove_file(&pending_manifest).await;
        let _ = tokio::fs::remove_file(&marker_path).await;

        let mut conn = self.open_connection_at(&self.db_path, true).await?;
        let schema_version = schema_version(&mut conn).await?;
        let sqlite_version = sqlite_version(&mut conn).await?;

        Ok(RestoreOutcome {
            applied_from: request.staged_path,
            pre_restore_snapshot: recovery_dir.join(PRE_RESTORE_FILE).display().to_string(),
            schema_version,
            sqlite_version,
            applied_at_utc: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            validated_after_swap,
        })
    }

    /// Explicit rollback for failures detected after the app re-opened the
    /// pool (for example migration failure). Requires the pool to be closed
    /// again before calling. Uses the `.pre-restore` snapshot kept by
    /// [`apply_pending_restore`](Self::apply_pending_restore).
    pub async fn rollback_restore(
        &self,
        recovery_dir: impl AsRef<Path>,
    ) -> Result<(), DataMaintenanceError> {
        let recovery_dir = recovery_dir.as_ref();
        if !recovery_dir.join(PRE_RESTORE_FILE).exists() {
            return Err(DataMaintenanceError::RestoreNotPending(format!(
                "no pre-restore snapshot in {}",
                recovery_dir.display()
            )));
        }
        self.rollback(recovery_dir).await
    }

    /// Removes the pre-restore snapshot after the restored database has been
    /// validated with the pool open and migrations applied.
    pub async fn discard_pre_restore_snapshot(
        &self,
        recovery_dir: impl AsRef<Path>,
    ) -> Result<(), DataMaintenanceError> {
        let recovery_dir = recovery_dir.as_ref();
        for name in [PRE_RESTORE_FILE, PRE_RESTORE_WAL_FILE, PRE_RESTORE_SHM_FILE] {
            let _ = tokio::fs::remove_file(recovery_dir.join(name)).await;
        }
        Ok(())
    }

    /// Runs `REINDEX` and `ANALYZE` online, then verifies integrity.
    pub async fn rebuild_indexes(&self) -> Result<MaintenanceReport, DataMaintenanceError> {
        self.require_existing_database()?;
        let pool = self.open_pool_at(&self.db_path, 1, false).await?;
        sqlx::query("REINDEX").execute(&pool).await?;
        sqlx::query("ANALYZE").execute(&pool).await?;
        drop(pool);

        let mut conn = self.open_connection_at(&self.db_path, true).await?;
        let quick_check = quick_check_rows(&mut conn).await?;
        let sqlite_version = sqlite_version(&mut conn).await?;

        Ok(MaintenanceReport {
            reindexed: true,
            analyzed: true,
            integrity_ok: quick_check.iter().all(|row| row == "ok"),
            sqlite_version,
            ran_at_utc: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        })
    }

    fn require_existing_database(&self) -> Result<(), DataMaintenanceError> {
        if !self.db_path.is_file() {
            return Err(DataMaintenanceError::InvalidDatabasePath(format!(
                "database file does not exist: {}",
                self.db_path.display()
            )));
        }
        Ok(())
    }

    fn resolve_backup_target(&self, destination: &Path) -> Result<PathBuf, DataMaintenanceError> {
        let destination = if destination.is_dir() {
            let stamp = Utc::now().format("%Y%m%d-%H%M%S");
            let mut candidate = destination.join(format!("scene-vault-backup-{stamp}.sqlite"));
            let mut index = 1_u32;
            while candidate.exists() {
                candidate = destination.join(format!("scene-vault-backup-{stamp}-{index}.sqlite"));
                index += 1;
            }
            candidate
        } else {
            destination.to_path_buf()
        };
        self.ensure_not_live_path(&destination)?;
        if !destination.is_dir() && destination.exists() {
            return Err(DataMaintenanceError::DestinationExists(
                destination.display().to_string(),
            ));
        }
        Ok(destination)
    }

    fn ensure_not_live_path(&self, candidate: &Path) -> Result<(), DataMaintenanceError> {
        let normalized = normalize_absolute_path(candidate);
        for live in [
            self.db_path.as_path(),
            self.wal_path().as_path(),
            self.shm_path().as_path(),
        ] {
            if normalize_absolute_path(live) == normalized {
                return Err(DataMaintenanceError::LivePathDestination(
                    candidate.display().to_string(),
                ));
            }
        }
        Ok(())
    }

    async fn collect_manifest(
        &self,
        backup_path: &Path,
    ) -> Result<BackupManifest, DataMaintenanceError> {
        let sha256 = sha256_file(backup_path).await?;
        let file_size = tokio::fs::metadata(backup_path).await?.len();

        let mut conn = self
            .open_connection_at(backup_path, true)
            .await
            .map_err(|error| {
                DataMaintenanceError::Validation(format!(
                    "backup is not a readable sqlite database ({}): {}",
                    error,
                    backup_path.display()
                ))
            })?;
        let quick_check = quick_check_rows(&mut conn).await?;
        if !quick_check.iter().all(|row| row == "ok") {
            return Err(DataMaintenanceError::Validation(format!(
                "backup failed quick_check: {quick_check:?}"
            )));
        }
        let foreign_key_issues = foreign_key_issues(&mut conn).await?;
        if !foreign_key_issues.is_empty() {
            return Err(DataMaintenanceError::Validation(format!(
                "backup failed foreign_key_check: {foreign_key_issues:?}"
            )));
        }
        let schema_version = schema_version(&mut conn).await?;
        let sqlite_version = sqlite_version(&mut conn).await?;
        let table_counts = table_counts(&mut conn).await?;

        Ok(BackupManifest {
            engine: BACKUP_ENGINE.to_owned(),
            content_scope: BACKUP_CONTENT_SCOPE.to_owned(),
            excludes_source_images: true,
            backup_file: backup_file_name(backup_path),
            app_version: self.app_version.clone(),
            schema_version,
            sqlite_version,
            created_at_utc: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            sha256,
            file_size,
            table_counts,
        })
    }

    async fn snapshot_live_database(
        &self,
        recovery_dir: &Path,
    ) -> Result<(), DataMaintenanceError> {
        tokio::fs::create_dir_all(recovery_dir).await?;
        for name in [PRE_RESTORE_FILE, PRE_RESTORE_WAL_FILE, PRE_RESTORE_SHM_FILE] {
            let _ = tokio::fs::remove_file(recovery_dir.join(name)).await;
        }
        copy_file(&self.db_path, &recovery_dir.join(PRE_RESTORE_FILE)).await?;
        for (pre_name, live_path) in [
            (PRE_RESTORE_WAL_FILE, self.wal_path()),
            (PRE_RESTORE_SHM_FILE, self.shm_path()),
        ] {
            if live_path.is_file() {
                copy_file(&live_path, &recovery_dir.join(pre_name)).await?;
            }
        }
        Ok(())
    }

    async fn validate_swapped_database(&self) -> Result<bool, DataMaintenanceError> {
        if !self.db_path.is_file() {
            return Ok(false);
        }
        let mut conn = self.open_connection_at(&self.db_path, true).await?;
        let quick_check = quick_check_rows(&mut conn).await?;
        if !quick_check.iter().all(|row| row == "ok") {
            return Ok(false);
        }
        let foreign_key_issues = foreign_key_issues(&mut conn).await?;
        if !foreign_key_issues.is_empty() {
            return Ok(false);
        }
        if let Some(version) = schema_version(&mut conn).await? {
            if version > self.max_supported_schema_version {
                return Ok(false);
            }
        }
        Ok(true)
    }

    async fn rollback(&self, recovery_dir: &Path) -> Result<(), DataMaintenanceError> {
        let pre = recovery_dir.join(PRE_RESTORE_FILE);
        if pre.exists() {
            remove_if_exists(&self.db_path).await;
            remove_if_exists(&self.wal_path()).await;
            remove_if_exists(&self.shm_path()).await;
            replace_file(&pre, &self.db_path).await?;
            for (pre_name, live_path) in [
                (PRE_RESTORE_WAL_FILE, self.wal_path()),
                (PRE_RESTORE_SHM_FILE, self.shm_path()),
            ] {
                let pre_path = recovery_dir.join(pre_name);
                if pre_path.exists() {
                    remove_if_exists(&live_path).await;
                    replace_file(&pre_path, &live_path).await?;
                }
            }
        }
        let _ = tokio::fs::remove_file(recovery_dir.join(RESTORE_REQUEST_FILE)).await;
        let _ = tokio::fs::remove_file(recovery_dir.join(PENDING_RESTORE_FILE)).await;
        let _ = tokio::fs::remove_file(recovery_dir.join(PENDING_RESTORE_MANIFEST_FILE)).await;
        Ok(())
    }

    async fn open_connection_at(
        &self,
        path: &Path,
        read_only: bool,
    ) -> Result<SqliteConnection, sqlx::Error> {
        connect_options(path, read_only).connect().await
    }

    async fn open_pool_at(
        &self,
        path: &Path,
        max_connections: u32,
        read_only: bool,
    ) -> Result<SqlitePool, sqlx::Error> {
        SqlitePoolOptions::new()
            .max_connections(max_connections)
            .connect_with(connect_options(path, read_only))
            .await
    }
}

fn connect_options(path: &Path, read_only: bool) -> SqliteConnectOptions {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));
    if read_only {
        options.read_only(true)
    } else {
        options
    }
}

async fn quick_check_rows(
    conn: &mut SqliteConnection,
) -> Result<Vec<String>, DataMaintenanceError> {
    let rows = sqlx::query("PRAGMA quick_check")
        .fetch_all(&mut *conn)
        .await?;
    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        result.push(row.try_get(0)?);
    }
    Ok(result)
}

async fn foreign_key_issues(
    conn: &mut SqliteConnection,
) -> Result<Vec<String>, DataMaintenanceError> {
    let rows = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&mut *conn)
        .await?;
    let mut issues = Vec::with_capacity(rows.len());
    for row in rows {
        let table: String = row.try_get(0)?;
        let rowid: i64 = row.try_get(1)?;
        let parent: String = row.try_get(2)?;
        let fkid: i64 = row.try_get(3)?;
        issues.push(format!(
            "table {table} row {rowid} references {parent} (foreign key {fkid})"
        ));
    }
    Ok(issues)
}

async fn migration_issues(
    conn: &mut SqliteConnection,
) -> Result<Vec<String>, DataMaintenanceError> {
    if !sqlite_master_has_table(conn, "_sqlx_migrations").await? {
        return Ok(Vec::new());
    }
    let rows =
        sqlx::query("SELECT version, description, success FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&mut *conn)
            .await?;
    let mut issues = Vec::new();
    for row in rows {
        let version: i64 = row.try_get(0)?;
        let description: String = row.try_get(1)?;
        let success: bool = row.try_get(2)?;
        if !success {
            issues.push(format!("migration {version} ({description}) is dirty"));
        }
    }
    Ok(issues)
}

async fn schema_version(conn: &mut SqliteConnection) -> Result<Option<i64>, DataMaintenanceError> {
    if !sqlite_master_has_table(conn, "_sqlx_migrations").await? {
        return Ok(None);
    }
    let version: Option<i64> = sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations")
        .fetch_one(&mut *conn)
        .await?;
    Ok(version)
}

async fn sqlite_master_has_table(
    conn: &mut SqliteConnection,
    name: &str,
) -> Result<bool, DataMaintenanceError> {
    let found: Option<String> =
        sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?")
            .bind(name)
            .fetch_optional(&mut *conn)
            .await?;
    Ok(found.is_some())
}

async fn sqlite_version(conn: &mut SqliteConnection) -> Result<String, DataMaintenanceError> {
    let version: String = sqlx::query_scalar("SELECT sqlite_version()")
        .fetch_one(&mut *conn)
        .await?;
    Ok(version)
}

async fn journal_mode(conn: &mut SqliteConnection) -> Result<String, DataMaintenanceError> {
    let mode: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(&mut *conn)
        .await?;
    Ok(mode)
}

async fn table_counts(
    conn: &mut SqliteConnection,
) -> Result<BTreeMap<String, i64>, DataMaintenanceError> {
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut counts = BTreeMap::new();
    for table in tables {
        let escaped = table.replace('"', "\"\"");
        let sql = format!("SELECT COUNT(*) FROM \"{escaped}\"");
        let count: i64 = sqlx::query_scalar(&sql).fetch_one(&mut *conn).await?;
        counts.insert(table, count);
    }
    Ok(counts)
}

async fn sha256_file(path: &Path) -> Result<String, std::io::Error> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; COPY_BUFFER_SIZE];
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

async fn sqlite_header_ok(path: &Path) -> Result<bool, std::io::Error> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut header = [0_u8; 16];
    let read = file.read(&mut header).await?;
    Ok(read == header.len() && &header == b"SQLite format 3\0")
}

async fn copy_file(source: &Path, destination: &Path) -> Result<(), DataMaintenanceError> {
    if let Some(parent) = destination.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::copy(source, destination).await?;
    let source_len = tokio::fs::metadata(source).await?.len();
    let destination_len = tokio::fs::metadata(destination).await?.len();
    if source_len != destination_len {
        return Err(DataMaintenanceError::Validation(format!(
            "copy length mismatch for {}",
            destination.display()
        )));
    }
    Ok(())
}

async fn replace_file(source: &Path, destination: &Path) -> Result<(), DataMaintenanceError> {
    match tokio::fs::rename(source, destination).await {
        Ok(()) => Ok(()),
        Err(error)
            if error.kind() == ErrorKind::AlreadyExists
                || (cfg!(unix) && error.raw_os_error() == Some(18)) =>
        {
            tokio::fs::copy(source, destination).await?;
            let _ = tokio::fs::remove_file(source).await;
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

async fn remove_if_exists(path: &Path) {
    let _ = tokio::fs::remove_file(path).await;
}

async fn write_json_atomic(
    path: &Path,
    value: &impl Serialize,
) -> Result<(), DataMaintenanceError> {
    let parent = path.parent().ok_or_else(|| {
        DataMaintenanceError::Manifest(format!(
            "cannot write {}: no parent directory",
            path.display()
        ))
    })?;
    tokio::fs::create_dir_all(parent).await?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("json");
    let temporary = parent.join(format!(".{name}.partial-{}", Uuid::new_v4()));
    let bytes = serde_json::to_vec_pretty(value)?;
    tokio::fs::write(&temporary, &bytes).await?;
    replace_file(&temporary, path).await
}

async fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, DataMaintenanceError> {
    let content = tokio::fs::read_to_string(path).await?;
    Ok(serde_json::from_str(&content)?)
}

fn manifest_path_for(backup_path: &Path) -> PathBuf {
    let mut name = backup_path.as_os_str().to_owned();
    name.push(format!(
        ".{}",
        crate::models::data_maintenance::BACKUP_MANIFEST_EXTENSION
    ));
    PathBuf::from(name)
}

fn backup_file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

fn sqlite_version_at_least(version: &str, minimum: (u32, u32, u32)) -> bool {
    let mut parts = version.split('.');
    let major = parts
        .next()
        .and_then(|p| p.parse::<u32>().ok())
        .unwrap_or(0);
    let minor = parts
        .next()
        .and_then(|p| p.parse::<u32>().ok())
        .unwrap_or(0);
    let patch = parts
        .next()
        .and_then(|p| p.split(['-', '+']).next())
        .and_then(|p| p.parse::<u32>().ok())
        .unwrap_or(0);
    (major, minor, patch) >= minimum
}

fn normalize_absolute_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = fs::canonicalize(path) {
        return canonical;
    }
    if let Some(parent) = path.parent() {
        if let Ok(canonical_parent) = fs::canonicalize(parent) {
            if let Some(name) = path.file_name() {
                return canonical_parent.join(name);
            }
        }
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

#[cfg(test)]
#[path = "data_maintenance_service_tests.rs"]
mod tests;
