use std::path::{Path, PathBuf};

use chrono::{SecondsFormat, Utc};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use tauri::{AppHandle, Manager};

use crate::{
    error::AppError,
    models::engine::{
        BridgeFileEntry, BridgeUninstallResult, InstallEngineBridgeInput, ProjectEngineInput,
        ProjectEngineIntegration, ProjectEngineIntegrationView, UninstallEngineBridgeInput,
    },
    models::project::AddProjectSourceDirectoryInput,
    services::{
        capture_service, engine_detection_service, log_service, renpy_screenshot_settings_service,
    },
};

pub const BRIDGE_FILE_NAME: &str = "00_scenevault_bridge.rpy";
pub const BRIDGE_COMPILED_FILE_NAME: &str = "00_scenevault_bridge.rpyc";
pub const BRIDGE_CONFIG_FILE_NAME: &str = "scenevault_bridge.json";
pub const BRIDGE_VERSION: &str = "1.1.7";

/// Distinctive string constant inside the compiled bridge; used together with
/// the verified source hash before the engine-compiled artifact is recorded
/// for deletion.
const COMPILED_MARKER: &[u8] = b"scenevault_bridge.json";

/// Bridge mutations are serialized: two projects racing on the same game
/// directory must never roll back each other's files, and uninstall must see
/// a stable directory while it verifies ownership.
static BRIDGE_MUTATION_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const INTEGRATION_COLUMNS: &str =
    "id, project_id, engine, engine_root, game_directory, python_major, engine_version, \
    bridge_version, auto_hide_ui, auto_hide_ui_override, metadata_directory, manifest_json, installed_at, updated_at";

