use std::path::{Path, PathBuf};

use chrono::{SecondsFormat, Utc};
use sqlx::SqlitePool;

use crate::{
    error::AppError,
    models::engine::{CaptureEngineMetadata, RenpySidecar, ENGINE_RENPY},
    services::{capture_service, log_service},
};

/// Oldest sidecar schema this build understands. Newer files are ignored
/// rather than rejected so a future bridge cannot break an older app.
const SUPPORTED_SCHEMA_VERSION: i64 = 1;

const METADATA_COLUMNS: &str = "capture_item_id, project_id, engine, schema_version, \
    bridge_version, engine_version, speaker_raw, speaker_source, dialogue, suggested_character_id, \
    virtual_width, virtual_height, physical_width, physical_height, payload_json, \
    created_at, updated_at";

/// Metadata file the bridge writes for one screenshot:
/// `<metadata dir>/<screenshot stem>.scenevault.json`.
pub fn metadata_file_name_for(source: &Path) -> String {
    let stem = source
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("capture");
    format!("{stem}.scenevault.json")
}

/// All installs of one project as (engine_root, game_directory, metadata dir).
async fn project_install_rows(
    pool: &SqlitePool,
    project_id: &str,
) -> Result<Vec<(String, String, Option<String>)>, AppError> {
    Ok(sqlx::query_as(
        "SELECT engine_root, game_directory, metadata_directory \
         FROM project_engine_integrations WHERE project_id = ? \
         ORDER BY installed_at ASC, id ASC",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?)
}

fn normalized_path(value: &str) -> String {
    value
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_lowercase()
}

fn is_inside(path: &str, directory: &str) -> bool {
    let path = normalized_path(path);
    let directory = normalized_path(directory);
    if directory.is_empty() {
        return false;
    }
    path == directory || path.starts_with(&format!("{directory}/"))
}

/// Picks the install a capture belongs to by the longest matching path
/// prefix (game root or `game/` folder). A capture whose path matches no
/// install stays un-attributed — never fall back to "the only install":
/// after uninstalling one game, the remaining game's metadata directory must
/// not consume another game's same-named sidecars.
fn resolve_directory_from_rows(
    rows: &[(String, String, Option<String>)],
    source_path: &str,
) -> Option<String> {
    let mut best: Option<(usize, String)> = None;
    for (engine_root, game_directory, metadata_directory) in rows {
        let Some(metadata_directory) = metadata_directory else {
            continue;
        };
        for prefix in [engine_root, game_directory] {
            if !is_inside(source_path, prefix) {
                continue;
            }
            let length = normalized_path(prefix).len();
            if best
                .as_ref()
                .map(|(current, _)| length > *current)
                .unwrap_or(true)
            {
                best = Some((length, metadata_directory.clone()));
            }
        }
    }
    if let Some((_, directory)) = best {
        return Some(directory);
    }
    None
}

async fn resolve_metadata_directory(
    pool: &SqlitePool,
    project_id: &str,
    source_path: &str,
) -> Result<Option<String>, AppError> {
    let rows = project_install_rows(pool, project_id).await?;
    Ok(resolve_directory_from_rows(&rows, source_path))
}

/// Attaches engine metadata for one capture and deletes the metadata file:
/// the cache copy is consumed exactly once. Missing or malformed files are a
/// no-op so the capture stays on the normal pipeline and a later poll retries.
pub async fn try_attach(pool: &SqlitePool, capture_item_id: &str) -> Result<bool, AppError> {
    let item = capture_service::get_item(pool, capture_item_id).await?;
    let Some(metadata_directory) =
        resolve_metadata_directory(pool, &item.project_id, &item.source_path).await?
    else {
        return Ok(false);
    };
    let file =
        Path::new(&metadata_directory).join(metadata_file_name_for(Path::new(&item.source_path)));
    let already: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM capture_engine_metadata WHERE capture_item_id = ?)",
    )
    .bind(capture_item_id)
    .fetch_one(pool)
    .await?;
    if already {
        // Leftover file for an already-ingested capture.
        let _ = tokio::fs::remove_file(&file).await;
        return Ok(false);
    }
    let raw = match tokio::fs::read_to_string(&file).await {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let payload: RenpySidecar = match serde_json::from_str(&raw) {
        Ok(payload) => payload,
        Err(error) => {
            log_service::warn(
                "engine.metadata",
                format!(
                    "ignoring invalid scenevault metadata for capture {capture_item_id}: {error}"
                ),
            );
            return Ok(false);
        }
    };
    if payload.schema_version > SUPPORTED_SCHEMA_VERSION {
        log_service::warn(
            "engine.metadata",
            format!(
                "metadata schema {} is newer than supported {}; capture {capture_item_id} kept as a plain screenshot",
                payload.schema_version, SUPPORTED_SCHEMA_VERSION
            ),
        );
        return Ok(false);
    }

    let suggested = match_speaker(pool, &item.project_id, payload.speaker.as_deref()).await?;
    let (virtual_width, virtual_height) = payload.virtual_resolution.unwrap_or((0, 0));
    let (physical_width, physical_height) = payload.physical_size.unwrap_or((0, 0));
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
    sqlx::query(
        r#"
        INSERT INTO capture_engine_metadata (
            capture_item_id, project_id, engine, schema_version, bridge_version,
            engine_version, speaker_raw, speaker_source, dialogue, suggested_character_id,
            virtual_width, virtual_height, physical_width, physical_height,
            payload_json, created_at, updated_at
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(capture_item_id) DO NOTHING
        "#,
    )
    .bind(capture_item_id)
    .bind(&item.project_id)
    .bind(ENGINE_RENPY)
    .bind(payload.schema_version)
    .bind(&payload.bridge_version)
    .bind(payload.engine_version.as_deref())
    .bind(payload.speaker.as_deref())
    .bind(payload.speaker_source.as_deref())
    .bind(payload.dialogue.as_deref())
    .bind(suggested.as_deref())
    .bind(virtual_width)
    .bind(virtual_height)
    .bind(physical_width)
    .bind(physical_height)
    .bind(&raw)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;
    let _ = tokio::fs::remove_file(&file).await;
    Ok(true)
}

/// Periodic retry for captures whose metadata file landed a moment after the
/// PNG (the bridge writes it right after the engine saved the screenshot).
/// Captures without a metadata file only cost one stat, so newer plain
/// screenshots can never starve an older late file out of the retry budget.
pub async fn retry_pending(
    pool: &SqlitePool,
    app: Option<&tauri::AppHandle>,
    limit: i64,
) -> Result<u32, AppError> {
    // Candidates are driven by the sidecar files actually present in each
    // install's metadata directory: a backlog of plain screenshots can never
    // starve an older late JSON out of the retry budget.
    struct InstallFiles {
        metadata_directory: String,
        stems: std::collections::HashSet<String>,
        files: Vec<PathBuf>,
    }
    let install_rows: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT project_id, metadata_directory \
         FROM project_engine_integrations ORDER BY installed_at ASC, id ASC",
    )
    .fetch_all(pool)
    .await?;
    let mut by_project: std::collections::HashMap<String, Vec<InstallFiles>> =
        std::collections::HashMap::new();
    for (project_id, metadata_directory) in install_rows {
        let Some(metadata_directory) = metadata_directory else {
            continue;
        };
        let mut stems = std::collections::HashSet::new();
        let mut files = Vec::new();
        if let Ok(mut entries) = tokio::fs::read_dir(&metadata_directory).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                if let Some(stem) = name.strip_suffix(".scenevault.json") {
                    stems.insert(stem.to_owned());
                    if entry.file_type().await.is_ok_and(|kind| kind.is_file()) {
                        files.push(entry.path());
                    }
                }
            }
        }
        if stems.is_empty() {
            continue;
        }
        by_project
            .entry(project_id)
            .or_default()
            .push(InstallFiles {
                metadata_directory,
                stems,
                files,
            });
    }
    let mut attached = 0_u32;
    let mut processed = 0_i64;
    for (project_id, installs) in by_project {
        if processed >= limit {
            break;
        }
        // Include installs without sidecars as well: a nested game still
        // owns its paths even while its metadata directory is empty.
        let rows = project_install_rows(pool, &project_id).await?;
        // Newest first: a sidecar lands seconds after its PNG, so the retry
        // pass only guarantees the recent window; sidecar-driven filtering
        // keeps plain screenshots from spending the budget.
        let candidates: Vec<(String, String)> = sqlx::query_as(
            r#"
            SELECT ci.id, ci.source_path
            FROM capture_items ci
            WHERE ci.project_id = ?
              AND ci.source_file_state = 'available'
              AND ci.created_at >= strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-7 days')
              AND NOT EXISTS (
                  SELECT 1 FROM capture_engine_metadata cem
                  WHERE cem.capture_item_id = ci.id
              )
            ORDER BY ci.created_at DESC, ci.id DESC
            LIMIT 5000
            "#,
        )
        .bind(&project_id)
        .fetch_all(pool)
        .await?;
        for (id, source_path) in candidates {
            if processed >= limit {
                break;
            }
            let Some(metadata_directory) = resolve_directory_from_rows(&rows, &source_path) else {
                continue;
            };
            let Some(install) = installs
                .iter()
                .find(|install| install.metadata_directory == metadata_directory)
            else {
                continue;
            };
            let stem = Path::new(&source_path)
                .file_stem()
                .and_then(|value| value.to_str())
                .map(str::to_lowercase)
                .unwrap_or_default();
            if !install.stems.contains(&stem) {
                continue;
            }
            processed = processed.saturating_add(1);
            match try_attach(pool, &id).await {
                Ok(true) => {
                    attached = attached.saturating_add(1);
                    emit_metadata_updated(app, &id);
                }
                Ok(false) => {}
                Err(error) => log_service::warn(
                    "engine.metadata",
                    format!("failed to attach engine metadata for capture {id}: {error}"),
                ),
            }
        }
        if processed >= limit {
            continue;
        }
        let directories: Vec<String> = sqlx::query_scalar(
            "SELECT directory FROM project_source_directories WHERE project_id = ? AND enabled = 1",
        )
        .bind(&project_id)
        .fetch_all(pool)
        .await?;
        let mut source_directories = Vec::new();
        for directory in directories {
            if let Ok(path) = tokio::fs::canonicalize(directory).await {
                source_directories.push(path);
            }
        }
        for install in installs {
            for file in install.files {
                if processed >= limit {
                    break;
                }
                if !tokio::fs::try_exists(&file).await.unwrap_or(false) {
                    continue;
                }
                processed = processed.saturating_add(1);
                if let Err(error) =
                    discard_redundant_metadata(pool, &project_id, &file, &rows, &source_directories)
                        .await
                {
                    log_service::warn(
                        "engine.metadata",
                        format!(
                            "failed to clean redundant metadata {}: {error}",
                            file.display()
                        ),
                    );
                }
            }
        }
    }
    Ok(attached)
}

