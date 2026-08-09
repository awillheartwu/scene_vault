use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Per-recognizer tunables under `capture.recognition.profiles`, keyed by
/// the recognition model id reported by Python (`opencv-sface`,
/// `arcface-r50-w600k`...). Every field is optional: unset fields fall back
/// to the benchmark-calibrated defaults of that model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ModelRecognitionProfile {
    /// Minimum cosine similarity for a face-bank suggestion.
    pub confidence_threshold: Option<f64>,
    /// The top candidate must beat the runner-up by at least this much, or
    /// no suggestion is made ("宁愿不推荐，也别自信地推荐错人").
    pub margin: Option<f64>,
    pub verification_high_threshold: Option<f64>,
    pub verification_low_threshold: Option<f64>,
    pub cross_check_delta: Option<f64>,
}

/// Recognition settings under the `capture.recognition` key.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct RecognitionSettings {
    /// Extract SFace/ArcFace features right after registration so the
    /// classify popup can show a recommendation before the user labels the
    /// capture.
    #[serde(default = "default_extract_at_registration")]
    pub extract_at_registration: bool,
    /// Closed-set verification at label time.
    #[serde(default = "default_true")]
    pub verification_enabled: bool,
    /// Per-recognizer profiles keyed by model id. A missing profile falls
    /// back to the known-model defaults (benchmark-calibrated); unknown
    /// models get the conservative base defaults.
    #[serde(default)]
    pub profiles: BTreeMap<String, ModelRecognitionProfile>,
    /// Sample-enrollment quality gate: faces whose sharpness (Laplacian
    /// variance) is below this value are not enrolled into the Face Bank
    /// (printed/photo faces in scenes score near zero, e.g. 1.4 vs ~35 for
    /// real game faces). `None` disables the gate. Default 3.0 keeps the
    /// extreme cases out without rejecting normal low-light faces
    /// (benchmark p10 ≈ 4.1).
    #[serde(default = "default_min_sample_sharpness")]
    pub min_sample_sharpness: Option<f64>,
    /// Minimum share of the image the primary face must occupy (0.0..=1.0)
    /// for its sample to be enrolled. Rejects background/tiny faces
    /// (benchmark p10 of normal primary faces ≈ 0.66%, so 0.005 keeps the
    /// extreme edge cases out).
    #[serde(default = "default_min_face_area_ratio")]
    pub min_face_area_ratio: Option<f64>,
    /// Reserved: minimum detection confidence for enrollment. The setting
    /// is exposed (disabled in the UI) but the gate does NOT check it yet —
    /// wired in a later iteration.
    #[serde(default = "default_min_face_confidence")]
    pub min_face_confidence: Option<f64>,
}

impl Default for RecognitionSettings {
    fn default() -> Self {
        Self {
            extract_at_registration: default_extract_at_registration(),
            verification_enabled: default_true(),
            profiles: BTreeMap::new(),
            min_sample_sharpness: default_min_sample_sharpness(),
            min_face_area_ratio: default_min_face_area_ratio(),
            min_face_confidence: default_min_face_confidence(),
        }
    }
}

/// The values recognition flows actually use, resolved from the stored
/// profile over the known-model defaults.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedRecognitionProfile {
    pub confidence_threshold: f64,
    pub margin: f64,
    pub verification_high_threshold: f64,
    pub verification_low_threshold: f64,
    pub cross_check_delta: f64,
}

impl RecognitionSettings {
    /// Resolves the effective profile for a recognizer model id. Known
    /// models carry benchmark-calibrated defaults (SFace: 0.50/0.05 with the
    /// existing verification bands; ArcFace R50: 0.50/0.10 with provisional
    /// bands from the same/cross score distributions — to be re-calibrated
    /// after the label-time verification data is collected). Stored profile
    /// values override the defaults.
    pub fn resolve(&self, model_id: &str) -> ResolvedRecognitionProfile {
        let defaults = known_defaults(model_id);
        let profile = self.profiles.get(model_id);
        ResolvedRecognitionProfile {
            confidence_threshold: profile
                .and_then(|p| p.confidence_threshold)
                .unwrap_or(defaults.confidence_threshold),
            margin: profile.and_then(|p| p.margin).unwrap_or(defaults.margin),
            verification_high_threshold: profile
                .and_then(|p| p.verification_high_threshold)
                .unwrap_or(defaults.verification_high_threshold),
            verification_low_threshold: profile
                .and_then(|p| p.verification_low_threshold)
                .unwrap_or(defaults.verification_low_threshold),
            cross_check_delta: profile
                .and_then(|p| p.cross_check_delta)
                .unwrap_or(defaults.cross_check_delta),
        }
    }
}

/// Benchmark-calibrated defaults for a recognizer model id (provisional:
/// subject to leave-one-game-out and real-usage data). The settings UI's
/// "restore defaults" button fills these values.
pub fn known_defaults(model_id: &str) -> ResolvedRecognitionProfile {
    match model_id {
        "opencv-sface" => ResolvedRecognitionProfile {
            confidence_threshold: 0.50,
            margin: 0.05,
            verification_high_threshold: 0.65,
            verification_low_threshold: 0.40,
            cross_check_delta: 0.10,
        },
        "arcface-r50" => ResolvedRecognitionProfile {
            confidence_threshold: 0.50,
            margin: 0.10,
            verification_high_threshold: 0.55,
            verification_low_threshold: 0.35,
            cross_check_delta: 0.10,
        },
        _ => ResolvedRecognitionProfile {
            confidence_threshold: 0.50,
            margin: 0.0,
            verification_high_threshold: 0.65,
            verification_low_threshold: 0.40,
            cross_check_delta: 0.10,
        },
    }
}

fn default_extract_at_registration() -> bool {
    true
}

fn default_true() -> bool {
    true
}

fn default_min_sample_sharpness() -> Option<f64> {
    Some(3.0)
}

fn default_min_face_area_ratio() -> Option<f64> {
    Some(0.005)
}

fn default_min_face_confidence() -> Option<f64> {
    Some(0.6)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRecognitionSettingsInput {
    pub settings: RecognitionSettings,
}
