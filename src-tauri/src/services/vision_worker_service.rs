use std::{
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        OnceLock,
    },
    time::{Duration, Instant},
};

use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStderr, ChildStdin, ChildStdout, Command},
    sync::{Mutex, Notify},
};

use crate::{
    error::AppError,
    models::vision::{VisionProcessData, VisionSettings},
    services::{log_service, vision_engine_service::EngineResponse},
};

use super::vision_engine_service::ProgressCallback;

const PROTOCOL_VERSION: i64 = 1;
const START_TIMEOUT: Duration = Duration::from_secs(15);
const START_RETRY_DELAY: Duration = Duration::from_secs(30);
const STOP_TIMEOUT: Duration = Duration::from_secs(2);
const PROGRESS_PREFIX: &str = "SVPROGRESS ";
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

static WORKER_MANAGER: OnceLock<Mutex<VisionWorkerManager>> = OnceLock::new();
static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);
static SHUTDOWN_NOTIFY: OnceLock<Notify> = OnceLock::new();

#[derive(Debug)]
pub(super) struct WorkerInvocation {
    pub response: EngineResponse,
    pub startup_ms: Option<f64>,
    pub total_ms: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkerSettingsFingerprint {
    python_executable_path: Option<String>,
    python_module_root: Option<String>,
    yunet_model_path: Option<String>,
    sface_model_path: Option<String>,
    recognizer: Option<String>,
    arcface_model_path: Option<String>,
    font_path: Option<String>,
}

impl From<&VisionSettings> for WorkerSettingsFingerprint {
    fn from(settings: &VisionSettings) -> Self {
        Self {
            python_executable_path: settings.python_executable_path.clone(),
            python_module_root: settings.python_module_root.clone(),
            yunet_model_path: settings.yunet_model_path.clone(),
            sface_model_path: settings.sface_model_path.clone(),
            recognizer: settings.recognizer.clone(),
            arcface_model_path: settings.arcface_model_path.clone(),
            font_path: settings.font_path.clone(),
        }
    }
}

#[derive(Default)]
struct VisionWorkerManager {
    fingerprint: Option<WorkerSettingsFingerprint>,
    worker: Option<WorkerProcess>,
    retry_after: Option<Instant>,
}

struct WorkerProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    stderr: BufReader<ChildStderr>,
    stderr_open: bool,
}

pub(super) async fn invoke(
    settings: &VisionSettings,
    request: &Value,
    request_id: &str,
    timeout: Duration,
    progress: &mut Option<ProgressCallback>,
) -> Result<WorkerInvocation, AppError> {
    let started = Instant::now();
    if is_shutdown_requested() {
        return Err(AppError::Vision(
            "Python worker shutdown is in progress".to_owned(),
        ));
    }
    let mut manager = tokio::time::timeout(timeout, manager().lock())
        .await
        .map_err(|_| AppError::Vision("Python worker request timed out".to_owned()))?;
    let remaining = remaining_timeout(started, timeout)?;
    manager
        .invoke(settings, request, request_id, remaining, progress)
        .await
}

pub async fn shutdown() {
    SHUTDOWN_REQUESTED.store(true, Ordering::SeqCst);
    shutdown_notify().notify_waiters();
    let mut manager = manager().lock().await;
    if let Some(worker) = manager.worker.take() {
        worker.stop("app_exit").await;
    }
}

pub(super) fn is_shutdown_requested() -> bool {
    SHUTDOWN_REQUESTED.load(Ordering::SeqCst)
}

#[cfg(all(test, target_os = "windows"))]
async fn reset_for_tests() {
    let mut manager = manager().lock().await;
    if let Some(worker) = manager.worker.take() {
        worker.stop("test_cleanup").await;
    }
    manager.fingerprint = None;
    manager.retry_after = None;
}

