//! Contracts for Scene Vault database maintenance.
//!
//! Backups produced by this module are consistent single-file SQLite
//! snapshots created with `VACUUM INTO`. They contain only the database index
//! and metadata (projects, roles, classifications, notes, Face Bank data and
//! settings). They never include source screenshots or NAS archives, which
//! stay at their user-controlled locations.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const BACKUP_ENGINE: &str = "vacuum-into";
pub const BACKUP_CONTENT_SCOPE: &str = "index-and-metadata-only";
pub const BACKUP_MANIFEST_EXTENSION: &str = "manifest.json";

pub const PENDING_RESTORE_FILE: &str = "scene-vault.db.restore-pending";
pub const PENDING_RESTORE_MANIFEST_FILE: &str = "scene-vault.db.restore-pending.manifest.json";
pub const RESTORE_REQUEST_FILE: &str = "restore-request.json";
pub const PRE_RESTORE_FILE: &str = "scene-vault.db.pre-restore";
pub const PRE_RESTORE_WAL_FILE: &str = "scene-vault.db.pre-restore-wal";
pub const PRE_RESTORE_SHM_FILE: &str = "scene-vault.db.pre-restore-shm";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PreflightReport {
    pub ok: bool,
    pub database_path: String,
    pub quick_check: Vec<String>,
    pub foreign_key_issues: Vec<String>,
    pub migration_issues: Vec<String>,
    pub schema_version: Option<i64>,
    pub sqlite_version: String,
    pub journal_mode: String,
    pub database_size_bytes: u64,
    pub wal_size_bytes: u64,
    pub checked_at_utc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BackupManifest {
    pub engine: String,
    pub content_scope: String,
    pub excludes_source_images: bool,
    pub backup_file: String,
    pub app_version: String,
    pub schema_version: Option<i64>,
    pub sqlite_version: String,
    pub created_at_utc: String,
    pub sha256: String,
    pub file_size: u64,
    pub table_counts: BTreeMap<String, i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RestoreRequest {
    pub engine: String,
    pub source_backup_path: String,
    pub staged_path: String,
    pub manifest_path: String,
    pub sha256: String,
    pub app_version: String,
    pub schema_version: Option<i64>,
    pub requested_at_utc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RestoreOutcome {
    pub applied_from: String,
    pub pre_restore_snapshot: String,
    pub schema_version: Option<i64>,
    pub sqlite_version: String,
    pub applied_at_utc: String,
    pub validated_after_swap: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceReport {
    pub reindexed: bool,
    pub analyzed: bool,
    pub integrity_ok: bool,
    pub sqlite_version: String,
    pub ran_at_utc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseStartupStatus {
    pub mode: String,
    pub database_path: String,
    pub backup_directory: String,
    pub recovery_directory: String,
    pub error_message: Option<String>,
    pub pending_restore: bool,
    pub restored_on_startup: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseBackupResult {
    pub backup_path: String,
    pub manifest_path: String,
    pub manifest: BackupManifest,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreateDatabaseBackupInput {
    pub destination: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StageDatabaseRestoreInput {
    pub backup_path: String,
}

#[derive(Debug, thiserror::Error)]
pub enum DataMaintenanceError {
    #[error("database path is invalid: {0}")]
    InvalidDatabasePath(String),

    #[error("destination is a live database path: {0}")]
    LivePathDestination(String),

    #[error("backup destination already exists: {0}")]
    DestinationExists(String),

    #[error("sqlite version {0} does not support VACUUM INTO (3.27 or newer required)")]
    UnsupportedSqliteVersion(String),

    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),

    #[error("filesystem error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("backup manifest error: {0}")]
    Manifest(String),

    #[error("backup validation failed: {0}")]
    Validation(String),

    #[error("checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },

    #[error("no pending restore in {0}")]
    RestoreNotPending(String),

    #[error("a restore is already staged in {0}; apply or clear it first")]
    RestoreAlreadyStaged(String),

    #[error("restore failed and the previous database was rolled back: {0}")]
    RestoreRolledBack(String),
}

impl Serialize for DataMaintenanceError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}
