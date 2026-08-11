use std::{
    path::Path,
    process::Stdio,
    time::{Duration, Instant},
};

use serde::Deserialize;
use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt},
    process::Command,
    task::JoinHandle,
};

use crate::{
    error::AppError,
    models::vision::{ProcessingSettings, VisionHealth, VisionProcessData, VisionSettings},
    services::{log_service, vision_settings_service, vision_worker_service},
};

const PROTOCOL_VERSION: i64 = 1;
const PROCESS_TIMEOUT: Duration = Duration::from_secs(120);
const HEALTH_TIMEOUT: Duration = Duration::from_secs(15);
const PROGRESS_PREFIX: &str = "SVPROGRESS ";

/// Receives one processing-stage update (stage name and rough percent).
pub type ProgressCallback = Box<dyn FnMut(&str, f64) + Send>;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct EngineResponse {
    pub(super) protocol_version: i64,
    #[serde(default)]
    pub(super) request_id: Option<String>,
    pub(super) ok: bool,
    pub(super) action: String,
    pub(super) data: Option<Value>,
    pub(super) error: Option<EngineError>,
}

#[derive(Debug, Deserialize)]
pub(super) struct EngineError {
    pub(super) code: String,
    pub(super) message: String,
}

#[derive(Debug, Clone, Copy)]
struct InvocationMetrics {
    spawn_ms: f64,
    total_ms: f64,
}

#[derive(Debug, Clone, Copy)]
enum ProcessMode {
    Worker,
    Oneshot,
    OneshotFallback,
}

impl ProcessMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Worker => "worker",
            Self::Oneshot => "oneshot",
            Self::OneshotFallback => "oneshot_fallback",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum VisionExecutionMode {
    Worker,
    Oneshot,
}