impl VisionWorkerManager {
    async fn invoke(
        &mut self,
        settings: &VisionSettings,
        request: &Value,
        request_id: &str,
        timeout: Duration,
        progress: &mut Option<ProgressCallback>,
    ) -> Result<WorkerInvocation, AppError> {
        let invocation_started = Instant::now();
        if is_shutdown_requested() {
            return Err(AppError::Vision(
                "Python worker shutdown is in progress".to_owned(),
            ));
        }
        let fingerprint = WorkerSettingsFingerprint::from(settings);
        if self.fingerprint.as_ref() != Some(&fingerprint) {
            if let Some(worker) = self.worker.take() {
                worker.stop("settings_changed").await;
            }
            self.fingerprint = Some(fingerprint);
            self.retry_after = None;
        }

        let mut startup_ms = None;
        if self.worker.is_none() {
            if self
                .retry_after
                .is_some_and(|retry_after| retry_after > Instant::now())
            {
                return Err(AppError::Vision(
                    "Python worker start is temporarily in cooldown".to_owned(),
                ));
            }
            let started = Instant::now();
            let startup_timeout =
                remaining_timeout(invocation_started, timeout)?.min(START_TIMEOUT);
            match WorkerProcess::start(settings, startup_timeout).await {
                Ok(worker) => {
                    let startup_elapsed_ms = elapsed_ms(started);
                    let pid = worker
                        .child
                        .id()
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "unknown".to_owned());
                    log_service::info(
                        "vision.worker",
                        format!("state=ready pid={pid} startup_ms={startup_elapsed_ms:.3}"),
                    );
                    self.worker = Some(worker);
                    startup_ms = Some(startup_elapsed_ms);
                    self.retry_after = None;
                }
                Err(error) => {
                    self.retry_after = Some(Instant::now() + START_RETRY_DELAY);
                    log_service::warn(
                        "vision.worker",
                        format!(
                            "state=start_failed retry_after_seconds={} error={}",
                            START_RETRY_DELAY.as_secs(),
                            sanitize_error(&error)
                        ),
                    );
                    return Err(error);
                }
            }
        }

        let remaining = remaining_timeout(invocation_started, timeout)?;
        let result = self
            .worker
            .as_mut()
            .expect("worker initialized above")
            .call(
                request,
                request_id,
                "processScreenshot",
                remaining,
                progress,
            )
            .await;
        match result {
            Ok(response) => Ok(WorkerInvocation {
                response,
                startup_ms,
                total_ms: elapsed_ms(invocation_started),
            }),
            Err(error) => {
                log_service::warn(
                    "vision.worker",
                    format!(
                        "state=request_failed request_id={request_id} error={}",
                        sanitize_error(&error)
                    ),
                );
                if let Some(worker) = self.worker.take() {
                    worker.stop("request_failed").await;
                }
                Err(error)
            }
        }
    }
}

impl WorkerProcess {
    async fn start(
        settings: &VisionSettings,
        handshake_timeout: Duration,
    ) -> Result<Self, AppError> {
        let started = Instant::now();
        let executable = settings
            .python_executable_path
            .as_deref()
            .ok_or_else(|| AppError::Vision("Python executable is not configured".to_owned()))?;
        let module_root = settings
            .python_module_root
            .as_deref()
            .ok_or_else(|| AppError::Vision("Python module root is not configured".to_owned()))?;
        log_service::info("vision.worker", "state=starting");
        let mut child = Command::new(executable)
            .arg("-m")
            .arg("scene_vault_ai")
            .arg("worker")
            .env("PYTHONPATH", module_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| AppError::Vision(format!("cannot start Python worker: {error}")))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| AppError::Vision("cannot open Python worker stdin".to_owned()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AppError::Vision("cannot open Python worker stdout".to_owned()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| AppError::Vision("cannot open Python worker stderr".to_owned()))?;
        let mut worker = Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            stderr: BufReader::new(stderr),
            stderr_open: true,
        };
        let request_id = format!("worker-health-{}", uuid::Uuid::new_v4());
        let request = json!({
            "protocolVersion": PROTOCOL_VERSION,
            "requestId": request_id,
            "action": "health",
            "payload": {}
        });
        let mut progress = None;
        let handshake_timeout = remaining_timeout(started, handshake_timeout)?;
        let response = worker
            .call(
                &request,
                &request_id,
                "health",
                handshake_timeout,
                &mut progress,
            )
            .await?;
        if !response.ok {
            return Err(AppError::Vision(
                "Python worker health handshake failed".to_owned(),
            ));
        }
        Ok(worker)
    }