/// Installs the bridge into the picked game: writes the `.rpy` glue plus a
/// small config file and records both hashes so uninstall only removes files
/// that still match what Scene Vault created.
pub async fn install(
    pool: &SqlitePool,
    app: &AppHandle,
    input: InstallEngineBridgeInput,
) -> Result<ProjectEngineIntegrationView, AppError> {
    let project_id = input.project_id.trim();
    if project_id.is_empty() {
        return Err(AppError::Validation(
            "project id cannot be empty".to_owned(),
        ));
    }
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM projects WHERE id = ?)")
        .bind(project_id)
        .fetch_one(pool)
        .await?;
    if !exists {
        return Err(AppError::NotFound("project".to_owned()));
    }

    let _guard = BRIDGE_MUTATION_LOCK.lock().await;
    let _policy_guard = renpy_screenshot_settings_service::lock().await;

    let detection = engine_detection_service::detect(Path::new(input.path.trim())).await?;
    let game_directory = PathBuf::from(&detection.game_directory);
    // One game directory can only be owned by one project; otherwise a later
    // uninstall would delete the bridge another project still uses.
    let owner: Option<String> = sqlx::query_scalar(
        "SELECT project_id FROM project_engine_integrations WHERE game_directory = ? AND project_id != ?",
    )
    .bind(&detection.game_directory)
    .bind(project_id)
    .fetch_optional(pool)
    .await?;
    if let Some(owner) = owner {
        let name: Option<String> = sqlx::query_scalar("SELECT name FROM projects WHERE id = ?")
            .bind(&owner)
            .fetch_optional(pool)
            .await?;
        return Err(AppError::Conflict(format!(
            "该游戏目录已由项目「{}」启用引擎适配：一个游戏目录只能有一个安装所有者",
            name.unwrap_or(owner)
        )));
    }
    // A project may own several games now: installing another game never
    // touches the existing installs. Reinstalling the same game reuses its
    // row (and its metadata directory) so pending sidecars survive.
    let existing_same_game: Option<ProjectEngineIntegration> =
        sqlx::query_as::<_, ProjectEngineIntegration>(&format!(
            "SELECT {INTEGRATION_COLUMNS} FROM project_engine_integrations \
             WHERE project_id = ? AND game_directory = ?"
        ))
        .bind(project_id)
        .bind(&detection.game_directory)
        .fetch_optional(pool)
        .await?;
    let auto_hide_ui_override = input.auto_hide_ui.or_else(|| {
        existing_same_game
            .as_ref()
            .and_then(|integration| integration.auto_hide_ui_override)
    });
    let auto_hide_ui =
        renpy_screenshot_settings_service::resolve(pool, project_id, auto_hide_ui_override).await?;
    let mut previous_manifest: Vec<BridgeFileEntry> = existing_same_game
        .as_ref()
        .map(|integration| serde_json::from_str(&integration.manifest_json))
        .transpose()
        .map_err(|error| AppError::Validation(format!("bridge manifest is invalid: {error}")))?
        .unwrap_or_default();
    let source = bundled_bridge_source(app).await?;
    // One install owns one subdirectory: screenshots with the same file name
    // from other games or projects can never pair with the wrong metadata.
    // Each install keeps its own metadata subdirectory. Legacy rows without
    // one (or rows that were patched to the shared root) get a fresh private
    // directory instead of re-sharing the root with other installs.
    let shared_root = bridge_metadata_directory(app)?;
    let metadata_directory = match existing_same_game
        .as_ref()
        .and_then(|integration| integration.metadata_directory.as_deref())
        .map(PathBuf::from)
        .filter(|directory| directory != &shared_root)
    {
        Some(directory) => directory,
        None => shared_root.join(format!("install-{}", uuid::Uuid::new_v4())),
    };
    tokio::fs::create_dir_all(&metadata_directory).await?;
    let log_file = bridge_cache_directory(app)?.join("bridge.log");

    let integration_id = existing_same_game
        .as_ref()
        .map(|integration| integration.id.clone())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let policy_path = renpy_screenshot_settings_service::policy_path(app)?;
    let config = serde_json::to_vec_pretty(&json!({
        "schemaVersion": 1,
        "bridgeVersion": BRIDGE_VERSION,
        "autoHideUi": auto_hide_ui,
        "projectId": project_id,
        "integrationId": integration_id,
        "screenshotPolicyFile": capture_service::path_to_string(&policy_path),
        "metadataDirectory": capture_service::path_to_string(&metadata_directory),
        "logFile": capture_service::path_to_string(&log_file),
        "generatedAt": Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
    }))
    .map_err(|error| AppError::Validation(format!("cannot encode bridge config: {error}")))?;

    let bridge_path = game_directory.join(BRIDGE_FILE_NAME);
    let config_path = game_directory.join(BRIDGE_CONFIG_FILE_NAME);
    let compiled_path = game_directory.join(BRIDGE_COMPILED_FILE_NAME);
    let compiled_text = capture_service::path_to_string(&compiled_path);
    // Reinstall audit for the engine-compiled artifact: our recorded artifact
    // is removed so the engine recompiles the new source (its registration is
    // cleared for first-time registration later); anything else is kept and
    // reported, because only the exact recorded bytes count as ours.
    let mut compiled_warning: Option<String> = None;
    let mut remove_recorded_compiled = false;
    let recorded_compiled = previous_manifest
        .iter()
        .find(|entry| entry.path == compiled_text)
        .map(|entry| entry.sha256.clone());
    if let Some(recorded_hash) = recorded_compiled {
        let file_present = file_exists(&compiled_path).await?;
        let current_hash = if file_present {
            capture_service::sha256_file(&compiled_path).await.ok()
        } else {
            None
        };
        match reinstall_compiled_state(Some(&recorded_hash), file_present, current_hash.as_deref())
        {
            ReinstallCompiledState::Missing => {
                previous_manifest.retain(|entry| entry.path != compiled_text);
            }
            ReinstallCompiledState::Ours => {
                previous_manifest.retain(|entry| entry.path != compiled_text);
                remove_recorded_compiled = true;
            }
            ReinstallCompiledState::Unverified => {
                let warning = format!(
                    "已保留无法确认归属的旧编译产物：{}（可手动删除后重新安装）",
                    compiled_path.display()
                );
                log_service::warn("engine.bridge", warning.clone());
                compiled_warning = Some(warning);
            }
            ReinstallCompiledState::NoEntry => {}
        }
    }
    // Never overwrite a file Scene Vault cannot prove it owns.
    for target in [&bridge_path, &config_path] {
        ensure_target_is_ours(target, &previous_manifest).await?;
    }
    // Two-phase write with rollback: keep the previous bytes, then write both
    // files; any failure restores what was there before.
    let backups = snapshot_targets([&bridge_path, &config_path]).await?;
    let mut transaction = pool.begin().await?;
    if let Err(error) = async {
        write_atomic(&bridge_path, source.as_bytes()).await?;
        write_atomic(&config_path, &config).await
    }
    .await
    {
        let failures = restore_backups(&backups).await;
        return Err(install_failure_with_restore(error, failures));
    }

    let manifest = merge_install_manifest(
        &previous_manifest,
        capture_service::path_to_string(&bridge_path),
        capture_service::path_to_string(&config_path),
        &source,
        &config,
    );
    let manifest_json = serde_json::to_string(&manifest)
        .map_err(|error| AppError::Validation(format!("cannot encode bridge manifest: {error}")))?;
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);

    let persisted = if existing_same_game.is_some() {
        sqlx::query(
            r#"
            UPDATE project_engine_integrations SET
                engine = ?,
                engine_root = ?,
                python_major = ?,
                engine_version = ?,
                bridge_version = ?,
                auto_hide_ui = ?,
                auto_hide_ui_override = ?,
                metadata_directory = ?,
                manifest_json = ?,
                installed_at = ?,
                updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(&detection.engine)
        .bind(&detection.engine_root)
        .bind(detection.python_major)
        .bind(detection.engine_version.as_deref())
        .bind(BRIDGE_VERSION)
        .bind(auto_hide_ui)
        .bind(auto_hide_ui_override)
        .bind(capture_service::path_to_string(&metadata_directory))
        .bind(&manifest_json)
        .bind(&now)
        .bind(&now)
        .bind(&integration_id)
        .execute(&mut *transaction)
        .await
    } else {
        sqlx::query(
            r#"
            INSERT INTO project_engine_integrations (
                id, project_id, engine, engine_root, game_directory, python_major,
                engine_version, bridge_version, auto_hide_ui, auto_hide_ui_override, metadata_directory, manifest_json,
                installed_at, updated_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&integration_id)
        .bind(project_id)
        .bind(&detection.engine)
        .bind(&detection.engine_root)
        .bind(&detection.game_directory)
        .bind(detection.python_major)
        .bind(detection.engine_version.as_deref())
        .bind(BRIDGE_VERSION)
        .bind(auto_hide_ui)
        .bind(auto_hide_ui_override)
        .bind(capture_service::path_to_string(&metadata_directory))
        .bind(&manifest_json)
        .bind(&now)
        .bind(&now)
        .execute(&mut *transaction)
        .await
    };
    if let Err(error) = persisted {
        let failures = restore_backups(&backups).await;
        return Err(install_failure_with_restore(error.into(), failures));
    }
    let policy_backup =
        match renpy_screenshot_settings_service::stage_snapshot(&mut transaction, &policy_path)
            .await
        {
            Ok(backup) => backup,
            Err(error) => {
                let failures = restore_backups(&backups).await;
                return Err(install_failure_with_restore(error, failures));
            }
        };
    if let Err(error) = transaction.commit().await {
        let mut failures = restore_backups(&backups).await;
        if let Err(restore_error) =
            renpy_screenshot_settings_service::restore_snapshot(policy_backup).await
        {
            failures.push(format!("截图设置缓存（{restore_error}）"));
        }
        return Err(install_failure_with_restore(error.into(), failures));
    }
    if remove_recorded_compiled {
        if let Err(error) = tokio::fs::remove_file(&compiled_path).await {
            log_service::warn(
                "engine.bridge",
                format!("could not remove the recorded compiled artifact: {error}"),
            );
            compiled_warning = Some(format!(
                "已登记的旧编译产物删除失败：{}（可手动删除后重新安装）",
                compiled_path.display()
            ));
        }
    }

    log_service::info(
        "engine.bridge",
        format!(
            "installed Ren'Py bridge {BRIDGE_VERSION} for project {project_id} at {}",
            detection.game_directory
        ),
    );
    // Screenshots land in the game root (config.renpy_base); make sure the
    // watcher covers it. A duplicate or a destination conflict is advisory:
    // the bridge itself is installed either way.
    if let Err(error) = super::project_service::add_source_directory(
        pool,
        AddProjectSourceDirectoryInput {
            project_id: project_id.to_owned(),
            directory: detection.engine_root.clone(),
        },
    )
    .await
    {
        log_service::info(
            "engine.bridge",
            format!("game root not added as a capture source: {error}"),
        );
    }
    Ok(ProjectEngineIntegrationView {
        resolved_auto_hide_ui: auto_hide_ui,
        integration: load_integration_by_id(pool, &integration_id)
            .await?
            .ok_or_else(|| AppError::NotFound("engine integration".to_owned()))?,
        bridge_file_present: true,
        compiled_file_present: compiled_path.is_file(),
        bundled_bridge_version: BRIDGE_VERSION.to_owned(),
        warning: compiled_warning,
    })
}

/// Removes only files whose ownership is proven: manifest entries must still
/// match their recorded hash, and the engine-compiled artifact is recorded
/// first when possible. Unproven files are kept and reported, and the
/// integration row stays so the user can retry after resolving them.
pub async fn uninstall(
    pool: &SqlitePool,
    app: Option<&AppHandle>,
    input: UninstallEngineBridgeInput,
) -> Result<BridgeUninstallResult, AppError> {
    let project_id = input.project_id.trim();
    if project_id.is_empty() {
        return Err(AppError::Validation(
            "project id cannot be empty".to_owned(),
        ));
    }
    let _guard = BRIDGE_MUTATION_LOCK.lock().await;
    let _policy_guard = renpy_screenshot_settings_service::lock().await;
    let integration = match input
        .integration_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(integration_id) => match load_integration_by_id(pool, integration_id).await? {
            Some(integration) if integration.project_id == project_id => integration,
            Some(_) => return Err(AppError::Validation("该插件安装不属于当前项目".to_owned())),
            None => {
                return Ok(BridgeUninstallResult {
                    removed_files: Vec::new(),
                    kept_files: Vec::new(),
                })
            }
        },
        None => {
            let mut integrations = load_integrations(pool, project_id).await?;
            match integrations.len() {
                0 => {
                    return Ok(BridgeUninstallResult {
                        removed_files: Vec::new(),
                        kept_files: Vec::new(),
                    })
                }
                1 => integrations.remove(0),
                _ => {
                    return Err(AppError::Validation(
                        "该项目安装了多个 Ren'Py 插件，请指定要卸载的游戏".to_owned(),
                    ))
                }
            }
        }
    };

    let (removed, kept) = remove_verified_files(pool, &integration).await?;
    if kept.is_empty() {
        sqlx::query("DELETE FROM project_engine_integrations WHERE id = ?")
            .bind(&integration.id)
            .execute(pool)
            .await?;
        remove_metadata_directory(app, integration.metadata_directory.as_deref());
        if let Some(app) = app {
            if let Err(error) = renpy_screenshot_settings_service::publish_locked(pool, app).await {
                log_service::warn(
                    "engine.screenshot",
                    format!("could not refresh screenshot policy after uninstall: {error}"),
                );
            }
        }
    }
    log_service::info(
        "engine.bridge",
        format!(
            "uninstalled Ren'Py bridge for project {project_id} at {}: removed {} file(s), kept {}",
            integration.game_directory,
            removed.len(),
            kept.len()
        ),
    );
    Ok(BridgeUninstallResult {
        removed_files: removed,
        kept_files: kept,
    })
}

