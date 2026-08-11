use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessResourceGroup {
    pub role: String,
    pub process_count: u32,
    pub pids: Vec<u32>,
    pub cpu_percent: Option<f64>,
    pub working_set_bytes: u64,
    pub peak_working_set_bytes: u64,
    pub private_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessResourceStatus {
    pub captured_at: String,
    pub approximate: bool,
    pub logical_processors: u32,
    pub groups: Vec<ProcessResourceGroup>,
    pub total_cpu_percent: Option<f64>,
    pub total_working_set_bytes: u64,
    pub total_private_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StorageResourceEntry {
    pub kind: String,
    pub label: String,
    pub path: Option<String>,
    pub total_bytes: u64,
    pub file_count: u64,
    pub cleanup_available: bool,
    pub cleanup_description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StorageResourceStatus {
    pub captured_at: String,
    pub entries: Vec<StorageResourceEntry>,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CleanupResourceInput {
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CleanupResourceResult {
    pub kind: String,
    pub removed_files: u64,
    pub reclaimed_bytes: u64,
    pub message: String,
}
