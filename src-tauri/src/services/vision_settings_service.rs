use std::path::Path;

use sqlx::SqlitePool;

use crate::{
    error::AppError,
    models::vision::{UpdateVisionSettingsInput, VisionSettings},
};

const SETTINGS_KEY: &str = "capture.vision";

pub async fn get(pool: &SqlitePool) -> Result<VisionSettings, AppError> {
    let value: Option<String> = sqlx::query_scalar("SELECT value_json FROM settings WHERE key = ?")
        .bind(SETTINGS_KEY)
        .fetch_optional(pool)
        .await?;
    value
        .map(|value| {
            serde_json::from_str(&value).map_err(|error| {
                AppError::Validation(format!("stored vision settings are invalid: {error}"))
            })
        })
        .transpose()
        .map(Option::unwrap_or_default)
}

pub async fn update(
    pool: &SqlitePool,
    input: UpdateVisionSettingsInput,
) -> Result<VisionSettings, AppError> {
    let settings = normalize(input.settings)?;
    let value = serde_json::to_string(&settings)
        .map_err(|error| AppError::Validation(format!("cannot encode vision settings: {error}")))?;
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

pub fn is_configured(settings: &VisionSettings) -> bool {
    let recognizer = settings.recognizer.as_deref().unwrap_or("sface");
    let recognizer_model_configured = if recognizer == "arcface" {
        settings.arcface_model_path.is_some()
    } else {
        settings.sface_model_path.is_some()
    };
    settings.python_executable_path.is_some()
        && settings.python_module_root.is_some()
        && settings.yunet_model_path.is_some()
        && recognizer_model_configured
}

fn normalize(mut settings: VisionSettings) -> Result<VisionSettings, AppError> {
    match settings.recognizer.as_deref() {
        None => settings.recognizer = Some("sface".to_owned()),
        Some("sface") | Some("arcface") => {}
        Some(other) => {
            return Err(AppError::Validation(format!(
                "recognizer must be sface or arcface, got {other}"
            )));
        }
    }
    settings.python_executable_path = normalize_path(
        settings.python_executable_path,
        "Python executable",
        PathKind::File,
    )?;
    settings.python_module_root = normalize_path(
        settings.python_module_root,
        "Python module root",
        PathKind::Directory,
    )?;
    settings.yunet_model_path =
        normalize_path(settings.yunet_model_path, "YuNet model", PathKind::File)?;
    settings.sface_model_path =
        normalize_path(settings.sface_model_path, "SFace model", PathKind::File)?;
    settings.arcface_model_path =
        normalize_path(settings.arcface_model_path, "ArcFace model", PathKind::File)?;
    settings.font_path = normalize_path(settings.font_path, "annotation font", PathKind::File)?;
    Ok(settings)
}

enum PathKind {
    File,
    Directory,
}

fn normalize_path(
    value: Option<String>,
    label: &str,
    kind: PathKind,
) -> Result<Option<String>, AppError> {
    let Some(value) = value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    let path = Path::new(&value);
    if !path.is_absolute() {
        return Err(AppError::Validation(format!(
            "{label} path must be absolute"
        )));
    }
    if value.starts_with(r"\\") {
        return Err(AppError::Validation(format!(
            "{label} must be stored on a local drive"
        )));
    }
    let metadata = std::fs::metadata(path)
        .map_err(|error| AppError::Validation(format!("{label} path is unavailable: {error}")))?;
    let expected_kind = match kind {
        PathKind::File => metadata.is_file(),
        PathKind::Directory => metadata.is_dir(),
    };
    if !expected_kind {
        return Err(AppError::Validation(format!(
            "{label} path has the wrong type"
        )));
    }
    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use tempfile::tempdir;

    #[tokio::test]
    async fn stores_and_reads_optional_vision_settings() {
        let pool = db::test_pool().await;
        let root = tempdir().expect("tempdir");
        let python = root.path().join("python");
        let model = root.path().join("yunet.onnx");
        let sface = root.path().join("sface.onnx");
        std::fs::write(&python, b"python").expect("python");
        std::fs::write(&model, b"model").expect("model");
        std::fs::write(&sface, b"model").expect("sface");
        let saved = update(
            &pool,
            UpdateVisionSettingsInput {
                settings: VisionSettings {
                    python_executable_path: Some(python.to_string_lossy().into_owned()),
                    python_module_root: Some(root.path().to_string_lossy().into_owned()),
                    yunet_model_path: Some(model.to_string_lossy().into_owned()),
                    sface_model_path: Some(sface.to_string_lossy().into_owned()),
                    recognizer: None,
                    arcface_model_path: None,
                    font_path: None,
                },
            },
        )
        .await
        .expect("save");
        assert!(is_configured(&saved));
        assert_eq!(
            get(&pool).await.expect("read").yunet_model_path,
            saved.yunet_model_path
        );
    }
}