impl VisionExecutionMode {
    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "worker" => Some(Self::Worker),
            "oneshot" | "one-shot" => Some(Self::Oneshot),
            _ => None,
        }
    }

    fn configured() -> Self {
        match std::env::var("SCENE_VAULT_VISION_MODE") {
            Ok(value) => match Self::parse(&value) {
                Some(mode) => mode,
                None => {
                    log_service::warn(
                        "vision.worker",
                        format!(
                            "state=invalid_mode value={} fallback=worker",
                            truncate(value.trim(), 100)
                        ),
                    );
                    Self::Worker
                }
            },
            Err(_) => Self::Worker,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ProcessInvocationMetrics {
    mode: ProcessMode,
    spawn_ms: Option<f64>,
    worker_startup_ms: Option<f64>,
    worker_attempt_ms: f64,
    invoke_ms: f64,
}

pub async fn health(settings: &VisionSettings) -> Result<VisionHealth, AppError> {
    if !vision_settings_service::is_configured(settings) {
        return Ok(VisionHealth {
            status: "unconfigured".to_owned(),
            engine_version: None,
            python_version: None,
            process_screenshot_available: false,
            error_message: Some("Python、模块目录或 YuNet 模型尚未配置".to_owned()),
        });
    }
    let response = invoke(settings, "health", None, HEALTH_TIMEOUT).await?;
    ensure_protocol(&response, "health")?;
    if !response.ok {
        return Err(response_error(response));
    }
    let data = response
        .data
        .ok_or_else(|| AppError::Vision("health response has no data".to_owned()))?;
    let capabilities = data
        .get("capabilities")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(VisionHealth {
        status: data
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("degraded")
            .to_owned(),
        engine_version: data
            .get("engineVersion")
            .and_then(Value::as_str)
            .map(str::to_owned),
        python_version: data
            .get("pythonVersion")
            .and_then(Value::as_str)
            .map(str::to_owned),
        process_screenshot_available: capabilities
            .iter()
            .any(|value| value.as_str() == Some("processScreenshot")),
        error_message: None,
    })
}

pub async fn process_screenshot(
    settings: &VisionSettings,
    input_path: &Path,
    annotated_output_path: &Path,
    avatar_output_path: &Path,
    character_name: &str,
    processing_settings: &ProcessingSettings,
    progress: Option<ProgressCallback>,
) -> Result<VisionProcessData, AppError> {
    if !vision_settings_service::is_configured(settings) {
        return Err(AppError::Vision(
            "vision engine is not configured".to_owned(),
        ));
    }
    validate_python_path(input_path, "Python input")?;
    validate_python_path(annotated_output_path, "annotated output")?;
    validate_python_path(avatar_output_path, "avatar output")?;
    let payload = processing_payload(
        settings,
        processing_settings,
        json!({
            "inputPath": input_path.to_string_lossy(),
            "annotatedOutputPath": annotated_output_path.to_string_lossy(),
            "avatarOutputPath": avatar_output_path.to_string_lossy(),
            "characterName": character_name,
            "detectFace": true,
            "annotate": true,
            "cropAvatar": true,
            "yunetModelPath": settings.yunet_model_path,
            "sfaceModelPath": settings.sface_model_path,
            "recognizer": settings.recognizer,
            "arcfaceModelPath": settings.arcface_model_path
        }),
    );
    let request = json!({
        "protocolVersion": PROTOCOL_VERSION,
        "action": "processScreenshot",
        "payload": payload
    });
    invoke_process(settings, request, progress).await
}

/// Feature-only extraction used by the pre-label pass: detects the primary
/// face and returns its SFace vector without writing any output files.
pub async fn extract_face_feature(
    settings: &VisionSettings,
    input_path: &Path,
    processing_settings: &ProcessingSettings,
    progress: Option<ProgressCallback>,
) -> Result<VisionProcessData, AppError> {
    if !vision_settings_service::is_configured(settings) {
        return Err(AppError::Vision(
            "vision engine is not configured".to_owned(),
        ));
    }
    validate_python_path(input_path, "Python input")?;
    let payload = processing_payload(
        settings,
        processing_settings,
        json!({
            "inputPath": input_path.to_string_lossy(),
            "annotatedOutputPath": Value::Null,
            "avatarOutputPath": Value::Null,
            "detectFace": true,
            "annotate": false,
            "cropAvatar": false,
            "yunetModelPath": settings.yunet_model_path,
            "sfaceModelPath": settings.sface_model_path,
            "recognizer": settings.recognizer,
            "arcfaceModelPath": settings.arcface_model_path
        }),
    );
    let request = json!({
        "protocolVersion": PROTOCOL_VERSION,
        "action": "processScreenshot",
        "payload": payload
    });
    invoke_process(settings, request, progress).await
}

/// Merges the optional detection/annotation/crop overrides into the base
/// payload. Unset fields are omitted so the Python side keeps its defaults.
fn processing_payload(
    settings: &VisionSettings,
    processing: &ProcessingSettings,
    base: Value,
) -> Value {
    let mut payload = base;
    if let Some(detection) = &processing.detection {
        let mut object = serde_json::Map::new();
        insert_number(&mut object, "scoreThreshold", detection.score_threshold);
        insert_number(&mut object, "nmsThreshold", detection.nms_threshold);
        if let Some(top_k) = detection.top_k {
            object.insert("topK".to_owned(), json!(top_k));
        }
        insert_number(&mut object, "areaWeight", detection.area_weight);
        insert_number(&mut object, "confidenceWeight", detection.confidence_weight);
        insert_number(&mut object, "centerWeight", detection.center_weight);
        insert_number(&mut object, "sharpnessWeight", detection.sharpness_weight);
        insert_number(
            &mut object,
            "edgePenaltyWeight",
            detection.edge_penalty_weight,
        );
        insert_number(&mut object, "edgeMarginRatio", detection.edge_margin_ratio);
        insert_number(&mut object, "minSharpness", detection.min_sharpness);
        insert_number(
            &mut object,
            "blurPenaltyWeight",
            detection.blur_penalty_weight,
        );
        payload["detection"] = Value::Object(object);
    }
    if let Some(annotation) = &processing.annotation {
        let mut object = serde_json::Map::new();
        if let Some(color) = annotation.text_color {
            object.insert("textColor".to_owned(), json!(color));
        }
        if let Some(color) = annotation.stroke_color {
            object.insert("strokeColor".to_owned(), json!(color));
        }
        if let Some(stroke_width) = annotation.stroke_width {
            object.insert("strokeWidth".to_owned(), json!(stroke_width));
        }
        if let Some(padding) = annotation.padding {
            object.insert("padding".to_owned(), json!(padding));
        }
        if let Some(face_box_expansion) = annotation.face_box_expansion {
            object.insert("faceBoxExpansion".to_owned(), json!(face_box_expansion));
        }
        if let Some(position) = &annotation.face_text_position {
            object.insert("faceTextPosition".to_owned(), json!(position));
        }
        if let Some(position) = &annotation.fallback_position {
            object.insert("fallbackPosition".to_owned(), json!(position));
        }
        insert_number(&mut object, "textOffsetX", annotation.text_offset_x);
        insert_number(&mut object, "textOffsetY", annotation.text_offset_y);
        if let Some(font_size) = annotation.font_size {
            object.insert("fontSize".to_owned(), json!(font_size));
        }
        if let Some(font_path) = settings.font_path.as_deref() {
            object.insert("fontPath".to_owned(), json!(font_path));
        }
        payload["annotation"] = Value::Object(object);
    } else if let Some(font_path) = settings.font_path.as_deref() {
        payload["annotation"] = json!({ "fontPath": font_path });
    }
    if let Some(crop) = &processing.crop {
        let mut object = serde_json::Map::new();
        if let Some(ratio) = &crop.aspect_ratio {
            object.insert("aspectRatio".to_owned(), json!(ratio));
        }
        insert_number(&mut object, "scaleX", crop.scale_x);
        insert_number(&mut object, "scaleTop", crop.scale_top);
        insert_number(&mut object, "scaleBottom", crop.scale_bottom);
        if let Some(min_size) = crop.min_size {
            object.insert("minSize".to_owned(), json!(min_size));
        }
        payload["crop"] = Value::Object(object);
    }
    payload
}

fn insert_number(object: &mut serde_json::Map<String, Value>, key: &str, value: Option<f64>) {
    if let Some(value) = value {
        object.insert(key.to_owned(), json!(value));
    }
}

pub(super) async fn invoke_process(
    settings: &VisionSettings,
    request: Value,
    progress: Option<ProgressCallback>,
) -> Result<VisionProcessData, AppError> {
    invoke_process_in_mode(
        settings,
        request,
        progress,
        VisionExecutionMode::configured(),
    )
    .await
}

pub(super) async fn invoke_process_in_mode(
    settings: &VisionSettings,
    mut request: Value,
    progress: Option<ProgressCallback>,
    execution_mode: VisionExecutionMode,
) -> Result<VisionProcessData, AppError> {
    let request_id = uuid::Uuid::new_v4().to_string();
    request["requestId"] = Value::String(request_id.clone());
    let request_started = Instant::now();
    let mut progress = progress;
    if execution_mode == VisionExecutionMode::Oneshot {
        log_service::debug(
            "vision.worker",
            format!("state=bypass request_id={request_id} mode=oneshot"),
        );
    }
    let worker_attempt_started = Instant::now();
    let worker_invocation = if execution_mode == VisionExecutionMode::Worker {
        Some(
            vision_worker_service::invoke(
                settings,
                &request,
                &request_id,
                PROCESS_TIMEOUT,
                &mut progress,
            )
            .await,
        )
    } else {
        None
    };
    let (response, metrics) = match worker_invocation {
        Some(Ok(invocation)) => (
            invocation.response,
            ProcessInvocationMetrics {
                mode: ProcessMode::Worker,
                spawn_ms: None,
                worker_startup_ms: invocation.startup_ms,
                worker_attempt_ms: elapsed_ms(worker_attempt_started),
                invoke_ms: invocation.total_ms,
            },
        ),
        Some(Err(worker_error)) => {
            if vision_worker_service::is_shutdown_requested() {
                log_request_failure(
                    &request_id,
                    request_started,
                    ProcessMode::Worker,
                    &worker_error,
                );
                return Err(worker_error);
            }
            let worker_attempt_ms = elapsed_ms(worker_attempt_started);
            log_service::warn(
                "vision.worker",
                format!(
                    "state=fallback request_id={request_id} mode=oneshot worker_attempt_ms={worker_attempt_ms:.3} error={}",
                    truncate(&worker_error.to_string().replace(['\r', '\n'], " | "), 500)
                ),
            );
            let remaining = remaining_timeout(request_started, PROCESS_TIMEOUT)?;
            let fallback_request = legacy_oneshot_request(&request);
            let stdin = serde_json::to_vec(&fallback_request).map_err(|error| {
                AppError::Vision(format!("cannot encode vision request: {error}"))
            })?;
            match invoke_with_progress(settings, "request", Some(&stdin), remaining, progress).await
            {
                Ok((response, invocation)) => (
                    response,
                    ProcessInvocationMetrics {
                        mode: ProcessMode::OneshotFallback,
                        spawn_ms: Some(invocation.spawn_ms),
                        worker_startup_ms: None,
                        worker_attempt_ms,
                        invoke_ms: invocation.total_ms,
                    },
                ),
                Err(error) => {
                    log_request_failure(
                        &request_id,
                        request_started,
                        ProcessMode::OneshotFallback,
                        &error,
                    );
                    return Err(error);
                }
            }
        }
        None => {
            let remaining = remaining_timeout(request_started, PROCESS_TIMEOUT)?;
            let oneshot_request = legacy_oneshot_request(&request);
            let stdin = serde_json::to_vec(&oneshot_request).map_err(|error| {
                AppError::Vision(format!("cannot encode vision request: {error}"))
            })?;
            match invoke_with_progress(settings, "request", Some(&stdin), remaining, progress).await
            {
                Ok((response, invocation)) => (
                    response,
                    ProcessInvocationMetrics {
                        mode: ProcessMode::Oneshot,
                        spawn_ms: Some(invocation.spawn_ms),
                        worker_startup_ms: None,
                        worker_attempt_ms: 0.0,
                        invoke_ms: invocation.total_ms,
                    },
                ),
                Err(error) => {
                    log_request_failure(&request_id, request_started, ProcessMode::Oneshot, &error);
                    return Err(error);
                }
            }
        }
    };
    if let Err(error) = ensure_protocol(&response, "processScreenshot") {
        log_request_failure(&request_id, request_started, metrics.mode, &error);
        return Err(error);
    }
    if response
        .request_id
        .as_deref()
        .is_some_and(|value| value != request_id)
    {
        let error = AppError::Vision(format!(
            "Python response request ID mismatch: expected {request_id}, received {}",
            response.request_id.as_deref().unwrap_or_default()
        ));
        log_request_failure(&request_id, request_started, metrics.mode, &error);
        return Err(error);
    }
    if !response.ok {
        let error = response_error(response);
        log_request_failure(&request_id, request_started, metrics.mode, &error);
        return Err(error);
    }
    let Some(data) = response.data else {
        let error = AppError::Vision("processing response has no data".to_owned());
        log_request_failure(&request_id, request_started, metrics.mode, &error);
        return Err(error);
    };
    let result: VisionProcessData = match serde_json::from_value(data) {
        Ok(result) => result,
        Err(error) => {
            let error = AppError::Vision(format!("invalid processing response: {error}"));
            log_request_failure(&request_id, request_started, metrics.mode, &error);
            return Err(error);
        }
    };
    let timings = result.timings.as_ref();
    log_service::debug(
        "vision.request",
        format!(
            "request_id={request_id} mode={} action=processScreenshot outcome=ok spawn_ms={} worker_startup_ms={} worker_attempt_ms={:.3} rust_invoke_ms={:.3} rust_total_ms={:.3} processor_init_ms={} read_ms={} detect_ms={} feature_ms={} annotate_ms={} crop_ms={} write_ms={} process_total_ms={} service_total_ms={}",
            metrics.mode.as_str(),
            timing_value(metrics.spawn_ms),
            timing_value(metrics.worker_startup_ms),
            metrics.worker_attempt_ms,
            metrics.invoke_ms,
            elapsed_ms(request_started),
            timing_value(timings.and_then(|value| value.processor_init_ms)),
            timing_value(timings.and_then(|value| value.read_ms)),
            timing_value(timings.and_then(|value| value.detect_ms)),
            timing_value(timings.and_then(|value| value.feature_ms)),
            timing_value(timings.and_then(|value| value.annotate_ms)),
            timing_value(timings.and_then(|value| value.crop_ms)),
            timing_value(timings.and_then(|value| value.write_ms)),
            timing_value(timings.and_then(|value| value.process_total_ms)),
            timing_value(timings.and_then(|value| value.service_total_ms)),
        ),
    );
    Ok(result)
}

fn validate_python_path(path: &Path, label: &str) -> Result<(), AppError> {
    let value = path.to_string_lossy();
    if value.starts_with(r"\\") {
        return Err(AppError::Vision(format!(
            "{label} must be on a local drive; UNC paths are reserved for Rust archival"
        )));
    }
    if !path.is_absolute() {
        return Err(AppError::Vision(format!("{label} path must be absolute")));
    }
    Ok(())
}

async fn invoke(
    settings: &VisionSettings,
    command_name: &str,
    stdin: Option<&[u8]>,
    timeout: Duration,
) -> Result<EngineResponse, AppError> {
    invoke_with_progress(settings, command_name, stdin, timeout, None)
        .await
        .map(|(response, _)| response)
}

async fn invoke_with_progress(
    settings: &VisionSettings,
    command_name: &str,
    stdin: Option<&[u8]>,
    timeout: Duration,
    mut progress: Option<ProgressCallback>,
) -> Result<(EngineResponse, InvocationMetrics), AppError> {
    let executable = settings
        .python_executable_path
        .as_deref()
        .ok_or_else(|| AppError::Vision("Python executable is not configured".to_owned()))?;
    let module_root = settings
        .python_module_root
        .as_deref()
        .ok_or_else(|| AppError::Vision("Python module root is not configured".to_owned()))?;
    let invocation_started = Instant::now();
    let spawn_started = Instant::now();
    let mut child = Command::new(executable)
        .arg("-m")
        .arg("scene_vault_ai")
        .arg(command_name)
        .env("PYTHONPATH", module_root)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| AppError::Vision(format!("cannot start Python: {error}")))?;
    let spawn_ms = elapsed_ms(spawn_started);

    // Stream SVPROGRESS lines from stderr while stdout keeps the single JSON
    // response contract. Old Python versions never write progress lines, so
    // this degrades gracefully to no progress events.
    let stderr_task = if progress.is_some() {
        if let Some(stderr) = child.stderr.take() {
            let mut callback = progress.take().expect("progress checked above");
            Some(tokio::spawn(async move {
                let mut lines = tokio::io::BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Some((stage, percent)) = parse_progress_line(&line) {
                        callback(&stage, percent);
                    } else if !line.trim().is_empty() {
                        log_service::debug(
                            "vision.python",
                            format!("stderr: {}", truncate(&line, 500)),
                        );
                    }
                }
            }))
        } else {
            None
        }
    } else {
        None
    };

    if let Some(bytes) = stdin {
        let Some(mut child_stdin) = child.stdin.take() else {
            abort_stderr_task(stderr_task).await;
            return Err(AppError::Vision("cannot open Python stdin".to_owned()));
        };
        let remaining = match remaining_timeout(invocation_started, timeout) {
            Ok(remaining) => remaining,
            Err(error) => {
                abort_stderr_task(stderr_task).await;
                return Err(error);
            }
        };
        let write_result = tokio::time::timeout(remaining, async {
            child_stdin.write_all(bytes).await.map_err(|error| {
                AppError::Vision(format!("cannot write Python request: {error}"))
            })?;
            child_stdin
                .shutdown()
                .await
                .map_err(|error| AppError::Vision(format!("cannot close Python stdin: {error}")))
        })
        .await;
        match write_result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                abort_stderr_task(stderr_task).await;
                return Err(error);
            }
            Err(_) => {
                abort_stderr_task(stderr_task).await;
                return Err(AppError::Vision("Python request timed out".to_owned()));
            }
        }
    }

    let remaining = match remaining_timeout(invocation_started, timeout) {
        Ok(remaining) => remaining,
        Err(error) => {
            abort_stderr_task(stderr_task).await;
            return Err(error);
        }
    };
    let output = match tokio::time::timeout(remaining, child.wait_with_output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            abort_stderr_task(stderr_task).await;
            return Err(AppError::Vision(format!("Python process failed: {error}")));
        }
        Err(_) => {
            abort_stderr_task(stderr_task).await;
            return Err(AppError::Vision("Python request timed out".to_owned()));
        }
    };
    finish_stderr_task(stderr_task).await;
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if !stderr.is_empty() {
        log_service::debug(
            "vision.python",
            format!(
                "stderr: {}",
                truncate(&stderr.replace(['\r', '\n'], " | "), 1_000)
            ),
        );
    }
    let response: EngineResponse = serde_json::from_slice(&output.stdout).map_err(|error| {
        AppError::Vision(format!(
            "Python returned invalid JSON: {error}{}",
            if stderr.is_empty() {
                String::new()
            } else {
                format!("; stderr: {}", truncate(&stderr, 1_000))
            }
        ))
    })?;
    if !output.status.success() && response.ok {
        return Err(AppError::Vision(format!(
            "Python exited with {}{}",
            output.status,
            if stderr.is_empty() {
                String::new()
            } else {
                format!(": {}", truncate(&stderr, 1_000))
            }
        )));
    }
    Ok((
        response,
        InvocationMetrics {
            spawn_ms,
            total_ms: elapsed_ms(invocation_started),
        },
    ))
}

