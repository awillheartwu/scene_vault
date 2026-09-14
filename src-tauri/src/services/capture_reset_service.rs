//! Undo-classification reset: one in-memory, serial, per-item job.
//!
//! A job is never persisted. The preview freezes its list in memory, every
//! deletion is bound to the file identity captured at preview time, and the log
//! is the only durable trace. The single durable coordination point is
//! capture_items.operation_owner, which keeps recognition and labeling away
//! from a picture while its files are removed; that claim is released when the
//! picture finishes, successfully or not, and swept at startup after an
//! interrupted run.
use std::{
    collections::{BTreeSet, HashMap},
    path::Path,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use crate::{
    error::AppError,
    models::{
        capture_reset::{ResetExecuteInput, ResetJob, ResetJobItem, ResetPreviewInput},
        diagnostics::{LogLevel, LogRecord},
    },
    services::{
        capture_operation_service, capture_service,
        file_recycle_service::{is_unc_path, FileRecycler},
        log_service, thumbnail_service,
    },
};

/// In-memory previews are bounded: a selection pins its snapshot in RAM until it
/// runs or ages out.
const MAX_ITEMS: usize = 2_000;
/// Finished jobs stay addressable for the open dialog, then age out.
const JOB_TTL: Duration = Duration::from_secs(30 * 60);
/// A preview that was never executed is dropped after this long, so abandoned
/// dialogs cannot pin memory for the whole session.
const PREVIEW_TTL: Duration = Duration::from_secs(2 * 60 * 60);
const MAX_JOBS: usize = 16;
const VARIANTS: [&str; 4] = ["source", "annotated", "avatar", "destination"];

#[derive(Debug, Clone, PartialEq, FromRow)]
struct Snapshot {
    id: String,
    project_id: String,
    source_path: String,
    content_hash: Option<String>,
    classification: String,
    character_id: Option<String>,
    status: String,
    asset_id: Option<String>,
    annotated_path: Option<String>,
    avatar_path: Option<String>,
    destination_path: Option<String>,
    destination_avatar_path: Option<String>,
    manual_face_roi_json: Option<String>,
    manual_face_roi_ready: i64,
    processing_version: i64,
    operation_owner: Option<String>,
    updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Identity {
    canonical: String,
    hash: String,
    len: u64,
    modified_ns: u128,
}

/// One file the preview plans to clean. A file that was already gone keeps a
/// None identity, so a file that appears later can never pass as the previewed
/// one.
#[derive(Debug, Clone)]
struct PlannedFile {
    path: String,
    kind: &'static str,
    network: bool,
    identity: Option<Identity>,
}

#[derive(Debug, Clone)]
struct PlannedItem {
    snapshot: Snapshot,
    files: Vec<PlannedFile>,
}

#[derive(Debug)]
struct JobItem {
    capture_item_id: String,
    source_path: String,
    status: &'static str,
    reason: Option<&'static str>,
    error: Option<String>,
    plan: Option<PlannedItem>,
}

#[derive(Debug)]
struct JobState {
    id: String,
    delete_destination_files: Option<bool>,
    allow_permanent_network_delete: Option<bool>,
    executing: bool,
    created: Instant,
    finished: Option<Instant>,
    destination_files: BTreeSet<String>,
    network_destination_files: BTreeSet<String>,
    items: Vec<JobItem>,
}

type JobHandle = Arc<Mutex<JobState>>;
static JOBS: OnceLock<Mutex<HashMap<String, JobHandle>>> = OnceLock::new();

fn jobs() -> &'static Mutex<HashMap<String, JobHandle>> {
    JOBS.get_or_init(Default::default)
}

fn lock<T>(value: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    value.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Inserts a job and reclaims inactive ones: finished jobs age out, and a
/// preview that was never executed is dropped after two hours. The cap then
/// evicts the oldest inactive job, so a session that only opens and closes the
/// dialog cannot grow the registry.
fn store(job: JobState) -> JobHandle {
    let id = job.id.clone();
    let handle = Arc::new(Mutex::new(job));
    let mut all = lock(jobs());
    prune(&mut all);
    all.insert(id, handle.clone());
    handle
}

/// Drops abandoned previews and finished jobs, then trims the registry so the
/// job about to be inserted still fits inside MAX_JOBS. Jobs that are currently
/// executing are never touched.
fn prune(all: &mut HashMap<String, JobHandle>) {
    let now = Instant::now();
    let mut expired: Vec<String> = Vec::new();
    let mut inactive: Vec<(String, Instant)> = Vec::new();
    for (job_id, handle) in all.iter() {
        let state = lock(handle);
        if state.executing {
            continue;
        }
        match state.finished {
            Some(finished) => {
                if now.duration_since(finished) >= JOB_TTL {
                    expired.push(job_id.clone());
                }
                inactive.push((job_id.clone(), finished));
            }
            None => {
                if now.duration_since(state.created) >= PREVIEW_TTL {
                    expired.push(job_id.clone());
                }
                inactive.push((job_id.clone(), state.created));
            }
        }
    }
    for job_id in &expired {
        all.remove(job_id);
    }
    inactive.retain(|(job_id, _)| all.contains_key(job_id));
    inactive.sort_by_key(|(_, at)| *at);
    // Called from store() before the new job is inserted, hence the +1.
    let overflow = (all.len() + 1).saturating_sub(MAX_JOBS);
    for (job_id, _) in inactive.iter().take(overflow) {
        all.remove(job_id);
    }
}

/// Drops a preview the user closed without confirming. A job that already
/// started or is running is left alone, so a stray call can never cancel work.
pub fn discard(job_id: &str) -> Result<(), AppError> {
    let mut all = lock(jobs());
    let removable = match all.get(job_id) {
        Some(handle) => {
            let state = lock(handle);
            !state.executing && state.delete_destination_files.is_none()
        }
        None => false,
    };
    if removable {
        all.remove(job_id);
    }
    Ok(())
}

fn handle_of(job_id: &str) -> Result<JobHandle, AppError> {
    lock(jobs())
        .get(job_id)
        .cloned()
        .ok_or_else(|| AppError::NotFound("reset job".into()))
}

fn dto(state: &JobState) -> ResetJob {
    let status = match (state.delete_destination_files.is_some(), state.executing) {
        (false, _) => "preview",
        (true, true) => "running",
        (true, false) => "completed",
    };
    ResetJob {
        id: state.id.clone(),
        status: status.into(),
        executing: state.executing,
        delete_destination_files: state.delete_destination_files,
        allow_permanent_network_delete: state.allow_permanent_network_delete,
        items: state
            .items
            .iter()
            .map(|item| ResetJobItem {
                capture_item_id: item.capture_item_id.clone(),
                source_path: item.source_path.clone(),
                status: item.status.into(),
                reason: item.reason.map(str::to_owned),
                error: item.error.clone(),
            })
            .collect(),
        destination_file_count: state.destination_files.len() as u32,
        network_destination_file_count: state.network_destination_files.len() as u32,
    }
}

fn conflict(message: impl Into<String>) -> AppError {
    AppError::Conflict(message.into())
}

fn key(path: &Path) -> String {
    let value = capture_service::normalized_path_string(path);
    if cfg!(windows) {
        value.replace('/', "\\").to_lowercase()
    } else {
        value
    }
}

async fn identity(path: &Path) -> Result<Option<Identity>, AppError> {
    if !path.is_absolute() {
        return Err(conflict("reset file path must be absolute"));
    }
    let metadata = match tokio::fs::symlink_metadata(path).await {
        Ok(value) => value,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    // Never follow a leaf symlink into a different deletion target.
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(conflict("reset target is not a regular file"));
    }
    let canonical = key(&tokio::fs::canonicalize(path).await?);
    let modified_ns = metadata
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| conflict(e.to_string()))?
        .as_nanos();
    let hash = capture_service::sha256_file(path).await?;
    let after = tokio::fs::metadata(path).await?;
    if after.len() != metadata.len()
        || after.modified()? != metadata.modified()?
        || key(&tokio::fs::canonicalize(path).await?) != canonical
    {
        return Err(conflict("file changed during reset validation"));
    }
    Ok(Some(Identity {
        canonical,
        hash,
        len: metadata.len(),
        modified_ns,
    }))
}

async fn load(pool: &SqlitePool, id: &str) -> Result<Snapshot, AppError> {
    sqlx::query_as::<_, Snapshot>("SELECT * FROM capture_items WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::NotFound("capture item".into()))
}

fn eligible(row: &Snapshot) -> Result<(), AppError> {
    if row.classification == "unclassified"
        || matches!(row.status.as_str(), "queued" | "processing" | "archive_pending")
    {
        return Err(conflict("capture is unclassified or busy"));
    }
    if row.operation_owner.is_some() {
        return Err(conflict("capture is owned by another operation"));
    }
    Ok(())
}

async fn source_valid(row: &Snapshot) -> Result<(), AppError> {
    let current = identity(Path::new(&row.source_path))
        .await?
        .ok_or_else(|| conflict("original source is missing"))?;
    if row.content_hash.as_deref() != Some(current.hash.as_str()) {
        return Err(conflict("original source content identity is invalid"));
    }
    Ok(())
}

/// Failure classes the interface can explain. They are derived from the error
/// the service raises instead of being guessed by the caller.
fn reason_of(error: &AppError) -> &'static str {
    if let AppError::Io(io) = error {
        return match io.raw_os_error() {
            // ERROR_FILE_NOT_FOUND / ERROR_PATH_NOT_FOUND
            Some(2) | Some(3) => "source_gone",
            // ERROR_BAD_NETPATH / ERROR_BAD_NET_NAME / ERROR_NETNAME_DELETED /
            // ERROR_NETWORK_UNREACHABLE / ERROR_HOST_UNREACHABLE
            Some(53) | Some(64) | Some(67) | Some(1231) | Some(1232) => "network",
            // ERROR_ACCESS_DENIED / ERROR_WRITE_PROTECT / ERROR_NOT_READY
            Some(5) | Some(19) | Some(21) => "denied",
            // ERROR_SHARING_VIOLATION / ERROR_LOCK_VIOLATION
            Some(32) | Some(33) => "locked",
            _ => match io.kind() {
                std::io::ErrorKind::NotFound => "source_gone",
                std::io::ErrorKind::PermissionDenied => "denied",
                std::io::ErrorKind::WouldBlock => "locked",
                _ => "unknown",
            },
        };
    }
    let message = error.to_string();
    if message.contains("unclassified or busy") || message.contains("owned by another operation") {
        "busy"
    } else if message.contains("aliases a protected source")
        || message.contains("original source is missing")
        || message.contains("original source content identity is invalid")
    {
        "source_gone"
    } else if message.contains("changed since preview") || message.contains("changed before deletion")
    {
        "changed"
    } else if message.contains("does not support recoverable deletion")
        || message.contains("recycle operation was cancelled")
    {
        "denied"
    } else if message.contains("not a regular file") {
        "changed"
    } else if message.contains("UNC") || message.contains("network") {
        "network"
    } else if message.contains("claim") || message.contains("cleanup did not remove") {
        "locked"
    } else {
        "unknown"
    }
}

/// IDs constrain the project and other filters; explicit private IDs bypass only
/// include_private. No pagination: freezes all matching classified items.
pub async fn candidates(pool: &SqlitePool, input: &ResetPreviewInput) -> Result<Vec<String>, AppError> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM projects WHERE id = ?)")
        .bind(&input.project_id)
        .fetch_one(pool)
        .await?;
    if !exists {
        return Err(AppError::NotFound("project".into()));
    }
    let selected = input
        .capture_item_ids
        .as_ref()
        .map(|v| v.iter().collect::<BTreeSet<_>>());
    if selected.as_ref().is_some_and(|ids| ids.len() > MAX_ITEMS) {
        return Err(conflict("reset selection exceeds 2000 items"));
    }
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM capture_items WHERE project_id = ? AND classification != 'unclassified'
         AND (? IS NULL OR session_id = ?) AND (? IS NULL OR character_id = ?)
         AND (? IS NULL OR status = ?) AND (? OR classification != 'private') ORDER BY id",
    )
    .bind(&input.project_id)
    .bind(&input.session_id)
    .bind(&input.session_id)
    .bind(&input.character_id)
    .bind(&input.character_id)
    .bind(&input.status)
    .bind(&input.status)
    .bind(selected.is_some() || input.include_private.unwrap_or(false))
    .fetch_all(pool)
    .await?;
    let ids: Vec<_> = ids
        .into_iter()
        .filter(|id| selected.as_ref().is_none_or(|set| set.contains(id)))
        .collect();
    if ids.len() > MAX_ITEMS {
        return Err(conflict("reset selection exceeds 2000 items"));
    }
    Ok(ids)
}

