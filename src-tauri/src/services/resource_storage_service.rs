use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicI64, Ordering},
        OnceLock,
    },
};

use chrono::{SecondsFormat, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    error::AppError,
    models::resource::{CleanupResourceResult, StorageResourceEntry, StorageResourceStatus},
    services::log_service,
};

#[derive(Debug, Clone)]
pub struct ResourcePaths {
    pub database_file: PathBuf,
    pub logs: PathBuf,
    pub thumbnail_cache: PathBuf,
    pub capture_output: PathBuf,
    pub models: PathBuf,
    pub fonts: PathBuf,
    pub python_runtime: PathBuf,
    pub webview_data: PathBuf,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct DiskUsage {
    bytes: u64,
    files: u64,
}

/// Cache root configured at startup; absent in unit tests and before setup.
static CAPTURE_OUTPUT_DIR: OnceLock<PathBuf> = OnceLock::new();
static LAST_AUTO_CLEANUP_MS: AtomicI64 = AtomicI64::new(0);
static AUTO_CLEANUP_RUNNING: AtomicBool = AtomicBool::new(false);

/// Automatic cleanup runs at most this often; archives can arrive in bursts.
const AUTO_CLEANUP_INTERVAL_MS: i64 = 5 * 60 * 1000;

pub fn configure_capture_output_dir(path: PathBuf) {
    let _ = CAPTURE_OUTPUT_DIR.set(path);
}

/// Best-effort, throttled cleanup request that is safe to call from every
/// archive path (IPC and the background worker). No-op until the cache root
/// is configured and at most one pass runs at a time.
pub fn request_auto_cleanup(pool: SqlitePool) {
    let Some(cache_root) = CAPTURE_OUTPUT_DIR.get().cloned() else {
        return;
    };
    let now = Utc::now().timestamp_millis();
    if now.saturating_sub(LAST_AUTO_CLEANUP_MS.load(Ordering::Relaxed)) < AUTO_CLEANUP_INTERVAL_MS {
        return;
    }
    if AUTO_CLEANUP_RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    LAST_AUTO_CLEANUP_MS.store(now, Ordering::Relaxed);
    tauri::async_runtime::spawn(async move {
        let _ = auto_cleanup_capture_output(&pool, &cache_root).await;
        AUTO_CLEANUP_RUNNING.store(false, Ordering::Release);
    });
}

pub fn scan(paths: &ResourcePaths) -> StorageResourceStatus {
    let definitions = [
        ("database", "数据库", &paths.database_file, false, None),
        (
            "logs",
            "运行日志",
            &paths.logs,
            true,
            Some("按保留策略清理过期日志"),
        ),
        (
            "thumbnail_cache",
            "缩略图缓存",
            &paths.thumbnail_cache,
            true,
            Some("可安全重建；清理后首次浏览会重新生成"),
        ),
        (
            "capture_output",
            "已归档处理缓存",
            &paths.capture_output,
            true,
            Some("仅清理已成功归档项目的本地中间产物"),
        ),
        ("models", "视觉模型", &paths.models, false, None),
        ("fonts", "字体资源", &paths.fonts, false, None),
        (
            "python_runtime",
            "Python 运行环境",
            &paths.python_runtime,
            false,
            None,
        ),
        (
            "webview_data",
            "WebView 数据",
            &paths.webview_data,
            false,
            None,
        ),
    ];

    let entries = definitions
        .into_iter()
        .map(
            |(kind, label, path, cleanup_available, cleanup_description)| {
                let usage = if kind == "database" {
                    database_usage(path)
                } else {
                    disk_usage(path)
                };
                // A missing path cannot be opened by the OS opener plugin, so it is
                // surfaced as "not configured" (button hidden) instead of an error.
                let exists = path.symlink_metadata().is_ok();
                StorageResourceEntry {
                    kind: kind.to_owned(),
                    label: label.to_owned(),
                    path: exists.then(|| path.to_string_lossy().into_owned()),
                    total_bytes: usage.bytes,
                    file_count: usage.files,
                    cleanup_available,
                    cleanup_description: cleanup_description.map(str::to_owned),
                }
            },
        )
        .collect::<Vec<_>>();
    let total_bytes = entries.iter().map(|entry| entry.total_bytes).sum();
    StorageResourceStatus {
        captured_at: Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
        entries,
        total_bytes,
    }
}

fn database_usage(database: &Path) -> DiskUsage {
    let mut total = disk_usage(database);
    let database_name = database.to_string_lossy();
    for suffix in ["-wal", "-shm"] {
        let usage = disk_usage(Path::new(&format!("{database_name}{suffix}")));
        total.bytes = total.bytes.saturating_add(usage.bytes);
        total.files = total.files.saturating_add(usage.files);
    }
    total
}

fn disk_usage(path: &Path) -> DiskUsage {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return DiskUsage::default();
    };
    if metadata.file_type().is_symlink() {
        return DiskUsage::default();
    }
    if metadata.is_file() {
        return DiskUsage {
            bytes: metadata.len(),
            files: 1,
        };
    }
    if !metadata.is_dir() {
        return DiskUsage::default();
    }

