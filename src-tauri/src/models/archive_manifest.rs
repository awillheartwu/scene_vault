use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RebuildArchiveManifestInput {
    pub project_id: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RebuildArchiveManifestResult {
    pub project_id: String,
    pub manifest_count: u32,
    pub entry_count: u32,
    pub character_count: u32,
    pub written_count: u32,
    pub unchanged_count: u32,
    pub skipped_count: u32,
    pub failed_count: u32,
    pub message: String,
}
