use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    error::AppError,
    models::{
        archive_manifest::RebuildArchiveManifestResult,
        diagnostics::{LogLevel, LogRecord},
    },
    services::{app_settings_service, capture_service, log_service},
};

/// File name the rating workflows look for inside the directory they scan. The
/// plugin ignore lists contain it as well, so it never becomes a candidate.
pub const MANIFEST_FILE_NAME: &str = "project.json";

/// Document kind marker; consumers may ignore unknown keys and versions.
const MANIFEST_KIND: &str = "scenevault.character-manifest";

/// Only canonical person directories carry a manifest. Files the user moved into
/// their own staging folders are deliberately not described, because the
/// consumer only scans the directory it was registered with.
const PERSON_DIRECTORIES: [&str; 3] = ["人物图", "人物图（原图）", "人物图（未识别）"];

/// Archives arrive in bursts, so rebuilds are coalesced: the background task
/// waits for a quiet period before touching the archive share again.
const QUIET_PERIOD: Duration = Duration::from_secs(10);
const TICK: Duration = Duration::from_secs(5);
const BACKGROUND_MODULE: &str = "capture.manifest";

static PENDING: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();

fn pending() -> &'static Mutex<HashMap<String, Instant>> {
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Debug, sqlx::FromRow)]
struct ArchivedRow {
    id: String,
    character_id: Option<String>,
    character_name: Option<String>,
    classification: String,
    source_path: String,
    content_hash: Option<String>,
    destination_path: String,
    file_size: Option<i64>,
    archived_at: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ManifestDocument {
    version: u32,
    kind: &'static str,
    generated_at: String,
    generator: ManifestGenerator,
    project: ManifestProject,
    files: Vec<String>,
    characters: BTreeMap<String, Vec<String>>,
    entries: Vec<ManifestEntry>,
}

#[derive(Debug, Serialize)]
struct ManifestGenerator {
    name: &'static str,
    version: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ManifestProject {
    id: String,
    name: String,
    directory: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ManifestEntry {
    file: String,
    capture_item_id: String,
    character_id: Option<String>,
    character_name: Option<String>,
    classification: String,
    source_path: String,
    content_hash: Option<String>,
    file_size: Option<i64>,
    archived_at: Option<String>,
}

enum ManifestWrite {
    Written,
    Unchanged,
}

/// Marks a project for a debounced manifest rebuild. Archive, reconcile,
/// character and reset paths call this instead of writing the manifest
/// themselves, so a burst of archives produces a single share write.
pub fn request_rebuild(project_id: &str) {
    let project_id = project_id.trim();
    if project_id.is_empty() {
        return;
    }
    if let Ok(mut guard) = pending().lock() {
        guard.insert(project_id.to_owned(), Instant::now());
    }
}

/// Automatic maintenance can be switched off globally; explicit rebuilds
/// from the workbench stay available either way.
async fn automatic_writes_enabled(pool: &SqlitePool) -> Result<bool, AppError> {
    Ok(app_settings_service::get(pool)
        .await?
        .archive_manifest_auto_write)
}

fn take_due() -> Vec<String> {
    let Ok(mut guard) = pending().lock() else {
        return Vec::new();
    };
    let now = Instant::now();
    let due: Vec<String> = guard
        .iter()
        .filter(|(_, requested)| now.duration_since(**requested) >= QUIET_PERIOD)
        .map(|(project_id, _)| project_id.clone())
        .collect();
    for project_id in &due {
        guard.remove(project_id);
    }
    due
}

/// Test helper: projects currently waiting for a debounced rebuild.
#[cfg(test)]
pub(crate) fn pending_project_ids() -> Vec<String> {
    pending()
        .lock()
        .map(|guard| guard.keys().cloned().collect())
        .unwrap_or_default()
}

/// Creates manifests that do not exist yet: that covers libraries archived
/// before this feature and manifests deleted by hand. Existing manifests are
/// left to the regular triggers so app start stays cheap.
pub async fn enqueue_missing(pool: &SqlitePool) -> Result<u32, AppError> {
    if !automatic_writes_enabled(pool).await? {
        return Ok(0);
    }
    let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
        r#"
        SELECT DISTINCT ci.project_id, ci.destination_path, p.destination_directory
        FROM capture_items ci
        JOIN projects p ON p.id = ci.project_id
        WHERE ci.status = 'completed'
          AND ci.classification = 'person'
          AND ci.destination_path IS NOT NULL
          AND ci.destination_file_state != 'missing'
        "#,
    )
    .fetch_all(pool)
    .await?;

    let mut seen = HashSet::new();
    let mut queued = 0_u32;
    for (project_id, destination_path, destination_root) in rows {
        let Some(root) = destination_root.as_deref() else {
            continue;
        };
        let Some(directory) = manifest_directory(root, &destination_path) else {
            continue;
        };
        if !seen.insert(capture_service::path_to_string(&directory)) {
            continue;
        }
        if tokio::fs::try_exists(directory.join(MANIFEST_FILE_NAME))
            .await
            .unwrap_or(false)
        {
            continue;
        }
        request_rebuild(&project_id);
        queued = queued.saturating_add(1);
    }
    Ok(queued)
}

/// Runs the debounced writer. Called once from the app setup hook, which is not
/// inside a Tokio runtime context, so it must go through the Tauri runtime.
pub fn run_background(pool: SqlitePool) {
    tauri::async_runtime::spawn(async move {
        match enqueue_missing(&pool).await {
            Ok(queued) if queued > 0 => log_service::record_event(LogRecord {
                level: LogLevel::Info,
                module: BACKGROUND_MODULE.to_owned(),
                message: format!("queued {queued} missing archive manifest directories"),
                event: Some("archive_manifest_backfill_queued".to_owned()),
                outcome: Some("succeeded".to_owned()),
                ..Default::default()
            }),
            Ok(_) => (),
            Err(error) => log_service::record_event(LogRecord {
                level: LogLevel::Warn,
                module: BACKGROUND_MODULE.to_owned(),
                message: format!("could not scan for missing archive manifests: {error}"),
                event: Some("archive_manifest_backfill_failed".to_owned()),
                outcome: Some("failed".to_owned()),
                error_code: Some("archive_manifest_backfill_error".to_owned()),
                ..Default::default()
            }),
        }
        let mut interval = tokio::time::interval(TICK);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            let automatic = match automatic_writes_enabled(&pool).await {
                Ok(value) => value,
                Err(error) => {
                    log_service::record_event(LogRecord {
                        level: LogLevel::Warn,
                        module: BACKGROUND_MODULE.to_owned(),
                        message: format!("could not read archive manifest settings: {error}"),
                        event: Some("archive_manifest_settings_failed".to_owned()),
                        outcome: Some("failed".to_owned()),
                        error_code: Some("archive_manifest_settings_error".to_owned()),
                        ..Default::default()
                    });
                    true
                }
            };
            let due = take_due();
            if !automatic {
                // Turning automatic maintenance off drops queued requests instead
                // of replaying them when it is switched back on later.
                continue;
            }
            for project_id in due {
                if let Err(error) = rebuild(&pool, &project_id).await {
                    log_service::record_event(LogRecord {
                        level: LogLevel::Warn,
                        module: BACKGROUND_MODULE.to_owned(),
                        message: format!("archive manifest rebuild failed: {error}"),
                        event: Some("archive_manifest_rebuild_failed".to_owned()),
                        outcome: Some("failed".to_owned()),
                        error_code: Some("archive_manifest_rebuild_error".to_owned()),
                        project_id: Some(project_id),
                        ..Default::default()
                    });
                }
            }
        }
    });
}

/// Resolves the directory a manifest belongs to: the canonical person
/// directory of the project archive root that holds the file.
fn manifest_directory(destination_root: &str, destination_path: &str) -> Option<PathBuf> {
    let directory = Path::new(destination_path).parent()?;
    let name = directory.file_name().and_then(|value| value.to_str())?;
    if !PERSON_DIRECTORIES.contains(&name) {
        return None;
    }
    let parent = directory.parent()?;
    let root = destination_root.trim_end_matches(|character| character == '\\' || character == '/');
    if !capture_service::path_to_string(parent).eq_ignore_ascii_case(root) {
        return None;
    }
    Some(directory.to_path_buf())
}

async fn directory_file_names(directory: &Path) -> Option<HashSet<String>> {
    let mut entries = tokio::fs::read_dir(directory).await.ok()?;
    let mut names = HashSet::new();
    loop {
        let Ok(next) = entries.next_entry().await else {
            break;
        };
        let Some(entry) = next else { break };
        if !entry.file_type().await.map(|kind| kind.is_file()).unwrap_or(false) {
            continue;
        }
        if let Some(name) = entry.file_name().to_str() {
            names.insert(name.to_lowercase());
        }
    }
    Some(names)
}

/// Timestamps change on every render, so the comparison ignores the generated
/// timestamp field and only reacts to mapping changes. An unchanged mapping
/// therefore leaves the existing file, and its timestamp, untouched.
fn manifest_payload_matches(existing: &str, candidate: &str) -> bool {
    let Ok(mut existing) = serde_json::from_str::<serde_json::Value>(existing) else {
        return false;
    };
    let Ok(mut candidate) = serde_json::from_str::<serde_json::Value>(candidate) else {
        return false;
    };
    for value in [&mut existing, &mut candidate] {
        if let Some(object) = value.as_object_mut() {
            object.remove("generatedAt");
        }
    }
    existing == candidate
}

async fn write_manifest(
    directory: &Path,
    document: &ManifestDocument,
) -> Result<ManifestWrite, AppError> {
    let target = directory.join(MANIFEST_FILE_NAME);
    let contents = serde_json::to_string_pretty(document)
        .map_err(|error| AppError::Validation(format!("cannot encode archive manifest: {error}")))?;
    if let Ok(existing) = tokio::fs::read_to_string(&target).await {
        if manifest_payload_matches(&existing, &contents) {
            return Ok(ManifestWrite::Unchanged);
        }
    }
    let temporary = directory.join(format!(
        ".{MANIFEST_FILE_NAME}.{}.partial",
        Uuid::new_v4()
    ));
    tokio::fs::write(&temporary, contents.as_bytes()).await?;
    let verified = tokio::fs::metadata(&temporary)
        .await
        .map(|metadata| metadata.is_file() && metadata.len() == contents.len() as u64)
        .unwrap_or(false);
    if !verified {
        let _ = tokio::fs::remove_file(&temporary).await;
        return Err(AppError::Validation(
            "archive manifest length verification failed".to_owned(),
        ));
    }
    if let Err(error) = tokio::fs::rename(&temporary, &target).await {
        let _ = tokio::fs::remove_file(&temporary).await;
        return Err(AppError::Io(error));
    }
    Ok(ManifestWrite::Written)
}

/// Rewrites the character manifest of every canonical person directory of one
/// project. The file describes an image-to-character mapping for rating
/// workflows and is always regenerated from the database, never patched.
pub async fn rebuild(
    pool: &SqlitePool,
    project_id: &str,
) -> Result<RebuildArchiveManifestResult, AppError> {
    let project_id = project_id.trim();
    if project_id.is_empty() {
        return Err(AppError::Validation("project id cannot be empty".to_owned()));
    }
    let project: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT name, destination_directory FROM projects WHERE id = ?")
            .bind(project_id)
            .fetch_optional(pool)
            .await?;
    let Some((project_name, destination_root)) = project else {
        return Err(AppError::NotFound("project".to_owned()));
    };
    let mut result = RebuildArchiveManifestResult {
        project_id: project_id.to_owned(),
        ..Default::default()
    };
    let destination_root = destination_root.unwrap_or_default();
    let rows = sqlx::query_as::<_, ArchivedRow>(
        r#"
        SELECT ci.id, ci.character_id, c.name AS character_name, ci.classification,
               ci.source_path, ci.content_hash, ci.destination_path, ci.file_size,
               ci.archived_at
        FROM capture_items ci
        LEFT JOIN characters c ON c.id = ci.character_id
        WHERE ci.project_id = ?
          AND ci.status = 'completed'
          AND ci.destination_path IS NOT NULL
          AND ci.destination_file_state != 'missing'
        ORDER BY ci.captured_at, ci.id
        "#,
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?;
    let generated_at: String =
        sqlx::query_scalar("SELECT strftime('%Y-%m-%dT%H:%M:%fZ','now')")
            .fetch_one(pool)
            .await?;

    let mut groups: Vec<(PathBuf, Vec<ArchivedRow>)> = Vec::new();
    for row in rows {
        let Some(directory) = manifest_directory(&destination_root, &row.destination_path) else {
            continue;
        };
        match groups
            .iter_mut()
            .find(|(existing, _)| *existing == directory)
        {
            Some((_, items)) => items.push(row),
            None => groups.push((directory, vec![row])),
        }
    }
    result.manifest_count = u32::try_from(groups.len()).unwrap_or(u32::MAX);
    let described: Vec<PathBuf> = groups
        .iter()
        .map(|(directory, _)| directory.clone())
        .collect();

    for (directory, rows) in groups {
        let Some(present) = directory_file_names(&directory).await else {
            result.skipped_count = result.skipped_count.saturating_add(1);
            continue;
        };
        let mut files = Vec::new();
        let mut characters: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut entries = Vec::new();
        for row in rows {
            let Some(file_name) = Path::new(&row.destination_path)
                .file_name()
                .and_then(|value| value.to_str())
                .map(str::to_owned)
            else {
                continue;
            };
            if !present.contains(&file_name.to_lowercase()) {
                continue;
            }
            files.push(file_name.clone());
            if let Some(name) = row
                .character_name
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                characters
                    .entry(name.to_owned())
                    .or_default()
                    .push(file_name.clone());
            }
            entries.push(ManifestEntry {
                file: file_name,
                capture_item_id: row.id,
                character_id: row.character_id,
                character_name: row.character_name,
                classification: row.classification,
                source_path: row.source_path,
                content_hash: row.content_hash,
                file_size: row.file_size,
                archived_at: row.archived_at,
            });
        }

        let target = directory.join(MANIFEST_FILE_NAME);
        if files.is_empty() {
            // Nothing left to describe: drop a stale manifest instead of
            // advertising files that are no longer archived.
            if tokio::fs::try_exists(&target).await.unwrap_or(false) {
                match tokio::fs::remove_file(&target).await {
                    Ok(()) => result.written_count = result.written_count.saturating_add(1),
                    Err(error) => {
                        result.failed_count = result.failed_count.saturating_add(1);
                        log_service::record_event(LogRecord {
                            level: LogLevel::Warn,
                            module: BACKGROUND_MODULE.to_owned(),
                            message: format!("could not remove stale archive manifest: {error}"),
                            event: Some("archive_manifest_remove_failed".to_owned()),
                            outcome: Some("failed".to_owned()),
                            error_code: Some("archive_manifest_remove_error".to_owned()),
                            project_id: Some(project_id.to_owned()),
                            ..Default::default()
                        });
                    }
                }
            }
            continue;
        }

        result.entry_count = result
            .entry_count
            .saturating_add(u32::try_from(files.len()).unwrap_or(u32::MAX));
        result.character_count = result
            .character_count
            .saturating_add(u32::try_from(characters.len()).unwrap_or(u32::MAX));
        let document = ManifestDocument {
            version: 1,
            kind: MANIFEST_KIND,
            generated_at: generated_at.clone(),
            generator: ManifestGenerator {
                name: "Scene Vault",
                version: env!("CARGO_PKG_VERSION"),
            },
            project: ManifestProject {
                id: project_id.to_owned(),
                name: project_name.clone(),
                directory: directory
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default()
                    .to_owned(),
            },
            files,
            characters,
            entries,
        };
        match write_manifest(&directory, &document).await {
            Ok(ManifestWrite::Written) => {
                result.written_count = result.written_count.saturating_add(1)
            }
            Ok(ManifestWrite::Unchanged) => {
                result.unchanged_count = result.unchanged_count.saturating_add(1)
            }
            Err(error) => {
                result.failed_count = result.failed_count.saturating_add(1);
                log_service::record_event(LogRecord {
                    level: LogLevel::Warn,
                    module: BACKGROUND_MODULE.to_owned(),
                    message: format!("could not write archive manifest: {error}"),
                    event: Some("archive_manifest_write_failed".to_owned()),
                    outcome: Some("failed".to_owned()),
                    error_code: Some("archive_manifest_write_error".to_owned()),
                    project_id: Some(project_id.to_owned()),
                    ..Default::default()
                });
            }
        }
    }

    // A directory can lose every archived file at once (reset, deletion or a
    // manual move); its rows then disappear from the query above, so probe the
    // canonical directories for manifests that no longer describe anything.
    for name in PERSON_DIRECTORIES {
        let directory = Path::new(&destination_root).join(name);
        if described.iter().any(|value| *value == directory) {
            continue;
        }
        let target = directory.join(MANIFEST_FILE_NAME);
        if !tokio::fs::try_exists(&target).await.unwrap_or(false) {
            continue;
        }
        match tokio::fs::remove_file(&target).await {
            Ok(()) => result.written_count = result.written_count.saturating_add(1),
            Err(error) => {
                result.failed_count = result.failed_count.saturating_add(1);
                log_service::record_event(LogRecord {
                    level: LogLevel::Warn,
                    module: BACKGROUND_MODULE.to_owned(),
                    message: format!("could not remove stale archive manifest: {error}"),
                    event: Some("archive_manifest_remove_failed".to_owned()),
                    outcome: Some("failed".to_owned()),
                    error_code: Some("archive_manifest_remove_error".to_owned()),
                    project_id: Some(project_id.to_owned()),
                    ..Default::default()
                });
            }
        }
    }

    if result.written_count == 0 && result.entry_count == 0 && result.manifest_count == 0 {
        result.message = "该项目还没有可生成评分清单的归档人物图".to_owned();
        return Ok(result);
    }
    result.message = if result.failed_count > 0 {
        format!(
            "评分清单部分失败：更新 {} 个、跳过 {} 个、失败 {} 个",
            result.written_count, result.skipped_count, result.failed_count
        )
    } else if result.written_count == 0 {
        format!(
            "评分清单已是最新：{} 张图、{} 个人物",
            result.entry_count, result.character_count
        )
    } else {
        format!(
            "已更新评分清单：{} 张图、{} 个人物",
            result.entry_count, result.character_count
        )
    };
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{db, services::test_support};

    async fn character(
        pool: &SqlitePool,
        fixture: &test_support::ProjectFixture,
        name: &str,
    ) -> String {
        let id = format!("character-{name}");
        sqlx::query(
            "INSERT INTO characters (id, project_id, name, aliases_json) VALUES (?, ?, ?, '[]')",
        )
        .bind(&id)
        .bind(&fixture.project_id)
        .bind(name)
        .execute(pool)
        .await
        .expect("character");
        id
    }

    /// Registers one completed person capture; an empty payload leaves the
    /// archive file missing so tests can cover stale database rows.
    async fn archived_item(
        pool: &SqlitePool,
        fixture: &test_support::ProjectFixture,
        directory: &str,
        file_name: &str,
        character_id: Option<&str>,
        content: &[u8],
    ) -> String {
        let session = test_support::start_session(pool, &fixture.project_id)
            .await
            .expect("session");
        let destination = fixture.destination_directory.join(directory).join(file_name);
        if !content.is_empty() {
            tokio::fs::create_dir_all(destination.parent().expect("parent"))
                .await
                .expect("archive directory");
            tokio::fs::write(&destination, content)
                .await
                .expect("archive file");
        }
        let item_id = format!("item-{file_name}");
        sqlx::query(
            "INSERT INTO capture_items (id, project_id, session_id, source_path, classification, character_id, status, destination_path, destination_file_state, archived_at) VALUES (?, ?, ?, ?, 'person', ?, 'completed', ?, 'available', '2026-09-09T06:00:00.000Z')",
        )
        .bind(&item_id)
        .bind(&fixture.project_id)
        .bind(&session.id)
        .bind(format!("D:/source/{file_name}"))
        .bind(character_id)
        .bind(capture_service::path_to_string(&destination))
        .execute(pool)
        .await
        .expect("capture item");
        item_id
    }

    fn manifest_path(fixture: &test_support::ProjectFixture, directory: &str) -> PathBuf {
        fixture
            .destination_directory
            .join(directory)
            .join(MANIFEST_FILE_NAME)
    }

    #[tokio::test]
    async fn writes_a_manifest_mapping_archived_files_to_characters() {
        let pool = db::test_pool().await;
        let fixture = test_support::project_with_directories(&pool, "Manifest")
            .await
            .expect("fixture");
        let ginevra = character(&pool, &fixture, "吉妮瓦").await;
        let tegan = character(&pool, &fixture, "蒂根").await;
        archived_item(
            &pool,
            &fixture,
            "人物图",
            "u4ia_0001 - 吉妮瓦 - 631cd9c5.png",
            Some(&ginevra),
            b"one",
        )
        .await;
        archived_item(
            &pool,
            &fixture,
            "人物图",
            "u4ia_0002 - 蒂根 - d40a1207.png",
            Some(&tegan),
            b"two",
        )
        .await;

        let result = rebuild(&pool, &fixture.project_id).await.expect("rebuild");
        assert_eq!(result.written_count, 1);
        assert_eq!(result.entry_count, 2);
        assert_eq!(result.character_count, 2);
        assert_eq!(result.failed_count, 0);
        assert_eq!(result.skipped_count, 0);

        let raw = tokio::fs::read_to_string(manifest_path(&fixture, "人物图"))
            .await
            .expect("manifest");
        let document: serde_json::Value = serde_json::from_str(&raw).expect("manifest json");
        assert_eq!(document["version"], 1);
        assert_eq!(document["kind"], MANIFEST_KIND);
        assert_eq!(document["project"]["directory"], "人物图");
        assert_eq!(document["files"].as_array().expect("files").len(), 2);
        let characters = document["characters"].as_object().expect("characters");
        assert_eq!(
            characters["吉妮瓦"][0],
            "u4ia_0001 - 吉妮瓦 - 631cd9c5.png"
        );
        assert_eq!(
            characters["蒂根"][0],
            "u4ia_0002 - 蒂根 - d40a1207.png"
        );
        assert_eq!(document["entries"][1]["characterName"], "蒂根");
    }

    #[tokio::test]
    async fn ignores_staging_directories_and_files_missing_on_disk() {
        let pool = db::test_pool().await;
        let fixture = test_support::project_with_directories(&pool, "Staging")
            .await
            .expect("fixture");
        let ginevra = character(&pool, &fixture, "吉妮瓦").await;
        archived_item(
            &pool,
            &fixture,
            "zancun1",
            "u4ia_0001 - 吉妮瓦 - 631cd9c5.png",
            Some(&ginevra),
            b"staged",
        )
        .await;
        archived_item(
            &pool,
            &fixture,
            "人物图",
            "u4ia_0002 - 吉妮瓦 - d40a1207.png",
            Some(&ginevra),
            b"",
        )
        .await;

        let result = rebuild(&pool, &fixture.project_id).await.expect("rebuild");
        assert_eq!(result.manifest_count, 1);
        assert_eq!(result.entry_count, 0);
        assert!(!manifest_path(&fixture, "人物图").exists());
        assert!(!manifest_path(&fixture, "zancun1").exists());
    }

    #[tokio::test]
    async fn leaves_an_unchanged_mapping_untouched() {
        let pool = db::test_pool().await;
        let fixture = test_support::project_with_directories(&pool, "Stable")
            .await
            .expect("fixture");
        let ginevra = character(&pool, &fixture, "吉妮瓦").await;
        archived_item(
            &pool,
            &fixture,
            "人物图",
            "u4ia_0001 - 吉妮瓦 - 631cd9c5.png",
            Some(&ginevra),
            b"one",
        )
        .await;

        let first = rebuild(&pool, &fixture.project_id).await.expect("first");
        assert_eq!(first.written_count, 1);
        let before = tokio::fs::read_to_string(manifest_path(&fixture, "人物图"))
            .await
            .expect("manifest");

        let second = rebuild(&pool, &fixture.project_id).await.expect("second");
        assert_eq!(second.written_count, 0);
        assert_eq!(second.unchanged_count, 1);
        let after = tokio::fs::read_to_string(manifest_path(&fixture, "人物图"))
            .await
            .expect("manifest");
        assert_eq!(before, after);
    }

    #[tokio::test]
    async fn drops_a_manifest_when_no_archived_file_remains() {
        let pool = db::test_pool().await;
        let fixture = test_support::project_with_directories(&pool, "Cleared")
            .await
            .expect("fixture");
        let ginevra = character(&pool, &fixture, "吉妮瓦").await;
        let item_id = archived_item(
            &pool,
            &fixture,
            "人物图",
            "u4ia_0001 - 吉妮瓦 - 631cd9c5.png",
            Some(&ginevra),
            b"one",
        )
        .await;
        rebuild(&pool, &fixture.project_id).await.expect("first");
        assert!(manifest_path(&fixture, "人物图").is_file());

        sqlx::query("UPDATE capture_items SET destination_path = NULL, destination_file_state = 'none' WHERE id = ?")
            .bind(&item_id)
            .execute(&pool)
            .await
            .expect("clear archive");

        let result = rebuild(&pool, &fixture.project_id).await.expect("second");
        assert_eq!(result.entry_count, 0);
        assert!(!manifest_path(&fixture, "人物图").exists());
    }

    #[tokio::test]
    async fn skips_the_startup_backfill_when_automatic_writes_are_disabled() {
        let pool = db::test_pool().await;
        let fixture = test_support::project_with_directories(&pool, "AutoWriteOff")
            .await
            .expect("fixture");
        let ginevra = character(&pool, &fixture, "吉妮瓦").await;
        archived_item(
            &pool,
            &fixture,
            "人物图",
            "u4ia_0001 - 吉妮瓦 - 631cd9c5.png",
            Some(&ginevra),
            b"one",
        )
        .await;

        assert_eq!(enqueue_missing(&pool).await.expect("scan"), 1);

        crate::services::app_settings_service::update(
            &pool,
            crate::models::app_settings::UpdateAppSettingsInput {
                settings: crate::models::app_settings::AppSettings {
                    archive_manifest_auto_write: false,
                    ..Default::default()
                },
            },
        )
        .await
        .expect("disable automatic maintenance");

        assert_eq!(enqueue_missing(&pool).await.expect("scan"), 0);
    }
}
