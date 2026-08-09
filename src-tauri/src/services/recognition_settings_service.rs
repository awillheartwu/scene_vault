use sqlx::SqlitePool;

use crate::{
    error::AppError,
    models::recognition_settings::{
        RecognitionSettings, ResolvedRecognitionProfile, UpdateRecognitionSettingsInput,
    },
};

const SETTINGS_KEY: &str = "capture.recognition";

pub async fn get(pool: &SqlitePool) -> Result<RecognitionSettings, AppError> {
    let value: Option<String> = sqlx::query_scalar("SELECT value_json FROM settings WHERE key = ?")
        .bind(SETTINGS_KEY)
        .fetch_optional(pool)
        .await?;
    match value {
        Some(value) => serde_json::from_str(&value).map_err(|error| {
            AppError::Validation(format!("stored recognition settings are invalid: {error}"))
        }),
        None => Ok(RecognitionSettings::default()),
    }
}

/// The effective tunables for a recognizer model id: stored profile over the
/// benchmark-calibrated known-model defaults. Recognition flows resolve the
/// profile of the model that produced the query feature, so each recognizer
/// keeps its own threshold/margin/verification bands.
pub async fn resolved(
    pool: &SqlitePool,
    model_id: &str,
) -> Result<ResolvedRecognitionProfile, AppError> {
    let settings = get(pool).await?;
    Ok(settings.resolve(model_id))
}

pub async fn update(
    pool: &SqlitePool,
    input: UpdateRecognitionSettingsInput,
) -> Result<RecognitionSettings, AppError> {
    let settings = normalize(input.settings)?;
    let value = serde_json::to_string(&settings).map_err(|error| {
        AppError::Validation(format!("cannot encode recognition settings: {error}"))
    })?;
    sqlx::query(
        r#"
        INSERT INTO settings (key, value_json)
        VALUES (?, ?)
        ON CONFLICT(key) DO UPDATE SET
            value_json = excluded.value_json,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        "#,
    )
    .bind(SETTINGS_KEY)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(settings)
}

