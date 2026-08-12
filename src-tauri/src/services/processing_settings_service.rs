use sqlx::SqlitePool;

use crate::{error::AppError, models::vision::ProcessingSettings};

const SETTINGS_KEY: &str = "capture.processing";
const FALLBACK_POSITIONS: [&str; 4] = ["top_left", "top_right", "bottom_left", "bottom_right"];
const FACE_TEXT_POSITIONS: [&str; 5] = ["above", "right", "below", "left", "custom"];

pub async fn get(pool: &SqlitePool) -> Result<ProcessingSettings, AppError> {
    let value: Option<String> = sqlx::query_scalar("SELECT value_json FROM settings WHERE key = ?")
        .bind(SETTINGS_KEY)
        .fetch_optional(pool)
        .await?;
    match value {
        Some(value) => serde_json::from_str(&value).map_err(|error| {
            AppError::Validation(format!("stored processing settings are invalid: {error}"))
        }),
        None => Ok(ProcessingSettings::default()),
    }
}

pub async fn update(
    pool: &SqlitePool,
    settings: ProcessingSettings,
) -> Result<ProcessingSettings, AppError> {
    normalize(&settings)?;
    let value = serde_json::to_string(&settings).map_err(|error| {
        AppError::Validation(format!("cannot encode processing settings: {error}"))
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

fn normalize(settings: &ProcessingSettings) -> Result<(), AppError> {
    if let Some(detection) = &settings.detection {
        for (label, value) in [
            ("scoreThreshold", detection.score_threshold),
            ("nmsThreshold", detection.nms_threshold),
            ("edgeMarginRatio", detection.edge_margin_ratio),
        ] {
            if let Some(value) = value {
                validate_finite(label, value)?;
                if !(0.0..=1.0).contains(&value) {
                    return Err(range_error(label, "0.0 and 1.0"));
                }
            }
        }
        for (label, value) in [
            ("topK", detection.top_k.map(|v| v as f64)),
            ("areaWeight", detection.area_weight),
            ("confidenceWeight", detection.confidence_weight),
            ("centerWeight", detection.center_weight),
            ("sharpnessWeight", detection.sharpness_weight),
            ("edgePenaltyWeight", detection.edge_penalty_weight),
            ("minSharpness", detection.min_sharpness),
            ("blurPenaltyWeight", detection.blur_penalty_weight),
        ] {
            if let Some(value) = value {
                validate_finite(label, value)?;
                if value < 0.0 {
                    return Err(range_error(label, "0.0 and larger"));
                }
            }
        }
    }
    if let Some(annotation) = &settings.annotation {
        for (label, value) in [
            ("strokeWidth", annotation.stroke_width),
            ("padding", annotation.padding),
            ("fontSize", annotation.font_size),
        ] {
            if let Some(value) = value {
                if value < 0 {
                    return Err(range_error(label, "0 and larger"));
                }
            }
        }
        if let Some(value) = annotation.face_box_expansion {
            if !(0..=4096).contains(&value) {
                return Err(range_error("faceBoxExpansion", "0 and 4096"));
            }
        }
        if let Some(position) = &annotation.fallback_position {
            if !FALLBACK_POSITIONS.contains(&position.as_str()) {
                return Err(AppError::Validation(format!(
                    "fallbackPosition must be one of {}",
                    FALLBACK_POSITIONS.join(", ")
                )));
            }
        }
        if let Some(position) = &annotation.face_text_position {
            if !FACE_TEXT_POSITIONS.contains(&position.as_str()) {
                return Err(AppError::Validation(format!(
                    "faceTextPosition must be one of {}",
                    FACE_TEXT_POSITIONS.join(", ")
                )));
            }
        }
        for (label, value) in [
            ("textOffsetX", annotation.text_offset_x),
            ("textOffsetY", annotation.text_offset_y),
        ] {
            if let Some(value) = value {
                validate_finite(label, value)?;
                if !(-3.0..=3.0).contains(&value) {
                    return Err(range_error(label, "-3.0 and 3.0"));
                }
            }
        }
    }
    if let Some(crop) = &settings.crop {
        if let Some(ratio) = &crop.aspect_ratio {
            let parts: Vec<&str> = ratio.split(':').collect();
            if parts.len() != 2 {
                return Err(AppError::Validation(
                    "aspectRatio must look like '1:1'".to_owned(),
                ));
            }
            for part in parts {
                let Ok(value) = part.parse::<i64>() else {
                    return Err(AppError::Validation(
                        "aspectRatio values must be integers".to_owned(),
                    ));
                };
                if !(1..=100).contains(&value) {
                    return Err(AppError::Validation(
                        "aspectRatio values must be between 1 and 100".to_owned(),
                    ));
                }
            }
        }
        for (label, value) in [
            ("scaleX", crop.scale_x),
            ("scaleTop", crop.scale_top),
            ("scaleBottom", crop.scale_bottom),
        ] {
            if let Some(value) = value {
                validate_finite(label, value)?;
            }
        }
    }
    Ok(())
}

fn validate_finite(label: &str, value: f64) -> Result<(), AppError> {
    if !value.is_finite() {
        return Err(AppError::Validation(format!(
            "{label} must be a finite number"
        )));
    }
    Ok(())
}

fn range_error(label: &str, range: &str) -> AppError {
    AppError::Validation(format!("{label} must be between {range}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::models::vision::{AnnotationSettings, CropSettings, DetectionSettings};

    #[tokio::test]
    async fn defaults_and_round_trips_saved_settings() {
        let pool = db::test_pool().await;
        assert_eq!(
            get(&pool).await.expect("defaults"),
            ProcessingSettings::default()
        );
        let settings = ProcessingSettings {
            detection: Some(DetectionSettings {
                score_threshold: Some(0.7),
                top_k: Some(1000),
                ..Default::default()
            }),
            annotation: Some(AnnotationSettings {
                font_size: Some(64),
                face_box_expansion: Some(48),
                text_color: Some([80, 220, 255]),
                ..Default::default()
            }),
            crop: Some(CropSettings {
                aspect_ratio: Some("3:4".to_owned()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let saved = update(&pool, settings.clone()).await.expect("save");
        assert_eq!(saved, settings);
        assert_eq!(get(&pool).await.expect("read"), settings);
    }

    #[tokio::test]
    async fn rejects_invalid_values() {
        let pool = db::test_pool().await;
        let error = update(
            &pool,
            ProcessingSettings {
                detection: Some(DetectionSettings {
                    score_threshold: Some(1.5),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await
        .expect_err("threshold out of range");
        assert!(matches!(error, AppError::Validation(_)));

        let error = update(
            &pool,
            ProcessingSettings {
                annotation: Some(AnnotationSettings {
                    fallback_position: Some("middle".to_owned()),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await
        .expect_err("unknown position");
        assert!(matches!(error, AppError::Validation(_)));

        let error = update(
            &pool,
            ProcessingSettings {
                annotation: Some(AnnotationSettings {
                    face_text_position: Some("custom".to_owned()),
                    text_offset_x: Some(4.0),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await
        .expect_err("offset out of range");
        assert!(matches!(error, AppError::Validation(_)));

        let error = update(
            &pool,
            ProcessingSettings {
                annotation: Some(AnnotationSettings {
                    face_box_expansion: Some(4097),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await
        .expect_err("expansion out of range");
        assert!(matches!(error, AppError::Validation(_)));

        let error = update(
            &pool,
            ProcessingSettings {
                annotation: Some(AnnotationSettings {
                    face_box_expansion: Some(-1),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await
        .expect_err("negative expansion");
        assert!(matches!(error, AppError::Validation(_)));

        let saved = update(
            &pool,
            ProcessingSettings {
                annotation: Some(AnnotationSettings {
                    face_box_expansion: Some(4096),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await
        .expect("boundary expansion");
        assert_eq!(
            saved.annotation.as_ref().and_then(|a| a.face_box_expansion),
            Some(4096)
        );

        let saved = update(
            &pool,
            ProcessingSettings {
                annotation: Some(AnnotationSettings {
                    face_text_position: Some("custom".to_owned()),
                    text_offset_x: Some(-0.3),
                    text_offset_y: Some(1.25),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await
        .expect("custom position with offsets");
        assert_eq!(
            saved.annotation.as_ref().and_then(|a| a.text_offset_x),
            Some(-0.3)
        );

        let error = update(
            &pool,
            ProcessingSettings {
                crop: Some(CropSettings {
                    aspect_ratio: Some("wide".to_owned()),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .await
        .expect_err("bad ratio");
        assert!(matches!(error, AppError::Validation(_)));
    }
}