/// Freezes the list and every file identity in memory. Ineligible pictures stay
/// visible as skipped with their reason instead of silently disappearing.
pub async fn preview(pool: &SqlitePool, input: ResetPreviewInput) -> Result<ResetJob, AppError> {
    let ids = candidates(pool, &input).await?;
    let mut items = Vec::new();
    let mut destination_files = BTreeSet::new();
    let mut network_destination_files = BTreeSet::new();
    for id in ids {
        let _guard = capture_operation_service::lock(pool, &id).await;
        let row = load(pool, &id).await?;
        let mut files = Vec::new();
        let result = async {
            eligible(&row)?;
            source_valid(&row).await?;
            let mut seen = BTreeSet::new();
            for (kind, path) in [
                ("destination", &row.destination_path),
                ("destination", &row.destination_avatar_path),
                ("derived", &row.annotated_path),
                ("derived", &row.avatar_path),
            ] {
                if let Some(path) = path {
                    if seen.insert(key(Path::new(path))) {
                        let file = Path::new(path);
                        files.push(PlannedFile {
                            path: path.clone(),
                            kind,
                            network: is_unc_path(file),
                            identity: identity(file).await?,
                        });
                    }
                }
            }
            Ok::<_, AppError>(())
        }
        .await;
        match result {
            Ok(()) => {
                for file in &files {
                    if file.kind == "destination" {
                        destination_files.insert(key(Path::new(&file.path)));
                        if file.network {
                            network_destination_files.insert(key(Path::new(&file.path)));
                        }
                    }
                }
                items.push(JobItem {
                    capture_item_id: row.id.clone(),
                    source_path: row.source_path.clone(),
                    status: "ready",
                    reason: None,
                    error: None,
                    plan: Some(PlannedItem {
                        snapshot: row,
                        files,
                    }),
                });
            }
            Err(error) => items.push(JobItem {
                capture_item_id: row.id.clone(),
                source_path: row.source_path.clone(),
                status: "skipped",
                reason: Some(reason_of(&error)),
                error: Some(error.to_string()),
                plan: None,
            }),
        }
    }
    let handle = store(JobState {
        id: uuid::Uuid::new_v4().to_string(),
        delete_destination_files: None,
        allow_permanent_network_delete: None,
        executing: false,
        created: Instant::now(),
        finished: None,
        destination_files,
        network_destination_files,
        items,
    });
    let state = lock(&handle);
    event(
        None,
        "reset_previewed",
        false,
        None,
        Some(format!(
            "job={} items={} archives={} network_archives={}",
            state.id,
            state.items.len(),
            state.destination_files.len(),
            state.network_destination_files.len()
        )),
    );
    Ok(dto(&state))
}