    async fn call(
        &mut self,
        request: &Value,
        request_id: &str,
        expected_action: &str,
        timeout: Duration,
        progress: &mut Option<ProgressCallback>,
    ) -> Result<EngineResponse, AppError> {
        let started = Instant::now();
        if let Some(status) = self
            .child
            .try_wait()
            .map_err(|error| AppError::Vision(format!("cannot inspect Python worker: {error}")))?
        {
            return Err(AppError::Vision(format!(
                "Python worker exited before request with {status}"
            )));
        }
        let mut bytes = serde_json::to_vec(request)
            .map_err(|error| AppError::Vision(format!("cannot encode worker request: {error}")))?;
        bytes.push(b'\n');
        tokio::time::timeout(timeout, async {
            self.stdin.write_all(&bytes).await.map_err(|error| {
                AppError::Vision(format!("cannot write Python worker request: {error}"))
            })?;
            self.stdin.flush().await.map_err(|error| {
                AppError::Vision(format!("cannot flush Python worker request: {error}"))
            })
        })
        .await
        .map_err(|_| AppError::Vision("Python worker request timed out".to_owned()))??;

        let remaining = remaining_timeout(started, timeout)?;
        let response = self.read_response(request_id, remaining, progress).await?;
        validate_response(&response, request_id, expected_action)?;
        if expected_action == "processScreenshot" && response.ok {
            let data = response.data.as_ref().ok_or_else(|| {
                AppError::Vision("Python worker processing response has no data".to_owned())
            })?;
            serde_json::from_value::<VisionProcessData>(data.clone()).map_err(|error| {
                AppError::Vision(format!(
                    "Python worker returned invalid processing data: {error}"
                ))
            })?;
        }
        Ok(response)
    }

    async fn read_response(
        &mut self,
        request_id: &str,
        timeout: Duration,
        progress: &mut Option<ProgressCallback>,
    ) -> Result<EngineResponse, AppError> {
        if is_shutdown_requested() {
            return Err(AppError::Vision(
                "Python worker shutdown requested".to_owned(),
            ));
        }
        let mut stdout_line = Vec::new();
        let mut stderr_line = Vec::new();
        let timeout = tokio::time::sleep(timeout);
        tokio::pin!(timeout);

        loop {
            tokio::select! {
                biased;
                _ = shutdown_notify().notified() => {
                    return Err(AppError::Vision("Python worker shutdown requested".to_owned()));
                }
                _ = &mut timeout => {
                    return Err(AppError::Vision("Python worker request timed out".to_owned()));
                }
                result = self.stderr.read_until(b'\n', &mut stderr_line), if self.stderr_open => {
                    let read = result.map_err(|error| {
                        AppError::Vision(format!("cannot read Python worker stderr: {error}"))
                    })?;
                    if read == 0 {
                        self.stderr_open = false;
                        continue;
                    }
                    let line = String::from_utf8_lossy(&stderr_line)
                        .trim_end_matches(['\r', '\n'])
                        .to_owned();
                    stderr_line.clear();
                    if let Some(event) = parse_progress_line(&line) {
                        if event.request_id.as_deref().is_none_or(|value| value == request_id) {
                            if let Some(callback) = progress.as_mut() {
                                callback(&event.stage, event.percent);
                            }
                        }
                    } else if !line.trim().is_empty() {
                        log_service::debug(
                            "vision.python",
                            format!("stderr: {}", truncate(&line, 500)),
                        );
                    }
                }
                result = self.stdout.read_until(b'\n', &mut stdout_line) => {
                    let read = result.map_err(|error| {
                        AppError::Vision(format!("cannot read Python worker response: {error}"))
                    })?;
                    if read == 0 {
                        let status = self.child.try_wait().ok().flatten();
                        return Err(AppError::Vision(match status {
                            Some(status) => format!("Python worker exited with {status}"),
                            None => "Python worker closed stdout unexpectedly".to_owned(),
                        }));
                    }
                    if stdout_line.len() > MAX_RESPONSE_BYTES {
                        return Err(AppError::Vision(
                            "Python worker response exceeds the size limit".to_owned(),
                        ));
                    }
                    return serde_json::from_slice(&stdout_line).map_err(|error| {
                        AppError::Vision(format!("Python worker returned invalid JSON: {error}"))
                    });
                }
            }
        }
    }