/// A duplicate has no row at its new filename, so the ordinary attachment
/// retry cannot consume it. Only discard a sidecar after its own install and
/// a unique, unchanged source prove the content was already indexed/ignored.
/// Unregistered new images, ambiguous paths and future schemas stay pending.
async fn discard_redundant_metadata(
    pool: &SqlitePool,
    project_id: &str,
    file: &Path,
    installs: &[(String, String, Option<String>)],
    source_directories: &[PathBuf],
) -> Result<bool, AppError> {
    let raw = match tokio::fs::read_to_string(file).await {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let Ok(payload) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return Ok(false);
    };
    if payload
        .get("schemaVersion")
        .and_then(serde_json::Value::as_i64)
        != Some(SUPPORTED_SCHEMA_VERSION)
    {
        return Ok(false);
    }
    let Some(filename) = payload
        .get("screenshot")
        .and_then(|screenshot| screenshot.get("file"))
        .and_then(serde_json::Value::as_str)
    else {
        return Ok(false);
    };
    if filename.contains(['/', '\\'])
        || !capture_service::is_supported_image(Path::new(filename))
        || !file.file_name().is_some_and(|name| {
            name.to_string_lossy()
                .eq_ignore_ascii_case(&metadata_file_name_for(Path::new(filename)))
        })
    {
        return Ok(false);
    }
    let metadata_directory = file.parent().unwrap_or_else(|| Path::new(""));
    let mut sources = std::collections::HashSet::new();
    for directory in source_directories {
        let Ok(source) = tokio::fs::canonicalize(directory.join(filename)).await else {
            continue;
        };
        if !capture_service::path_is_within(&source, directory) {
            continue;
        }
        let source_path = capture_service::path_to_string(&source);
        if resolve_directory_from_rows(installs, &source_path).is_some_and(|resolved| {
            normalized_path(&resolved)
                == normalized_path(&capture_service::path_to_string(metadata_directory))
        }) {
            sources.insert(source);
        }
    }
    if sources.len() != 1 {
        return Ok(false);
    }
    let source = sources.into_iter().next().expect("one metadata source");
    let before = tokio::fs::metadata(&source).await?;
    if !before.is_file() || before.len() == 0 {
        return Ok(false);
    }
    let hash = capture_service::sha256_file(&source).await?;
    let source_path = capture_service::path_to_string(&source);
    let original: Option<(String, bool)> = sqlx::query_as(
        r#"
        SELECT ci.source_path,
               EXISTS(SELECT 1 FROM capture_engine_metadata cem WHERE cem.capture_item_id = ci.id)
        FROM capture_items ci
        WHERE ci.project_id = ? AND ci.content_hash = ?
        LIMIT 1
        "#,
    )
    .bind(project_id)
    .bind(&hash)
    .fetch_optional(pool)
    .await?;
    if original.as_ref().is_some_and(|(path, has_metadata)| {
        !*has_metadata && normalized_path(path) == normalized_path(&source_path)
    }) || (original.is_none()
        && !capture_service::is_content_ignored(pool, project_id, &hash).await?)
    {
        return Ok(false);
    }
    let after = tokio::fs::metadata(&source).await?;
    if before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
        || capture_service::sha256_file(&source).await? != hash
        || tokio::fs::read_to_string(file).await? != raw
    {
        return Ok(false);
    }
    match tokio::fs::remove_file(file).await {
        Ok(()) => {
            log_service::debug(
                "engine.metadata",
                format!("removed redundant screenshot metadata {}", file.display()),
            );
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

pub async fn get_capture_metadata(
    pool: &SqlitePool,
    capture_item_id: &str,
) -> Result<Option<CaptureEngineMetadata>, AppError> {
    Ok(sqlx::query_as::<_, CaptureEngineMetadata>(&format!(
        "SELECT {METADATA_COLUMNS} FROM capture_engine_metadata WHERE capture_item_id = ?"
    ))
    .bind(capture_item_id)
    .fetch_optional(pool)
    .await?)
}

/// Exact speaker-to-character match (name or alias, case-insensitive, text
/// tags stripped). Only ever a suggestion for the user to confirm.
async fn match_speaker(
    pool: &SqlitePool,
    project_id: &str,
    speaker: Option<&str>,
) -> Result<Option<String>, AppError> {
    let Some(speaker) = speaker else {
        return Ok(None);
    };
    let needle = normalize_speaker(speaker);
    if needle.is_empty() {
        return Ok(None);
    }
    let rows: Vec<(String, String, String)> =
        sqlx::query_as("SELECT id, name, aliases_json FROM characters WHERE project_id = ?")
            .bind(project_id)
            .fetch_all(pool)
            .await?;
    for (id, name, aliases_json) in rows {
        if normalize_speaker(&name) == needle {
            return Ok(Some(id));
        }
        for alias in parse_aliases(&aliases_json) {
            if normalize_speaker(&alias) == needle {
                return Ok(Some(id));
            }
        }
    }
    Ok(None)
}

fn parse_aliases(aliases_json: &str) -> Vec<String> {
    if let Ok(values) = serde_json::from_str::<Vec<String>>(aliases_json) {
        return values;
    }
    aliases_json
        .split([',', '、', ';'])
        .map(|value| value.trim().trim_matches('"').to_owned())
        .filter(|value| !value.is_empty())
        .collect()
}

fn normalize_speaker(value: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for ch in value.chars() {
        match ch {
            '{' => in_tag = true,
            '}' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.trim().to_lowercase()
}

/// Emits a refresh hint so an open capture page re-reads the metadata of the
/// selected item after a late metadata file was attached.
fn emit_metadata_updated(app: Option<&tauri::AppHandle>, capture_item_id: &str) {
    use tauri::Emitter;
    if let Some(app) = app {
        let _ = app.emit(
            "capture:engine-metadata-updated",
            serde_json::json!({ "captureItemId": capture_item_id }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db,
        models::capture::CaptureItem,
        models::character::CreateCharacterInput,
        services::{character_service, test_support},
    };
    use std::path::PathBuf;

    fn png_bytes(width: u32, height: u32) -> Vec<u8> {
        let image = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            width,
            height,
            image::Rgb([12, 34, 56]),
        ));
        let mut bytes = Vec::new();
        image
            .write_with_encoder(image::codecs::png::PngEncoder::new(&mut bytes))
            .expect("encode png");
        bytes
    }

    struct Fixture {
        _workspace: tempfile::TempDir,
        pool: SqlitePool,
        project_id: String,
        session_id: String,
        source_directory: PathBuf,
        metadata_directory: PathBuf,
    }

    async fn fixture(name: &str) -> Fixture {
        let pool = db::test_pool().await;
        let test_support::ProjectFixture {
            _workspace,
            project_id,
            source_directory,
            ..
        } = test_support::project_with_directories(&pool, name)
            .await
            .expect("fixture");
        let session = test_support::start_session(&pool, &project_id)
            .await
            .expect("session");
        let metadata_directory = _workspace.path().join("metadata");
        tokio::fs::create_dir_all(&metadata_directory)
            .await
            .unwrap();
        let now = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
        // The recorded game root is the fixture's real source directory so
        // captures resolve by path: the lenient "only install" fallback is
        // intentionally gone, and these fixtures must exercise attribution.
        sqlx::query(
            "INSERT INTO project_engine_integrations (id, project_id, engine, engine_root, game_directory, python_major, bridge_version, metadata_directory, manifest_json, installed_at, updated_at) VALUES (?, ?, 'renpy', ?, ?, 3, '1.1.0', ?, '[]', ?, ?)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(&project_id)
        .bind(capture_service::path_to_string(&source_directory))
        .bind(capture_service::path_to_string(&source_directory.join("game")))
        .bind(capture_service::path_to_string(&metadata_directory))
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("integration row");
        Fixture {
            _workspace,
            pool,
            project_id,
            session_id: session.id,
            source_directory,
            metadata_directory,
        }
    }

    async fn register_capture(fixture: &Fixture, stem: &str, variant: u32) -> CaptureItem {
        let screenshot = fixture.source_directory.join(format!("{stem}.png"));
        tokio::fs::write(&screenshot, png_bytes(320 + variant, 180))
            .await
            .unwrap();
        let (item, inserted) = capture_service::register_discovered_path(
            &fixture.pool,
            &fixture.session_id,
            &screenshot,
            true,
        )
        .await
        .expect("register capture");
        assert!(inserted);
        item
    }

    fn write_metadata(fixture: &Fixture, stem: &str, document: &serde_json::Value) {
        let file = fixture
            .metadata_directory
            .join(format!("{stem}.scenevault.json"));
        std::fs::write(file, serde_json::to_vec(document).unwrap()).expect("write metadata");
    }

    #[tokio::test]
    async fn attaches_metadata_and_consumes_the_cache_file() {
        let fixture = fixture("metadata").await;
        let character = character_service::create(
            &fixture.pool,
            CreateCharacterInput {
                project_id: fixture.project_id.clone(),
                name: "Ashley".to_owned(),
                aliases_json: Some("[\"Ash\"]".to_owned()),
            },
        )
        .await
        .expect("character");
        let item = register_capture(&fixture, "screenshot0001", 0).await;
        write_metadata(
            &fixture,
            "screenshot0001",
            &serde_json::json!({
                "schemaVersion": 1,
                "bridgeVersion": "1.1.0",
                "speaker": "Ashley",
                "speakerSource": "_last_say_who",
                "dialogue": "Are you serious?",
                "virtualResolution": [1920, 1080],
                "physicalSize": [3840, 2160],
                "sceneTags": ["bedroom"],
                "errors": []
            }),
        );

        assert!(try_attach(&fixture.pool, &item.id).await.expect("attach"));
        let metadata = get_capture_metadata(&fixture.pool, &item.id)
            .await
            .expect("metadata")
            .expect("row");
        assert_eq!(metadata.speaker_raw.as_deref(), Some("Ashley"));
        assert_eq!(
            metadata.suggested_character_id.as_deref(),
            Some(character.id.as_str())
        );
        assert_eq!(metadata.virtual_width, Some(1920));
        assert!(
            !fixture
                .metadata_directory
                .join("screenshot0001.scenevault.json")
                .exists(),
            "the cache copy is consumed exactly once"
        );
        assert!(!try_attach(&fixture.pool, &item.id)
            .await
            .expect("re-attach"));
    }

    async fn register_capture_in(
        fixture: &Fixture,
        directory: &Path,
        stem: &str,
        variant: u32,
    ) -> CaptureItem {
        let screenshot = directory.join(format!("{stem}.png"));
        tokio::fs::write(&screenshot, png_bytes(320 + variant, 180))
            .await
            .unwrap();
        let (item, inserted) = capture_service::register_discovered_path(
            &fixture.pool,
            &fixture.session_id,
            &screenshot,
            true,
        )
        .await
        .expect("register capture");
        assert!(inserted);
        item
    }

    #[tokio::test]
    async fn same_named_screenshots_stay_with_their_own_install() {
        let fixture = fixture("multi-install").await;
        let game_a = fixture._workspace.path().join("game-a");
        let game_b = fixture._workspace.path().join("game-b");
        let meta_a = fixture._workspace.path().join("meta-a");
        let meta_b = fixture._workspace.path().join("meta-b");
        for directory in [&game_a, &game_b, &meta_a, &meta_b] {
            tokio::fs::create_dir_all(directory).await.unwrap();
        }
        let now = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
        // The fixture already inserted one unrelated install row; these two
        // make the project multi-game and disable the lenient fallback.
        for (root, metadata) in [(&game_a, &meta_a), (&game_b, &meta_b)] {
            sqlx::query(
                "INSERT INTO project_engine_integrations (id, project_id, engine, engine_root, game_directory, python_major, bridge_version, metadata_directory, manifest_json, installed_at, updated_at) VALUES (?, ?, 'renpy', ?, ?, 3, '1.1.1', ?, '[]', ?, ?)",
            )
            .bind(uuid::Uuid::new_v4().to_string())
            .bind(&fixture.project_id)
            .bind(capture_service::path_to_string(root))
            .bind(capture_service::path_to_string(&root.join("game")))
            .bind(capture_service::path_to_string(metadata))
            .bind(&now)
            .bind(&now)
            .execute(&fixture.pool)
            .await
            .expect("install row");
        }
        let capture_a = register_capture_in(&fixture, &game_a, "screenshot0001", 0).await;
        let capture_b = register_capture_in(&fixture, &game_b, "screenshot0001", 1).await;
        for (metadata, speaker) in [(&meta_a, "Alice"), (&meta_b, "Bob")] {
            std::fs::write(
                metadata.join("screenshot0001.scenevault.json"),
                serde_json::to_vec(&serde_json::json!({
                    "schemaVersion": 1,
                    "bridgeVersion": "1.1.1",
                    "speaker": speaker,
                    "errors": []
                }))
                .unwrap(),
            )
            .expect("metadata file");
        }

        assert!(try_attach(&fixture.pool, &capture_a.id)
            .await
            .expect("attach a"));
        assert!(try_attach(&fixture.pool, &capture_b.id)
            .await
            .expect("attach b"));
        let metadata_a = get_capture_metadata(&fixture.pool, &capture_a.id)
            .await
            .expect("metadata a")
            .expect("row a");
        let metadata_b = get_capture_metadata(&fixture.pool, &capture_b.id)
            .await
            .expect("metadata b")
            .expect("row b");
        assert_eq!(metadata_a.speaker_raw.as_deref(), Some("Alice"));
        assert_eq!(metadata_b.speaker_raw.as_deref(), Some("Bob"));
        assert!(!meta_a.join("screenshot0001.scenevault.json").exists());
        assert!(!meta_b.join("screenshot0001.scenevault.json").exists());
    }

    #[tokio::test]
    async fn unmatched_sources_never_fall_back_to_the_only_install() {
        let fixture = fixture("no-fallback").await;
        let elsewhere = fixture._workspace.path().join("elsewhere");
        tokio::fs::create_dir_all(&elsewhere).await.unwrap();
        let item = register_capture_in(&fixture, &elsewhere, "screenshot0001", 0).await;
        write_metadata(
            &fixture,
            "screenshot0001",
            &serde_json::json!({
                "schemaVersion": 1,
                "bridgeVersion": "1.1.1",
                "speaker": "Alice",
                "errors": []
            }),
        );

        assert!(!try_attach(&fixture.pool, &item.id).await.expect("attach"));
        assert!(get_capture_metadata(&fixture.pool, &item.id)
            .await
            .expect("metadata")
            .is_none());
        assert!(
            fixture
                .metadata_directory
                .join("screenshot0001.scenevault.json")
                .exists(),
            "another source's sidecar stays untouched"
        );
    }

    #[tokio::test]
    async fn retry_finds_late_metadata_behind_many_plain_screenshots() {
        let fixture = fixture("starvation").await;
        let older = register_capture(&fixture, "screenshot0000", 0).await;
        // Enough newer plain screenshots to exceed any per-call budget; the
        // retry pass must be driven by sidecar files, not by recency alone.
        for index in 0..205u32 {
            let _ = register_capture(&fixture, &format!("plain{index:04}"), index + 1).await;
        }
        write_metadata(
            &fixture,
            "screenshot0000",
            &serde_json::json!({
                "schemaVersion": 1,
                "bridgeVersion": "1.1.1",
                "speaker": "Alice",
                "errors": []
            }),
        );

        let attached = retry_pending(&fixture.pool, None, 200)
            .await
            .expect("retry");
        assert!(attached >= 1, "the late sidecar must not starve");
        assert!(get_capture_metadata(&fixture.pool, &older.id)
            .await
            .expect("metadata")
            .is_some());
    }

    #[tokio::test]
    async fn retry_discards_duplicate_sidecar_without_changing_original_metadata() {
        let fixture = fixture("duplicate-metadata").await;
        let original = register_capture(&fixture, "screenshot0140", 0).await;
        write_metadata(
            &fixture,
            "screenshot0140",
            &serde_json::json!({ "schemaVersion": 1, "speaker": "Original" }),
        );
        assert!(try_attach(&fixture.pool, &original.id).await.unwrap());
        let duplicate = fixture.source_directory.join("screenshot0217.png");
        tokio::fs::copy(&original.source_path, &duplicate)
            .await
            .unwrap();
        write_metadata(
            &fixture,
            "screenshot0217",
            &serde_json::json!({
                "schemaVersion": 1, "speaker": "Duplicate",
                "screenshot": { "file": "screenshot0217.png" }
            }),
        );
        let (item, inserted) = capture_service::register_discovered_path(
            &fixture.pool,
            &fixture.session_id,
            &duplicate,
            false,
        )
        .await
        .unwrap();
        assert!(!inserted);
        assert_eq!(item.id, original.id);

        assert_eq!(retry_pending(&fixture.pool, None, 200).await.unwrap(), 0);
        assert!(!fixture
            .metadata_directory
            .join("screenshot0217.scenevault.json")
            .exists());
        assert!(duplicate.exists(), "cleanup never removes the screenshot");
        let metadata = get_capture_metadata(&fixture.pool, &original.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(metadata.speaker_raw.as_deref(), Some("Original"));
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM capture_items")
            .fetch_one(&fixture.pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn retry_discards_late_duplicate_after_session_stops_without_attaching_to_original() {
        let fixture = fixture("late-duplicate").await;
        let original = register_capture(&fixture, "screenshot0140", 0).await;
        let duplicate = fixture.source_directory.join("screenshot0217.png");
        tokio::fs::copy(&original.source_path, &duplicate)
            .await
            .unwrap();
        let (_, inserted) = capture_service::register_discovered_path(
            &fixture.pool,
            &fixture.session_id,
            &duplicate,
            false,
        )
        .await
        .unwrap();
        assert!(!inserted);
        assert_eq!(retry_pending(&fixture.pool, None, 200).await.unwrap(), 0);
        sqlx::query("UPDATE capture_sessions SET status = 'completed' WHERE id = ?")
            .bind(&fixture.session_id)
            .execute(&fixture.pool)
            .await
            .unwrap();
        write_metadata(
            &fixture,
            "screenshot0217",
            &serde_json::json!({
                "schemaVersion": 1, "speaker": "Late duplicate",
                "screenshot": { "file": "screenshot0217.png" }
            }),
        );

        assert_eq!(retry_pending(&fixture.pool, None, 200).await.unwrap(), 0);
        assert!(!fixture
            .metadata_directory
            .join("screenshot0217.scenevault.json")
            .exists());
        assert!(get_capture_metadata(&fixture.pool, &original.id)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn retry_keeps_unregistered_metadata_even_when_another_project_has_the_image() {
        let fixture = fixture("pending-metadata").await;
        let other = test_support::project_with_directories(&fixture.pool, "Other project")
            .await
            .unwrap();
        let other_session = test_support::start_session(&fixture.pool, &other.project_id)
            .await
            .unwrap();
        let screenshot = fixture.source_directory.join("screenshot0001.png");
        let other_screenshot = other.source_directory.join("screenshot0001.png");
        tokio::fs::write(&screenshot, png_bytes(320, 180))
            .await
            .unwrap();
        tokio::fs::copy(&screenshot, &other_screenshot)
            .await
            .unwrap();
        capture_service::register_discovered_path(
            &fixture.pool,
            &other_session.id,
            &other_screenshot,
            true,
        )
        .await
        .unwrap();
        write_metadata(
            &fixture,
            "screenshot0001",
            &serde_json::json!({
                "schemaVersion": 1, "speaker": "Pending",
                "screenshot": { "file": "screenshot0001.png" }
            }),
        );
        assert_eq!(retry_pending(&fixture.pool, None, 200).await.unwrap(), 0);
        assert!(fixture
            .metadata_directory
            .join("screenshot0001.scenevault.json")
            .exists());

        // Starting the other project ended this project's earlier session.
        let session = test_support::start_session(&fixture.pool, &fixture.project_id)
            .await
            .unwrap();
        let (item, inserted) = capture_service::register_discovered_path(
            &fixture.pool,
            &session.id,
            &screenshot,
            false,
        )
        .await
        .unwrap();
        assert!(inserted);
        assert_eq!(retry_pending(&fixture.pool, None, 200).await.unwrap(), 1);
        let metadata = get_capture_metadata(&fixture.pool, &item.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(metadata.speaker_raw.as_deref(), Some("Pending"));
    }

    #[tokio::test]
    async fn retry_discards_metadata_for_explicitly_ignored_content() {
        let fixture = fixture("ignored-metadata").await;
        let original = register_capture(&fixture, "screenshot0001", 0).await;
        sqlx::query(
            "INSERT INTO ignored_capture_contents (project_id, content_hash, source_path) VALUES (?, ?, ?)",
        )
        .bind(&fixture.project_id)
        .bind(&original.content_hash)
        .bind(&original.source_path)
        .execute(&fixture.pool)
        .await
        .unwrap();
        sqlx::query("DELETE FROM capture_items WHERE id = ?")
            .bind(&original.id)
            .execute(&fixture.pool)
            .await
            .unwrap();
        write_metadata(
            &fixture,
            "screenshot0001",
            &serde_json::json!({
                "schemaVersion": 1,
                "screenshot": { "file": "screenshot0001.png" }
            }),
        );

        assert_eq!(retry_pending(&fixture.pool, None, 200).await.unwrap(), 0);
        assert!(!fixture
            .metadata_directory
            .join("screenshot0001.scenevault.json")
            .exists());
        assert!(Path::new(&original.source_path).exists());
    }

    #[tokio::test]
    async fn duplicate_cleanup_respects_nested_installs_with_same_named_screenshots() {
        let fixture = fixture("nested-install-cleanup").await;
        let original = register_capture(&fixture, "screenshot0000", 0).await;
        let nested = fixture.source_directory.join("nested");
        let nested_metadata = fixture._workspace.path().join("nested-metadata");
        tokio::fs::create_dir_all(&nested).await.unwrap();
        tokio::fs::create_dir_all(&nested_metadata).await.unwrap();
        sqlx::query(
            "INSERT INTO project_source_directories (id, project_id, directory) VALUES (?, ?, ?)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(&fixture.project_id)
        .bind(capture_service::path_to_string(&nested))
        .execute(&fixture.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO project_engine_integrations (id, project_id, engine, engine_root, game_directory, python_major, bridge_version, metadata_directory, manifest_json, installed_at, updated_at) VALUES (?, ?, 'renpy', ?, ?, 3, '1.1.7', ?, '[]', datetime('now'), datetime('now'))",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(&fixture.project_id)
        .bind(capture_service::path_to_string(&nested))
        .bind(capture_service::path_to_string(&nested.join("game")))
        .bind(capture_service::path_to_string(&nested_metadata))
        .execute(&fixture.pool)
        .await
        .unwrap();
        let nested_screenshot = nested.join("screenshot0001.png");
        tokio::fs::copy(&original.source_path, &nested_screenshot)
            .await
            .unwrap();
        write_metadata(
            &fixture,
            "screenshot0001",
            &serde_json::json!({
                "schemaVersion": 1,
                "screenshot": { "file": "screenshot0001.png" }
            }),
        );
        retry_pending(&fixture.pool, None, 200).await.unwrap();
        let parent_sidecar = fixture
            .metadata_directory
            .join("screenshot0001.scenevault.json");
        assert!(
            parent_sidecar.exists(),
            "an empty nested install still owns its paths"
        );

        tokio::fs::copy(
            &original.source_path,
            fixture.source_directory.join("screenshot0001.png"),
        )
        .await
        .unwrap();
        tokio::fs::write(&nested_screenshot, png_bytes(640, 360))
            .await
            .unwrap();
        let nested_sidecar = nested_metadata.join("screenshot0001.scenevault.json");
        tokio::fs::write(
            &nested_sidecar,
            tokio::fs::read(&parent_sidecar).await.unwrap(),
        )
        .await
        .unwrap();
        retry_pending(&fixture.pool, None, 200).await.unwrap();
        assert!(!parent_sidecar.exists());
        assert!(
            nested_sidecar.exists(),
            "the other game's new metadata stays pending"
        );
    }

    #[tokio::test]
    async fn duplicate_cleanup_keeps_ambiguous_sources_and_unsupported_metadata() {
        let fixture = fixture("guarded-cleanup").await;
        let original = register_capture(&fixture, "screenshot0000", 0).await;
        let duplicate = fixture.source_directory.join("screenshot0001.png");
        tokio::fs::copy(&original.source_path, &duplicate)
            .await
            .unwrap();
        for document in [
            serde_json::json!({ "schemaVersion": 2, "screenshot": { "file": "screenshot0001.png" } }),
            serde_json::json!({ "schemaVersion": 1, "screenshot": { "file": "../screenshot0001.png" } }),
            serde_json::json!({ "schemaVersion": 1, "screenshot": { "file": "screenshot0000.png" } }),
        ] {
            write_metadata(&fixture, "screenshot0001", &document);
            retry_pending(&fixture.pool, None, 200).await.unwrap();
            assert!(fixture
                .metadata_directory
                .join("screenshot0001.scenevault.json")
                .exists());
        }
        let additional = fixture.source_directory.join("additional");
        tokio::fs::create_dir_all(&additional).await.unwrap();
        tokio::fs::copy(&duplicate, additional.join("screenshot0001.png"))
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO project_source_directories (id, project_id, directory) VALUES (?, ?, ?)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(&fixture.project_id)
        .bind(capture_service::path_to_string(&additional))
        .execute(&fixture.pool)
        .await
        .unwrap();
        write_metadata(
            &fixture,
            "screenshot0001",
            &serde_json::json!({
                "schemaVersion": 1, "screenshot": { "file": "screenshot0001.png" }
            }),
        );
        retry_pending(&fixture.pool, None, 200).await.unwrap();
        assert!(fixture
            .metadata_directory
            .join("screenshot0001.scenevault.json")
            .exists());
    }
}