pub async fn status(
    pool: &SqlitePool,
    app: Option<&AppHandle>,
    input: ProjectEngineInput,
) -> Result<Vec<ProjectEngineIntegrationView>, AppError> {
    let project_id = input.project_id.trim();
    // status can repair the metadata path and record compiled hashes; share
    // the mutation lock so it cannot overwrite a concurrent reinstall.
    let _guard = BRIDGE_MUTATION_LOCK.lock().await;
    let integrations = load_integrations(pool, project_id).await?;
    if integrations.is_empty() {
        return Ok(Vec::new());
    }
    let policy = renpy_screenshot_settings_service::project_policy(pool, project_id).await?;
    let mut views = Vec::new();
    for mut integration in integrations {
        // Legacy rows may miss the staging root entirely, or still point at
        // the shared root from before installs were isolated. Both get a
        // fresh private subdirectory rather than sharing one again.
        let needs_private_directory = match (integration.metadata_directory.as_deref(), app) {
            (None, Some(_)) => true,
            (Some(current), Some(app)) => bridge_metadata_directory(app)
                .map(|root| Path::new(current) == root)
                .unwrap_or(false),
            _ => false,
        };
        if needs_private_directory {
            if let Some(app) = app {
                if let Ok(root) = bridge_metadata_directory(app) {
                    let directory = root.join(format!("install-{}", uuid::Uuid::new_v4()));
                    if tokio::fs::create_dir_all(&directory).await.is_ok() {
                        let text = capture_service::path_to_string(&directory);
                        sqlx::query(
                            "UPDATE project_engine_integrations SET metadata_directory = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
                        )
                        .bind(&text)
                        .bind(&integration.id)
                        .execute(pool)
                        .await?;
                        integration.metadata_directory = Some(text);
                    }
                }
            }
        }
        let game_directory = PathBuf::from(&integration.game_directory);
        let bridge_path = game_directory.join(BRIDGE_FILE_NAME);
        let bridge_file_present = tokio::fs::metadata(&bridge_path)
            .await
            .is_ok_and(|metadata| metadata.is_file());
        let compiled_path = game_directory.join(BRIDGE_COMPILED_FILE_NAME);
        let compiled_file_present = tokio::fs::metadata(&compiled_path)
            .await
            .is_ok_and(|metadata| metadata.is_file());
        if bridge_file_present && compiled_file_present {
            if let Err(error) =
                record_compiled_hash(pool, &mut integration, &bridge_path, &compiled_path).await
            {
                log_service::warn(
                    "engine.bridge",
                    format!("could not record compiled bridge artifact: {error}"),
                );
            }
        }
        views.push(ProjectEngineIntegrationView {
            resolved_auto_hide_ui: integration
                .auto_hide_ui_override
                .unwrap_or(policy.resolved_auto_hide_ui),
            integration,
            bridge_file_present,
            compiled_file_present,
            bundled_bridge_version: BRIDGE_VERSION.to_owned(),
            warning: None,
        });
    }
    Ok(views)
}