async fn abort_stderr_task(task: Option<JoinHandle<()>>) {
    if let Some(task) = task {
        task.abort();
        let _ = task.await;
    }
}

async fn finish_stderr_task(task: Option<JoinHandle<()>>) {
    if let Some(mut task) = task {
        if tokio::time::timeout(Duration::from_secs(2), &mut task)
            .await
            .is_err()
        {
            task.abort();
            let _ = task.await;
        }
    }
}

fn legacy_oneshot_request(request: &Value) -> Value {
    let mut request = request.clone();
    if let Some(object) = request.as_object_mut() {
        object.remove("requestId");
    }
    request
}

fn remaining_timeout(started: Instant, total: Duration) -> Result<Duration, AppError> {
    total
        .checked_sub(started.elapsed())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| AppError::Vision("Python request timed out".to_owned()))
}

/// Parses one `SVPROGRESS {"stage": "...", "percent": N}` line. Lines without
/// the prefix (OpenCV warnings, tracebacks) are ignored.
fn parse_progress_line(line: &str) -> Option<(String, f64)> {
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
    Some((stage.to_owned(), percent))
}

fn ensure_protocol(response: &EngineResponse, action: &str) -> Result<(), AppError> {
    if response.protocol_version != PROTOCOL_VERSION {
        return Err(AppError::Vision(format!(
            "unsupported Python protocol version {}",
            response.protocol_version
        )));
    }
    if response.action != action {
        return Err(AppError::Vision(format!(
            "Python response action mismatch: expected {action}, received {}",
            response.action
        )));
    }
    Ok(())
}