    let Ok(entries) = fs::read_dir(path) else {
        return DiskUsage::default();
    };
    entries
        .flatten()
        .fold(DiskUsage::default(), |mut total, entry| {
            let usage = disk_usage(&entry.path());
            total.bytes = total.bytes.saturating_add(usage.bytes);
            total.files = total.files.saturating_add(usage.files);
            total
        })
}

/// Low-frequency diagnostics of app-owned paths only; never follows links.
pub fn performance_snapshot(
    paths: &[(String, PathBuf)],
    should_continue: impl Fn() -> bool,
) -> serde_json::Value {
    let mut entries = Vec::new();
    for (kind, root) in paths {
        if !should_continue() {
            return serde_json::Value::Null;
        }
        let mut usage = DiskUsage::default();
        let mut complete = true;
        let mut pending = vec![root.clone()];
        while let Some(path) = pending.pop() {
            if !should_continue() {
                return serde_json::Value::Null;
            }
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => {
                    complete = false;
                    continue;
                }
            };
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if metadata.file_attributes() & 0x400 != 0 {
                    complete = false;
                    continue;
                }
            }
            if metadata.file_type().is_symlink() {
                complete = false;
                continue;
            }
            if metadata.is_file() {
                usage.bytes = usage.bytes.saturating_add(metadata.len());
                usage.files = usage.files.saturating_add(1);
            } else if metadata.is_dir() {
                match fs::read_dir(&path) {
                    Ok(children) => {
                        for child in children {
                            match child {
                                Ok(child) => pending.push(child.path()),
                                Err(_) => complete = false,
                            }
                        }
                    }
                    Err(_) => complete = false,
                }
            }
        }
        entries.push(serde_json::json!({
            "kind": kind, "bytes": usage.bytes, "files": usage.files, "complete": complete,
        }));
    }
    serde_json::json!({ "entries": entries })
}

fn clear_directory(root: &Path) -> Result<DiskUsage, AppError> {
    if fs::symlink_metadata(root).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(AppError::Validation(format!(
            "refusing to clean a symlinked cache root: {}",
            root.display()
        )));
    }
    fs::create_dir_all(root)?;
    let before = disk_usage(root);
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            fs::remove_dir_all(path)?;
        } else {
            fs::remove_file(path)?;
        }
    }
    fs::create_dir_all(root)?;
    Ok(before)
}

pub fn cleanup_thumbnail_cache(root: &Path) -> Result<CleanupResourceResult, AppError> {
    let removed = clear_directory(root)?;
    Ok(CleanupResourceResult {
        kind: "thumbnail_cache".to_owned(),
        removed_files: removed.files,
        reclaimed_bytes: removed.bytes,
        message: "缩略图缓存已清理，需要时会自动重新生成。".to_owned(),
    })
}

pub async fn cleanup_capture_output(
    pool: &SqlitePool,
    cache_root: &Path,
) -> Result<CleanupResourceResult, AppError> {
    cleanup_capture_output_inner(pool, cache_root, None, None).await
}

/// Budget-driven cleanup used after a successful archive. Only directories
/// whose capture was archived at least the configured retention window ago
/// are eligible, and every archived target must still exist on disk.
pub async fn auto_cleanup_capture_output(
    pool: &SqlitePool,
    cache_root: &Path,
) -> Result<Option<CleanupResourceResult>, AppError> {
    let settings = crate::services::app_settings_service::get(pool).await?;
    if !settings.capture_output_auto_cleanup_enabled {
        return Ok(None);
    }
    if !cache_root.is_dir() {
        return Ok(None);
    }
    let budget_bytes =
        settings.capture_output_auto_cleanup_threshold_mb.max(128) as u64 * 1024 * 1024;
    let usage = disk_usage(cache_root);
    if usage.bytes <= budget_bytes {
        return Ok(None);
    }
    let retention_days = settings
        .capture_output_auto_cleanup_retention_days
        .clamp(1, 365);
    let result =
        cleanup_capture_output_inner(pool, cache_root, Some(retention_days), Some(budget_bytes))
            .await?;
    if result.removed_files > 0 {
        log_service::info(
            "resource.cleanup",
            format!(
                "automatic capture-output cleanup reclaimed {} bytes in {} files",
                result.reclaimed_bytes, result.removed_files
            ),
        );
    }
    Ok(Some(result))
}