pub async fn load_integrations(
    pool: &SqlitePool,
    project_id: &str,
) -> Result<Vec<ProjectEngineIntegration>, AppError> {
    if project_id.is_empty() {
        return Err(AppError::Validation(
            "project id cannot be empty".to_owned(),
        ));
    }
    Ok(sqlx::query_as::<_, ProjectEngineIntegration>(&format!(
        "SELECT {INTEGRATION_COLUMNS} FROM project_engine_integrations \
         WHERE project_id = ? ORDER BY installed_at ASC, id ASC"
    ))
    .bind(project_id)
    .fetch_all(pool)
    .await?)
}

/// Backfills old registrations from the version files without reinstalling
/// or launching a game. A guarded update cannot recreate an uninstalled row
/// or overwrite a version recorded by a concurrent reinstall.
pub(crate) async fn refresh_missing_engine_versions(pool: &SqlitePool) -> Result<(), AppError> {
    let rows: Vec<(String, String, Option<i64>)> = sqlx::query_as(
        "SELECT id, engine_root, python_major FROM project_engine_integrations \
         WHERE engine = 'renpy' AND (engine_version IS NULL OR TRIM(engine_version) = '')",
    )
    .fetch_all(pool)
    .await?;
    for (id, root, python_major) in rows {
        let Some(version) =
            engine_detection_service::detect_engine_version(Path::new(&root), python_major).await
        else {
            continue;
        };
        sqlx::query(
            "UPDATE project_engine_integrations SET engine_version = ? \
             WHERE id = ? AND engine_root = ? \
             AND (engine_version IS NULL OR TRIM(engine_version) = '')",
        )
        .bind(version)
        .bind(id)
        .bind(root)
        .execute(pool)
        .await?;
    }
    Ok(())
}

pub async fn load_integration_by_id(
    pool: &SqlitePool,
    integration_id: &str,
) -> Result<Option<ProjectEngineIntegration>, AppError> {
    if integration_id.is_empty() {
        return Err(AppError::Validation(
            "integration id cannot be empty".to_owned(),
        ));
    }
    Ok(sqlx::query_as::<_, ProjectEngineIntegration>(&format!(
        "SELECT {INTEGRATION_COLUMNS} FROM project_engine_integrations WHERE id = ?"
    ))
    .bind(integration_id)
    .fetch_optional(pool)
    .await?)
}

/// Cache root the bridge stages extracted assets into. Lives under the same
/// capture-output tree the existing cleanup budgets over.
pub fn bridge_cache_directory(app: &AppHandle) -> Result<PathBuf, AppError> {
    Ok(app
        .path()
        .app_cache_dir()?
        .join("capture-output")
        .join("renpy-bridge"))
}

pub fn bridge_metadata_directory(app: &AppHandle) -> Result<PathBuf, AppError> {
    Ok(bridge_cache_directory(app)?.join("metadata"))
}

/// Only directories strictly inside the bridge metadata root are ever
/// removed; the shared root and unrelated paths are left alone.
pub(crate) fn is_removable_metadata_directory(root: &Path, candidate: &Path) -> bool {
    candidate != root && candidate.starts_with(root)
}

/// Best-effort cleanup of one install's metadata directory after uninstalling
/// or after switching the game directory.
pub(crate) fn remove_metadata_directory(app: Option<&AppHandle>, metadata_directory: Option<&str>) {
    let (Some(app), Some(metadata_directory)) = (app, metadata_directory) else {
        return;
    };
    let Ok(root) = bridge_metadata_directory(app) else {
        return;
    };
    let candidate = PathBuf::from(metadata_directory);
    if !is_removable_metadata_directory(&root, &candidate) {
        return;
    }
    let _ = std::fs::remove_dir_all(&candidate);
}

