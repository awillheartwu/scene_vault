use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, RwLock};
use std::time::SystemTime;

use tokio::sync::Semaphore;

use crate::{
    error::AppError,
    models::diagnostics::{LogLevel, LogRecord},
    services::log_service,
};

/// Longest edge of generated thumbnails. 320px keeps text in annotated views
/// barely readable while staying a few KB as JPEG.
const MAX_DIMENSION: u32 = 320;
const JPEG_QUALITY: u8 = 80;

/// Run the size maintenance only every N writes to avoid scanning the
/// directory on every thumbnail.
const MAINTENANCE_INTERVAL: u64 = 64;

static WRITE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Bounds concurrent thumbnail decoding so a page of placeholders cannot
/// spike CPU/RAM (each decode is a ~30MB buffer for a 3.6K screenshot).
/// The limit is configurable through app settings; changing it swaps in a new
/// semaphore so in-flight permits never need to be revoked.
struct GenerationLimiter {
    configured_limit: u32,
    semaphore: Arc<Semaphore>,
}

impl GenerationLimiter {
    fn new(limit: u32) -> Self {
        Self {
            configured_limit: limit,
            semaphore: Arc::new(Semaphore::new(limit as usize)),
        }
    }

    fn semaphore(&self) -> Arc<Semaphore> {
        Arc::clone(&self.semaphore)
    }

    fn set_limit(&mut self, limit: u32) {
        if self.configured_limit == limit {
            return;
        }
        self.configured_limit = limit;
        self.semaphore = Arc::new(Semaphore::new(limit as usize));
    }
}

static GENERATION_LIMIT: LazyLock<RwLock<GenerationLimiter>> =
    LazyLock::new(|| RwLock::new(GenerationLimiter::new(1)));

fn generation_limit() -> Arc<Semaphore> {
    GENERATION_LIMIT.read().expect("limit lock").semaphore()
}

/// Updates how many thumbnail decodes may run in parallel. The new limit is
/// fixed at one so imported screenshots decode strictly one by one. Tasks
/// that already hold permits from the previous
/// semaphore finish naturally and do not count against the new limit.
pub fn set_generation_limit(_limit: u32) {
    let limit = 1;
    let mut guard = GENERATION_LIMIT.write().expect("limit lock");
    guard.set_limit(limit);
}

pub fn thumbnail_cache_dir(root: &Path) -> PathBuf {
    root.join("thumbnails")
}

pub fn cache_path(root: &Path, item_id: &str, variant: &str) -> PathBuf {
    thumbnail_cache_dir(root).join(format!("{item_id}-{variant}.jpg"))
}

/// Returns cached thumbnail bytes, generating and caching them on first use.
pub async fn read_thumbnail(
    root: &Path,
    item_id: &str,
    variant: &str,
    source: &Path,
    cache_limit_bytes: u64,
) -> Result<Vec<u8>, AppError> {
    let cache = cache_path(root, item_id, variant);
    if let Ok(bytes) = tokio::fs::read(&cache).await {
        return Ok(bytes);
    }

    let bytes = match generate(source).await {
        Ok(bytes) => bytes,
        Err(error) => {
            log_service::record_event(LogRecord {
                level: LogLevel::Warn,
                module: "thumbnail.cache".to_owned(),
                message: "thumbnail generation failed".to_owned(),
                event: Some("generation_failed".to_owned()),
                capture_item_id: Some(item_id.to_owned()),
                outcome: Some("failed".to_owned()),
                error_code: Some("thumbnail_generation_error".to_owned()),
                ..Default::default()
            });
            return Err(error);
        }
    };
    if let Some(parent) = cache.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(&cache, &bytes).await?;

    let writes = WRITE_COUNTER.fetch_add(1, Ordering::Relaxed) + 1;
    if writes.is_multiple_of(MAINTENANCE_INTERVAL) {
        enforce_cache_limit(&thumbnail_cache_dir(root), cache_limit_bytes);
    }
    Ok(bytes)
}

/// One-time relocation: earlier builds kept the thumbnail cache next to the
/// database (app data / Roaming); caches now live in the Local app directory
/// together with the other intermediate/cache folders.
pub fn migrate_legacy_cache(old_root: &Path, new_root: &Path) {
    let old_dir = thumbnail_cache_dir(old_root);
    let new_dir = thumbnail_cache_dir(new_root);
    if !old_dir.is_dir() {
        return;
    }
    if std::fs::create_dir_all(&new_dir).is_err() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(&old_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name() else {
            continue;
        };
        let _ = std::fs::rename(&path, new_dir.join(name));
    }
    let _ = std::fs::remove_dir(&old_dir);
}

/// Decodes the source image and encodes a small JPEG thumbnail.
/// Decoding is CPU bound, so it runs on the blocking pool.
async fn generate(source: &Path) -> Result<Vec<u8>, AppError> {
    let semaphore = generation_limit();
    let _permit = semaphore
        .acquire()
        .await
        .map_err(|_| AppError::Image("thumbnail generation limit closed".to_owned()))?;
    let source = source.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let image = image::open(&source).map_err(|error| {
            AppError::Image(format!("cannot decode {}: {error}", source.display()))
        })?;
        let thumb = image.thumbnail(MAX_DIMENSION, MAX_DIMENSION);
        let mut bytes = Vec::new();
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, JPEG_QUALITY);
        thumb
            .write_with_encoder(encoder)
            .map_err(|error| AppError::Image(format!("cannot encode thumbnail: {error}")))?;
        Ok(bytes)
    })
    .await
    .map_err(|error| AppError::Image(format!("thumbnail task failed: {error}")))?
}

