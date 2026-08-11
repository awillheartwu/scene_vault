use std::{
    cmp::Reverse,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, SyncSender, TrySendError},
        Arc, Mutex, OnceLock,
    },
    time::SystemTime,
};

use chrono::{DateTime, Duration, SecondsFormat, Utc};

use crate::{
    error::AppError,
    models::diagnostics::{
        LogCleanupResult, LogLevel, LogPolicySettings, LogQuery, LogQueryResult, LogRecord,
        LogStatus,
    },
};

const CURRENT_LOG_FILE: &str = "scene-vault-current.jsonl";
const LOG_FILE_PREFIX: &str = "scene-vault-";
const LOG_FILE_SUFFIX: &str = ".jsonl";
const DEFAULT_MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;
const DEFAULT_RETENTION_DAYS: i64 = 14;
const DEFAULT_MAX_ARCHIVED_FILES: usize = 20;
const DEFAULT_QUERY_LIMIT: usize = 300;
const MAX_QUERY_LIMIT: usize = 1_000;
const LOG_QUEUE_CAPACITY: usize = 1_024;

static GLOBAL_LOG_SERVICE: OnceLock<GlobalLogService> = OnceLock::new();
static DROPPED_RECORDS: AtomicU64 = AtomicU64::new(0);

struct GlobalLogService {
    store: Arc<LogStore>,
    sender: SyncSender<LogRecord>,
}

#[derive(Debug, Clone)]
struct LogPolicy {
    max_file_bytes: u64,
    retention_days: i64,
    max_archived_files: usize,
    automatic_cleanup: bool,
}

impl Default for LogPolicy {
    fn default() -> Self {
        Self {
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            retention_days: DEFAULT_RETENTION_DAYS,
            max_archived_files: DEFAULT_MAX_ARCHIVED_FILES,
            automatic_cleanup: true,
        }
    }
}

#[derive(Debug)]
pub struct LogStore {
    directory: PathBuf,
    policy: Mutex<LogPolicy>,
    operation_lock: Mutex<()>,
}

impl LogStore {
    fn new(directory: PathBuf, policy: LogPolicy) -> Result<Self, AppError> {
        fs::create_dir_all(&directory)?;
        let store = Self {
            directory,
            policy: Mutex::new(policy),
            operation_lock: Mutex::new(()),
        };
        Ok(store)
    }

    fn current_path(&self) -> PathBuf {
        self.directory.join(CURRENT_LOG_FILE)
    }