async fn bundled_bridge_source(app: &AppHandle) -> Result<String, AppError> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(resource_dir) = app.path().resource_dir() {
        candidates.push(
            resource_dir
                .join("resources")
                .join("renpy_bridge")
                .join(BRIDGE_FILE_NAME),
        );
        candidates.push(resource_dir.join("renpy_bridge").join(BRIDGE_FILE_NAME));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            candidates.push(
                parent
                    .join("resources")
                    .join("renpy_bridge")
                    .join(BRIDGE_FILE_NAME),
            );
        }
    }
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join("renpy_bridge")
            .join(BRIDGE_FILE_NAME),
    );
    for candidate in candidates {
        if candidate.is_file() {
            return Ok(tokio::fs::read_to_string(candidate).await?);
        }
    }
    Err(AppError::Validation(
        "找不到随包分发的 Ren'Py bridge 文件，安装包可能不完整".to_owned(),
    ))
}

pub(crate) async fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let parent = path
        .parent()
        .ok_or_else(|| AppError::Validation("bridge target has no parent".to_owned()))?;
    tokio::fs::create_dir_all(parent).await?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("bridge");
    let temporary = parent.join(format!(".{file_name}.tmp-{}", uuid::Uuid::new_v4()));
    let result = async {
        tokio::fs::write(&temporary, bytes).await?;
        tokio::fs::rename(&temporary, path).await
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&temporary).await;
    }
    result.map_err(AppError::from)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The engine's compiled artifact stores its data in a zlib-compressed slot
/// inside the RPC2 container, so a plain substring search on the raw bytes
/// never finds the marker in real files. RPC2 slot tables are followed by a
/// zero slot; anything else is the legacy single-stream layout.
fn compiled_bytes_contain_marker(bytes: &[u8]) -> bool {
    fn contains(bytes: &[u8]) -> bool {
        bytes
            .windows(COMPILED_MARKER.len())
            .any(|window| window == COMPILED_MARKER)
    }
    if contains(bytes) {
        return true;
    }
    const RPC2_HEADER: &[u8] = b"RENPY RPC2";
    if bytes.starts_with(RPC2_HEADER) {
        let mut position = RPC2_HEADER.len();
        while position + 12 <= bytes.len() {
            let slot =
                u32::from_le_bytes(bytes[position..position + 4].try_into().unwrap_or([0; 4]));
            let start = u32::from_le_bytes(
                bytes[position + 4..position + 8]
                    .try_into()
                    .unwrap_or([0; 4]),
            ) as usize;
            let length = u32::from_le_bytes(
                bytes[position + 8..position + 12]
                    .try_into()
                    .unwrap_or([0; 4]),
            ) as usize;
            position += 12;
            if slot == 0 {
                break;
            }
            let Some(end) = start.checked_add(length) else {
                continue;
            };
            if end > bytes.len() {
                continue;
            }
            if let Ok(data) = miniz_oxide::inflate::decompress_to_vec_zlib(&bytes[start..end]) {
                if contains(&data) {
                    return true;
                }
            }
        }
        return false;
    }
    miniz_oxide::inflate::decompress_to_vec_zlib(bytes)
        .map(|data| contains(&data))
        .unwrap_or(false)
}

/// The two files an install owns are rewritten on every (re)install; any
/// other recorded artifact — the engine-compiled `.rpyc` entry — keeps its
/// historical hash. A swapped file must never re-earn ownership through the
/// hash of its current bytes.
fn merge_install_manifest(
    previous_manifest: &[BridgeFileEntry],
    bridge_path: String,
    config_path: String,
    source: &str,
    config: &[u8],
) -> Vec<BridgeFileEntry> {
    let mut manifest = vec![
        BridgeFileEntry {
            path: bridge_path.clone(),
            sha256: sha256_hex(source.as_bytes()),
        },
        BridgeFileEntry {
            path: config_path.clone(),
            sha256: sha256_hex(config),
        },
    ];
    for entry in previous_manifest {
        if entry.path != bridge_path && entry.path != config_path {
            manifest.push(entry.clone());
        }
    }
    manifest
}

/// Installing may only replace files that match this project's previous
/// manifest; anything else must be resolved by the user first.
async fn ensure_target_is_ours(
    path: &Path,
    previous_manifest: &[BridgeFileEntry],
) -> Result<(), AppError> {
    let Ok(metadata) = tokio::fs::symlink_metadata(path).await else {
        return Ok(());
    };
    if !metadata.is_file() {
        return Err(AppError::Conflict(format!(
            "目标路径已存在且不是普通文件，已停止安装：{}",
            path.display()
        )));
    }
    let expected = previous_manifest
        .iter()
        .find(|entry| Path::new(&entry.path) == path)
        .map(|entry| entry.sha256.as_str());
    let Some(expected) = expected else {
        return Err(AppError::Conflict(format!(
            "目标文件已存在且不属于 Scene Vault，已停止安装：{}",
            path.display()
        )));
    };
    let actual = capture_service::sha256_file(path).await?;
    if actual != expected {
        return Err(AppError::Conflict(format!(
            "目标文件已被修改，已停止安装：{}",
            path.display()
        )));
    }
    Ok(())
}

/// Reads the current bytes of every target so a failed install can restore
/// exactly what was there before (including "did not exist").
async fn snapshot_targets<const N: usize>(
    targets: [&Path; N],
) -> Result<Vec<(PathBuf, Option<Vec<u8>>)>, AppError> {
    let mut backups = Vec::with_capacity(N);
    for target in targets {
        let backup = match tokio::fs::read(target).await {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        backups.push((target.to_path_buf(), backup));
    }
    Ok(backups)
}

/// Restores the bytes captured before an install; returns the paths whose
/// recovery failed so the caller can report a stuck, half-written state
/// instead of silently leaving it behind.
async fn restore_backups(backups: &[(PathBuf, Option<Vec<u8>>)]) -> Vec<String> {
    let mut failed = Vec::new();
    for (path, backup) in backups {
        let result = match backup {
            Some(bytes) => write_atomic(path, bytes).await,
            None => match tokio::fs::remove_file(path).await {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error.into()),
            },
        };
        if let Err(error) = result {
            failed.push(format!("{}（{error}）", path.display()));
        }
    }
    failed
}

