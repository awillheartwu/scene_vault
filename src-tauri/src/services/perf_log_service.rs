//! Performance instrumentation channel.
//!
//! Timing records (image reads, archive resizes, scans, AI requests) go into
//! their own rotating JSONL file under `logs/perf` instead of the debug log,
//! so the main log stays readable and the whole channel can be switched off
//! with one setting without touching the call sites.

use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::SyncSender,
    Arc, OnceLock,
};

use chrono::{SecondsFormat, Utc};

use crate::{
    error::AppError,
    models::diagnostics::{LogLevel, LogPolicySettings, LogRecord},
    services::log_service::{spawn_jsonl_writer, LogPolicy, LogStore},
};

const PERF_DIRECTORY: &str = "perf";
const PERF_CURRENT_FILE: &str = "perf-current.jsonl";
const PERF_FILE_PREFIX: &str = "perf-";

static GLOBAL_PERF_SERVICE: OnceLock<PerfService> = OnceLock::new();
static PERF_ENABLED: AtomicBool = AtomicBool::new(true);

struct PerfService {
    store: Arc<LogStore>,
    sender: SyncSender<LogRecord>,
}

/// Starts the performance channel next to the main log directory.
pub fn initialize(log_directory: PathBuf) -> Result<(), AppError> {
    let store = Arc::new(LogStore::new(
        log_directory.join(PERF_DIRECTORY),
        PERF_FILE_PREFIX,
        PERF_CURRENT_FILE,
        LogPolicy::default(),
    )?);
    let sender = spawn_jsonl_writer(Arc::clone(&store), "scene-vault-perf-writer")?;
    GLOBAL_PERF_SERVICE
        .set(PerfService { store, sender })
        .map_err(|_| AppError::Conflict("perf log service is already initialized".to_owned()))
}

fn set_enabled(enabled: bool) {
    PERF_ENABLED.store(enabled, Ordering::Relaxed);
}

/// Applies the persisted logging policy: the on/off switch plus the same
/// rotation numbers the main log uses.
pub fn configure(settings: &LogPolicySettings) {
    set_enabled(settings.performance_logging);
    if let Some(service) = GLOBAL_PERF_SERVICE.get() {
        if let Err(error) = service.store.configure(settings) {
            eprintln!("[WARN] [perf] could not apply the log policy: {error}");
        }
    }
}

pub fn is_enabled() -> bool {
    PERF_ENABLED.load(Ordering::Relaxed)
}

/// Best-effort perf record: silently skipped while disabled or while the
/// writer queue is full. Perf data must never block or spam the app.
pub fn record_event(mut record: LogRecord) {
    if !is_enabled() {
        return;
    }
    let Some(service) = GLOBAL_PERF_SERVICE.get() else {
        return;
    };
    record.timestamp = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let _ = service.sender.try_send(record);
}

/// Removes expired performance archives; returns how many files were removed.
pub fn cleanup() -> Result<usize, AppError> {
    match GLOBAL_PERF_SERVICE.get() {
        Some(service) => service.store.cleanup_policy(),
        None => Ok(0),
    }
}

/// Bytes currently used by the performance channel (0 while uninitialized).
pub fn total_bytes() -> Result<u64, AppError> {
    match GLOBAL_PERF_SERVICE.get() {
        Some(service) => Ok(service.store.status()?.total_bytes),
        None => Ok(0),
    }
}

pub fn debug(module: impl Into<String>, message: impl Into<String>) {
    record_event(LogRecord {
        level: LogLevel::Debug,
        module: module.into(),
        message: message.into(),
        ..Default::default()
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    use tempfile::tempdir;

    #[test]
    fn records_into_its_own_directory_and_respects_the_switch() {
        let workspace = tempdir().expect("tempdir");
        let log_directory = workspace.path().to_path_buf();
        if GLOBAL_PERF_SERVICE.get().is_none() {
            initialize(log_directory.clone()).expect("initialize");
        }
        set_enabled(true);
        record_event(LogRecord {
            level: LogLevel::Debug,
            module: "perf.test".to_owned(),
            message: "enabled-marker".to_owned(),
            ..Default::default()
        });
        let file = log_directory.join(PERF_DIRECTORY).join(PERF_CURRENT_FILE);
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if std::fs::read_to_string(&file)
                .map(|text| text.contains("enabled-marker"))
                .unwrap_or(false)
            {
                break;
            }
            assert!(Instant::now() < deadline, "perf record was not written");
            std::thread::sleep(Duration::from_millis(25));
        }

        set_enabled(false);
        record_event(LogRecord {
            level: LogLevel::Debug,
            module: "perf.test".to_owned(),
            message: "disabled-marker".to_owned(),
            ..Default::default()
        });
        std::thread::sleep(Duration::from_millis(150));
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        assert!(!text.contains("disabled-marker"));
        assert!(!is_enabled());
        set_enabled(true);
    }
}
