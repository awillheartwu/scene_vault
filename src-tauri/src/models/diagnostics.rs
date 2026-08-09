use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Debug,
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

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LogRecord {
    pub timestamp: String,
    pub level: LogLevel,
    pub module: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogQuery {
    pub since: Option<String>,
    pub until: Option<String>,
    #[serde(default)]
    pub levels: Vec<LogLevel>,
    pub module: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogQueryResult {
    pub records: Vec<LogRecord>,
    pub matched_count: usize,
    pub truncated: bool,
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
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogCleanupResult {
    pub removed_files: usize,
    pub status: LogStatus,
}