/// What a reinstall may do with the previously recorded compiled artifact.
#[derive(Debug, PartialEq, Eq)]
enum ReinstallCompiledState {
    NoEntry,
    Missing,
    Ours,
    Unverified,
}

/// Only the exact recorded bytes count as ours. A different (or unreadable)
/// file is kept untouched and reported, never adopted by re-hashing.
fn reinstall_compiled_state(
    recorded_hash: Option<&str>,
    file_present: bool,
    current_hash: Option<&str>,
) -> ReinstallCompiledState {
    match recorded_hash {
        None => ReinstallCompiledState::NoEntry,
        Some(_) if !file_present => ReinstallCompiledState::Missing,
        Some(recorded) if current_hash == Some(recorded) => ReinstallCompiledState::Ours,
        Some(_) => ReinstallCompiledState::Unverified,
    }
}

/// Combines the original install failure with any rollback failures so a
/// half-restored state is always surfaced instead of being swallowed.
fn install_failure_with_restore(error: AppError, failures: Vec<String>) -> AppError {
    if failures.is_empty() {
        return error;
    }
    log_service::error(
        "engine.bridge",
        format!("bridge rollback left files behind: {}", failures.join("；")),
    );
    AppError::Conflict(format!(
        "安装失败，且回滚未能完全恢复以下文件（请检查后重试）：{}；原始错误：{error}",
        failures.join("；")
    ))
}

