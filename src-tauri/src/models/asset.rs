use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    pub id: String,
    pub name: String,
    pub asset_type: String,
    pub path: String,
    pub thumbnail_path: Option<String>,
    pub mime_type: Option<String>,
    pub file_size: Option<i64>,
    pub file_modified_at: Option<String>,
    pub content_hash: Option<String>,
    pub metadata_json: String,
    pub ai_description: Option<String>,
    pub embedding_id: Option<String>,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateAssetInput {
    pub name: String,
    pub asset_type: String,
    pub path: String,
    pub mime_type: Option<String>,
    pub file_size: Option<i64>,
    pub file_modified_at: Option<String>,
    pub metadata_json: Option<String>,
}
