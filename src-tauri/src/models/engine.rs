use serde::{Deserialize, Serialize};

/// Engine kind identifiers persisted in `project_engine_integrations.engine`.
pub const ENGINE_RENPY: &str = "renpy";

/// Archive output size values a user can pick (besides `inherit`).
pub const ARCHIVE_OUTPUT_SIZES: [&str; 4] = ["keep", "1080p", "1440p", "2160p"];

/// Project value meaning "use the global default".
pub const ARCHIVE_OUTPUT_INHERIT: &str = "inherit";

pub fn is_archive_output_size(value: &str) -> bool {
    ARCHIVE_OUTPUT_SIZES.contains(&value)
}

/// Result of inspecting a user-selected folder or executable. Read-only: the
/// caller only records or displays it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EngineDetection {
    pub engine: String,
    /// Folder that contains `game/` (the game root next to the executable).
    pub engine_root: String,
    /// The `game/` directory itself.
    pub game_directory: String,
    /// Python major version shipped with the engine build, when detectable.
    pub python_major: Option<i64>,
    /// Whether a Scene Vault bridge file already sits in `game/`.
    pub bridge_installed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectEngineInput {
    pub path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallEngineBridgeInput {
    pub project_id: String,
    /// Game folder or executable the user picked for this project.
    pub path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectEngineInput {
    pub project_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectIdInput {
    pub project_id: String,
}

/// Removes one installation. The id may be omitted only while the project
/// owns exactly one installation (legacy single-game projects).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UninstallEngineBridgeInput {
    pub project_id: String,
    #[serde(default)]
    pub integration_id: Option<String>,
}

/// One launchable Ren'Py game of a project: the installation it belongs to
/// plus the executable that was found inside its game root.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectGameLaunchOption {
    pub integration_id: String,
    pub engine_root: String,
    pub executable: String,
    pub game_name: String,
    /// Best guess for the main executable; alternatives stay selectable.
    pub recommended: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchProjectGameInput {
    pub project_id: String,
    /// Which installation to launch; may be omitted only when exactly one
    /// game of the project has a launchable executable.
    #[serde(default)]
    pub integration_id: Option<String>,
    /// Exact executable to start (from the scanned candidate list); used when
    /// one game folder contains several launchable programs.
    #[serde(default)]
    pub executable: Option<String>,
}

/// One file written into the game folder, recorded so uninstalling only ever
/// removes files that still match what Scene Vault created.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BridgeFileEntry {
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ProjectEngineIntegration {
    pub id: String,
    pub project_id: String,
    pub engine: String,
    pub engine_root: String,
    pub game_directory: String,
    pub python_major: Option<i64>,
    pub bridge_version: String,
    /// App-cache directory the bridge writes screenshot metadata files into.
    pub metadata_directory: Option<String>,
    /// Serialized `Vec<BridgeFileEntry>`.
    pub manifest_json: String,
    pub installed_at: String,
    pub updated_at: String,
}

/// Result of an uninstall: files that were removed, and files kept because
/// their ownership could not be proven against the recorded manifest.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeUninstallResult {
    pub removed_files: Vec<String>,
    pub kept_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectEngineIntegrationView {
    pub integration: ProjectEngineIntegration,
    /// Live filesystem state so the UI can prompt a reinstall after a game
    /// update or a store integrity check removed the bridge files.
    pub bridge_file_present: bool,
    pub compiled_file_present: bool,
    /// Bridge version bundled with this app; a mismatch means "reinstall".
    pub bundled_bridge_version: String,
    /// Install-time advisory, for example a kept compiled artifact whose
    /// ownership could not be verified.
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct CaptureEngineMetadata {
    pub capture_item_id: String,
    pub project_id: String,
    pub engine: String,
    pub schema_version: i64,
    pub bridge_version: String,
    pub speaker_raw: Option<String>,
    pub speaker_source: Option<String>,
    pub dialogue: Option<String>,
    pub suggested_character_id: Option<String>,
    pub virtual_width: Option<i64>,
    pub virtual_height: Option<i64>,
    pub physical_width: Option<i64>,
    pub physical_height: Option<i64>,
    pub payload_json: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Parsed `*.scenevault.json` sidecar written by the bridge. Unknown fields
/// (including legacy `sourceAssets`) are ignored and every field is optional
/// so newer bridges keep working with older Scene Vault builds.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RenpySidecar {
    pub schema_version: i64,
    pub bridge_version: String,
    pub speaker: Option<String>,
    pub speaker_source: Option<String>,
    pub dialogue: Option<String>,
    pub virtual_resolution: Option<(i64, i64)>,
    pub physical_size: Option<(i64, i64)>,
    pub scene_tags: Vec<String>,
    pub errors: Vec<String>,
}

/// Effective archive output policy for one project: its own value plus the
/// resolved value after inheritance from the global default.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturePolicyView {
    pub project_size: String,
    pub resolved_size: String,
    /// The global default, shown as "inherit" on its own line so the label
    /// never turns into the project override.
    pub global_size: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetArchiveOutputSizeInput {
    pub project_id: String,
    /// `inherit`, `keep`, `1080p`, `1440p` or `2160p`.
    pub archive_output_size: String,
}

/// Global archive output default, stored under the `archive.output_size`
/// settings key. Projects inherit it unless they override the value.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveOutputSettings {
    #[serde(default = "default_archive_output_size")]
    pub size: String,
}

impl Default for ArchiveOutputSettings {
    fn default() -> Self {
        Self {
            size: default_archive_output_size(),
        }
    }
}

fn default_archive_output_size() -> String {
    "keep".to_owned()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateArchiveOutputSettingsInput {
    pub settings: ArchiveOutputSettings,
}