/// Deletes files whose ownership is proven, trying to record the
/// engine-compiled artifact first. Unproven files are kept and reported.
async fn remove_verified_files(
    pool: &SqlitePool,
    integration: &ProjectEngineIntegration,
) -> Result<(Vec<String>, Vec<String>), AppError> {
    let mut integration = integration.clone();
    let bridge_path = PathBuf::from(&integration.game_directory).join(BRIDGE_FILE_NAME);
    let compiled_path = PathBuf::from(&integration.game_directory).join(BRIDGE_COMPILED_FILE_NAME);
    if file_exists(&bridge_path).await? && file_exists(&compiled_path).await? {
        let _ = record_compiled_hash(pool, &mut integration, &bridge_path, &compiled_path).await;
    }
    let manifest: Vec<BridgeFileEntry> = serde_json::from_str(&integration.manifest_json)
        .map_err(|error| AppError::Validation(format!("bridge manifest is invalid: {error}")))?;
    let mut removed = Vec::new();
    let mut kept = Vec::new();
    for entry in &manifest {
        let path = Path::new(&entry.path);
        // Only a genuinely absent file counts as "already gone". Access
        // errors must stop the uninstall and keep the install record so a
        // retry can still remove files that cannot be read right now.
        let metadata = match tokio::fs::symlink_metadata(path).await {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if !metadata.is_file() {
            kept.push(entry.path.clone());
            continue;
        }
        let actual = capture_service::sha256_file(path).await?;
        if actual != entry.sha256 {
            kept.push(entry.path.clone());
            continue;
        }
        tokio::fs::remove_file(path).await?;
        removed.push(entry.path.clone());
    }
    // The engine-compiled artifact is reported even when it was never
    // recorded: it cannot be byte-verified, so it is never deleted blindly.
    let compiled_path = PathBuf::from(&integration.game_directory).join(BRIDGE_COMPILED_FILE_NAME);
    let compiled_text = capture_service::path_to_string(&compiled_path);
    if !manifest.iter().any(|entry| entry.path == compiled_text)
        && file_exists(&compiled_path).await?
    {
        kept.push(compiled_text);
    }
    Ok((removed, kept))
}

/// Existence check that distinguishes "absent" from "unreadable": access
/// errors are propagated so callers never treat them as a missing file.
async fn file_exists(path: &Path) -> Result<bool, AppError> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => Ok(metadata.is_file()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

/// Records the compiled artifact's hash once ownership can be proven: our
/// source is on disk with its recorded hash, and the compiled file carries
/// the bridge's distinctive marker string.
async fn record_compiled_hash(
    pool: &SqlitePool,
    integration: &mut ProjectEngineIntegration,
    bridge_path: &Path,
    compiled_path: &Path,
) -> Result<(), AppError> {
    let mut manifest: Vec<BridgeFileEntry> = serde_json::from_str(&integration.manifest_json)
        .map_err(|error| AppError::Validation(format!("bridge manifest is invalid: {error}")))?;
    let compiled_text = capture_service::path_to_string(compiled_path);
    if manifest.iter().any(|entry| entry.path == compiled_text) {
        return Ok(());
    }
    let bridge_text = capture_service::path_to_string(bridge_path);
    let Some(expected_source) = manifest
        .iter()
        .find(|entry| entry.path == bridge_text)
        .map(|entry| entry.sha256.clone())
    else {
        return Ok(());
    };
    if capture_service::sha256_file(bridge_path).await? != expected_source {
        return Ok(());
    }
    let Ok(bytes) = tokio::fs::read(compiled_path).await else {
        return Ok(());
    };
    if !compiled_bytes_contain_marker(&bytes) {
        return Ok(());
    }
    manifest.push(BridgeFileEntry {
        path: compiled_text,
        sha256: sha256_hex(&bytes),
    });
    let manifest_json = serde_json::to_string(&manifest)
        .map_err(|error| AppError::Validation(format!("cannot encode bridge manifest: {error}")))?;
    sqlx::query(
        "UPDATE project_engine_integrations SET manifest_json = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
    )
    .bind(&manifest_json)
    .bind(&integration.id)
    .execute(pool)
    .await?;
    integration.manifest_json = manifest_json;
    log_service::info(
        "engine.bridge",
        "recorded compiled bridge artifact for safe uninstall".to_owned(),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bridge ships as a bundled resource. It must stay ASCII so Ren'Py 7
    /// (Python 2) games can parse it, and it must keep the markers the bridge
    /// logic tests extract.
    #[test]
    fn bundled_bridge_source_is_present_and_ascii() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("resources/renpy_bridge")
            .join(BRIDGE_FILE_NAME);
        let source = std::fs::read_to_string(&path).expect("bundled bridge source");
        assert!(source.contains("SCENEVAULT-BRIDGE-LOGIC-BEGIN"));
        assert!(source.contains("SCENEVAULT-BRIDGE-LOGIC-END"));
        assert!(source.is_ascii(), "bridge source must stay ASCII");
    }

    #[tokio::test]
    async fn install_targets_must_be_owned_or_absent() {
        let workspace = tempfile::tempdir().expect("workspace");
        let target = workspace.path().join(BRIDGE_FILE_NAME);
        tokio::fs::write(&target, b"user content").await.unwrap();

        let error = ensure_target_is_ours(&target, &[])
            .await
            .expect_err("unknown file must not be overwritten");
        assert!(matches!(error, AppError::Conflict(_)));

        let path_text = crate::services::capture_service::path_to_string(&target);
        let owned = crate::models::engine::BridgeFileEntry {
            path: path_text.clone(),
            sha256: crate::services::capture_service::sha256_file(&target)
                .await
                .unwrap(),
        };
        ensure_target_is_ours(&target, &[owned])
            .await
            .expect("a file matching the previous manifest is replaceable");

        let stale = crate::models::engine::BridgeFileEntry {
            path: path_text,
            sha256: "0".repeat(64),
        };
        assert!(ensure_target_is_ours(&target, &[stale]).await.is_err());

        let absent = workspace.path().join("missing.rpy");
        ensure_target_is_ours(&absent, &[])
            .await
            .expect("an absent target is fine");
    }

    #[test]
    fn metadata_cleanup_only_touches_install_subdirectories() {
        let root = std::path::Path::new("/cache/renpy-bridge/metadata");
        assert!(is_removable_metadata_directory(
            root,
            std::path::Path::new("/cache/renpy-bridge/metadata/install-x")
        ));
        assert!(!is_removable_metadata_directory(root, root));
        assert!(!is_removable_metadata_directory(
            root,
            std::path::Path::new("/cache/other")
        ));
    }

    #[tokio::test]
    async fn compiled_artifact_is_kept_until_ownership_is_proven() {
        let pool = crate::db::test_pool().await;
        let workspace = tempfile::tempdir().expect("workspace");
        let game = workspace.path().join("game");
        tokio::fs::create_dir_all(&game).await.unwrap();
        let rpy = game.join(BRIDGE_FILE_NAME);
        let rpyc = game.join(BRIDGE_COMPILED_FILE_NAME);
        tokio::fs::write(&rpy, b"bridge source").await.unwrap();
        tokio::fs::write(&rpyc, b"compiled without the marker")
            .await
            .unwrap();

        let manifest = vec![crate::models::engine::BridgeFileEntry {
            path: crate::services::capture_service::path_to_string(&rpy),
            sha256: crate::services::capture_service::sha256_file(&rpy)
                .await
                .unwrap(),
        }];
        let integration = crate::models::engine::ProjectEngineIntegration {
            id: "install-1".to_owned(),
            project_id: "project-1".to_owned(),
            engine: crate::models::engine::ENGINE_RENPY.to_owned(),
            engine_root: crate::services::capture_service::path_to_string(workspace.path()),
            game_directory: crate::services::capture_service::path_to_string(&game),
            python_major: Some(3),
            engine_version: None,
            bridge_version: BRIDGE_VERSION.to_owned(),
            auto_hide_ui: false,
            auto_hide_ui_override: None,
            metadata_directory: None,
            manifest_json: serde_json::to_string(&manifest).unwrap(),
            installed_at: "2026-10-10T00:00:00Z".to_owned(),
            updated_at: "2026-10-10T00:00:00Z".to_owned(),
        };

        // Without the marker the compiled file cannot be proven and is kept.
        let (removed, kept) = remove_verified_files(&pool, &integration)
            .await
            .expect("remove");
        assert_eq!(removed.len(), 1);
        assert!(kept
            .iter()
            .any(|path| path.ends_with(BRIDGE_COMPILED_FILE_NAME)));
        assert!(rpyc.is_file());

        // Recorded hash: both artifacts are removed.
        tokio::fs::write(&rpy, b"bridge source").await.unwrap();
        let mut recorded_manifest = manifest;
        recorded_manifest.push(crate::models::engine::BridgeFileEntry {
            path: crate::services::capture_service::path_to_string(&rpyc),
            sha256: crate::services::capture_service::sha256_file(&rpyc)
                .await
                .unwrap(),
        });
        let recorded = crate::models::engine::ProjectEngineIntegration {
            manifest_json: serde_json::to_string(&recorded_manifest).unwrap(),
            ..integration.clone()
        };
        let (removed, kept) = remove_verified_files(&pool, &recorded)
            .await
            .expect("remove recorded");
        assert_eq!(removed.len(), 2);
        assert!(kept.is_empty());
        assert!(!rpyc.is_file());
    }

    #[tokio::test]
    async fn compiling_requires_the_bridge_marker_before_recording() {
        let pool = crate::db::test_pool().await;
        let workspace = tempfile::tempdir().expect("workspace");
        let game = workspace.path().join("game");
        tokio::fs::create_dir_all(&game).await.unwrap();
        let rpy = game.join(BRIDGE_FILE_NAME);
        let rpyc = game.join(BRIDGE_COMPILED_FILE_NAME);
        tokio::fs::write(&rpy, b"bridge source").await.unwrap();

        let manifest = vec![crate::models::engine::BridgeFileEntry {
            path: crate::services::capture_service::path_to_string(&rpy),
            sha256: crate::services::capture_service::sha256_file(&rpy)
                .await
                .unwrap(),
        }];
        let mut integration = crate::models::engine::ProjectEngineIntegration {
            id: "install-1".to_owned(),
            project_id: "project-1".to_owned(),
            engine: crate::models::engine::ENGINE_RENPY.to_owned(),
            engine_root: crate::services::capture_service::path_to_string(workspace.path()),
            game_directory: crate::services::capture_service::path_to_string(&game),
            python_major: Some(3),
            engine_version: None,
            bridge_version: BRIDGE_VERSION.to_owned(),
            auto_hide_ui: false,
            auto_hide_ui_override: None,
            metadata_directory: None,
            manifest_json: serde_json::to_string(&manifest).unwrap(),
            installed_at: "2026-10-10T00:00:00Z".to_owned(),
            updated_at: "2026-10-10T00:00:00Z".to_owned(),
        };

        tokio::fs::write(&rpyc, b"compiled without marker")
            .await
            .unwrap();
        record_compiled_hash(&pool, &mut integration, &rpy, &rpyc)
            .await
            .expect("record without marker");
        assert!(!integration
            .manifest_json
            .contains(BRIDGE_COMPILED_FILE_NAME));

        tokio::fs::write(&rpyc, b"compiled scenevault_bridge.json marker")
            .await
            .unwrap();
        record_compiled_hash(&pool, &mut integration, &rpy, &rpyc)
            .await
            .expect("record");
        assert!(integration
            .manifest_json
            .contains(BRIDGE_COMPILED_FILE_NAME));
    }

    fn rpc2_container(compiled: &[u8]) -> Vec<u8> {
        let payload = miniz_oxide::deflate::compress_to_vec_zlib(compiled, 3);
        let mut bytes = b"RENPY RPC2".to_vec();
        // Slot table: the data slot (1, start, length), then the terminator.
        let table_size = 12 * 2u32;
        let start = 10 + table_size;
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&start.to_le_bytes());
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&payload);
        bytes
    }

    #[test]
    fn marker_detection_understands_rpc2_and_legacy_streams() {
        // Real compiled payloads are large enough to actually compress; a
        // tiny buffer may be stored uncompressed and would not prove that the
        // inflate path works.
        let mut compiled = vec![b'x'; 4096];
        compiled.extend_from_slice(b"scenevault_bridge.json");
        compiled.extend_from_slice(&[b'x'; 4096]);
        let rpc2 = rpc2_container(&compiled);
        assert!(
            !rpc2
                .windows(COMPILED_MARKER.len())
                .any(|window| window == COMPILED_MARKER),
            "the raw container never carries the plain marker"
        );
        assert!(compiled_bytes_contain_marker(&rpc2));
        let legacy = miniz_oxide::deflate::compress_to_vec_zlib(&compiled, 3);
        assert!(compiled_bytes_contain_marker(&legacy));
        assert!(!compiled_bytes_contain_marker(b"unrelated compiled bytes"));
    }

    #[test]
    fn reinstall_keeps_recorded_compiled_identity() {
        let previous = vec![
            BridgeFileEntry {
                path: "/games/demo/game/00_scenevault_bridge.rpy".to_owned(),
                sha256: "1".repeat(64),
            },
            BridgeFileEntry {
                path: "/games/demo/game/scenevault_bridge.json".to_owned(),
                sha256: "2".repeat(64),
            },
            BridgeFileEntry {
                path: "/games/demo/game/00_scenevault_bridge.rpyc".to_owned(),
                sha256: "3".repeat(64),
            },
        ];
        let merged = merge_install_manifest(
            &previous,
            "/games/demo/game/00_scenevault_bridge.rpy".to_owned(),
            "/games/demo/game/scenevault_bridge.json".to_owned(),
            "source",
            b"config",
        );
        assert_eq!(merged.len(), 3, "no duplicate entries");
        let compiled = merged
            .iter()
            .find(|entry| entry.path.ends_with(".rpyc"))
            .expect("compiled entry is preserved");
        assert_eq!(compiled.sha256, "3".repeat(64));
    }

    #[test]
    fn reinstall_compiled_state_only_trusts_exact_recorded_bytes() {
        assert_eq!(
            reinstall_compiled_state(None, true, Some("a")),
            ReinstallCompiledState::NoEntry
        );
        assert_eq!(
            reinstall_compiled_state(Some("a"), false, None),
            ReinstallCompiledState::Missing
        );
        assert_eq!(
            reinstall_compiled_state(Some("a"), true, Some("a")),
            ReinstallCompiledState::Ours
        );
        assert_eq!(
            reinstall_compiled_state(Some("a"), true, Some("b")),
            ReinstallCompiledState::Unverified
        );
        assert_eq!(
            reinstall_compiled_state(Some("a"), true, None),
            ReinstallCompiledState::Unverified
        );
    }
}
