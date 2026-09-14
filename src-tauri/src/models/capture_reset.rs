use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetPreviewInput {
    pub project_id: String,
    pub capture_item_ids: Option<Vec<String>>,
    pub session_id: Option<String>,
    pub character_id: Option<String>,
    pub status: Option<String>,
    pub include_private: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetExecuteInput {
    pub job_id: String,
    pub delete_destination_files: bool,
    pub allow_permanent_network_delete: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetJob {
    pub id: String,
    /// preview until the run starts, running while it works, completed when the
    /// pass ends. Failed items are reported, not queued: the job simply ends.
    pub status: String,
    pub executing: bool,
    pub delete_destination_files: Option<bool>,
    pub allow_permanent_network_delete: Option<bool>,
    pub items: Vec<ResetJobItem>,
    pub destination_file_count: u32,
    pub network_destination_file_count: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetJobItem {
    pub capture_item_id: String,
    pub source_path: String,
    pub status: String,
    /// Machine-readable failure class (source_gone, network, locked, denied,
    /// changed, busy, unknown) so the interface can explain and advise instead
    /// of echoing a raw error.
    pub reason: Option<String>,
    pub error: Option<String>,
}