/// Reads one in-memory job for the open dialog.
pub fn get(job_id: &str) -> Result<ResetJob, AppError> {
    let handle = handle_of(job_id)?;
    let state = lock(&handle);
    Ok(dto(&state))
}

/// Clears claims a killed process left behind. A job is not resumed after a
/// restart, so without this sweep a picture would stay "being reset" forever.
pub async fn recover_interrupted(pool: &SqlitePool) -> Result<u64, AppError> {
    let released = sqlx::query(
        "UPDATE capture_items SET operation_owner = NULL WHERE operation_owner LIKE 'reset:%'",
    )
    .execute(pool)
    .await?
    .rows_affected();
    if released > 0 {
        event(
            None,
            "reset_interrupted",
            true,
            None,
            Some(format!(
                "released {released} unfinished reset claims; those pictures keep their classification"
            )),
        );
    }
    Ok(released)
}

/// Runs every unfinished picture of one preview, serially. Failures are reported
/// and the job ends; the preview list stays fixed, so a retry repeats exactly
/// this selection against the same file identities.
pub async fn execute<R: FileRecycler>(
    pool: &SqlitePool,
    cache_root: &Path,
    local_root: &Path,
    input: ResetExecuteInput,
    recycler: &R,
) -> Result<ResetJob, AppError> {
    let handle = handle_of(&input.job_id)?;
    let started = Instant::now();
    let order = {
        let mut state = lock(&handle);
        if let (Some(delete), Some(network)) = (
            state.delete_destination_files,
            state.allow_permanent_network_delete,
        ) {
            if delete != input.delete_destination_files || network != input.allow_permanent_network_delete
            {
                return Err(conflict("reset cleanup choices are immutable for this preview"));
            }
        }
        if state.executing {
            return Err(conflict("reset job is already running"));
        }
        state.delete_destination_files = Some(input.delete_destination_files);
        state.allow_permanent_network_delete = Some(input.allow_permanent_network_delete);
        state.executing = true;
        state.finished = None;
        state
            .items
            .iter_mut()
            .map(|item| {
                if !matches!(item.status, "succeeded" | "skipped") {
                    item.status = "ready";
                    item.reason = None;
                    item.error = None;
                }
                item.capture_item_id.clone()
            })
            .collect::<Vec<_>>()
    };
    event(
        None,
        "reset_started",
        false,
        None,
        Some(format!(
            "job={} items={} delete_archives={} allow_permanent_network_delete={}",
            input.job_id,
            order.len(),
            input.delete_destination_files,
            input.allow_permanent_network_delete
        )),
    );
    // One alias cache per run: references are re-read per picture, but each
    // reference path reaches the filesystem at most once.
    let mut canonical = CanonicalCache::new();
    let mut totals = Totals::default();
    for capture_item_id in order {
        let run = {
            let mut state = lock(&handle);
            let Some(item) = state
                .items
                .iter_mut()
                .find(|item| item.capture_item_id == capture_item_id)
            else {
                continue;
            };
            if matches!(item.status, "succeeded" | "skipped") {
                continue;
            }
            item.status = "running";
            item.plan.clone().map(|plan| (plan, item.source_path.clone()))
        };
        let Some((plan, source_path)) = run else {
            continue;
        };
        let _guard = capture_operation_service::lock(pool, &capture_item_id).await;
        let item_started = Instant::now();
        let mut stats = ItemStats::default();
        let result = execute_item(
            pool,
            cache_root,
            local_root,
            &input,
            &capture_item_id,
            &plan,
            recycler,
            &mut canonical,
            &mut stats,
        )
        .await;
        let duration_ms = Some(item_started.elapsed().as_secs_f64() * 1000.0);
        {
            let mut state = lock(&handle);
            if let Some(item) = state
                .items
                .iter_mut()
                .find(|item| item.capture_item_id == capture_item_id)
            {
                match &result {
                    Ok(()) => {
                        item.status = "succeeded";
                        item.reason = None;
                        item.error = None;
                    }
                    Err(error) => {
                        item.status = "failed";
                        item.reason = Some(reason_of(error));
                        item.error = Some(error.to_string());
                    }
                }
            }
        }
        match result {
            Ok(()) => {
                totals.succeeded += 1;
                event(
                    Some(&capture_item_id),
                    "reset_item_finished",
                    false,
                    duration_ms,
                    Some(format!(
                        "files={} deleted={} retained={} missing={} scan_ms={:.1} resolved={}",
                        stats.considered,
                        stats.deleted,
                        stats.retained,
                        stats.missing,
                        stats.scan_ms,
                        canonical.len()
                    )),
                );
            }
            Err(error) => {
                totals.failed += 1;
                event(
                    Some(&capture_item_id),
                    "reset_item_failed",
                    true,
                    duration_ms,
                    Some(format!(
                        "file={} reason={} error={error}",
                        path_file_name(&source_path),
                        reason_of(&error)
                    )),
                );
            }
        }
    }
    let (job, skipped) = {
        let mut state = lock(&handle);
        state.executing = false;
        state.finished = Some(Instant::now());
        let skipped = state
            .items
            .iter()
            .filter(|item| item.status == "skipped")
            .count() as u32;
        (dto(&state), skipped)
    };
    event(
        None,
        "reset_completed",
        totals.failed > 0,
        Some(started.elapsed().as_secs_f64() * 1000.0),
        Some(format!(
            "job={} succeeded={} failed={} skipped={skipped}",
            input.job_id, totals.succeeded, totals.failed
        )),
    );
    Ok(job)
}