/// Evicts the oldest cache files until the directory fits the budget.
/// Best-effort: failures are ignored so a broken cache never blocks reads.
pub fn enforce_cache_limit(dir: &Path, limit_bytes: u64) {
    let evict_to = (limit_bytes as u128 * 3 / 4) as u64;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(SystemTime, PathBuf)> = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            if !path.is_file() {
                return None;
            }
            let modified = entry
                .metadata()
                .ok()?
                .modified()
                .unwrap_or(SystemTime::UNIX_EPOCH);
            Some((modified, path))
        })
        .collect();
    let total: u64 = files
        .iter()
        .filter_map(|(_, path)| std::fs::metadata(path).ok().map(|meta| meta.len()))
        .sum();
    if total <= limit_bytes {
        return;
    }
    files.sort_by_key(|(modified, _)| *modified);
    let mut remaining = total;
    let mut removed_files = 0_u32;
    for (_, path) in files {
        if remaining <= evict_to {
            break;
        }
        let bytes = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
        if std::fs::remove_file(path).is_ok() {
            remaining = remaining.saturating_sub(bytes);
            removed_files = removed_files.saturating_add(1);
        }
    }
    if removed_files > 0 {
        log_service::record_event(LogRecord {
            level: LogLevel::Info,
            module: "thumbnail.cache".to_owned(),
            message: format!(
                "evicted {removed_files} thumbnail files and reclaimed {} bytes",
                total.saturating_sub(remaining)
            ),
            event: Some("eviction_completed".to_owned()),
            outcome: Some("succeeded".to_owned()),
            ..Default::default()
        });
    }
}

/// Total bytes currently stored in the thumbnail cache directory.
pub fn cache_size(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.metadata().ok())
        .map(|meta| meta.len())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageBuffer;
    use tempfile::tempdir;

    fn write_test_png(path: &Path, width: u32, height: u32) {
        let buffer: ImageBuffer<image::Rgba<u8>, Vec<u8>> =
            ImageBuffer::from_fn(width, height, |x, y| {
                image::Rgba([(x % 255) as u8, (y % 255) as u8, 128, 255])
            });
        buffer.save(path).expect("save test png");
    }

    #[tokio::test]
    async fn generates_and_caches_thumbnail() {
        let dir = tempdir().expect("temp dir");
        let source = dir.path().join("source.png");
        write_test_png(&source, 1200, 800);
        let app_data = dir.path().join("app-data");

        let first = read_thumbnail(&app_data, "item-1", "source", &source, 10 * 1024 * 1024)
            .await
            .expect("first read");
        let second = read_thumbnail(&app_data, "item-1", "source", &source, 10 * 1024 * 1024)
            .await
            .expect("cached read");

        assert_eq!(first, second);
        assert!(!first.is_empty());
        let image = image::load_from_memory(&first).expect("valid jpeg");
        assert_eq!(image.width(), 320);
        assert_eq!(image.height(), 213);
        assert!(cache_path(&app_data, "item-1", "source").is_file());
    }

    #[tokio::test]
    async fn missing_source_is_a_not_found_error() {
        let dir = tempdir().expect("temp dir");
        let error = read_thumbnail(
            dir.path(),
            "item-1",
            "source",
            &dir.path().join("missing.png"),
            10 * 1024 * 1024,
        )
        .await
        .expect_err("missing source");
        assert!(matches!(error, AppError::Image(_)));
    }

    #[test]
    fn cache_limit_evicts_oldest_files() {
        let dir = tempdir().expect("temp dir");
        let old = dir.path().join("old.jpg");
        let fresh = dir.path().join("fresh.jpg");
        std::fs::write(&old, vec![0u8; 50]).expect("old file");
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&fresh, vec![0u8; 20]).expect("fresh file");

        enforce_cache_limit(dir.path(), 30);

        assert!(!old.exists());
        assert!(fresh.exists());
    }

    #[test]
    fn migrates_legacy_thumbnails_into_new_root() {
        let dir = tempdir().expect("temp dir");
        let old_root = dir.path().join("roaming");
        let new_root = dir.path().join("local");
        let old_dir = thumbnail_cache_dir(&old_root);
        std::fs::create_dir_all(&old_dir).expect("old dir");
        std::fs::write(old_dir.join("a.jpg"), b"a").expect("a");
        std::fs::write(old_dir.join("b.jpg"), b"b").expect("b");

        migrate_legacy_cache(&old_root, &new_root);

        assert!(thumbnail_cache_dir(&new_root).join("a.jpg").is_file());
        assert!(thumbnail_cache_dir(&new_root).join("b.jpg").is_file());
        assert!(!old_dir.exists());
    }

    #[tokio::test]
    async fn generation_limit_gates_concurrent_decodes() {
        // Keep this test independent from the process-wide limiter, which may
        // be updated by settings-related tests running in parallel.
        let mut limiter = GenerationLimiter::new(4);
        limiter.set_limit(1);
        let semaphore = limiter.semaphore();
        let first = semaphore.acquire().await.expect("first permit");
        let waiting = semaphore.clone();
        let second =
            tokio::time::timeout(std::time::Duration::from_millis(50), waiting.acquire()).await;
        assert!(second.is_err(), "second decode must wait for the slot");
        drop(first);
        let waiting = semaphore.clone();
        let third =
            tokio::time::timeout(std::time::Duration::from_millis(50), waiting.acquire()).await;
        assert!(third.is_ok(), "slot is released after the task finishes");
    }
}
