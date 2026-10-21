use std::{
    collections::{BTreeMap, VecDeque},
    fs, io,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

use chrono::{SecondsFormat, Utc};
use sqlx::SqlitePool;
use tokio::{
    sync::{Mutex as AsyncMutex, OnceCell, Semaphore},
    task::JoinSet,
};

use crate::{
    error::AppError,
    models::project::{ProjectArchiveUsage, ProjectArchiveUsageState as State},
};

use super::capture_service;

const SCAN_CONCURRENCY: usize = 2;
const SCAN_TIMEOUT: Duration = Duration::from_secs(15);
static SCAN_SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();
type ActiveRequest = (String, Arc<AtomicBool>);
#[derive(Default)]
struct Requests {
    active: BTreeMap<String, ActiveRequest>,
    early_cancellations: VecDeque<(String, String)>,
}
static REQUESTS: OnceLock<Mutex<Requests>> = OnceLock::new();

struct RequestGuard {
    window: String,
    id: String,
    cancelled: Arc<AtomicBool>,
}

impl RequestGuard {
    fn begin(window: &str, id: &str) -> Result<Self, AppError> {
        if id.is_empty() || id.len() > 128 {
            return Err(AppError::Validation(
                "invalid archive scan request id".to_owned(),
            ));
        }
        let mut requests = REQUESTS
            .get_or_init(Mutex::default)
            .lock()
            .map_err(|_| AppError::Conflict("archive scan state unavailable".to_owned()))?;
        let early = requests
            .early_cancellations
            .iter()
            .position(|(owner, pending)| owner == window && pending == id);
        let cancelled = Arc::new(AtomicBool::new(early.is_some()));
        if let Some(position) = early {
            requests.early_cancellations.remove(position);
        } else if let Some((_, previous)) = requests
            .active
            .insert(window.to_owned(), (id.to_owned(), cancelled.clone()))
        {
            previous.store(true, Ordering::Relaxed);
        }
        Ok(Self {
            window: window.to_owned(),
            id: id.to_owned(),
            cancelled,
        })
    }
}

impl Drop for RequestGuard {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
        if let Ok(mut requests) = REQUESTS.get_or_init(Mutex::default).lock() {
            if requests
                .active
                .get(&self.window)
                .is_some_and(|(id, _)| id == &self.id)
            {
                requests.active.remove(&self.window);
            }
        }
    }
}

pub fn cancel(window: &str, request_id: &str) -> Result<(), AppError> {
    if request_id.is_empty() || request_id.len() > 128 {
        return Err(AppError::Validation(
            "invalid archive scan request id".to_owned(),
        ));
    }
    let mut requests = REQUESTS
        .get_or_init(Mutex::default)
        .lock()
        .map_err(|_| AppError::Conflict("archive scan state unavailable".to_owned()))?;
    if let Some((id, cancelled)) = requests.active.get(window) {
        if id == request_id {
            cancelled.store(true, Ordering::Relaxed);
            return Ok(());
        }
    }
    // The synchronous cancel command can arrive before the async scan starts.
    // Keep a bounded set of these IDs so a departed page cannot start late.
    if !requests
        .early_cancellations
        .iter()
        .any(|(owner, id)| owner == window && id == request_id)
    {
        if requests.early_cancellations.len() == 64 {
            requests.early_cancellations.pop_front();
        }
        requests
            .early_cancellations
            .push_back((window.to_owned(), request_id.to_owned()));
    }
    Ok(())
}

struct RootGroup {
    directory: String,
    project_ids: Vec<String>,
}

#[derive(Clone)]
struct Snapshot {
    state: State,
    main_images: u64,
    avatars: u64,
    bytes: u64,
    message: Option<String>,
}

type RootScans = Arc<AsyncMutex<BTreeMap<PathBuf, Arc<OnceCell<Snapshot>>>>>;

impl Snapshot {
    fn failed(state: State, message: impl Into<String>) -> Self {
        Self {
            state,
            main_images: 0,
            avatars: 0,
            bytes: 0,
            message: Some(message.into()),
        }
    }