async fn cleanup_capture_output_inner(
    pool: &SqlitePool,
    cache_root: &Path,
    retention_days: Option<i64>,
    stop_under_bytes: Option<u64>,
) -> Result<CleanupResourceResult, AppError> {
    if fs::symlink_metadata(cache_root).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(AppError::Validation(format!(
            "refusing to clean a symlinked capture cache root: {}",
            cache_root.display()
        )));
    }
    fs::create_dir_all(cache_root)?;
    let retention_modifier = retention_days.map(|days| format!("-{days} days"));
    let rows = sqlx::query_as::<_, (String, Option<String>, Option<String>, Option<String>)>(
        r#"
        SELECT id, avatar_path, destination_path, destination_avatar_path
        FROM capture_items
        WHERE status = 'completed'
          AND archived_at IS NOT NULL
          AND destination_path IS NOT NULL
          AND (? IS NULL OR archived_at <= strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?))
        ORDER BY archived_at ASC
        "#,
    )
    .bind(&retention_modifier)
    .bind(&retention_modifier)
    .fetch_all(pool)
    .await?;

    let initial = disk_usage(cache_root);
    let mut removed = DiskUsage::default();
    for (id, avatar_path, destination_path, destination_avatar_path) in rows {
        if let Some(budget) = stop_under_bytes {
            if initial.bytes.saturating_sub(removed.bytes) <= budget {
                break;
            }
        }
        if Uuid::parse_str(&id).is_err() {
            continue;
        }
        let Some(destination_path) = destination_path.filter(|path| Path::new(path).is_file())
        else {
            continue;
        };
        if avatar_path.is_some()
            && !destination_avatar_path
                .as_deref()
                .is_some_and(|path| Path::new(path).is_file())
        {
            continue;
        }
        let target = cache_root.join(&id);
        let Ok(metadata) = fs::symlink_metadata(&target) else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        let usage = disk_usage(&target);
        let updated = sqlx::query(
            r#"
            UPDATE capture_items
            SET annotated_path = CASE WHEN annotated_path IS NULL THEN NULL ELSE ? END,
                avatar_path = CASE WHEN avatar_path IS NULL THEN NULL ELSE ? END,
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
            WHERE id = ?
              AND status = 'completed'
              AND archived_at IS NOT NULL
              AND destination_path IS NOT NULL
            "#,
        )
        .bind(&destination_path)
        .bind(destination_avatar_path.as_deref())
        .bind(&id)
        .execute(pool)
        .await?;
        if updated.rows_affected() != 1 {
            continue;
        }
        if metadata.is_dir() {
            fs::remove_dir_all(&target)?;
        } else if metadata.is_file() {
            fs::remove_file(&target)?;
        }
        removed.bytes = removed.bytes.saturating_add(usage.bytes);
        removed.files = removed.files.saturating_add(usage.files);
    }

    // Late bridge metadata lives outside the per-capture UUID directories:
    // sweep the files old enough that no pending ingest can still consume
    // them, while recent files stay for the retry pass.
    let metadata_root = cache_root.join("renpy-bridge").join("metadata");
    if fs::symlink_metadata(&metadata_root).is_ok_and(|metadata| metadata.is_dir()) {
        let grace_days = retention_days.unwrap_or(7).clamp(1, 365);
        let cutoff = std::time::SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(
                u64::try_from(grace_days).unwrap_or(7) * 86_400,
            ))
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        let mut pending = vec![metadata_root.clone()];
        while let Some(directory) = pending.pop() {
            let Ok(entries) = fs::read_dir(&directory) else {
                continue;
            };
            for entry in entries.flatten() {
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                let path = entry.path();
                if file_type.is_dir() {
                    pending.push(path);
                    continue;
                }
                if !file_type.is_file() {
                    continue;
                }
                let Ok(metadata) = entry.metadata() else {
                    continue;
                };
                if metadata
                    .modified()
                    .map(|modified| modified > cutoff)
                    .unwrap_or(true)
                {
                    continue;
                }
                if fs::remove_file(&path).is_ok() {
                    removed.bytes = removed.bytes.saturating_add(metadata.len());
                    removed.files = removed.files.saturating_add(1);
                }
            }
        }
        if let Ok(entries) = fs::read_dir(&metadata_root) {
            for entry in entries.flatten() {
                let is_directory = entry
                    .file_type()
                    .map(|file_type| file_type.is_dir())
                    .unwrap_or(false);
                if is_directory {
                    let _ = fs::remove_dir(entry.path());
                }
            }
        }
    }

    Ok(CleanupResourceResult {
        kind: "capture_output".to_owned(),
        removed_files: removed.files,
        reclaimed_bytes: removed.bytes,
        message: "已将文件引用切换到归档位置，并清理本地处理缓存。".to_owned(),
    })
}

#[cfg(test)]
#[path = "../../tests/unit/resource_storage_service_tests.rs"]
mod tests;
