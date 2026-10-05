use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Notify;

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
struct GenerationLimiter {
    state: std::sync::Mutex<(usize, usize)>, // (active, limit)
    changed: Notify,
}

struct GenerationPermit(Arc<GenerationLimiter>);

impl Drop for GenerationPermit {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        state.0 -= 1;
        drop(state);
        self.0.changed.notify_waiters();
    }
}

impl GenerationLimiter {
    fn new(limit: u32) -> Self {
        Self {
            state: std::sync::Mutex::new((0, limit.max(1) as usize)),
            changed: Notify::new(),
        }
    }

    fn try_acquire(self: &Arc<Self>) -> Option<GenerationPermit> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.0 >= state.1 {
            return None;
        }
        state.0 += 1;
        Some(GenerationPermit(Arc::clone(self)))
    }

    async fn acquire(self: &Arc<Self>) -> GenerationPermit {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            // Register before checking to avoid missing a release/resize.
            changed.as_mut().enable();
            if let Some(permit) = self.try_acquire() {
                return permit;
            }
            changed.await;
        }
    }

    fn set_limit(&self, limit: u32) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.1 = limit.max(1) as usize;
        drop(state);
        // Existing work retains its slot. After a reduction, new work waits
        // until active work drains below the new bound.
        self.changed.notify_waiters();
    }
}

static GENERATION_LIMIT: LazyLock<Arc<GenerationLimiter>> =
    LazyLock::new(|| Arc::new(GenerationLimiter::new(1)));

fn generation_limit() -> Arc<GenerationLimiter> {
    Arc::clone(&GENERATION_LIMIT)
}

/// Parallel thumbnail decodes best kept to a small multiple of the user's
/// image-processing limit: one permit per generation makes a grid of fresh
/// thumbnails load strictly one after another (seconds of waiting), while a
/// permit per core would spike memory with 30 MB decodes.
pub fn thumbnail_generation_limit(core_limit: u32) -> u32 {
    core_limit.clamp(1, 4)
}

/// Applies the user's image-processing core limit to thumbnail decoding.
pub fn set_processing_parallelism(core_limit: u32) {
    GENERATION_LIMIT.set_limit(thumbnail_generation_limit(core_limit));
}

pub fn thumbnail_cache_dir(root: &Path) -> PathBuf {
    root.join("thumbnails")
}

pub fn cache_path(root: &Path, item_id: &str, variant: &str) -> PathBuf {
    thumbnail_cache_dir(root).join(format!("{item_id}-{variant}.jpg"))
}

/// Only the lifecycle that produces this variant can invalidate it. Original
/// images are tracked by their source fingerprint, never by archive timestamps.
pub fn capture_generation(
    variant: &str,
    processing_version: i64,
    processed_at: Option<&str>,
    archived_at: Option<&str>,
) -> String {
    match variant {
        "source" => "source".to_owned(),
        "destination" => format!("archive:{}", archived_at.unwrap_or("")),
        _ => format!(
            "derived:{processing_version}:{}",
            processed_at.unwrap_or("")
        ),
    }
}

/// Bounded source identity: stat plus three 4 KiB samples. Processing callers
/// can additionally supply their durable generation to cover metadata-preserving
/// rewrites outside the sampled regions without hashing full screenshots on hits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SourceFingerprint {
    path: PathBuf,
    size: u64,
    modified: SystemTime,
    created: Option<SystemTime>,
    change_identity: String,
    samples: Vec<u8>,
    generation: String,
}

#[derive(Serialize, Deserialize)]
struct CacheMetadata {
    // v2 is written only after decoding this source. v1 could adopt unrelated
    // legacy JPEGs by timestamp and cannot be treated as proven source identity.
    #[serde(default)]
    version: u32,
    source: SourceFingerprint,
    thumbnail_sha256: Vec<u8>,
}

