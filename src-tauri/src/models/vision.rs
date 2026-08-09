use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisionSettings {
    pub python_executable_path: Option<String>,
    pub python_module_root: Option<String>,
    pub yunet_model_path: Option<String>,
    pub sface_model_path: Option<String>,
    /// Recognition embedding provider: `sface` (default) or `arcface`.
    /// ArcFace is optional and never bundled with the app (non-commercial
    /// model license); the user points at their own ONNX file.
    pub recognizer: Option<String>,
    pub arcface_model_path: Option<String>,
    pub font_path: Option<String>,
}

/// One font file available for annotation, either bundled with the app or
/// dropped into the local fonts directory by the setup script.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundledFont {
    pub name: String,
    pub path: String,
}

/// Tunable Python vision parameters, stored under `capture.processing`.
/// Every field is optional: unset fields keep the Python-side defaults.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ProcessingSettings {
    pub detection: Option<DetectionSettings>,
    pub annotation: Option<AnnotationSettings>,
    pub crop: Option<CropSettings>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct DetectionSettings {
    pub score_threshold: Option<f64>,
    pub nms_threshold: Option<f64>,
    pub top_k: Option<i64>,
    pub area_weight: Option<f64>,
    pub confidence_weight: Option<f64>,
    pub center_weight: Option<f64>,
    pub sharpness_weight: Option<f64>,
    pub edge_penalty_weight: Option<f64>,
    pub edge_margin_ratio: Option<f64>,
    pub min_sharpness: Option<f64>,
    pub blur_penalty_weight: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct AnnotationSettings {
    pub text_color: Option<[u8; 3]>,
    pub stroke_color: Option<[u8; 3]>,
    pub stroke_width: Option<i64>,
    pub padding: Option<i64>,
    /// Symmetric pixel expansion of the detected face reference box used for
    /// text placement. `padding` remains the canvas safety/text gap.
    pub face_box_expansion: Option<i64>,
    /// Preferred text position relative to the detected face:
    /// `above`, `right`, `below` or `left`.
    pub face_text_position: Option<String>,
    pub fallback_position: Option<String>,
    /// Custom text placement relative to the face box when
    /// `face_text_position` is `custom`; multiples of box width/height.
    pub text_offset_x: Option<f64>,
    pub text_offset_y: Option<f64>,
    pub font_size: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct CropSettings {
    pub aspect_ratio: Option<String>,
    pub scale_x: Option<f64>,
    pub scale_top: Option<f64>,
    pub scale_bottom: Option<f64>,
    pub min_size: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateVisionSettingsInput {
    pub settings: VisionSettings,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VisionHealth {
    pub status: String,
    pub engine_version: Option<String>,
    pub python_version: Option<String>,
    pub process_screenshot_available: bool,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureRuntimeStatus {
    pub engine_status: String,
    pub worker_status: String,
    pub active_capture_item_id: Option<String>,
    pub queued_count: i64,
    pub archive_pending_count: i64,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VisionProcessData {
    // Feature-only extraction requests (annotate=false) return a null
    // annotatedPath, so this must stay optional.
    pub annotated_path: Option<String>,
    pub avatar_path: Option<String>,
    pub face_box: Option<serde_json::Value>,
    #[serde(default)]
    pub face_feature: Option<Vec<f64>>,
    /// Recognition model identity that produced `face_feature`, reported by
    /// Python. Absent when no feature was produced (old engines also omit it).
    #[serde(default)]
    pub face_feature_model_id: Option<String>,
    #[serde(default)]
    pub face_feature_model_version: Option<String>,
    /// Number of faces YuNet detected on the screenshot; absent when
    /// detection was skipped or the engine predates the field.
    #[serde(default)]
    pub face_count: Option<i64>,
    /// Laplacian-variance sharpness of the primary face; used by the
    /// sample-enrollment quality gate.
    #[serde(default)]
    pub face_sharpness: Option<f64>,
    /// Share of the image occupied by the primary face (0.0..=1.0); used by
    /// the sample-enrollment quality gate to reject background/tiny faces.
    #[serde(default)]
    pub face_area_ratio: Option<f64>,
    #[serde(default)]
    pub warnings: Vec<String>,
}
