//! Facts about the files behind one capture: the original screenshot and the
//! archived output Scene Vault wrote. Everything here describes files that
//! were actually seen; missing values mean "unknown" and are never guessed
//! from the current settings.

use serde::{Deserialize, Serialize};

/// Original capture file. Width/height/format come from a header read, so a
/// file whose format cannot be decoded simply has fewer fields filled in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceImageFacts {
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Canonical lowercase format name (`png`, `jpeg`, `webp`, ...).
    pub format: Option<String>,
}

/// Archived output written by Scene Vault. The policy fields describe what
/// the archive step was asked to do and what it actually did; the quality
/// fields are only meaningful for files this app encoded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveImageFacts {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub format: Option<String>,
    pub bytes: Option<u64>,
    /// `lossless`, `quality` or `unknown`.
    pub quality_kind: Option<String>,
    /// JPEG quality actually used; None for lossless or unknown outputs.
    pub quality: Option<i64>,
    /// Requested size policy: keep / 1080p / 1440p / 2160p / native.
    pub size_policy: Option<String>,
    /// False when the requested size policy could not be applied.
    pub size_policy_applied: Option<bool>,
    /// False when a requested format conversion could not be applied.
    pub format_applied: Option<bool>,
}

/// Both file variants of one capture, as returned to the UI.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureImageFacts {
    pub source: Option<SourceImageFacts>,
    pub destination: Option<ArchiveImageFacts>,
}