    fn for_project(&self, project_id: String, scanned_at: &str) -> ProjectArchiveUsage {
        let complete = matches!(self.state, State::Ready);
        ProjectArchiveUsage {
            project_id,
            state: self.state,
            main_image_count: complete.then_some(self.main_images),
            avatar_image_count: complete.then_some(self.avatars),
            total_bytes: complete.then_some(self.bytes),
            scanned_at: scanned_at.to_owned(),
            message: self.message.clone(),
        }
    }
}

/// Shares each directory scan within this request and reports completed roots
/// immediately. No source files, capture records, or archive files are changed.
pub async fn list(
    pool: &SqlitePool,
    window: &str,
    request_id: &str,
    mut report: impl FnMut(ProjectArchiveUsage) + Send,
) -> Result<Vec<ProjectArchiveUsage>, AppError> {
    let request = RequestGuard::begin(window, request_id)?;
    if request.cancelled.load(Ordering::Relaxed) {
        return Ok(Vec::new());
    }
    let projects: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT id, destination_directory FROM projects ORDER BY id")
            .fetch_all(pool)
            .await?;
    let mut groups = BTreeMap::<String, RootGroup>::new();
    let mut results = Vec::new();
    for (project_id, directory) in projects {
        if request.cancelled.load(Ordering::Relaxed) {
            return Ok(results);
        }
        let directory = directory
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty());
        if let Some(directory) = directory {
            groups
                // Only identical configured strings can share preparation:
                // resolving `..` lexically can cross a symlink or junction.
                .entry(directory.to_owned())
                .or_insert_with(|| RootGroup {
                    directory: directory.to_owned(),
                    project_ids: Vec::new(),
                })
                .project_ids
                .push(project_id);
        } else {
            let result = Snapshot::failed(State::Unconfigured, "尚未设置归档目录")
                .for_project(project_id, &timestamp());
            report(result.clone());
            results.push(result);
        }
    }

    let root_scans = RootScans::default();
    let mut pending = VecDeque::from_iter(groups.into_values());
    let mut running = JoinSet::new();
    loop {
        if request.cancelled.load(Ordering::Relaxed) {
            break;
        }
        while running.len() < SCAN_CONCURRENCY && !request.cancelled.load(Ordering::Relaxed) {
            let Some(group) = pending.pop_front() else {
                break;
            };
            let cancelled = request.cancelled.clone();
            let root_scans = root_scans.clone();
            running.spawn(async move {
                let snapshot = scan(&group.directory, cancelled, root_scans).await;
                (group.project_ids, snapshot)
            });
        }
        let next = tokio::select! {
            next = running.join_next() => next,
            _ = tokio::time::sleep(Duration::from_millis(250)) => continue,
        };
        let Some(result) = next else {
            break;
        };
        if request.cancelled.load(Ordering::Relaxed) {
            break;
        }
        let (project_ids, snapshot) = result.map_err(|error| {
            AppError::Io(io::Error::other(format!(
                "archive usage task failed: {error}"
            )))
        })?;
        let scanned_at = timestamp();
        for project_id in project_ids {
            if request.cancelled.load(Ordering::Relaxed) {
                break;
            }
            let result = snapshot.for_project(project_id, &scanned_at);
            report(result.clone());
            results.push(result);
        }
    }
    Ok(results)
}

struct ScanProgress {
    started: Instant,
    last_progress_ms: AtomicU64,
    opened: AtomicBool,
    stopped: AtomicBool,
    request_cancelled: Arc<AtomicBool>,
}

impl ScanProgress {
    fn advance(&self) {
        self.last_progress_ms
            .store(self.started.elapsed().as_millis() as u64, Ordering::Relaxed);
    }

    fn cancelled(&self) -> bool {
        self.stopped.load(Ordering::Relaxed) || self.request_cancelled.load(Ordering::Relaxed)
    }
}