    fn append(&self, record: &LogRecord) -> Result<(), AppError> {
        let _guard = self
            .operation_lock
            .lock()
            .map_err(|_| AppError::Io(std::io::Error::other("log writer lock was poisoned")))?;
        self.rotate_if_needed()?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.current_path())?;
        serde_json::to_writer(&mut file, record)
            .map_err(|error| AppError::Io(std::io::Error::other(error)))?;
        file.write_all(b"\n")?;
        Ok(())
    }

    fn rotate_if_needed(&self) -> Result<(), AppError> {
        let current = self.current_path();
        let size = match fs::metadata(&current) {
            Ok(metadata) => metadata.len(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        let policy = self
            .policy
            .lock()
            .map_err(|_| AppError::Io(std::io::Error::other("log policy lock was poisoned")))?
            .clone();
        if size < policy.max_file_bytes {
            return Ok(());
        }

        let stamp = Utc::now().format("%Y%m%d-%H%M%S%.3f");
        let mut archived = self.directory.join(format!("scene-vault-{stamp}.jsonl"));
        let mut suffix = 1_u16;
        while archived.exists() {
            archived = self
                .directory
                .join(format!("scene-vault-{stamp}-{suffix}.jsonl"));
            suffix = suffix.saturating_add(1);
        }
        if let Err(error) = fs::rename(current, archived) {
            // Indexers and antivirus tools can briefly hold the current file
            // on Windows. Keep appending instead of losing all later logs.
            eprintln!("[WARN] [logging] could not rotate current log: {error}");
            return Ok(());
        }
        if policy.automatic_cleanup {
            self.cleanup_unlocked()?;
        }
        Ok(())
    }

    fn log_files(&self) -> Result<Vec<(PathBuf, SystemTime, u64)>, AppError> {
        let mut files = Vec::new();
        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if !name.starts_with(LOG_FILE_PREFIX) || !name.ends_with(LOG_FILE_SUFFIX) {
                continue;
            }
            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            if metadata.is_file() {
                files.push((
                    path,
                    metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                    metadata.len(),
                ));
            }
        }
        files.sort_by_key(|file| Reverse(file.1));
        Ok(files)
    }

    fn cleanup_unlocked(&self) -> Result<usize, AppError> {
        let policy = self
            .policy
            .lock()
            .map_err(|_| AppError::Io(std::io::Error::other("log policy lock was poisoned")))?
            .clone();
        let cutoff = Utc::now() - Duration::days(policy.retention_days);
        let mut removed = 0_usize;
        let mut kept_archives = 0_usize;
        for (path, modified, _) in self.log_files()? {
            if path.file_name().and_then(|value| value.to_str()) == Some(CURRENT_LOG_FILE) {
                continue;
            }
            let modified: DateTime<Utc> = modified.into();
            let expired = modified < cutoff;
            let over_limit = kept_archives >= policy.max_archived_files;
            if expired || over_limit {
                match fs::remove_file(&path) {
                    Ok(()) => removed += 1,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            } else {
                kept_archives += 1;
            }
        }
        Ok(removed)
    }

    fn query(&self, query: LogQuery) -> Result<LogQueryResult, AppError> {
        let _guard = self
            .operation_lock
            .lock()
            .map_err(|_| AppError::Io(std::io::Error::other("log operation lock was poisoned")))?;
        let since = parse_boundary(query.since.as_deref(), "start time")?;
        let until = parse_boundary(query.until.as_deref(), "end time")?;
        if since.zip(until).is_some_and(|(start, end)| start > end) {
            return Err(AppError::Validation(
                "log start time cannot be after end time".to_owned(),
            ));
        }
        let module = query
            .module
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_lowercase);
        let event = normalized_filter(query.event.as_deref());
        let correlation_id = normalized_filter(query.correlation_id.as_deref());
        let outcome = normalized_filter(query.outcome.as_deref());
        let limit = query
            .limit
            .unwrap_or(DEFAULT_QUERY_LIMIT)
            .clamp(1, MAX_QUERY_LIMIT);
        let offset = query.offset.unwrap_or(0).min(1_000_000);
        let keep_count = offset.saturating_add(limit);
        let mut records = Vec::with_capacity(keep_count.saturating_add(1));
        let mut matched_count = 0_usize;

        for (path, _, _) in self.log_files()? {
            let file = match File::open(path) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            for line in BufReader::new(file).lines() {
                let Ok(line) = line else { continue };
                let Ok(record) = serde_json::from_str::<LogRecord>(&line) else {
                    continue;
                };
                let Ok(timestamp) = DateTime::parse_from_rfc3339(&record.timestamp) else {
                    continue;
                };
                let timestamp = timestamp.with_timezone(&Utc);
                if since.is_some_and(|boundary| timestamp < boundary)
                    || until.is_some_and(|boundary| timestamp > boundary)
                    || (!query.levels.is_empty() && !query.levels.contains(&record.level))
                    || module
                        .as_ref()
                        .is_some_and(|needle| !record.module.to_lowercase().contains(needle))
                    || event.as_ref().is_some_and(|needle| {
                        !record
                            .event
                            .as_deref()
                            .unwrap_or_default()
                            .to_lowercase()
                            .contains(needle)
                    })
                    || outcome.as_ref().is_some_and(|needle| {
                        !record
                            .outcome
                            .as_deref()
                            .unwrap_or_default()
                            .to_lowercase()
                            .contains(needle)
                    })
                    || correlation_id
                        .as_ref()
                        .is_some_and(|needle| !record_matches_correlation(&record, needle))
                {
                    continue;
                }
                matched_count = matched_count.saturating_add(1);
                records.push(record);
                if records.len() > keep_count.saturating_mul(2).max(1) {
                    records.sort_by(|left, right| right.timestamp.cmp(&left.timestamp));
                    records.truncate(keep_count);
                }
            }
        }
        records.sort_by(|left, right| right.timestamp.cmp(&left.timestamp));
        records.truncate(keep_count);
        let records = records.into_iter().skip(offset).take(limit).collect();
        let next_offset =
            (offset.saturating_add(limit) < matched_count).then_some(offset.saturating_add(limit));
        Ok(LogQueryResult {
            records,
            matched_count,
            truncated: next_offset.is_some(),
            offset,
            next_offset,
            has_more: next_offset.is_some(),
        })
    }

    fn status(&self) -> Result<LogStatus, AppError> {
        let _guard = self
            .operation_lock
            .lock()
            .map_err(|_| AppError::Io(std::io::Error::other("log operation lock was poisoned")))?;
        self.status_unlocked()
    }

    fn status_unlocked(&self) -> Result<LogStatus, AppError> {
        let files = self.log_files()?;
        let policy = self
            .policy
            .lock()
            .map_err(|_| AppError::Io(std::io::Error::other("log policy lock was poisoned")))?
            .clone();
        Ok(LogStatus {
            directory: self.directory.to_string_lossy().into_owned(),
            file_count: files.len(),
            total_bytes: files.iter().map(|(_, _, bytes)| bytes).sum(),
            retention_days: policy.retention_days,
            max_file_bytes: policy.max_file_bytes,
            max_archived_files: policy.max_archived_files,
            dropped_records: DROPPED_RECORDS.load(Ordering::Relaxed),
            automatic_cleanup: policy.automatic_cleanup,
        })
    }

    fn cleanup_policy(&self) -> Result<usize, AppError> {
        let _guard = self
            .operation_lock
            .lock()
            .map_err(|_| AppError::Io(std::io::Error::other("log operation lock was poisoned")))?;
        self.cleanup_unlocked()
    }

    fn configure(&self, settings: &LogPolicySettings) -> Result<(), AppError> {
        let _guard = self
            .operation_lock
            .lock()
            .map_err(|_| AppError::Io(std::io::Error::other("log operation lock was poisoned")))?;
        {
            let mut policy = self
                .policy
                .lock()
                .map_err(|_| AppError::Io(std::io::Error::other("log policy lock was poisoned")))?;
            *policy = LogPolicy {
                max_file_bytes: settings.max_file_size_mb.saturating_mul(1024 * 1024),
                retention_days: settings.retention_days,
                max_archived_files: settings.max_archived_files,
                automatic_cleanup: settings.automatic_cleanup,
            };
        }
        if settings.automatic_cleanup {
            self.cleanup_unlocked()?;
        }
        Ok(())
    }
}