fn response_error(response: EngineResponse) -> AppError {
    match response.error {
        Some(error) => AppError::Vision(format!("{}: {}", error.code, error.message)),
        None => AppError::Vision("Python returned an unspecified error".to_owned()),
    }
}

fn truncate(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1_000.0
}

fn timing_value(value: Option<f64>) -> String {
    value
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| format!("{value:.3}"))
        .unwrap_or_else(|| "na".to_owned())
}

fn log_request_failure(request_id: &str, started: Instant, mode: ProcessMode, error: &AppError) {
    log_service::error(
        "vision.request",
        format!(
            "request_id={request_id} mode={} action=processScreenshot outcome=error rust_total_ms={:.3} error={}",
            mode.as_str(),
            elapsed_ms(started),
            truncate(&error.to_string().replace(['\r', '\n'], " | "), 500)
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::vision::{
        AnnotationSettings, CropSettings, DetectionSettings, ProcessingSettings, VisionSettings,
    };

    #[test]
    fn parses_progress_lines_and_ignores_noise() {
        assert_eq!(
            parse_progress_line(r#"SVPROGRESS {"stage":"annotate","percent":75}"#),
            Some(("annotate".to_owned(), 75.0))
        );
        assert_eq!(
            parse_progress_line(r#"SVPROGRESS {"stage":"detect_face","percent":20.0}"#),
            Some(("detect_face".to_owned(), 20.0))
        );
        // OpenCV warnings and other stderr noise must be ignored.
        assert!(parse_progress_line("[ WARN] global net_impl_backend.cpp:345").is_none());
        assert!(parse_progress_line("traceback noise").is_none());
        assert!(parse_progress_line(r#"SVPROGRESS {"stage":"","percent":50}"#).is_none());
        assert!(parse_progress_line(r#"SVPROGRESS {"stage":"x","percent":200}"#).is_none());
        assert!(parse_progress_line(r#"SVPROGRESS not-json"#).is_none());
    }

    #[test]
    fn processing_payload_merges_only_set_fields() {
        let settings = VisionSettings::default();
        let processing = ProcessingSettings {
            detection: Some(DetectionSettings {
                score_threshold: Some(0.7),
                top_k: Some(1000),
                ..Default::default()
            }),
            annotation: Some(AnnotationSettings {
                font_size: Some(64),
                stroke_width: Some(4),
                padding: Some(40),
                face_box_expansion: Some(48),
                text_color: Some([1, 2, 3]),
                face_text_position: Some("custom".to_owned()),
                text_offset_x: Some(-0.3),
                text_offset_y: Some(1.25),
                ..Default::default()
            }),
            crop: Some(CropSettings {
                min_size: Some(256),
                ..Default::default()
            }),
        };
        let payload = processing_payload(&settings, &processing, json!({ "base": true }));
        assert_eq!(payload["base"], true);
        assert_eq!(payload["detection"]["scoreThreshold"], 0.7);
        // Integer settings must stay JSON integers: the Python protocol
        // rejects float-encoded integers such as 64.0.
        assert_eq!(payload["detection"]["topK"].as_i64(), Some(1000));
        assert!(payload["detection"].get("nmsThreshold").is_none());
        assert_eq!(payload["annotation"]["fontSize"].as_i64(), Some(64));
        assert_eq!(payload["annotation"]["strokeWidth"].as_i64(), Some(4));
        assert_eq!(payload["annotation"]["padding"].as_i64(), Some(40));
        assert_eq!(payload["annotation"]["faceBoxExpansion"].as_i64(), Some(48));
        assert_eq!(payload["annotation"]["textColor"], json!([1, 2, 3]));
        assert_eq!(payload["annotation"]["faceTextPosition"], "custom");
        assert_eq!(payload["annotation"]["textOffsetX"].as_f64(), Some(-0.3));
        assert_eq!(payload["annotation"]["textOffsetY"].as_f64(), Some(1.25));
        assert_eq!(payload["crop"]["minSize"].as_i64(), Some(256));
    }

    #[test]
    fn processing_payload_keeps_font_path_annotation() {
        let settings = VisionSettings {
            font_path: Some("C:\\fonts\\test.ttf".to_owned()),
            ..Default::default()
        };
        let payload = processing_payload(&settings, &ProcessingSettings::default(), json!({}));
        assert_eq!(payload["annotation"]["fontPath"], "C:\\fonts\\test.ttf");
    }

    #[test]
    fn rejects_protocol_and_action_mismatches() {
        let wrong_protocol = EngineResponse {
            protocol_version: 99,
            request_id: None,
            ok: true,
            action: "health".to_owned(),
            data: Some(json!({})),
            error: None,
        };
        assert!(matches!(
            ensure_protocol(&wrong_protocol, "health"),
            Err(AppError::Vision(_))
        ));

        let wrong_action = EngineResponse {
            protocol_version: PROTOCOL_VERSION,
            request_id: None,
            ok: true,
            action: "health".to_owned(),
            data: Some(json!({})),
            error: None,
        };
        assert!(matches!(
            ensure_protocol(&wrong_action, "processScreenshot"),
            Err(AppError::Vision(_))
        ));
    }

    #[test]
    fn preserves_structured_python_error_codes() {
        let error = response_error(EngineResponse {
            protocol_version: PROTOCOL_VERSION,
            request_id: None,
            ok: false,
            action: "processScreenshot".to_owned(),
            data: None,
            error: Some(EngineError {
                code: "image_decode_failed".to_owned(),
                message: "cannot decode".to_owned(),
            }),
        });
        assert!(error.to_string().contains("image_decode_failed"));
    }

    #[test]
    fn parses_optional_request_id_and_python_timings() {
        let response: EngineResponse = serde_json::from_value(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "requestId": "vision-123",
            "ok": true,
            "action": "processScreenshot",
            "data": {
                "annotatedPath": null,
                "avatarPath": null,
                "faceBox": null,
                "warnings": [],
                "timings": {
                    "processorInitMs": 12.5,
                    "detectMs": 34.25,
                    "serviceTotalMs": 50.0
                }
            },
            "error": null
        }))
        .expect("response should deserialize");
        let data: VisionProcessData =
            serde_json::from_value(response.data.expect("processing data should exist"))
                .expect("processing timings should deserialize");

        assert_eq!(response.request_id.as_deref(), Some("vision-123"));
        let timings = data.timings.expect("timings should exist");
        assert_eq!(timings.processor_init_ms, Some(12.5));
        assert_eq!(timings.detect_ms, Some(34.25));
        assert_eq!(timings.service_total_ms, Some(50.0));
        assert_eq!(timings.read_ms, None);
    }

    #[test]
    fn legacy_oneshot_fallback_omits_worker_request_id() {
        let request = json!({
            "protocolVersion": PROTOCOL_VERSION,
            "requestId": "worker-only-id",
            "action": "processScreenshot",
            "payload": {"inputPath": "C:\\input.png"}
        });

        let fallback = legacy_oneshot_request(&request);

        assert!(fallback.get("requestId").is_none());
        assert_eq!(fallback["action"], "processScreenshot");
        assert_eq!(fallback["payload"], request["payload"]);
    }

    #[test]
    fn exhausted_request_budget_returns_timeout() {
        let started = Instant::now() - Duration::from_millis(10);
        let error = remaining_timeout(started, Duration::from_millis(1))
            .expect_err("exhausted budget should time out");

        assert!(error.to_string().contains("timed out"));
    }

    #[test]
    fn parses_diagnostic_execution_modes() {
        assert_eq!(
            VisionExecutionMode::parse("worker"),
            Some(VisionExecutionMode::Worker)
        );
        assert_eq!(
            VisionExecutionMode::parse(" ONE-SHOT "),
            Some(VisionExecutionMode::Oneshot)
        );
        assert_eq!(VisionExecutionMode::parse("unsupported"), None);
    }

    #[test]
    fn rejects_unc_paths_before_starting_python() {
        let error = validate_python_path(Path::new(r"\\server\share\capture.png"), "Python input")
            .expect_err("UNC paths must be rejected");
        assert!(error.to_string().contains("UNC"));
    }
}