async fn scan(
    directory: &str,
    request_cancelled: Arc<AtomicBool>,
    root_scans: RootScans,
) -> Snapshot {
    let directory = PathBuf::from(directory);
    let root = match limited_io(request_cancelled.clone(), move |progress| {
        prepare_root(&directory, progress)
    })
    .await
    {
        Ok(root) => root,
        Err(snapshot) => return snapshot,
    };
    let cell = {
        let mut scans = root_scans.lock().await;
        scans
            .entry(root.clone())
            .or_insert_with(|| Arc::new(OnceCell::new()))
            .clone()
    };
    // Both successful and incomplete scans are shared for this request. Waiting
    // on an alias must not hold the map lock or an OS I/O slot.
    cell.get_or_init(|| async move {
        match limited_io(request_cancelled, move |progress| {
            Ok(scan_directory(&root, progress))
        })
        .await
        {
            Ok(snapshot) | Err(snapshot) => snapshot,
        }
    })
    .await
    .clone()
}

async fn limited_io<T, F>(request_cancelled: Arc<AtomicBool>, operation: F) -> Result<T, Snapshot>
where
    T: Send + 'static,
    F: FnOnce(&ScanProgress) -> Result<T, Snapshot> + Send + 'static,
{
    if request_cancelled.load(Ordering::Relaxed) {
        return Err(Snapshot::failed(State::Partial, "目录统计已取消"));
    }
    let slots = SCAN_SLOTS.get_or_init(|| Arc::new(Semaphore::new(SCAN_CONCURRENCY)));
    let permit = match tokio::time::timeout(SCAN_TIMEOUT, slots.clone().acquire_owned()).await {
        Ok(Ok(permit)) => permit,
        _ => {
            return Err(Snapshot::failed(
                State::Error,
                "目录统计繁忙，请重新进入首页重试",
            ))
        }
    };
    let progress = Arc::new(ScanProgress {
        started: Instant::now(),
        last_progress_ms: AtomicU64::new(0),
        opened: AtomicBool::new(false),
        stopped: AtomicBool::new(false),
        request_cancelled,
    });
    let worker_progress = progress.clone();
    let mut worker = tokio::task::spawn_blocking(move || {
        // An OS request to an offline share may outlive the timeout. Retain its
        // slot until it returns so repeated visits cannot spawn unbounded I/O.
        let _permit = permit;
        if worker_progress.cancelled() {
            return Err(Snapshot::failed(State::Partial, "目录统计已取消"));
        }
        operation(&worker_progress)
    });
    loop {
        tokio::select! {
            result = &mut worker => return if progress.cancelled() {
                Err(Snapshot::failed(State::Partial, "目录统计已取消"))
            } else {
                match result {
                    Ok(result) => result,
                    Err(error) => Err(Snapshot::failed(State::Error, format!("目录统计失败：{error}"))),
                }
            },
            _ = tokio::time::sleep(Duration::from_millis(250)) => {
                if progress.cancelled() {
                    return Err(Snapshot::failed(State::Partial, "目录统计已取消"));
                }
                let idle_ms = progress.started.elapsed().as_millis()
                    .saturating_sub(progress.last_progress_ms.load(Ordering::Relaxed) as u128);
                if idle_ms >= SCAN_TIMEOUT.as_millis() {
                    progress.stopped.store(true, Ordering::Relaxed);
                    return Err(if progress.opened.load(Ordering::Relaxed) {
                        Snapshot::failed(State::Partial, "目录读取长时间无响应，未取得完整结果")
                    } else {
                        Snapshot::failed(State::Unavailable, "归档目录连接超时")
                    });
                }
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ImageRole {
    Unknown,
    Main,
    Avatar,
}

fn check_root(directory: &Path, progress: &ScanProgress) -> Result<(), Snapshot> {
    if !directory.is_absolute() {
        return Err(Snapshot::failed(State::Error, "归档目录必须为绝对路径"));
    }
    match fs::symlink_metadata(directory) {
        Ok(metadata) if is_link(&metadata) => {
            return Err(Snapshot::failed(
                State::Error,
                "归档根目录是链接，未跟随统计",
            ))
        }
        Ok(_) => progress.advance(),
        Err(error) => return Err(root_error(&error)),
    }
    if progress.cancelled() {
        return Err(Snapshot::failed(State::Partial, "目录统计已取消"));
    }
    Ok(())
}

fn prepare_root(directory: &Path, progress: &ScanProgress) -> Result<PathBuf, Snapshot> {
    check_root(directory, progress)?;
    let root = fs::canonicalize(directory).map_err(|error| root_error(&error))?;
    progress.advance();
    Ok(root)
}

fn scan_directory(root: &Path, progress: &ScanProgress) -> Snapshot {
    // Preparation and enumeration run separately; reject a root replaced by a
    // link while it was waiting for the shared scan or an I/O slot.
    if let Err(snapshot) = check_root(root, progress) {
        return snapshot;
    }
    let mut result = Snapshot {
        state: State::Ready,
        main_images: 0,
        avatars: 0,
        bytes: 0,
        message: None,
    };
    let mut pending = vec![(root.to_path_buf(), ImageRole::Unknown)];
    while let Some((directory, role)) = pending.pop() {
        if progress.cancelled() {
            break;
        }
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if directory.as_path() == root => return root_error(&error),
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(_) => {
                result.state = State::Partial;
                continue;
            }
        };
        progress.opened.store(true, Ordering::Relaxed);
        progress.advance();
        for entry in entries {
            if progress.cancelled() {
                break;
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    result.state = State::Partial;
                    continue;
                }
            };
            progress.advance();
            let path = entry.path();
            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(_) => {
                    result.state = State::Partial;
                    continue;
                }
            };
            progress.advance();
            if is_link(&metadata) {
                continue;
            }
            if metadata.is_dir() {
                if !skip_directory(&path) {
                    let child_role = match directory_role(&path) {
                        ImageRole::Unknown => role,
                        known => known,
                    };
                    pending.push((path, child_role));
                }
            } else if metadata.is_file()
                && metadata.len() > 0
                && capture_service::is_supported_image(&path)
                && !is_temporary(&path)
            {
                result.bytes += metadata.len();
                if role == ImageRole::Avatar
                    || (role == ImageRole::Unknown && is_avatar_name(&path))
                {
                    result.avatars += 1;
                } else {
                    result.main_images += 1;
                }
            }
        }
    }
    if progress.cancelled() {
        return Snapshot::failed(State::Partial, "目录统计已取消");
    }
    if matches!(result.state, State::Partial) {
        result.message = Some("部分目录或文件无法读取，未取得完整统计".to_owned());
    }
    result
}

fn directory_role(path: &Path) -> ImageRole {
    match path.file_name().and_then(|name| name.to_str()) {
        Some("头像图") => ImageRole::Avatar,
        Some("人物图" | "人物图（原图）" | "人物图（未识别）" | "游戏截图" | "收藏图") => {
            ImageRole::Main
        }
        _ => ImageRole::Unknown,
    }
}

fn is_avatar_name(path: &Path) -> bool {
    let stem = path.file_stem().and_then(|v| v.to_str()).unwrap_or("");
    let stem = stem.split(" - repaired-").next().unwrap_or(stem);
    super::file_relink::split_avatar_stem(stem).1
}

fn skip_directory(path: &Path) -> bool {
    let name = path.file_name().and_then(|v| v.to_str()).unwrap_or("");
    name.starts_with(['.', '@', '#'])
        || name.eq_ignore_ascii_case("$RECYCLE.BIN")
        || name.eq_ignore_ascii_case("System Volume Information")
}

fn is_temporary(path: &Path) -> bool {
    let name = path.file_name().and_then(|v| v.to_str()).unwrap_or("");
    name.starts_with(".scenevault-") || name.starts_with(".scene-vault-")
}

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    metadata.file_type().is_symlink()
}

fn root_error(error: &io::Error) -> Snapshot {
    #[cfg(windows)]
    if matches!(
        error.raw_os_error(),
        Some(53 | 59 | 64 | 67 | 121 | 1201 | 1231 | 1232)
    ) {
        return Snapshot::failed(State::Unavailable, format!("无法连接归档目录：{error}"));
    }
    let state = match error.raw_os_error() {
        _ if error.kind() == io::ErrorKind::NotFound => State::Missing,
        _ if matches!(
            error.kind(),
            io::ErrorKind::TimedOut
                | io::ErrorKind::NotConnected
                | io::ErrorKind::ConnectionAborted
        ) =>
        {
            State::Unavailable
        }
        _ => State::Error,
    };
    Snapshot::failed(state, format!("无法读取归档目录：{error}"))
}

fn timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

#[cfg(test)]
#[path = "../../tests/unit/project_archive_usage_service.rs"]
mod tests;