fn fingerprint(source: &Path, generation: &str) -> Result<SourceFingerprint, AppError> {
    let mut file = std::fs::File::open(source)
        .map_err(|error| AppError::Image(format!("cannot open {}: {error}", source.display())))?;
    let metadata = file.metadata()?;
    #[cfg(unix)]
    let change_identity = {
        use std::os::unix::fs::MetadataExt;
        format!(
            "{}:{}:{}:{}",
            metadata.dev(),
            metadata.ino(),
            metadata.ctime(),
            metadata.ctime_nsec()
        )
    };
    #[cfg(not(unix))]
    let change_identity = String::new();
    let mut hash = Sha256::new();
    let mut sample = [0_u8; 4096];
    let length = metadata.len().min(sample.len() as u64) as usize;
    for offset in [
        0,
        metadata.len().saturating_sub(length as u64) / 2,
        metadata.len().saturating_sub(length as u64),
    ] {
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(&mut sample[..length])?;
        hash.update(&sample[..length]);
    }
    Ok(SourceFingerprint {
        path: source.to_path_buf(),
        size: metadata.len(),
        modified: metadata.modified()?,
        created: metadata.created().ok(),
        change_identity,
        samples: hash.finalize().to_vec(),
        generation: generation.to_owned(),
    })
}

/// Pass the capture processing_version (and source/derived variant identity)
/// here when available. Generation must change whenever derived files change.
pub async fn read_thumbnail_with_generation(
    root: &Path,
    item_id: &str,
    variant: &str,
    source: &Path,
    cache_limit_bytes: u64,
    generation: &str,
) -> Result<Vec<u8>, AppError> {
    // Hold the permit through publication, including if the async caller is
    // cancelled while the blocking decoder finishes. No late writer can pair
    // an old JPEG with another request's metadata.
    let permit = generation_limit().acquire().await;
    let cache = cache_path(root, item_id, variant);
    let source = source.to_path_buf();
    let generation = generation.to_owned();
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        read_cached(&cache, &source, &generation, cache_limit_bytes, generate)
    })
    .await
    .map_err(|error| AppError::Image(format!("thumbnail task failed: {error}")))?;
    if result.is_err() {
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
    }
    result
}