fn normalized_filter(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_lowercase)
}

fn record_matches_correlation(record: &LogRecord, needle: &str) -> bool {
    [
        record.operation_id.as_deref(),
        record.request_id.as_deref(),
        record.project_id.as_deref(),
        record.session_id.as_deref(),
        record.capture_item_id.as_deref(),
        record.note_id.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|value| value.to_lowercase().contains(needle))
}

fn parse_boundary(value: Option<&str>, label: &str) -> Result<Option<DateTime<Utc>>, AppError> {
    value
        .map(|value| {
            DateTime::parse_from_rfc3339(value)
                .map(|value| value.with_timezone(&Utc))
                .map_err(|_| AppError::Validation(format!("invalid log {label}")))
        })
        .transpose()
}

pub fn initialize(directory: PathBuf) -> Result<(), AppError> {
    let store = Arc::new(LogStore::new(directory, LogPolicy::default())?);
    let (sender, receiver) = mpsc::sync_channel::<LogRecord>(LOG_QUEUE_CAPACITY);
    let writer_store = Arc::clone(&store);
    std::thread::Builder::new()
        .name("scene-vault-log-writer".to_owned())
        .spawn(move || {
            while let Ok(record) = receiver.recv() {
                if let Err(error) = writer_store.append(&record) {
                    eprintln!("[ERROR] [logging] could not persist log record: {error}");
                }
            }
        })?;
    GLOBAL_LOG_SERVICE
        .set(GlobalLogService { store, sender })
        .map_err(|_| AppError::Conflict("log service is already initialized".to_owned()))
}

