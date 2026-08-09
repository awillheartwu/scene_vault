use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub cover_asset_id: Option<String>,
    /// Last capture source directory used for this project, if any.
    pub last_source_directory: Option<String>,
    /// Last capture destination (archive) directory used for this project, if any.
    pub last_destination_directory: Option<String>,
    /// Archive directory of the project; captures are archived here.
    pub destination_directory: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Project-level data needed by the home project library. Keeping this as a
/// single aggregate avoids one history/session query per project in Vue.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ProjectOverviewSummary {
    pub project_id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: String,
    pub source_count: i64,
    pub destination_configured: bool,
    pub session_count: i64,
    pub active_session_count: i64,
    pub capture_count: i64,
    pub awaiting_count: i64,
    pub processing_count: i64,
    pub completed_count: i64,
    pub failed_count: i64,
    pub last_activity_at: String,
    pub latest_capture_item_id: Option<String>,
}

/// One source directory in a project's authoritative capture list.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSourceDirectory {
    pub id: String,
    pub project_id: String,
    pub directory: String,
    pub enabled: i64,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddProjectSourceDirectoryInput {
    pub project_id: String,
    pub directory: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveProjectSourceDirectoryInput {
    pub id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetProjectSourceDirectoryEnabledInput {
    pub id: String,
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetProjectDestinationInput {
    pub project_id: String,
    pub directory: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameProjectInput {
    pub project_id: String,
    pub name: String,
}

/// What deleting a project will remove. Shown to the user for confirmation;
/// on-disk source and archive files are never touched.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDeletionPreview {
    pub capture_count: i64,
    pub character_count: i64,
    pub session_count: i64,
    pub has_active_session: bool,
    pub note_count: i64,
    pub collection_count: i64,
    pub orphan_asset_count: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateProjectInput {
    pub name: String,
    pub description: Option<String>,
    pub cover_asset_id: Option<String>,
}
