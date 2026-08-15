use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    /// Global shortcut that opens the classify popup, e.g. "Ctrl+Shift+S".
    #[serde(default = "default_classify_shortcut")]
    pub classify_shortcut: String,
    /// Global shortcut that opens the note popup, e.g. "Ctrl+Shift+N".
    #[serde(default = "default_note_shortcut")]
    pub note_shortcut: String,
    /// Whether history views include private shots by default.
    #[serde(default)]
    pub show_private_by_default: bool,
    /// Whether the note popup auto-saves when it loses focus.
    #[serde(default = "default_true")]
    pub auto_save_notes: bool,
    /// Whether classify and notes use two separate popup windows instead of
    /// the shared workbench window.
    #[serde(default)]
    pub split_popup_windows: bool,
    /// Whether the popup auto-hides after 3 seconds when there are no
    /// captures waiting for classification. Defaults to off: with the
    /// combined workbench the popup is also the note editor, so closing it
    /// would hide both panes.
    #[serde(default)]
    pub auto_close_empty_popup: bool,
    /// Disk budget (MiB) of the thumbnail cache directory. Oldest files are
    /// evicted once the directory exceeds this size.
    #[serde(default = "default_thumbnail_cache_size_mb")]
    pub thumbnail_cache_size_mb: i64,
    /// Maximum worker threads exposed to CPU-heavy image processing
    /// libraries. Capture jobs remain serial; this limits the parallelism
    /// used inside one OpenCV/YuNet/SFace/ArcFace request.
    #[serde(default = "default_image_processing_core_limit")]
    pub image_processing_core_limit: u32,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            classify_shortcut: default_classify_shortcut(),
            note_shortcut: default_note_shortcut(),
            show_private_by_default: false,
            auto_save_notes: default_true(),
            split_popup_windows: false,
            auto_close_empty_popup: false,
            thumbnail_cache_size_mb: default_thumbnail_cache_size_mb(),
            image_processing_core_limit: default_image_processing_core_limit(),
        }
    }
}

fn default_thumbnail_cache_size_mb() -> i64 {
    256
}

fn default_image_processing_core_limit() -> u32 {
    4
}

fn default_classify_shortcut() -> String {
    "Ctrl+Shift+S".to_owned()
}

fn default_note_shortcut() -> String {
    "Ctrl+Shift+N".to_owned()
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAppSettingsInput {
    pub settings: AppSettings,
}