    async fn stop(mut self, reason: &str) {
        drop(self.stdin);
        let status = match tokio::time::timeout(STOP_TIMEOUT, self.child.wait()).await {
            Ok(Ok(status)) => Some(status),
            _ => {
                let _ = self.child.kill().await;
                self.child.try_wait().ok().flatten()
            }
        };
        log_service::info(
            "vision.worker",
            format!(
                "state=stopped reason={reason} status={}",
                status
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "unknown".to_owned())
            ),
        );
    }
}

#[derive(Debug, PartialEq)]
struct ProgressEvent {
    request_id: Option<String>,
    stage: String,
    percent: f64,
}

fn parse_progress_line(line: &str) -> Option<ProgressEvent> {
    let payload = line.strip_prefix(PROGRESS_PREFIX)?;
    let value: Value = serde_json::from_str(payload).ok()?;
    let stage = value.get("stage")?.as_str()?.trim();
    if stage.is_empty() {
        return None;
    }
    let percent = value.get("percent")?.as_f64()?;
    if !percent.is_finite() || !(0.0..=100.0).contains(&percent) {
        return None;
    }
    let request_id = value
        .get("requestId")
        .and_then(Value::as_str)
        .map(str::to_owned);
    Some(ProgressEvent {
        request_id,
        stage: stage.to_owned(),
        percent,
    })
}

fn validate_response(
    response: &EngineResponse,
    request_id: &str,
    expected_action: &str,
) -> Result<(), AppError> {
    if response.protocol_version != PROTOCOL_VERSION {
        return Err(AppError::Vision(format!(
            "unsupported Python worker protocol version {}",
            response.protocol_version
        )));
    }
    if response.request_id.as_deref() != Some(request_id) {
        return Err(AppError::Vision(
            "Python worker response request ID mismatch".to_owned(),
        ));
    }
    if response.action != expected_action {
        return Err(AppError::Vision(format!(
            "Python worker response action mismatch: expected {expected_action}, received {}",
            response.action
        )));
    }
    Ok(())
}

fn manager() -> &'static Mutex<VisionWorkerManager> {
    WORKER_MANAGER.get_or_init(|| Mutex::new(VisionWorkerManager::default()))
}

fn shutdown_notify() -> &'static Notify {
    SHUTDOWN_NOTIFY.get_or_init(Notify::new)
}

fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1_000.0
}

fn remaining_timeout(started: Instant, total: Duration) -> Result<Duration, AppError> {
    total
        .checked_sub(started.elapsed())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| AppError::Vision("Python worker request timed out".to_owned()))
}