pub fn record(level: LogLevel, module: impl Into<String>, message: impl Into<String>) {
    let module = module.into();
    let message = message.into();
    eprintln!("[{}] [{module}] {message}", level.as_str().to_uppercase());
    let record = LogRecord {
        timestamp: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        level,
        module,
        message,
        ..Default::default()
    };
    enqueue(record);
}

pub fn record_event(mut record: LogRecord) {
    record.timestamp = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    eprintln!(
        "[{}] [{}] {}",
        record.level.as_str().to_uppercase(),
        record.module,
        record.message
    );
    enqueue(record);
}

fn enqueue(record: LogRecord) {
    let Some(service) = GLOBAL_LOG_SERVICE.get() else {
        return;
    };
    match service.sender.try_send(record) {
        Ok(()) => {}
        Err(TrySendError::Full(_)) => {
            DROPPED_RECORDS.fetch_add(1, Ordering::Relaxed);
            eprintln!("[WARN] [logging] log queue is full; dropping one record")
        }
        Err(TrySendError::Disconnected(_)) => {
            DROPPED_RECORDS.fetch_add(1, Ordering::Relaxed);
            eprintln!("[ERROR] [logging] log writer is unavailable")
        }
    }
}

pub fn debug(module: impl Into<String>, message: impl Into<String>) {
    record(LogLevel::Debug, module, message);
}

pub fn info(module: impl Into<String>, message: impl Into<String>) {
    record(LogLevel::Info, module, message);
}

pub fn warn(module: impl Into<String>, message: impl Into<String>) {
    record(LogLevel::Warn, module, message);
}

pub fn error(module: impl Into<String>, message: impl Into<String>) {
    record(LogLevel::Error, module, message);
}

fn global_store() -> Result<Arc<LogStore>, AppError> {
    GLOBAL_LOG_SERVICE
        .get()
        .map(|service| Arc::clone(&service.store))
        .ok_or_else(|| AppError::Conflict("log service is unavailable".to_owned()))
}

pub fn query(query: LogQuery) -> Result<LogQueryResult, AppError> {
    global_store()?.query(query)
}

pub fn status() -> Result<LogStatus, AppError> {
    global_store()?.status()
}

pub fn configure(settings: &LogPolicySettings) -> Result<(), AppError> {
    global_store()?.configure(settings)
}

pub fn cleanup() -> Result<LogCleanupResult, AppError> {
    let store = global_store()?;
    let removed_files = store.cleanup_policy()?;
    Ok(LogCleanupResult {
        removed_files,
        status: store.status()?,
    })
}