fn normalize(settings: RecognitionSettings) -> Result<RecognitionSettings, AppError> {
    if settings
        .min_sample_sharpness
        .is_some_and(|value| !value.is_finite() || value < 0.0)
    {
        return Err(AppError::Validation(
            "minSampleSharpness must be a non-negative number or null".to_owned(),
        ));
    }
    for (label, value) in [
        ("minFaceAreaRatio", settings.min_face_area_ratio),
        ("minFaceConfidence", settings.min_face_confidence),
    ] {
        if value.is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value)) {
            return Err(AppError::Validation(format!(
                "{label} must be between 0.0 and 1.0 or null"
            )));
        }
    }
    for (model_id, profile) in &settings.profiles {
        for (label, value) in [
            ("confidenceThreshold", profile.confidence_threshold),
            ("margin", profile.margin),
            (
                "verificationHighThreshold",
                profile.verification_high_threshold,
            ),
            (
                "verificationLowThreshold",
                profile.verification_low_threshold,
            ),
            ("crossCheckDelta", profile.cross_check_delta),
        ] {
            if value.is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value)) {
                return Err(AppError::Validation(format!(
                    "{label} for {model_id} must be between 0.0 and 1.0"
                )));
            }
        }
        if let (Some(low), Some(high)) = (
            profile.verification_low_threshold,
            profile.verification_high_threshold,
        ) {
            if low > high {
                return Err(AppError::Validation(format!(
                    "verificationLowThreshold for {model_id} cannot exceed verificationHighThreshold"
                )));
            }
        }
    }
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use std::collections::BTreeMap;

    #[test]
    fn known_defaults_serialize_with_camel_case_field_names() {
        // The settings UI reads camelCase fields; a snake_case leak shows up
        // as blank inputs everywhere except field names that match by luck.
        let json = serde_json::to_string(&crate::models::recognition_settings::known_defaults(
            "arcface-r50",
        ))
        .expect("serialize defaults");
        for field in [
            "confidenceThreshold",
            "margin",
            "verificationHighThreshold",
            "verificationLowThreshold",
            "crossCheckDelta",
        ] {
            assert!(json.contains(field), "missing {field} in {json}");
        }
        assert!(
            !json.contains("confidence_threshold"),
            "snake_case field name leaked into the API: {json}"
        );
    }

    #[tokio::test]
    async fn defaults_and_round_trips_saved_settings() {
        let pool = db::test_pool().await;
        assert_eq!(
            get(&pool).await.expect("defaults"),
            RecognitionSettings::default()
        );

        let settings = RecognitionSettings {
            extract_at_registration: false,
            verification_enabled: true,
            min_sample_sharpness: Some(2.0),
            min_face_area_ratio: Some(0.005),
            min_face_confidence: Some(0.6),
            profiles: BTreeMap::from([(
                "opencv-sface".to_owned(),
                crate::models::recognition_settings::ModelRecognitionProfile {
                    confidence_threshold: Some(0.62),
                    margin: Some(0.05),
                    verification_high_threshold: Some(0.65),
                    verification_low_threshold: Some(0.4),
                    cross_check_delta: Some(0.1),
                },
            )]),
        };
        let saved = update(
            &pool,
            UpdateRecognitionSettingsInput {
                settings: settings.clone(),
            },
        )
        .await
        .expect("save");
        assert_eq!(saved, settings);
        assert_eq!(get(&pool).await.expect("read"), settings);
        assert_eq!(
            resolved(&pool, "opencv-sface").await.expect("resolve"),
            ResolvedRecognitionProfile {
                confidence_threshold: 0.62,
                margin: 0.05,
                verification_high_threshold: 0.65,
                verification_low_threshold: 0.4,
                cross_check_delta: 0.1,
            }
        );
        // Unknown models fall back to the conservative base defaults.
        assert_eq!(
            resolved(&pool, "unknown-model")
                .await
                .expect("resolve unknown"),
            ResolvedRecognitionProfile {
                confidence_threshold: 0.5,
                margin: 0.0,
                verification_high_threshold: 0.65,
                verification_low_threshold: 0.4,
                cross_check_delta: 0.1,
            }
        );
        // ArcFace carries its own benchmark-calibrated defaults.
        assert_eq!(
            resolved(&pool, "arcface-r50")
                .await
                .expect("resolve arcface"),
            ResolvedRecognitionProfile {
                confidence_threshold: 0.5,
                margin: 0.1,
                verification_high_threshold: 0.55,
                verification_low_threshold: 0.35,
                cross_check_delta: 0.1,
            }
        );
        assert_eq!(
            RecognitionSettings::default().min_sample_sharpness,
            Some(3.0),
            "default gate must keep printed/photo faces out"
        );
        // Explicit null disables the gate.
        update(
            &pool,
            UpdateRecognitionSettingsInput {
                settings: RecognitionSettings {
                    extract_at_registration: true,
                    verification_enabled: true,
                    min_sample_sharpness: None,
                    min_face_area_ratio: None,
                    min_face_confidence: None,
                    profiles: BTreeMap::new(),
                },
            },
        )
        .await
        .expect("disable gate");
        assert!(
            get(&pool)
                .await
                .expect("read")
                .min_sample_sharpness
                .is_none(),
            "explicit null must disable the gate"
        );
    }

    #[tokio::test]
    async fn rejects_out_of_range_threshold() {
        let pool = db::test_pool().await;
        let error = update(
            &pool,
            UpdateRecognitionSettingsInput {
                settings: RecognitionSettings {
                    extract_at_registration: true,
                    verification_enabled: true,
                    min_sample_sharpness: None,
                    min_face_area_ratio: None,
                    min_face_confidence: None,
                    profiles: BTreeMap::from([(
                        "opencv-sface".to_owned(),
                        crate::models::recognition_settings::ModelRecognitionProfile {
                            confidence_threshold: Some(1.5),
                            margin: None,
                            verification_high_threshold: None,
                            verification_low_threshold: None,
                            cross_check_delta: None,
                        },
                    )]),
                },
            },
        )
        .await
        .expect_err("out of range");
        assert!(matches!(error, AppError::Validation(_)));
    }

    #[tokio::test]
    async fn rejects_inverted_verification_bands() {
        let pool = db::test_pool().await;
        let error = update(
            &pool,
            UpdateRecognitionSettingsInput {
                settings: RecognitionSettings {
                    extract_at_registration: true,
                    verification_enabled: true,
                    min_sample_sharpness: None,
                    min_face_area_ratio: None,
                    min_face_confidence: None,
                    profiles: BTreeMap::from([(
                        "opencv-sface".to_owned(),
                        crate::models::recognition_settings::ModelRecognitionProfile {
                            confidence_threshold: Some(0.5),
                            margin: None,
                            verification_high_threshold: Some(0.4),
                            verification_low_threshold: Some(0.65),
                            cross_check_delta: None,
                        },
                    )]),
                },
            },
        )
        .await
        .expect_err("inverted bands");
        assert!(matches!(error, AppError::Validation(_)));
    }
}
