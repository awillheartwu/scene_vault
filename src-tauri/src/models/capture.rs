use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct CaptureSession {
    pub id: String,
    pub project_id: String,
    pub status: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// The new session plus the names of projects whose active sessions were
/// automatically ended, so only one project listens at a time.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartCaptureSessionResult {
    pub session: CaptureSession,
    pub stopped_projects: Vec<String>,
}

/// One source directory watched by a work period, snapshotted at session
/// start so later project changes do not rewrite finished periods.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct SessionSourceDirectory {
    pub session_id: String,
    pub directory: String,
    pub enabled: i64,
    pub discovery_started_at_ms: i64,
    pub baseline_initialized: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartCaptureSessionInput {
    pub project_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EndCaptureSessionInput {
    pub session_id: String,
    /// `completed` is used when omitted. `cancelled` is also accepted.
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct CaptureItem {
    pub id: String,
    pub project_id: String,
    pub session_id: String,
    pub asset_id: Option<String>,
    pub character_id: Option<String>,
    pub classification: String,
    pub source_path: String,
    pub file_size: Option<i64>,
    pub modified_at_ms: Option<i64>,
    pub content_hash: Option<String>,
    pub annotated_path: Option<String>,
    pub avatar_path: Option<String>,
    pub destination_path: Option<String>,
    pub destination_avatar_path: Option<String>,
    pub status: String,
    pub face_box_json: Option<String>,
    /// Number of faces YuNet detected on the source screenshot. `None` for
    /// legacy captures or when detection was skipped. UI uses it to warn
    /// that the suggestion is based on the primary face only.
    pub face_count: Option<i64>,
    pub suggested_character_id: Option<String>,
    pub recognition_confidence: Option<f64>,
    pub recognition_source: Option<String>,
    pub review_status: String,
    pub error_message: Option<String>,
    pub failure_stage: Option<String>,
    pub attempt_count: i64,
    pub next_retry_at: Option<String>,
    pub processing_warnings_json: String,
    pub captured_at: String,
    pub processed_at: Option<String>,
    pub archived_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub source_file_state: String,
    pub destination_file_state: String,
    pub destination_avatar_file_state: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterCaptureInput {
    pub session_id: String,
    pub source_path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LabelCaptureInput {
    pub capture_item_id: String,
    pub character_id: Option<String>,
    /// `person` when omitted. `scene` and `private` do not require a character.
    pub classification: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelabelCaptureInput {
    pub capture_item_id: String,
    pub character_id: Option<String>,
    /// `person` when omitted. `scene` and `private` do not require a character.
    pub classification: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverCapturesInput {
    pub session_id: String,
    /// Delay between the two metadata reads used to reject files that are still
    /// being written. The production default is 250 ms.
    pub stability_delay_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverCapturesResult {
    pub discovered_items: Vec<CaptureItem>,
    pub discovered_count: u32,
    pub already_known_count: u32,
    pub unstable_count: u32,
    pub ignored_count: u32,
    /// Total directory entries enumerated during the scan (including
    /// unsupported files), used to judge whether polling cost matters.
    pub entries_scanned: u32,
    /// Wall-clock duration of the scan in milliseconds.
    pub scan_duration_ms: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureItemIdInput {
    pub capture_item_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteCaptureInput {
    pub capture_item_id: String,
    #[serde(default)]
    pub delete_destination_files: bool,
    #[serde(default)]
    pub allow_permanent_network_delete: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteCharacterCapturesInput {
    pub character_id: String,
    #[serde(default)]
    pub delete_destination_files: bool,
    #[serde(default)]
    pub allow_permanent_network_delete: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureDeletionPreview {
    pub capture_count: u32,
    pub destination_file_count: u32,
    pub network_destination_file_count: u32,
    pub local_derived_file_count: u32,
    pub source_files_preserved: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileRecycleFailure {
    pub path: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureDeletionResult {
    pub completed: bool,
    pub records_deleted: u32,
    pub deleted_capture_item_ids: Vec<String>,
    pub destination_files_recycled: u32,
    pub destination_files_permanently_deleted: u32,
    pub destination_files_already_missing: u32,
    pub failures: Vec<FileRecycleFailure>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconcileCaptureFilesInput {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureFileReconcileResult {
    pub discovered_count: u32,
    pub source_missing_count: u32,
    pub source_replaced_count: u32,
    pub destination_missing_count: u32,
    pub destination_unavailable_count: u32,
    pub unstable_count: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReconcileProjectFilesInput {
    pub project_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectFileReconcileResult {
    pub scanned_directory_count: u32,
    pub scanned_file_count: u32,
    pub source_checked_count: u32,
    pub relocated_count: u32,
    pub source_missing_count: u32,
    pub source_replaced_count: u32,
    pub ambiguous_count: u32,
    pub unavailable_source_directory_count: u32,
    pub destination_missing_count: u32,
    pub destination_unavailable_count: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadCaptureImageInput {
    pub capture_item_id: String,
    pub variant: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThumbnailCacheStatus {
    pub dir: String,
    pub total_bytes: u64,
    pub limit_bytes: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompleteCaptureProcessingInput {
    pub capture_item_id: String,
    /// Absent when explicit person annotation is disabled.
    pub annotated_path: Option<String>,
    pub avatar_path: Option<String>,
    pub face_box_json: Option<String>,
    /// SFace feature vector from the processing pass; absent when the engine
    /// is not configured or no face was detected.
    #[serde(default)]
    pub face_feature: Option<Vec<f64>>,
    /// Recognition model identity that produced `face_feature`; `None` when
    /// there is no feature.
    #[serde(default)]
    pub face_feature_model_id: Option<String>,
    #[serde(default)]
    pub face_feature_model_version: Option<String>,
    /// Number of faces YuNet detected; `None` when detection was skipped.
    #[serde(default)]
    pub face_count: Option<i64>,
    /// Sharpness of the primary face (Python reports it; `None` when
    /// detection was skipped).
    #[serde(default)]
    pub face_sharpness: Option<f64>,
    /// Area share of the primary face (Python reports it; `None` when
    /// detection was skipped).
    #[serde(default)]
    pub face_area_ratio: Option<f64>,
    pub warnings_json: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkCaptureFailedInput {
    pub capture_item_id: String,
    pub error_message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetryCaptureInput {
    pub capture_item_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetryDegradedCapturesInput {
    pub project_id: String,
    /// Restricts the batch to one character when provided.
    pub character_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListCategoryItemsInput {
    pub project_id: String,
    /// `unclassified` (awaiting label), `scene` or `private`.
    pub category: String,
    /// Optional server-side pagination. When either is provided, both must be
    /// provided; the command then returns a `CaptureItemPage` instead of a
    /// plain array.
    pub page: Option<u32>,
    pub page_size: Option<u32>,
}

/// A page of capture items plus the total number of matching records, so the
/// UI can render page controls without a second count call.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureItemPage {
    pub items: Vec<CaptureItem>,
    pub total: i64,
    pub page: u32,
    pub page_size: u32,
}

/// Backward-compatible response for the capture-item list commands. Callers
/// that omit paging options keep receiving the legacy plain array; paged
/// callers receive a `{ items, total, page, pageSize }` object.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum CaptureItemListResponse {
    Legacy(Vec<CaptureItem>),
    Paged(CaptureItemPage),
}

/// One pre-existing image in the session source directory that has no
/// capture record yet.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnimportedCapture {
    pub path: String,
    pub file_size: i64,
    pub modified_at_ms: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportDirectoryCapturesInput {
    pub session_id: String,
    /// Absolute image paths inside the session source directory.
    pub paths: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedRecognitionInput {
    pub session_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListCaptureHistoryInput {
    pub project_id: String,
    pub session_id: Option<String>,
    pub character_id: Option<String>,
    pub status: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    /// When false (default), `private` captures are hidden from history.
    pub include_private: Option<bool>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct CaptureHistoryEntry {
    pub id: String,
    pub session_id: String,
    pub project_id: String,
    pub project_name: String,
    pub session_status: String,
    pub character_id: Option<String>,
    pub classification: String,
    pub character_name: Option<String>,
    pub asset_id: Option<String>,
    pub source_path: String,
    pub annotated_path: Option<String>,
    pub avatar_path: Option<String>,
    pub destination_path: Option<String>,
    pub destination_avatar_path: Option<String>,
    pub status: String,
    pub face_box_json: Option<String>,
    pub error_message: Option<String>,
    pub failure_stage: Option<String>,
    pub attempt_count: i64,
    pub next_retry_at: Option<String>,
    pub processing_warnings_json: String,
    pub captured_at: String,
    pub processed_at: Option<String>,
    pub archived_at: Option<String>,
    pub source_file_state: String,
    pub destination_file_state: String,
    pub destination_avatar_file_state: String,
}

/// A page of history entries plus the total number of matching records, so
/// the UI can render page controls without a second count call.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureHistoryPage {
    pub entries: Vec<CaptureHistoryEntry>,
    pub total: i64,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ClassifyPopupContext {
    /// All captures waiting for classification, oldest first.
    pub items: Vec<CaptureItem>,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub characters: Vec<crate::models::character::Character>,
}
