use std::path::{Path, PathBuf};

use sqlx::SqlitePool;

use crate::{
    error::AppError,
    models::vision::{UpdateVisionSettingsInput, VisionSettings},
};

const SETTINGS_KEY: &str = "capture.vision";
const BUNDLED_MODELS_REL: &str = "models";
const BUNDLED_FONTS_REL: &str = "fonts";

/// How the AI engine is launched. The PyInstaller sidecar ships next to the
/// main executable and embeds its own Python interpreter; the legacy mode
/// runs a configured system/venv Python with `-m scene_vault_ai`.
#[derive(Debug, Clone)]
pub enum EngineRuntime {
    Python {
        executable: PathBuf,
        module_root: PathBuf,
    },
    Sidecar {
        path: PathBuf,
    },
}

/// The bundled sidecar next to the current executable, if any. Tauri places
/// `externalBin` binaries in the same directory as the main app binary, so
/// `current_exe()`'s parent is the install directory on Windows.
pub fn sidecar_executable() -> Option<PathBuf> {
    let install_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    sidecar_executable_in(&install_dir)
}

fn sidecar_executable_in(install_dir: &Path) -> Option<PathBuf> {
    for name in ["scene-vault-ai.exe", "scene-vault-ai"] {
        let candidate = install_dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// The engine is usable when either the legacy Python settings are fully
/// configured or the bundled sidecar is present.
pub fn is_engine_available(settings: &VisionSettings) -> bool {
    is_configured(settings) || sidecar_executable().is_some()
}

/// Resolves how the engine should be spawned: the bundled sidecar wins over a
/// configured Python executable so a clean AI installer works without any
/// system Python.
pub fn engine_runtime(settings: &VisionSettings) -> Option<EngineRuntime> {
    engine_runtime_with(settings, sidecar_executable().as_deref())
}

fn engine_runtime_with(settings: &VisionSettings, sidecar: Option<&Path>) -> Option<EngineRuntime> {
    if let Some(path) = sidecar {
        return Some(EngineRuntime::Sidecar {
            path: path.to_path_buf(),
        });
    }
    let executable = settings.python_executable_path.as_ref()?;
    let module_root = settings.python_module_root.as_ref()?;
    Some(EngineRuntime::Python {
        executable: PathBuf::from(executable),
        module_root: PathBuf::from(module_root),
    })
}

/// First-run convenience for the AI-enabled installer: when the bundled
/// sidecar exists, fill any missing model/font settings from the bundled
/// resources. Existing user-configured values are never overwritten. The font
/// is copied into the app-local fonts directory so the settings UI lists it.
pub async fn autoconfigure_bundled(
    pool: &SqlitePool,
    app_local_dir: &Path,
) -> Result<(), AppError> {
    let Some(sidecar) = sidecar_executable() else {
        return Ok(());
    };
    autoconfigure_bundled_with(pool, app_local_dir, &sidecar).await
}

async fn autoconfigure_bundled_with(
    pool: &SqlitePool,
    app_local_dir: &Path,
    sidecar: &Path,
) -> Result<(), AppError> {
    let resources = sidecar
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join("resources");
    let mut settings = get(pool).await?;
    let mut changed = false;

    let yunet = resources
        .join(BUNDLED_MODELS_REL)
        .join("face_detection_yunet_2023mar.onnx");
    if settings.yunet_model_path.is_none() && yunet.is_file() {
        settings.yunet_model_path = Some(yunet.to_string_lossy().into_owned());
        changed = true;
    }
    let sface = resources
        .join(BUNDLED_MODELS_REL)
        .join("face_recognition_sface_2021dec.onnx");
    if settings.sface_model_path.is_none() && sface.is_file() {
        settings.sface_model_path = Some(sface.to_string_lossy().into_owned());
        changed = true;
    }
    // An arcface recognizer without its model file is a broken configuration;
    // the bundled SFace model is always available, so fall back to it.
    if settings.recognizer.as_deref() == Some("arcface")
        && settings.arcface_model_path.is_none()
        && settings.sface_model_path.is_some()
    {
        settings.recognizer = Some("sface".to_owned());
        changed = true;
    }
    if settings.font_path.is_none() {
        let bundled_font = resources
            .join(BUNDLED_FONTS_REL)
            .join("SmileySans-Oblique.ttf");
        if bundled_font.is_file() {
            std::fs::create_dir_all(app_local_dir.join("fonts"))?;
            let target = app_local_dir.join("fonts").join("SmileySans-Oblique.ttf");
            if !target.is_file() {
                std::fs::copy(&bundled_font, &target)?;
            }
            settings.font_path = Some(target.to_string_lossy().into_owned());
            changed = true;
        }
    }
    if changed {
        update(
            pool,
            UpdateVisionSettingsInput {
                settings: settings.clone(),
            },
        )
        .await?;
    }
    Ok(())
}

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
    let previous = get(pool).await?;
    let settings = normalize(input.settings)?;
    let previous_recognizer = previous.recognizer.as_deref().unwrap_or("sface");
    let next_recognizer = settings.recognizer.as_deref().unwrap_or("sface");
    if previous_recognizer != next_recognizer {
        // Pending suggestions were computed with the old recognizer model and
        // are meaningless under the new one. Clear them so stale suggestions
        // are never shown or accepted after a model switch; the next prelabel
        // pass or face-bank rebuild recomputes them with the active model.
        sqlx::query(
            r#"
            UPDATE capture_items
            SET suggested_character_id = NULL,
                recognition_confidence = NULL,
                recognition_source = NULL,
                review_status = 'none',
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
            WHERE review_status = 'pending'
              AND suggested_character_id IS NOT NULL
            "#,
        )
        .execute(pool)
        .await?;
        // Awaiting-label items keep their extracted feature until it is
        // cleared: the prelabel pass only re-picks items without a feature,
        // so a model switch would otherwise leave them forever carrying a
        // vector from the old recognizer. Clear theirs so the next prelabel
        // pass re-extracts with the active model and fresh suggestions.
        sqlx::query(
            r#"
            UPDATE capture_faces
            SET feature_json = NULL,
                feature_model_id = NULL,
                feature_model_version = NULL,
                feature_dim = NULL
            WHERE capture_item_id IN (
                SELECT id FROM capture_items WHERE status = 'awaiting_label'
            )
            "#,
        )
        .execute(pool)
        .await?;
    }
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

    #[test]
    fn sidecar_resolution_prefers_the_bundled_executable() {
        let install = tempdir().expect("tempdir");
        let sidecar = install.path().join("scene-vault-ai.exe");
        std::fs::write(&sidecar, b"exe").expect("sidecar");

        assert_eq!(sidecar_executable_in(install.path()), Some(sidecar.clone()));

        let settings = VisionSettings {
            python_executable_path: Some("C:\\python.exe".to_owned()),
            python_module_root: Some("C:\\src".to_owned()),
            yunet_model_path: None,
            sface_model_path: None,
            recognizer: None,
            arcface_model_path: None,
            font_path: None,
        };
        // The sidecar wins over a configured Python executable.
        assert!(matches!(
            engine_runtime_with(&settings, Some(&sidecar)),
            Some(EngineRuntime::Sidecar { .. })
        ));
        // Without a sidecar the configured Python is used.
        assert!(matches!(
            engine_runtime_with(&settings, None),
            Some(EngineRuntime::Python { .. })
        ));
        // Neither present: no runtime.
        let empty = VisionSettings::default();
        assert!(engine_runtime_with(&empty, None).is_none());
    }

    #[test]
    fn sidecar_is_not_found_when_absent() {
        let install = tempdir().expect("tempdir");
        assert!(sidecar_executable_in(install.path()).is_none());
    }

    #[tokio::test]
    async fn autoconfigure_fills_bundled_models_and_font_without_overwriting() {
        let pool = db::test_pool().await;
        let install = tempdir().expect("install dir");
        let app_local = tempdir().expect("app local dir");

        let sidecar = install.path().join("scene-vault-ai.exe");
        std::fs::write(&sidecar, b"exe").expect("sidecar");
        let models = install.path().join("resources/models");
        let fonts = install.path().join("resources/fonts");
        std::fs::create_dir_all(&models).expect("models dir");
        std::fs::create_dir_all(&fonts).expect("fonts dir");
        let yunet = models.join("face_detection_yunet_2023mar.onnx");
        let sface = models.join("face_recognition_sface_2021dec.onnx");
        let font = fonts.join("SmileySans-Oblique.ttf");
        std::fs::write(&yunet, b"yunet").expect("yunet");
        std::fs::write(&sface, b"sface").expect("sface");
        std::fs::write(&font, b"font").expect("font");

        // First run: everything missing is filled from the bundle.
        autoconfigure_bundled_with(&pool, app_local.path(), &sidecar)
            .await
            .expect("autoconfigure");
        let settings = get(&pool).await.expect("read");
        assert_eq!(
            settings.yunet_model_path,
            Some(yunet.to_string_lossy().into_owned())
        );
        assert_eq!(
            settings.sface_model_path,
            Some(sface.to_string_lossy().into_owned())
        );
        assert!(settings.font_path.is_some());
        assert!(app_local
            .path()
            .join("fonts/SmileySans-Oblique.ttf")
            .is_file());

        // User-configured values are never overwritten.
        let custom_yunet = install.path().join("custom-yunet.onnx");
        std::fs::write(&custom_yunet, b"custom").expect("custom yunet");
        let custom_python = install.path().join("custom-python.exe");
        std::fs::write(&custom_python, b"python").expect("custom python");
        std::fs::create_dir_all(install.path().join("custom-src")).expect("custom src");
        let custom = VisionSettings {
            python_executable_path: Some(custom_python.to_string_lossy().into_owned()),
            python_module_root: Some(
                install
                    .path()
                    .join("custom-src")
                    .to_string_lossy()
                    .into_owned(),
            ),
            yunet_model_path: Some(custom_yunet.to_string_lossy().into_owned()),
            sface_model_path: None,
            recognizer: Some("sface".to_owned()),
            arcface_model_path: None,
            font_path: None,
        };
        update(
            &pool,
            UpdateVisionSettingsInput {
                settings: custom.clone(),
            },
        )
        .await
        .expect("save custom");
        autoconfigure_bundled_with(&pool, app_local.path(), &sidecar)
            .await
            .expect("autoconfigure again");
        let settings = get(&pool).await.expect("read again");
        assert_eq!(settings.yunet_model_path, custom.yunet_model_path);
        assert_eq!(
            settings.sface_model_path,
            Some(sface.to_string_lossy().into_owned())
        );

        // An arcface recognizer without its model file falls back to the
        // bundled SFace model on the next startup.
        let broken = VisionSettings {
            python_executable_path: None,
            python_module_root: None,
            yunet_model_path: Some(custom_yunet.to_string_lossy().into_owned()),
            sface_model_path: None,
            recognizer: Some("arcface".to_owned()),
            arcface_model_path: None,
            font_path: None,
        };
        update(
            &pool,
            UpdateVisionSettingsInput {
                settings: broken.clone(),
            },
        )
        .await
        .expect("save broken");
        autoconfigure_bundled_with(&pool, app_local.path(), &sidecar)
            .await
            .expect("autoconfigure broken");
        let settings = get(&pool).await.expect("read broken");
        assert_eq!(settings.recognizer.as_deref(), Some("sface"));
        assert!(settings.sface_model_path.is_some());
    }

    #[tokio::test]
    async fn recognizer_change_clears_pending_suggestions() {
        use crate::{
            models::{
                capture::{LabelCaptureInput, RegisterCaptureInput},
                character::CreateCharacterInput,
                project::{
                    AddProjectSourceDirectoryInput, CreateProjectInput, SetProjectDestinationInput,
                },
                recognition::SetRecognitionSuggestionInput,
            },
            services::{
                capture_service, character_service, project_service, recognition_service,
                test_support,
            },
        };
        let pool = db::test_pool().await;
        let project = project_service::create(
            &pool,
            CreateProjectInput {
                name: "RecognizerSwitch".to_owned(),
                description: None,
                cover_asset_id: None,
            },
        )
        .await
        .expect("project");
        let character = character_service::create(
            &pool,
            CreateCharacterInput {
                project_id: project.id.clone(),
                name: "Ava".to_owned(),
                aliases_json: None,
            },
        )
        .await
        .expect("character");
        let other = character_service::create(
            &pool,
            CreateCharacterInput {
                project_id: project.id.clone(),
                name: "Bella".to_owned(),
                aliases_json: None,
            },
        )
        .await
        .expect("other");
        let workspace = tempdir().expect("tempdir");
        let source_directory = workspace.path().join("source-dir");
        std::fs::create_dir(&source_directory).expect("source dir");
        project_service::add_source_directory(
            &pool,
            AddProjectSourceDirectoryInput {
                project_id: project.id.clone(),
                directory: capture_service::path_to_string(&source_directory),
            },
        )
        .await
        .expect("add source directory");
        let destination = workspace.path().join("archive");
        std::fs::create_dir(&destination).expect("destination");
        project_service::set_destination_directory(
            &pool,
            SetProjectDestinationInput {
                project_id: project.id.clone(),
                directory: capture_service::path_to_string(&destination),
            },
        )
        .await
        .expect("set destination");
        let session = test_support::start_session(&pool, &project.id)
            .await
            .expect("session");
        let source = source_directory.join("source.png");
        std::fs::write(&source, b"image").expect("source");
        let item = capture_service::register_capture(
            &pool,
            RegisterCaptureInput {
                session_id: session.id.clone(),
                source_path: source.to_string_lossy().into_owned(),
            },
        )
        .await
        .expect("register");
        let item = capture_service::label_capture(
            &pool,
            LabelCaptureInput {
                capture_item_id: item.id,
                character_id: Some(character.id),
                classification: Some("person".to_owned()),
            },
        )
        .await
        .expect("label");
        // A second capture stays awaiting-label with an extracted feature.
        let awaiting_source = source_directory.join("awaiting.png");
        std::fs::write(&awaiting_source, b"image").expect("awaiting source");
        let awaiting = capture_service::register_capture(
            &pool,
            RegisterCaptureInput {
                session_id: session.id,
                source_path: awaiting_source.to_string_lossy().into_owned(),
            },
        )
        .await
        .expect("register awaiting");
        sqlx::query("UPDATE capture_items SET status = 'awaiting_label' WHERE id = ?")
            .bind(&awaiting.id)
            .execute(&pool)
            .await
            .expect("awaiting status");
        sqlx::query(
            r#"
            INSERT INTO capture_faces (
                id, capture_item_id, face_index, is_primary, feature_json,
                feature_model_id, feature_model_version, feature_dim
            )
            VALUES (?, ?, 0, 1, '[0.1,0.2]', 'arcface-r50', 'w600k', 2)
            "#,
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(&awaiting.id)
        .execute(&pool)
        .await
        .expect("store awaiting feature");
        recognition_service::set_suggestion(
            &pool,
            SetRecognitionSuggestionInput {
                capture_item_id: item.id.clone(),
                suggested_character_id: Some(other.id),
                confidence: Some(0.8),
                source: Some("face_bank".to_owned()),
            },
        )
        .await
        .expect("suggest");

        // Saving with the same recognizer keeps the pending suggestion.
        update(
            &pool,
            UpdateVisionSettingsInput {
                settings: VisionSettings {
                    recognizer: Some("sface".to_owned()),
                    ..VisionSettings::default()
                },
            },
        )
        .await
        .expect("save same recognizer");
        let suggested: Option<String> =
            sqlx::query_scalar("SELECT suggested_character_id FROM capture_items WHERE id = ?")
                .bind(&item.id)
                .fetch_one(&pool)
                .await
                .expect("read suggestion");
        let awaiting_feature: Option<String> =
            sqlx::query_scalar("SELECT feature_json FROM capture_faces WHERE capture_item_id = ?")
                .bind(&awaiting.id)
                .fetch_one(&pool)
                .await
                .expect("read awaiting feature");
        assert!(suggested.is_some());
        assert!(awaiting_feature.is_some());

        // Switching the recognizer clears it.
        update(
            &pool,
            UpdateVisionSettingsInput {
                settings: VisionSettings {
                    recognizer: Some("arcface".to_owned()),
                    ..VisionSettings::default()
                },
            },
        )
        .await
        .expect("save recognizer switch");
        let suggested: Option<String> =
            sqlx::query_scalar("SELECT suggested_character_id FROM capture_items WHERE id = ?")
                .bind(&item.id)
                .fetch_one(&pool)
                .await
                .expect("read cleared suggestion");
        let awaiting_feature: Option<String> =
            sqlx::query_scalar("SELECT feature_json FROM capture_faces WHERE capture_item_id = ?")
                .bind(&awaiting.id)
                .fetch_one(&pool)
                .await
                .expect("read cleared awaiting feature");
        let awaiting_model: Option<String> = sqlx::query_scalar(
            "SELECT feature_model_id FROM capture_faces WHERE capture_item_id = ?",
        )
        .bind(&awaiting.id)
        .fetch_one(&pool)
        .await
        .expect("read cleared awaiting model");
        let review: String =
            sqlx::query_scalar("SELECT review_status FROM capture_items WHERE id = ?")
                .bind(&item.id)
                .fetch_one(&pool)
                .await
                .expect("read review");
        assert!(suggested.is_none());
        assert!(awaiting_feature.is_none());
        assert!(awaiting_model.is_none());
        assert_eq!(review, "none");
    }
}