pub fn diagnostic_summary(app_version: &str) -> Result<String, AppError> {
    let store = global_store()?;
    let since = (Utc::now() - Duration::hours(24)).to_rfc3339();
    let recent = store.query(LogQuery {
        since: Some(since),
        levels: vec![LogLevel::Warn, LogLevel::Error],
        limit: Some(200),
        ..Default::default()
    })?;
    let status = store.status()?;
    let mut output = format!(
        "# Scene Vault diagnostic summary\n\nGenerated: {}\nApp version: {}\nPlatform: {} {}\nLog policy: {} days, {} MiB per file, {} archived files\nRecent warnings/errors (24h): {}{}\n",
        Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
        app_version,
        std::env::consts::OS,
        std::env::consts::ARCH,
        status.retention_days,
        status.max_file_bytes / (1024 * 1024),
        status.max_archived_files,
        recent.matched_count,
        if recent.truncated { " (showing newest 200)" } else { "" },
    );
    if recent.records.is_empty() {
        output.push_str("\nNo warnings or errors were recorded in this window.\n");
    } else {
        output.push('\n');
        for record in recent.records.iter().rev() {
            output.push_str(&format!(
                "- {} [{}] [{}] {}\n",
                record.timestamp,
                record.level.as_str().to_uppercase(),
                record.module,
                record.message.replace(['\r', '\n'], " ")
            ));
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn record(timestamp: &str, level: LogLevel, module: &str, message: &str) -> LogRecord {
        LogRecord {
            timestamp: timestamp.to_owned(),
            level,
            module: module.to_owned(),
            message: message.to_owned(),
            ..Default::default()
        }
    }

    #[test]
    fn writes_queries_and_filters_structured_records() {
        let temp = tempdir().unwrap();
        let store = LogStore::new(temp.path().to_path_buf(), LogPolicy::default()).unwrap();
        store
            .append(&record(
                "2026-08-09T01:00:00.000Z",
                LogLevel::Info,
                "capture.worker",
                "started",
            ))
            .unwrap();
        store
            .append(&record(
                "2026-08-09T02:00:00.000Z",
                LogLevel::Error,
                "vision.engine",
                "failed",
            ))
            .unwrap();

        let result = store
            .query(LogQuery {
                since: Some("2026-08-09T01:30:00Z".to_owned()),
                levels: vec![LogLevel::Error],
                module: Some("VISION".to_owned()),
                limit: Some(10),
                ..Default::default()
            })
            .unwrap();

        assert_eq!(result.matched_count, 1);
        assert_eq!(result.records[0].message, "failed");

        let limited = store
            .query(LogQuery {
                limit: Some(1),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(limited.matched_count, 2);
        assert!(limited.truncated);
        assert!(limited.has_more);
        assert_eq!(limited.next_offset, Some(1));
        assert_eq!(limited.records[0].message, "failed");

        let second_page = store
            .query(LogQuery {
                offset: limited.next_offset,
                limit: Some(1),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(second_page.records[0].message, "started");
        assert!(!second_page.has_more);
        assert_eq!(second_page.next_offset, None);
    }

    #[test]
    fn reads_v1_records_and_filters_v2_correlation_fields() {
        let legacy: LogRecord = serde_json::from_str(
            r#"{"timestamp":"2026-08-09T01:00:00Z","level":"info","module":"legacy","message":"old"}"#,
        )
        .unwrap();
        assert!(legacy.event.is_none());

        let temp = tempdir().unwrap();
        let store = LogStore::new(temp.path().to_path_buf(), LogPolicy::default()).unwrap();
        let mut structured = record(
            "2026-08-09T02:00:00.000Z",
            LogLevel::Error,
            "capture.worker",
            "failed",
        );
        structured.event = Some("processing_failed".to_owned());
        structured.capture_item_id = Some("capture-123".to_owned());
        structured.outcome = Some("failed".to_owned());
        store.append(&structured).unwrap();

        let result = store
            .query(LogQuery {
                event: Some("processing".to_owned()),
                correlation_id: Some("CAPTURE-123".to_owned()),
                outcome: Some("fail".to_owned()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(result.matched_count, 1);
        assert_eq!(
            result.records[0].event.as_deref(),
            Some("processing_failed")
        );
    }

    #[test]
    fn rotates_large_files_and_enforces_archive_limit() {
        let temp = tempdir().unwrap();
        let store = LogStore::new(
            temp.path().to_path_buf(),
            LogPolicy {
                max_file_bytes: 1,
                retention_days: 14,
                max_archived_files: 1,
                automatic_cleanup: true,
            },
        )
        .unwrap();
        let sample = record("2026-08-09T01:00:00.000Z", LogLevel::Info, "test", "record");
        store.append(&sample).unwrap();
        store.append(&sample).unwrap();
        store.append(&sample).unwrap();

        let files = store.log_files().unwrap();
        assert!(files.len() <= 2, "current file plus one archive");
    }

    #[test]
    fn rejects_reversed_time_range() {
        let temp = tempdir().unwrap();
        let store = LogStore::new(temp.path().to_path_buf(), LogPolicy::default()).unwrap();
        let error = store
            .query(LogQuery {
                since: Some("2026-08-10T00:00:00Z".to_owned()),
                until: Some("2026-08-09T00:00:00Z".to_owned()),
                ..Default::default()
            })
            .unwrap_err();
        assert!(error.to_string().contains("cannot be after"));
    }
}