fn truncate(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn sanitize_error(error: &AppError) -> String {
    truncate(&error.to_string().replace(['\r', '\n'], " | "), 500)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_fingerprint_changes_for_every_worker_relevant_setting() {
        let base = VisionSettings::default();
        let base_fingerprint = WorkerSettingsFingerprint::from(&base);
        for changed in [
            VisionSettings {
                python_executable_path: Some("python.exe".to_owned()),
                ..Default::default()
            },
            VisionSettings {
                python_module_root: Some("C:\\engine".to_owned()),
                ..Default::default()
            },
            VisionSettings {
                yunet_model_path: Some("C:\\models\\yunet.onnx".to_owned()),
                ..Default::default()
            },
            VisionSettings {
                sface_model_path: Some("C:\\models\\sface.onnx".to_owned()),
                ..Default::default()
            },
            VisionSettings {
                recognizer: Some("arcface".to_owned()),
                ..Default::default()
            },
            VisionSettings {
                arcface_model_path: Some("C:\\models\\arcface.onnx".to_owned()),
                ..Default::default()
            },
            VisionSettings {
                font_path: Some("C:\\fonts\\cjk.ttf".to_owned()),
                ..Default::default()
            },
        ] {
            assert_ne!(WorkerSettingsFingerprint::from(&changed), base_fingerprint);
        }
    }

    #[test]
    fn parses_request_correlated_progress_and_rejects_invalid_values() {
        assert_eq!(
            parse_progress_line(
                r#"SVPROGRESS {"requestId":"req-1","stage":"detect_face","percent":20}"#
            ),
            Some(ProgressEvent {
                request_id: Some("req-1".to_owned()),
                stage: "detect_face".to_owned(),
                percent: 20.0,
            })
        );
        assert!(parse_progress_line(r#"SVPROGRESS {"stage":"","percent":20}"#).is_none());
        assert!(parse_progress_line(r#"SVPROGRESS {"stage":"read","percent":101}"#).is_none());
        assert!(parse_progress_line("Python warning").is_none());
    }

    #[test]
    fn worker_response_requires_exact_request_and_action() {
        let response = EngineResponse {
            protocol_version: PROTOCOL_VERSION,
            request_id: Some("req-1".to_owned()),
            ok: true,
            action: "processScreenshot".to_owned(),
            data: Some(json!({})),
            error: None,
        };

        assert!(validate_response(&response, "req-1", "processScreenshot").is_ok());
        assert!(validate_response(&response, "req-2", "processScreenshot").is_err());
        assert!(validate_response(&response, "req-1", "health").is_err());
    }

    #[test]
    fn successful_worker_processing_data_must_match_the_rust_contract() {
        let invalid = json!({"warnings": "not-an-array"});
        let error = serde_json::from_value::<VisionProcessData>(invalid)
            .expect_err("warnings must remain a string array");

        assert!(error.to_string().contains("sequence"));
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn windows_worker_reuses_process_and_restarts_after_settings_change() {
        let repository_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("src-tauri should have a repository parent");
        let python = repository_root
            .join("python")
            .join(".venv")
            .join("Scripts")
            .join("python.exe");
        if !python.is_file() {
            return;
        }
        let module_root = repository_root.join("python").join("src");
        let settings = VisionSettings {
            python_executable_path: Some(python.to_string_lossy().into_owned()),
            python_module_root: Some(module_root.to_string_lossy().into_owned()),
            ..Default::default()
        };
        let temporary = tempfile::tempdir().expect("temporary directory should be created");
        let mut invocations = Vec::new();
        for index in 0..2 {
            let input = temporary.path().join(format!("输入-{index}.png"));
            let output = temporary.path().join(format!("输出-{index}.png"));
            image::RgbImage::from_pixel(32, 32, image::Rgb([20, 30, 40]))
                .save(&input)
                .expect("test image should be written");
            let request_id = format!("rust-worker-{index}");
            let request = json!({
                "protocolVersion": PROTOCOL_VERSION,
                "requestId": request_id,
                "action": "processScreenshot",
                "payload": {
                    "inputPath": input.to_string_lossy(),
                    "annotatedOutputPath": output.to_string_lossy(),
                    "avatarOutputPath": Value::Null,
                    "characterName": "星见",
                    "detectFace": false,
                    "annotate": true,
                    "cropAvatar": false
                }
            });
            let mut progress = None;
            let invocation = invoke(
                &settings,
                &request,
                &request_id,
                Duration::from_secs(15),
                &mut progress,
            )
            .await;
            invocations.push((invocation, output));
        }
        let changed_settings = VisionSettings {
            font_path: Some(
                repository_root
                    .join("changed-font.ttf")
                    .to_string_lossy()
                    .into_owned(),
            ),
            ..settings.clone()
        };
        let input = temporary.path().join("输入-设置变化.png");
        let output = temporary.path().join("输出-设置变化.png");
        image::RgbImage::from_pixel(32, 32, image::Rgb([20, 30, 40]))
            .save(&input)
            .expect("test image should be written");
        let request_id = "rust-worker-settings-changed";
        let request = json!({
            "protocolVersion": PROTOCOL_VERSION,
            "requestId": request_id,
            "action": "processScreenshot",
            "payload": {
                "inputPath": input.to_string_lossy(),
                "annotatedOutputPath": output.to_string_lossy(),
                "avatarOutputPath": Value::Null,
                "characterName": "星见",
                "detectFace": false,
                "annotate": true,
                "cropAvatar": false
            }
        });
        let mut progress = None;
        let restarted = invoke(
            &changed_settings,
            &request,
            request_id,
            Duration::from_secs(15),
            &mut progress,
        )
        .await;

        let fake_module_root = temporary.path().join("fake-engine");
        let fake_package = fake_module_root.join("scene_vault_ai");
        std::fs::create_dir_all(&fake_package).expect("fake Python package should be created");
        std::fs::write(fake_package.join("__init__.py"), "")
            .expect("fake package marker should be written");
        std::fs::write(
            fake_package.join("__main__.py"),
            r#"import json
import sys
import time

def write_response(request, action, data):
    response = {
        "protocolVersion": 1,
        "requestId": request.get("requestId"),
        "ok": True,
        "action": action,
        "data": data,
        "error": None,
    }
    sys.stdout.buffer.write(json.dumps(response).encode("utf-8") + b"\n")
    sys.stdout.buffer.flush()

if sys.argv[1] == "worker":
    health = json.loads(sys.stdin.buffer.readline().decode("utf-8"))
    write_response(health, "health", {"status": "ready"})
    worker_request = json.loads(sys.stdin.buffer.readline().decode("utf-8"))
    if worker_request.get("payload", {}).get("hang"):
        time.sleep(10)
    raise SystemExit(3)

request = json.loads(sys.stdin.buffer.read().decode("utf-8"))
if "requestId" in request:
    raise SystemExit("legacy one-shot protocol rejects requestId")
write_response(request, "processScreenshot", {
    "annotatedPath": None,
    "avatarPath": None,
    "faceBox": None,
    "warnings": [],
})
"#,
        )
        .expect("fake Python entrypoint should be written");
        let fallback_settings = VisionSettings {
            python_executable_path: Some(python.to_string_lossy().into_owned()),
            python_module_root: Some(fake_module_root.to_string_lossy().into_owned()),
            ..Default::default()
        };
        let fallback_request = json!({
            "protocolVersion": PROTOCOL_VERSION,
            "action": "processScreenshot",
            "payload": {}
        });
        let fallback = crate::services::vision_engine_service::invoke_process(
            &fallback_settings,
            fallback_request.clone(),
            None,
        )
        .await;
        let fallback_after_restart = crate::services::vision_engine_service::invoke_process(
            &fallback_settings,
            fallback_request.clone(),
            None,
        )
        .await;
        let forced_oneshot = crate::services::vision_engine_service::invoke_process_in_mode(
            &fallback_settings,
            fallback_request.clone(),
            None,
            crate::services::vision_engine_service::VisionExecutionMode::Oneshot,
        )
        .await;
        let timeout_request_id = "rust-worker-timeout";
        let timeout_request = json!({
            "protocolVersion": PROTOCOL_VERSION,
            "requestId": timeout_request_id,
            "action": "processScreenshot",
            "payload": {"hang": true}
        });
        let timeout_started = Instant::now();
        let mut timeout_progress = None;
        let timed_out = invoke(
            &fallback_settings,
            &timeout_request,
            timeout_request_id,
            Duration::from_millis(100),
            &mut timeout_progress,
        )
        .await;
        let timeout_elapsed = timeout_started.elapsed();
        reset_for_tests().await;

        let (first, first_output) = &invocations[0];
        let (second, second_output) = &invocations[1];
        let first = first.as_ref().expect("first worker request should succeed");
        let second = second
            .as_ref()
            .expect("second worker request should succeed");
        let restarted = restarted.expect("request after settings change should succeed");
        let fallback = fallback.expect("one-shot fallback should preserve the request");
        let fallback_after_restart = fallback_after_restart
            .expect("next request should restart the worker and retain fallback");
        let forced_oneshot = forced_oneshot.expect("forced one-shot mode should use legacy IPC");
        assert!(first.response.ok);
        assert!(second.response.ok);
        assert!(restarted.response.ok);
        assert!(first.startup_ms.is_some());
        assert!(second.startup_ms.is_none());
        assert!(restarted.startup_ms.is_some());
        assert!(first_output.is_file());
        assert!(second_output.is_file());
        assert!(output.is_file());
        assert!(fallback.warnings.is_empty());
        assert!(fallback_after_restart.warnings.is_empty());
        assert!(forced_oneshot.warnings.is_empty());
        assert!(timed_out
            .expect_err("hanging worker request should time out")
            .to_string()
            .contains("timed out"));
        assert!(timeout_elapsed < Duration::from_secs(4));
    }
}