#[derive(Default)]
struct Totals {
    succeeded: u32,
    failed: u32,
}

fn path_file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_owned())
}

/// Claims the picture, cleans its files and commits the classification reset.
/// The claim is always released afterwards, with or without success, so a
/// failure never locks a picture out of labeling or recognition.
async fn execute_item<R: FileRecycler>(
    pool: &SqlitePool,
    cache_root: &Path,
    local_root: &Path,
    input: &ResetExecuteInput,
    id: &str,
    plan: &PlannedItem,
    recycler: &R,
    canonical: &mut CanonicalCache,
    stats: &mut ItemStats,
) -> Result<(), AppError> {
    let snapshot = &plan.snapshot;
    let owner = format!("reset:{}", input.job_id);
    let current = load(pool, id).await?;
    // Reconciliation may relocate the same content or update timestamps while a
    // preview waits; anything else means the preview no longer describes it.
    // The picture is validated against its *current* location afterwards.
    let mut resolved = snapshot.clone();
    let mut expected = snapshot.clone();
    let mut comparable = current;
    expected.source_path = comparable.source_path.clone();
    resolved.source_path = comparable.source_path.clone();
    comparable.updated_at = snapshot.updated_at.clone();
    if comparable != expected {
        return Err(conflict("capture changed since reset preview; create a new preview"));
    }
    source_valid(&resolved).await?;
    // The claim only marks the picture as busy. The processing version is
    // bumped when the reset commits, so a failed attempt stays retryable while
    // a committed one invalidates any in-flight recognition result.
    let claimed = sqlx::query(
        "UPDATE capture_items SET operation_owner = ?
         WHERE id = ? AND operation_owner IS NULL AND processing_version = ?",
    )
    .bind(&owner)
    .bind(id)
    .bind(snapshot.processing_version)
    .execute(pool)
    .await?
    .rows_affected();
    if claimed != 1 {
        return Err(conflict("capture claim unavailable"));
    }
    let result = clean_and_commit(
        pool, cache_root, local_root, input, id, &resolved, &plan.files, recycler, canonical, stats,
    )
    .await;
    if result.is_err() && input.delete_destination_files {
        mark_missing_targets(pool, snapshot).await;
    }
    if let Err(error) = release_claim(pool, id, &owner).await {
        event(
            Some(id),
            "reset_claim_release_failed",
            true,
            None,
            Some(error.to_string()),
        );
    }
    result
}