fn read_cached(
    cache: &Path,
    source: &Path,
    generation: &str,
    cache_limit_bytes: u64,
    decode: impl Fn(&Path) -> Result<Vec<u8>, AppError>,
) -> Result<Vec<u8>, AppError> {
    let sidecar = cache.with_extension("json");
    for _ in 0..3 {
        let before = fingerprint(source, generation)?;
        if let Ok(metadata) = std::fs::read(&sidecar) {
            if let Ok(metadata) = serde_json::from_slice::<CacheMetadata>(&metadata) {
                if metadata.version == 2 && metadata.source == before {
                    if let Ok(bytes) = std::fs::read(cache) {
                        // Also rejects interrupted/cross-process two-file publication.
                        if Sha256::digest(&bytes).to_vec() == metadata.thumbnail_sha256
                            && fingerprint(source, generation)? == before
                        {
                            return Ok(bytes);
                        }
                    }
                }
            }
        }
        // Unproven old caches are regenerated lazily on first visible use.
        // Neither timestamps nor an empty generation prove source association.
        let bytes = decode(source)?;
        if fingerprint(source, generation)? != before {
            continue;
        }
        let metadata = CacheMetadata {
            version: 2,
            source: before,
            thumbnail_sha256: Sha256::digest(&bytes).to_vec(),
        };
        let parent = cache.parent().expect("thumbnail has a cache directory");
        std::fs::create_dir_all(parent)?;
        let temporary = cache.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        let temporary_metadata = temporary.with_extension("json.tmp");
        let publish = (|| -> Result<(), AppError> {
            std::fs::write(&temporary, &bytes)?;
            std::fs::write(
                &temporary_metadata,
                serde_json::to_vec(&metadata).map_err(|error| {
                    AppError::Image(format!("cannot encode thumbnail metadata: {error}"))
                })?,
            )?;
            std::fs::rename(&temporary, cache)?;
            std::fs::rename(&temporary_metadata, &sidecar)?;
            Ok(())
        })();
        let _ = std::fs::remove_file(&temporary);
        let _ = std::fs::remove_file(&temporary_metadata);
        publish?;
        if fingerprint(source, generation)? != metadata.source {
            continue;
        }
        let writes = WRITE_COUNTER.fetch_add(1, Ordering::Relaxed) + 1;
        if writes.is_multiple_of(MAINTENANCE_INTERVAL) {
            enforce_cache_limit(parent, cache_limit_bytes);
        }
        return Ok(bytes);
    }
    Err(AppError::Image(
        "thumbnail source changed repeatedly during generation".to_owned(),
    ))
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
/// Called on the blocking pool while holding the generation permit.
fn generate(source: &Path) -> Result<Vec<u8>, AppError> {
    // Sniff the format from the file content: some captures carry a
    // misleading extension (a PNG named .jpg), which extension-based
    // decoding rejects even though every other reader accepts the file.
    let image = image::ImageReader::open(source)
        .map_err(|error| AppError::Image(format!("cannot open {}: {error}", source.display())))?
        .with_guessed_format()
        .map_err(|error| AppError::Image(format!("cannot sniff {}: {error}", source.display())))?
        .decode()
        .map_err(|error| AppError::Image(format!("cannot decode {}: {error}", source.display())))?;
    let thumb = image.thumbnail(MAX_DIMENSION, MAX_DIMENSION);
    let mut bytes = Vec::new();
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, JPEG_QUALITY);
    thumb
        .write_with_encoder(encoder)
        .map_err(|error| AppError::Image(format!("cannot encode thumbnail: {error}")))?;
    Ok(bytes)
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
            // Reset/deletion in older callers only removes the deterministic
            // JPEG. Sweep its orphan sidecar even below the byte budget.
            if path
                .extension()
                .is_some_and(|extension| extension == "json")
                && !path.with_extension("jpg").is_file()
            {
                let _ = std::fs::remove_file(&path);
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
        if std::fs::remove_file(&path).is_ok() {
            remaining = remaining.saturating_sub(bytes);
            removed_files = removed_files.saturating_add(1);
            if path.extension().is_some_and(|extension| extension == "jpg") {
                let sidecar = path.with_extension("json");
                let sidecar_bytes = std::fs::metadata(&sidecar)
                    .map(|meta| meta.len())
                    .unwrap_or(0);
                if std::fs::remove_file(sidecar).is_ok() {
                    remaining = remaining.saturating_sub(sidecar_bytes);
                    removed_files = removed_files.saturating_add(1);
                }
            }
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

    /// Test-local convenience wrapper: no durable generation hint.
    async fn read_thumbnail(
        root: &Path,
        item_id: &str,
        variant: &str,
        source: &Path,
        cache_limit_bytes: u64,
    ) -> Result<Vec<u8>, AppError> {
        read_thumbnail_with_generation(root, item_id, variant, source, cache_limit_bytes, "").await
    }

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

    #[test]
    fn original_cache_survives_reprocessing_and_archiving_other_variants() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source.png");
        write_test_png(&source, 80, 40);
        let cache = cache_path(dir.path(), "item", "source");
        let first = capture_generation("source", 0, None, None);
        let after = capture_generation("source", 3, Some("processed"), Some("archived"));
        let expected = read_cached(&cache, &source, &first, u64::MAX, generate).unwrap();
        assert_eq!(
            read_cached(&cache, &source, &after, u64::MAX, |_| panic!(
                "unchanged original must hit"
            ))
            .unwrap(),
            expected
        );
        assert_eq!(
            capture_generation("avatar", 1, Some("processed"), None),
            capture_generation("avatar", 1, Some("processed"), Some("archived"))
        );
        assert_ne!(
            capture_generation("avatar", 1, Some("old"), None),
            capture_generation("avatar", 1, Some("new"), None)
        );
        assert_ne!(
            capture_generation("destination", 1, None, Some("old")),
            capture_generation("destination", 1, None, Some("new"))
        );
    }

    #[tokio::test]
    async fn replaced_derived_file_at_same_path_regenerates() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("avatar.png");
        write_test_png(&source, 400, 200);
        let first = read_thumbnail(dir.path(), "item", "avatar", &source, u64::MAX)
            .await
            .unwrap();
        // Atomic replacement can preserve size/mtime. Sample content and file
        // identity supplement timestamps; normal in-place writes work as well.
        let replacement = dir.path().join("replacement.png");
        image::RgbImage::from_pixel(400, 200, image::Rgb([240, 20, 30]))
            .save(&replacement)
            .unwrap();
        let original_time = std::fs::metadata(&source).unwrap().modified().unwrap();
        std::fs::File::options()
            .write(true)
            .open(&replacement)
            .unwrap()
            .set_modified(original_time)
            .unwrap();
        std::fs::rename(&replacement, &source).unwrap();
        let second = read_thumbnail(dir.path(), "item", "avatar", &source, u64::MAX)
            .await
            .unwrap();
        assert_ne!(first, second);
        assert_eq!(second, generate(&source).unwrap());
    }

    #[test]
    fn thumbnail_parallelism_follows_the_core_limit_with_bounds() {
        assert_eq!(thumbnail_generation_limit(1), 1);
        assert_eq!(thumbnail_generation_limit(3), 3);
        assert_eq!(thumbnail_generation_limit(8), 4);
    }

    #[test]
    fn raising_the_limit_allows_parallel_decodes() {
        let limiter = Arc::new(GenerationLimiter::new(1));
        limiter.set_limit(3);
        let semaphore = limiter.clone();
        let _first = semaphore.try_acquire().expect("permit 1");
        let _second = semaphore.try_acquire().expect("permit 2");
        let _third = semaphore.try_acquire().expect("permit 3");
        assert!(
            semaphore.try_acquire().is_none(),
            "the new limit is enforced"
        );
    }

    #[test]
    fn legacy_cache_cannot_bless_a_replaced_source_with_an_old_timestamp() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("avatar.png");
        write_test_png(&source, 80, 40);
        let old_time = std::fs::metadata(&source).unwrap().modified().unwrap();
        let cache = cache_path(dir.path(), "item", "avatar");
        std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
        let old_bytes = generate(&source).unwrap();
        std::fs::write(&cache, &old_bytes).unwrap();
        image::RgbImage::from_pixel(80, 40, image::Rgb([240, 10, 10]))
            .save(&source)
            .unwrap();
        std::fs::File::options()
            .write(true)
            .open(&source)
            .unwrap()
            .set_modified(old_time)
            .unwrap();
        let fresh = read_cached(&cache, &source, "new", u64::MAX, generate).unwrap();
        assert_ne!(fresh, old_bytes);
        assert_eq!(fresh, generate(&source).unwrap());
        assert_eq!(
            read_cached(&cache, &source, "new", u64::MAX, |_| panic!(
                "verified cache should hit"
            ))
            .unwrap(),
            fresh
        );
    }

    #[tokio::test]
    async fn resizing_counts_existing_work_and_cancelled_waiters() {
        use std::time::Duration;
        let limiter = Arc::new(GenerationLimiter::new(4));
        let mut permits: Vec<_> = (0..4).map(|_| limiter.try_acquire().unwrap()).collect();
        limiter.set_limit(4);
        assert!(limiter.try_acquire().is_none());
        limiter.set_limit(1);
        permits.truncate(1);
        assert!(
            tokio::time::timeout(Duration::from_millis(20), limiter.acquire())
                .await
                .is_err()
        );
        permits.clear();
        let first = limiter.try_acquire().unwrap();
        assert!(limiter.try_acquire().is_none());
        limiter.set_limit(2);
        let second = limiter.acquire().await;
        assert!(limiter.try_acquire().is_none());
        drop((first, second));
        assert_eq!(limiter.state.lock().unwrap().0, 0);
    }

    #[test]
    fn previously_adopted_sidecar_is_revalidated_once() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("avatar.png");
        write_test_png(&source, 80, 40);
        let cache = cache_path(dir.path(), "item", "avatar");
        read_cached(&cache, &source, "new", u64::MAX, generate).unwrap();
        let sidecar = cache.with_extension("json");
        let mut metadata: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&sidecar).unwrap()).unwrap();
        metadata.as_object_mut().unwrap().remove("version");
        std::fs::write(&sidecar, serde_json::to_vec(&metadata).unwrap()).unwrap();
        let calls = std::cell::Cell::new(0);
        let expected = read_cached(&cache, &source, "new", u64::MAX, |path| {
            calls.set(calls.get() + 1);
            generate(path)
        })
        .unwrap();
        assert_eq!(calls.get(), 1);
        assert_eq!(
            read_cached(&cache, &source, "new", u64::MAX, |_| panic!(
                "must not repeatedly regenerate"
            ))
            .unwrap(),
            expected
        );
    }

    #[test]
    fn legacy_mismatched_and_new_generation_caches_regenerate() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("avatar.png");
        write_test_png(&source, 80, 40);
        let cache = cache_path(dir.path(), "item", "avatar");
        std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
        std::fs::write(&cache, b"legacy JPEG without metadata").unwrap();
        let fresh = read_cached(&cache, &source, "1", u64::MAX, generate).unwrap();
        assert_eq!(fresh, generate(&source).unwrap());
        assert_eq!(
            read_cached(&cache, &source, "1", u64::MAX, |_| panic!(
                "cache hit must not decode"
            ))
            .unwrap(),
            fresh
        );
        std::fs::write(&cache, b"JPEG from interrupted publication").unwrap();
        assert_eq!(
            read_cached(&cache, &source, "1", u64::MAX, generate).unwrap(),
            fresh
        );
        let decoded = std::cell::Cell::new(false);
        read_cached(&cache, &source, "2", u64::MAX, |path| {
            decoded.set(true);
            generate(path)
        })
        .unwrap();
        assert!(
            decoded.get(),
            "a new processing generation must miss even with unchanged metadata"
        );
    }

    #[test]
    fn source_changed_during_decode_cannot_publish_stale_thumbnail() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("avatar.png");
        write_test_png(&source, 80, 40);
        let cache = cache_path(dir.path(), "item", "avatar");
        let calls = std::cell::Cell::new(0);
        let result = read_cached(&cache, &source, "1", u64::MAX, |path| {
            let bytes = generate(path)?;
            calls.set(calls.get() + 1);
            if calls.get() == 1 {
                write_test_png(path, 40, 80);
            }
            Ok(bytes)
        })
        .unwrap();
        assert_eq!(calls.get(), 2);
        assert_eq!(result, generate(&source).unwrap());
        assert_eq!(
            result,
            read_cached(&cache, &source, "1", u64::MAX, |_| panic!(
                "fresh cache expected"
            ))
            .unwrap()
        );
    }

    #[test]
    fn continuously_changing_source_has_bounded_retries() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("avatar.png");
        write_test_png(&source, 80, 40);
        let cache = cache_path(dir.path(), "item", "avatar");
        let calls = std::cell::Cell::new(0);
        let result = read_cached(&cache, &source, "1", u64::MAX, |path| {
            let bytes = generate(path)?;
            calls.set(calls.get() + 1);
            write_test_png(path, 80 + calls.get(), 40);
            Ok(bytes)
        });
        assert!(result.is_err());
        assert_eq!(calls.get(), 3);
        assert!(!cache.exists());
    }

    #[tokio::test]
    async fn generates_a_thumbnail_when_the_extension_lies() {
        let dir = tempdir().expect("temp dir");
        let png_source = dir.path().join("source.png");
        write_test_png(&png_source, 400, 200);
        // A PNG saved under a .jpg name: extension-based decoding failed on it.
        let source = dir.path().join("mismatched.jpg");
        tokio::fs::copy(&png_source, &source)
            .await
            .expect("copy source");
        let app_data = dir.path().join("app-data");

        let bytes = read_thumbnail(&app_data, "item-2", "source", &source, 10 * 1024 * 1024)
            .await
            .expect("thumbnail from mismatched extension");

        let image = image::load_from_memory(&bytes).expect("valid jpeg");
        assert_eq!(image.width(), 320);
        assert_eq!(image.height(), 160);
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
    fn cache_maintenance_cleans_orphan_metadata_below_budget() {
        let dir = tempdir().unwrap();
        let orphan = dir.path().join("deleted-avatar.json");
        let cache = dir.path().join("kept-avatar.jpg");
        std::fs::write(&orphan, b"orphan").unwrap();
        std::fs::write(&cache, b"jpeg").unwrap();
        std::fs::write(cache.with_extension("json"), b"metadata").unwrap();
        enforce_cache_limit(dir.path(), u64::MAX);
        assert!(!orphan.exists());
        assert!(cache.with_extension("json").exists());
        enforce_cache_limit(dir.path(), 0);
        assert!(!cache.exists());
        assert!(!cache.with_extension("json").exists());
    }

    #[test]
    fn fingerprint_samples_detect_same_size_and_mtime_rewrites() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source.bin");
        std::fs::write(&source, vec![0_u8; 20_000]).unwrap();
        let before = fingerprint(&source, "1").unwrap();
        std::fs::write(&source, vec![1_u8; 20_000]).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&source)
            .unwrap()
            .set_modified(before.modified)
            .unwrap();
        let after = fingerprint(&source, "1").unwrap();
        assert_eq!(before.size, after.size);
        assert_eq!(before.modified, after.modified);
        assert_ne!(before.samples, after.samples);
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
        let limiter = Arc::new(GenerationLimiter::new(1));
        let semaphore = limiter.clone();
        let first = semaphore.acquire().await;
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
