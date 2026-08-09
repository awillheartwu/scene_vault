use serde::{Deserialize, Serialize};

/// User-configurable archive naming rule stored under the
/// `capture.archive_naming` settings key. The template may use the
/// placeholders `{source}`, `{character}`, `{date}`, `{time}`,
/// `{datetime}`, `{id}`, `{classification}` and `{seq}`. Unknown
/// placeholders are rejected when the settings are saved.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveNamingSettings {
    #[serde(default = "default_template")]
    pub template: String,
    #[serde(default = "default_separator")]
    pub separator: String,
}

impl Default for ArchiveNamingSettings {
    fn default() -> Self {
        Self {
            template: default_template(),
            separator: default_separator(),
        }
    }
}

fn default_template() -> String {
    "{source} - {character} - {id}".to_owned()
}

fn default_separator() -> String {
    " - ".to_owned()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateArchiveNamingSettingsInput {
    pub settings: ArchiveNamingSettings,
}