/// A failed cleanup can leave some archives already removed. The picture keeps
/// its classification, so its file state has to say the archive is gone instead
/// of pointing at a path that no longer exists.
async fn mark_missing_targets(pool: &SqlitePool, snapshot: &Snapshot) {
    for (path, column) in [
        (&snapshot.destination_path, "destination_file_state"),
        (&snapshot.destination_avatar_path, "destination_avatar_file_state"),
    ] {
        let Some(path) = path else { continue };
        if tokio::fs::try_exists(path).await.unwrap_or(true) {
            continue;
        }
        let updated = sqlx::query(&format!(
            "UPDATE capture_items SET {column} = 'missing' WHERE id = ? AND {column} != 'missing'"
        ))
        .bind(&snapshot.id)
        .execute(pool)
        .await;
        if let Err(error) = updated {
            event(
                Some(&snapshot.id),
                "reset_file_state_failed",
                true,
                None,
                Some(error.to_string()),
            );
        }
    }
}

async fn release_claim(pool: &SqlitePool, id: &str, owner: &str) -> Result<(), AppError> {
    sqlx::query("UPDATE capture_items SET operation_owner = NULL WHERE id = ? AND operation_owner = ?")
        .bind(id)
        .bind(owner)
        .execute(pool)
        .await?;
    Ok(())
}

