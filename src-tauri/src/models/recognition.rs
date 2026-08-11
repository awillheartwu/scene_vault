use serde::Deserialize;
use serde::Serialize;

/// Identity of the recognition model bundled with the app (OpenCV Zoo SFace).
/// Face embeddings from different models live in different vector spaces;
/// matching is only valid within one model identity.
pub const FACE_MODEL_ID: &str = "opencv-sface";
pub const FACE_MODEL_VERSION: &str = "2021dec";

/// One detected face on a capture. Phase 1 writes only the primary face
/// (face_index = 0, is_primary = 1); the schema allows N rows for a future
/// multi-face phase. `feature_json` is the encoded feature vector, or `[]`
/// for the "attempted, no face detected" marker.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct FaceRow {
    pub id: String,
    pub capture_item_id: String,
    pub face_index: i64,
    pub is_primary: i64,
    pub box_json: Option<String>,
    pub feature_json: Option<String>,
    pub feature_model_id: Option<String>,
    pub feature_model_version: Option<String>,
    pub feature_dim: Option<i64>,
    pub face_sharpness: Option<f64>,
    pub face_area_ratio: Option<f64>,
    pub confirmed_character_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl FaceRow {
    /// True when the row carries a usable feature (not the `[]` no-face
    /// marker and not an unfilled backfill row).
    pub fn has_feature(&self) -> bool {
        self.feature_json
            .as_deref()
            .is_some_and(|value| value != "[]")
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetRecognitionSuggestionInput {
    pub capture_item_id: String,
    /// `None` clears the current suggestion and resets `review_status`.
    pub suggested_character_id: Option<String>,
    /// 0.0..=1.0; `None` clears it together with the suggestion.
    pub confidence: Option<f64>,
    /// `face_bank`, `vision` or `manual`.
    pub source: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRecognitionInput {
    pub capture_item_id: String,
    /// `accepted` or `rejected`.
    pub decision: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListCharacterItemsInput {
    pub project_id: String,
    pub character_id: String,
    /// Optional server-side pagination. When either is provided, both must be
    /// provided; the command then returns a `CaptureItemPage` instead of a
    /// plain array.
    pub page: Option<u32>,
    pub page_size: Option<u32>,
}

/// One row of the character face-bank sample strip.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct FaceSample {
    pub id: String,
    pub character_id: String,
    pub capture_item_id: String,
    pub face_box_json: Option<String>,
    pub confidence: Option<f64>,
    pub status: String,
    pub flagged: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetFaceSampleStatusInput {
    pub sample_id: String,
    /// `active` or `revoked`. Revoked samples stop participating in matching
    /// but stay recorded.
    pub status: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyCaptureIdentityInput {
    pub capture_item_id: String,
    pub character_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetFaceSampleFlaggedInput {
    pub sample_id: String,
    pub flagged: bool,
}

/// Result of verifying one capture against one character's face-bank
/// samples. `level` is `ok`, `low`, `strong` or `unverified`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationResult {
    pub score: Option<f64>,
    pub best_other_score: Option<f64>,
    pub best_other_character_id: Option<String>,
    pub has_samples: bool,
    pub has_feature: bool,
    pub level: String,
}

/// Outcome of a Face Bank rebuild: re-extracting every person capture's
/// primary-face feature with the currently configured model and re-enrolling
/// the samples. Missing sources and per-item extraction failures are counted,
/// not fatal.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FaceBankRebuildSummary {
    pub total: u32,
    pub rebuilt: u32,
    pub no_face: u32,
    pub not_enrolled: u32,
    pub skipped_missing_source: u32,
    pub failed: u32,
    pub stale_preserved: u32,
    /// Unclassified captures re-evaluated against the freshly rebuilt bank.
    pub suggestions_refreshed: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FaceBankModelCount {
    pub model_id: String,
    pub model_version: String,
    pub sample_count: i64,
    pub compatible: bool,
}

/// Every active model identity in the Face Bank versus the currently
/// configured recognizer. Mismatched embeddings are never compared, so the UI
/// shows an explicit rebuild hint instead of silently empty suggestions.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FaceBankModelStatus {
    pub bank_model_id: Option<String>,
    pub bank_model_version: Option<String>,
    pub active_model_id: Option<String>,
    pub active_model_version: Option<String>,
    pub sample_count: i64,
    pub incompatible_sample_count: i64,
    pub compatible: bool,
    pub model_counts: Vec<FaceBankModelCount>,
}
