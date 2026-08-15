use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Debug,
    #[default]
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LogRecord {
    pub timestamp: String,
    pub level: LogLevel,
    pub module: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_item_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogQuery {
    pub since: Option<String>,
    pub until: Option<String>,
    #[serde(default)]
    pub levels: Vec<LogLevel>,
    pub module: Option<String>,
    pub event: Option<String>,
    pub correlation_id: Option<String>,
    pub outcome: Option<String>,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientLogEventInput {
    pub level: LogLevel,
    pub module: String,
    pub event: String,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub operation_id: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<f64>,
    #[serde(default)]
    pub outcome: Option<String>,
    #[serde(default)]
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogQueryResult {
    pub records: Vec<LogRecord>,
    pub matched_count: usize,
    pub truncated: bool,
    pub offset: usize,
    pub next_offset: Option<usize>,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogStatus {
    pub directory: String,
    pub file_count: usize,
    pub total_bytes: u64,
    pub retention_days: i64,
    pub max_file_bytes: u64,
    pub max_archived_files: usize,
    pub dropped_records: u64,
    pub automatic_cleanup: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LogPolicySettings {
    #[serde(default = "default_log_retention_days")]
    pub retention_days: i64,
    #[serde(default = "default_log_max_file_size_mb")]
    pub max_file_size_mb: u64,
    #[serde(default = "default_log_max_archived_files")]
    pub max_archived_files: usize,
    #[serde(default = "default_true")]
    pub automatic_cleanup: bool,
}

impl Default for LogPolicySettings {
    fn default() -> Self {
        Self {
            retention_days: default_log_retention_days(),
            max_file_size_mb: default_log_max_file_size_mb(),
            max_archived_files: default_log_max_archived_files(),
            automatic_cleanup: true,
        }
    }
}

fn default_log_retention_days() -> i64 {
    14
}
fn default_log_max_file_size_mb() -> u64 {
    5
}
fn default_log_max_archived_files() -> usize {
    20
}
fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogCleanupResult {
    pub removed_files: usize,
    pub status: LogStatus,
}