async fn clean_and_commit<R: FileRecycler>(
    pool: &SqlitePool,
    cache_root: &Path,
    local_root: &Path,
    input: &ResetExecuteInput,
    id: &str,
    snapshot: &Snapshot,
    planned_files: &[PlannedFile],
    recycler: &R,
    canonical: &mut CanonicalCache,
    stats: &mut ItemStats,
) -> Result<(), AppError> {
    // Deterministic thumbnails are only addressable here, where the app cache
    // roots are known; they are validated like every other derived file.
    let mut files = planned_files.to_vec();
    for variant in VARIANTS {
        let path = thumbnail_service::cache_path(local_root, id, variant);
        files.push(PlannedFile {
            path: capture_service::path_to_string(&path),
            kind: "thumbnail",
            network: false,
            identity: identity(&path).await?,
        });
    }
    files.sort_by_key(|file| (file.kind != "destination", file.path.clone()));
    let index = ReferenceIndex::load(pool, snapshot).await?;
    let owner = format!("reset:{}", input.job_id);
    let version = snapshot.processing_version;
    for file in files {
        stats.considered += 1;
        if file.kind == "destination" && !input.delete_destination_files {
            stats.retained += 1;
            continue;
        }
        let path = Path::new(&file.path);
        let actual = identity(path).await?;
        if actual.is_none() {
            stats.missing += 1;
            continue;
        }
        if file.identity != actual {
            return Err(conflict(format!(
                "reset file changed since preview: {}",
                file.path
            )));
        }
        // Check all captures, not merely this project, and asset references.
        // Shared paths are retained; a source alias always fails closed.
        let scan_started = Instant::now();
        let protection = protection(&index, canonical, path).await?;
        stats.scan_ms += scan_started.elapsed().as_secs_f64() * 1000.0;
        if protection == Protection::Source {
            return Err(conflict("reset target aliases a protected source"));
        }
        if protection == Protection::Shared {
            stats.retained += 1;
            continue;
        }
        if file.kind != "destination" && !within_roots(path, cache_root, local_root).await? {
            stats.retained += 1;
            continue;
        }
        // Revalidate the source before every destructive operation.
        source_valid(snapshot).await?;
        if identity(path).await? != actual {
            return Err(conflict("reset target changed before deletion"));
        }
        if file.kind == "destination" {
            if file.network {
                if !input.allow_permanent_network_delete {
                    return Err(conflict(
                        "UNC deletion requires explicit permanent-delete authorization",
                    ));
                }
                recycler.delete_permanently(path).await?;
            } else {
                recycler.recycle(path).await?;
            }
        } else {
            tokio::fs::remove_file(path).await?;
        }
        // A recycler returning success without moving the file must not cause
        // the original classification to be discarded.
        if tokio::fs::try_exists(path).await? {
            return Err(conflict("reset cleanup did not remove the file"));
        }
        stats.deleted += 1;
    }
    source_valid(snapshot).await?;
    let mut tx = pool.begin().await?;
    let changed = sqlx::query(
        "UPDATE capture_items SET classification = 'unclassified', character_id = NULL,
         status = 'awaiting_label', asset_id = NULL, annotated_path = NULL, avatar_path = NULL,
         destination_path = NULL, destination_avatar_path = NULL, destination_file_state = 'none',
         destination_avatar_file_state = 'none', face_box_json = NULL, face_count = NULL,
         face_feature_json = NULL, manual_face_roi_json = NULL, manual_face_roi_ready = 0,
         suggested_character_id = NULL, recognition_confidence = NULL, recognition_source = NULL,
         review_status = 'none', recognition_deferred = 0, verification_score = NULL,
         verification_status = 'unverified', best_other_score = NULL, best_other_character_id = NULL,
         error_message = NULL, failure_stage = NULL, attempt_count = 0, next_retry_at = NULL,
         processing_warnings_json = '[]', processed_at = NULL, archived_at = NULL,
         source_file_state = 'available', operation_owner = NULL,
         processing_version = processing_version + 1,
         reset_generation = reset_generation + 1,
         updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
         WHERE id = ? AND operation_owner = ? AND processing_version = ?",
    )
    .bind(id)
    .bind(&owner)
    .bind(version)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if changed != 1 {
        return Err(conflict("reset claim lost before final commit"));
    }
    for table in ["capture_faces", "character_face_samples"] {
        sqlx::query(&format!("DELETE FROM {table} WHERE capture_item_id = ?"))
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    // Unclassified/private-cover leakage: release explicit capture cover only.
    sqlx::query("UPDATE projects SET cover_capture_item_id = NULL WHERE cover_capture_item_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    if let Some(asset) = &snapshot.asset_id {
        // Keep every shared asset and all its links. Remove only exclusive
        // ownership; sources are indexed independently and also retain assets.
        sqlx::query(
            "DELETE FROM assets WHERE id = ?
            AND NOT EXISTS(SELECT 1 FROM capture_items WHERE asset_id = assets.id)
            AND NOT EXISTS(SELECT 1 FROM project_assets WHERE asset_id = assets.id AND project_id != ?)
            AND NOT EXISTS(SELECT 1 FROM collection_assets WHERE asset_id = assets.id)
            AND NOT EXISTS(SELECT 1 FROM projects WHERE cover_asset_id = assets.id)
            AND NOT EXISTS(SELECT 1 FROM characters WHERE avatar_asset_id = assets.id)
            AND NOT EXISTS(SELECT 1 FROM asset_tags WHERE asset_id = assets.id)
            AND path IN (?, ?, ?, ?)",
        )
        .bind(asset)
        .bind(&snapshot.project_id)
        .bind(&snapshot.destination_path)
        .bind(&snapshot.destination_avatar_path)
        .bind(&snapshot.annotated_path)
        .bind(&snapshot.avatar_path)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Per-item accounting for the completion log line.
#[derive(Default)]
struct ItemStats {
    considered: u32,
    deleted: u32,
    retained: u32,
    missing: u32,
    scan_ms: f64,
}

async fn within_roots(path: &Path, cache_root: &Path, local_root: &Path) -> Result<bool, AppError> {
    let canonical = tokio::fs::canonicalize(path).await?;
    for root in [cache_root, local_root] {
        match tokio::fs::canonicalize(root).await {
            Ok(root) if canonical.starts_with(&root) && canonical != root => return Ok(true),
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(false)
}

#[derive(Debug, PartialEq)]
enum Protection {
    None,
    Source,
    Shared,
}

/// Reference rows another capture or asset could share with a reset target.
/// Membership is evaluated once per picture instead of once per reset file.
struct ReferenceIndex {
    sources: Vec<String>,
    shared: Vec<String>,
    assets: Vec<(String, Option<String>)>,
}

impl ReferenceIndex {
    async fn load(pool: &SqlitePool, row: &Snapshot) -> Result<Self, AppError> {
        Ok(Self {
            sources: sqlx::query_scalar("SELECT DISTINCT source_path FROM capture_items")
                .fetch_all(pool)
                .await?,
            shared: sqlx::query_scalar(
                "SELECT annotated_path FROM capture_items WHERE id != ? AND annotated_path IS NOT NULL
        UNION SELECT avatar_path FROM capture_items WHERE id != ? AND avatar_path IS NOT NULL
        UNION SELECT destination_path FROM capture_items WHERE id != ? AND destination_path IS NOT NULL
        UNION SELECT destination_avatar_path FROM capture_items WHERE id != ? AND destination_avatar_path IS NOT NULL",
            )
            .bind(&row.id)
            .bind(&row.id)
            .bind(&row.id)
            .bind(&row.id)
            .fetch_all(pool)
            .await?,
            // All assets except a positively exclusive capture-owned asset protect both
            // their original file and thumbnail. Collection, cover and tag refs count.
            assets: sqlx::query_as(
                "SELECT path,thumbnail_path FROM assets a WHERE NOT (
        a.id = COALESCE(?, '')
        AND NOT EXISTS(SELECT 1 FROM capture_items WHERE asset_id = a.id AND id != ?)
        AND NOT EXISTS(SELECT 1 FROM project_assets WHERE asset_id = a.id AND project_id != ?)
        AND NOT EXISTS(SELECT 1 FROM collection_assets WHERE asset_id = a.id)
        AND NOT EXISTS(SELECT 1 FROM projects WHERE cover_asset_id = a.id)
        AND NOT EXISTS(SELECT 1 FROM characters WHERE avatar_asset_id = a.id)
        AND NOT EXISTS(SELECT 1 FROM asset_tags WHERE asset_id = a.id))",
            )
            .bind(&row.asset_id)
            .bind(&row.id)
            .bind(&row.project_id)
            .fetch_all(pool)
            .await?,
        })
    }
}

/// Resolved canonical keys for one run, keyed by normalized spelling and
/// memoized so a reference costs at most one filesystem lookup. The list is
/// scanned for every reset file, and repeated resolutions of the same paths
/// (plus the repeated resolution of the target itself) dominated reset time.
type CanonicalCache = HashMap<String, Option<String>>;

/// A missing path resolves to None. Any other failure is returned, so an
/// offline or inaccessible reference can never pass as "not an alias".
async fn resolved(cache: &mut CanonicalCache, path: &Path) -> Result<Option<String>, AppError> {
    let cache_key = key(path);
    if let Some(hit) = cache.get(&cache_key) {
        return Ok(hit.clone());
    }
    let value = match tokio::fs::canonicalize(path).await {
        Ok(resolved) => Some(key(&resolved)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        // If a reference is offline/inaccessible its alias cannot be ruled out.
        Err(e) => return Err(e.into()),
    };
    cache.insert(cache_key, value.clone());
    Ok(value)
}

async fn aliases(
    cache: &mut CanonicalCache,
    candidate: &str,
    reference: &str,
) -> Result<bool, AppError> {
    if candidate == key(Path::new(reference)) {
        return Ok(true);
    }
    Ok(resolved(cache, Path::new(reference)).await?.as_deref() == Some(candidate))
}

async fn protection(
    index: &ReferenceIndex,
    cache: &mut CanonicalCache,
    path: &Path,
) -> Result<Protection, AppError> {
    // Resolved once per file; every alias test below reuses it.
    let candidate = key(&tokio::fs::canonicalize(path).await?);
    for source in &index.sources {
        if aliases(cache, &candidate, source).await? {
            return Ok(Protection::Source);
        }
    }
    for reference in &index.shared {
        if aliases(cache, &candidate, reference).await? {
            return Ok(Protection::Shared);
        }
    }
    for (reference, thumbnail) in &index.assets {
        if aliases(cache, &candidate, reference).await? {
            return Ok(Protection::Shared);
        }
        if let Some(thumbnail) = thumbnail {
            if aliases(cache, &candidate, thumbnail).await? {
                return Ok(Protection::Shared);
            }
        }
    }
    Ok(Protection::None)
}

/// Reset progress events keep their numbers inside the message text, so one log
/// line answers "which phase was slow" without a schema change.
fn event(
    item_id: Option<&str>,
    name: &str,
    failed: bool,
    duration_ms: Option<f64>,
    detail: Option<String>,
) {
    log_service::record_event(LogRecord {
        level: if failed { LogLevel::Warn } else { LogLevel::Info },
        module: "capture.reset".into(),
        message: detail.map_or_else(|| name.to_owned(), |detail| format!("{name} {detail}")),
        event: Some(name.into()),
        capture_item_id: item_id.map(str::to_owned),
        duration_ms,
        ..Default::default()
    });
}

#[cfg(test)]
#[path = "capture_reset_service_tests.rs"]
mod tests;
