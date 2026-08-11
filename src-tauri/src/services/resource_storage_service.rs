use std::{
    fs,
    path::{Path, PathBuf},
};

use chrono::{SecondsFormat, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    error::AppError,
    models::resource::{CleanupResourceResult, StorageResourceEntry, StorageResourceStatus},
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
                StorageResourceEntry {
                    kind: kind.to_owned(),
                    label: label.to_owned(),
                    path: Some(path.to_string_lossy().into_owned()),
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
    if fs::symlink_metadata(cache_root).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(AppError::Validation(format!(
            "refusing to clean a symlinked capture cache root: {}",
            cache_root.display()
        )));
    }
    fs::create_dir_all(cache_root)?;
    let rows = sqlx::query_as::<_, (String, Option<String>, Option<String>, Option<String>)>(
        r#"
        SELECT id, avatar_path, destination_path, destination_avatar_path
        FROM capture_items
        WHERE status = 'completed'
          AND archived_at IS NOT NULL
          AND destination_path IS NOT NULL
        "#,
    )
    .fetch_all(pool)
    .await?;

    let mut removed = DiskUsage::default();
    for (id, avatar_path, destination_path, destination_avatar_path) in rows {
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

    Ok(CleanupResourceResult {
        kind: "capture_output".to_owned(),
        removed_files: removed.files,
        reclaimed_bytes: removed.bytes,
        message: "已将文件引用切换到归档位置，并清理本地处理缓存。".to_owned(),
    })
}

#[cfg(test)]
#[path = "resource_storage_service_tests.rs"]
mod tests;
