use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Character {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub aliases_json: String,
    pub avatar_asset_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateCharacterInput {
    pub project_id: String,
    pub name: String,
    pub aliases_json: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameCharacterInput {
    pub character_id: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeCharactersInput {
    pub source_character_id: String,
    pub target_character_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetCharacterAvatarInput {
    pub character_id: String,
    pub avatar_asset_id: Option<String>,
}

/// One row of the workbench character overview: per-character capture count,
/// pending suggestion count and most recent capture, beside the display data.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct CharacterSummary {
    pub id: String,
    pub name: String,
    pub aliases_json: String,
    pub avatar_asset_id: Option<String>,
    pub capture_count: i64,
    pub pending_review_count: i64,
    /// Active face-bank samples of this character.
    pub sample_count: i64,
    /// Degraded person captures (completed without annotation) waiting for
    /// re-recognition.
    pub degraded_count: i64,
    pub last_captured_at: Option<String>,
    /// Newest capture of this character, used by the overview card thumbnail.
    pub latest_capture_item_id: Option<String>,
    /// Avatar crop path of the newest capture, when the vision engine
    /// produced one; the overview card prefers it over the full screenshot.
    pub latest_avatar_path: Option<String>,
}
