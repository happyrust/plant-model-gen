use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fs,
    io::Read,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use axum::{
    Json, Router,
    extract::{Path as AxumPath, State, WebSocketUpgrade},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use parse_pdms_db::parse::parse_file_basic_info;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::process::{Child, Command};
use tokio::{
    net::TcpListener,
    sync::{Mutex, broadcast, oneshot},
    task,
};
use uuid::Uuid;

const SUPPORTED_PARSE_DB_TYPES: &[&str] = &["SYST", "DESI", "CATA", "DICT", "GLB", "GLOB"];
const REPARSE_REUSE_DB_TYPES: &[&str] = &["SYST"];
const MANDATORY_PREPARSE_DB_TYPES: &[&str] = &["DICT", "GLOB", "GLB"];
const SCAN_MAX_DEPTH: usize = 6;
const SCAN_MAX_FILES: usize = 200_000;
const CLI_JOB_KILL_GRACE_MS: u64 = 1500;

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone)]
pub struct ParseSidecarOptions {
    pub site_key: String,
    pub bind_host: String,
    pub http_port: u16,
    pub runtime_dir: PathBuf,
    pub token: Option<String>,
    pub shutdown_after_job: bool,
    pub shutdown_delay_ms: u64,
    pub idle_timeout_secs: u64,
}

#[derive(Debug, Clone)]
struct ParseSidecarState {
    site_key: String,
    runtime_dir: PathBuf,
    token: Option<String>,
    shutdown_after_job: bool,
    shutdown_delay_ms: u64,
    idle_timeout_secs: u64,
    last_activity: Arc<std::sync::Mutex<Instant>>,
    shutdown_tx: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    events_tx: broadcast::Sender<Value>,
    jobs: Arc<Mutex<BTreeMap<String, SidecarJobRecord>>>,
    job_cancels: Arc<Mutex<BTreeMap<String, oneshot::Sender<()>>>>,
}

#[derive(Debug, Serialize)]
struct SidecarEnvelope<T>
where
    T: Serialize,
{
    success: bool,
    message: String,
    data: Option<T>,
    error: Option<SidecarError>,
}

#[derive(Debug, Serialize)]
struct SidecarError {
    code: String,
    message: String,
    detail: Option<String>,
    field: Option<String>,
    retryable: bool,
}

#[derive(Debug, Deserialize)]
pub struct ParsePreviewRequest {
    #[serde(default)]
    pub project_name: String,
    #[serde(default)]
    pub project_path: String,
    #[serde(default)]
    pub projects: Vec<SidecarSiteProject>,
    #[serde(default)]
    pub manual_db_nums: Vec<u32>,
    #[serde(default)]
    pub manual_db_files: Vec<String>,
    #[serde(default)]
    pub parse_db_types: Vec<String>,
    #[serde(default)]
    pub force_rebuild_system_db: bool,
    #[serde(default)]
    pub auto_parse_related_dbnums: bool,
    #[serde(default = "default_true")]
    pub cata_partial_parse: bool,
    #[serde(default)]
    pub db_index_path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ProjectScanRequest {
    pub root: String,
}

/// MBD 候选发现请求：工程组成与 preview 请求同构，便于 UI 复用站点表单状态。
#[derive(Debug, Deserialize)]
pub struct MdbCandidatesRequest {
    #[serde(default)]
    pub project_name: String,
    #[serde(default)]
    pub project_path: String,
    #[serde(default)]
    pub projects: Vec<SidecarSiteProject>,
}

#[derive(Debug, Deserialize)]
pub struct DbFileResolveRequest {
    pub project_roots: Vec<String>,
    pub db_file: String,
}

#[derive(Debug, Serialize)]
pub struct DbFileResolveResponse {
    pub dbnum: u32,
    pub file_name: String,
}

#[derive(Debug, Deserialize)]
pub struct DbIndexRebuildRequest {
    pub roots: Vec<DbIndexRoot>,
    pub index_path: String,
    #[serde(default)]
    pub force: bool,
    #[serde(default)]
    pub manual_db_nums: Vec<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DbIndexRoot {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Default, Serialize)]
pub struct DbIndexRebuildSummary {
    pub scanned: usize,
    pub skipped: usize,
    pub db_files: usize,
    pub ref0_total: usize,
    pub dependency_edges: usize,
    pub errors: usize,
}

#[derive(Debug, Deserialize)]
pub struct RunCliJobRequest {
    pub config_no_ext: String,
    pub cwd: String,
    pub stdout_path: String,
    pub stderr_path: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Serialize)]
pub struct RunCliJobResponse {
    pub success: bool,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SubmitCliJobResponse {
    pub job_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SidecarJobRecord {
    pub job_id: String,
    pub kind: String,
    pub status: String,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
    pub stdout_path: Option<String>,
    pub stderr_path: Option<String>,
    pub submitted_at_ms: i64,
    pub started_at_ms: Option<i64>,
    pub finished_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProjectRole {
    Design,
    Library,
}

impl Default for ProjectRole {
    fn default() -> Self {
        Self::Design
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SidecarSiteProject {
    pub path: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub role: ProjectRole,
    #[serde(default)]
    pub is_primary: bool,
    #[serde(default)]
    pub sort_order: u32,
}

#[derive(Debug, Serialize)]
pub struct ScannedProject {
    pub path: String,
    pub name: String,
    pub role: ProjectRole,
    pub is_primary: bool,
    pub sort_order: u32,
    pub dbnums: Vec<u32>,
    pub db_types: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ScannedDbnumConflict {
    pub dbnum: u32,
    pub projects: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ScanProjectsResult {
    pub root: String,
    pub projects: Vec<ScannedProject>,
    pub conflicts: Vec<ScannedDbnumConflict>,
    pub has_conflict: bool,
}

#[derive(Debug, Serialize)]
pub struct ManagedSiteParsePlan {
    pub mode: ManagedSiteParsePlanMode,
    pub label: String,
    pub detail: String,
    #[serde(default)]
    pub includes_system_db_files: bool,
    #[serde(default)]
    pub included_db_files: Vec<String>,
    #[serde(default)]
    pub auto_related_db_files: Vec<String>,
    #[serde(default)]
    pub entries: Vec<ParsePlanFact>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ParsePlanFact {
    pub file_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dbnum: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub db_type: Option<String>,
    pub source: String,
    pub priority: u32,
}

#[derive(Debug, Serialize)]
pub enum ManagedSiteParsePlanMode {
    Full,
    Bootstrap,
    RebuildSystem,
    Selective,
    FastReparse,
}

pub async fn run_parse_sidecar(options: ParseSidecarOptions) -> Result<()> {
    let (events_tx, _) = broadcast::channel(256);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let state = ParseSidecarState {
        site_key: options.site_key,
        runtime_dir: options.runtime_dir,
        token: options.token,
        shutdown_after_job: options.shutdown_after_job,
        shutdown_delay_ms: options.shutdown_delay_ms,
        idle_timeout_secs: options.idle_timeout_secs,
        last_activity: Arc::new(std::sync::Mutex::new(Instant::now())),
        shutdown_tx: Arc::new(Mutex::new(Some(shutdown_tx))),
        events_tx,
        jobs: Arc::new(Mutex::new(BTreeMap::new())),
        job_cancels: Arc::new(Mutex::new(BTreeMap::new())),
    };
    spawn_idle_watchdog(&state);
    let bind_host = options.bind_host;
    let http_port = options.http_port;
    let app = Router::new()
        .route("/health", get(health))
        .route("/parse/preview-plan", post(preview_plan))
        .route("/projects/scan", post(scan_projects))
        .route("/projects/mdb-candidates", post(mdb_candidates))
        .route("/db-files/resolve", post(resolve_db_file))
        .route("/db-index/rebuild", post(rebuild_db_index))
        .route("/jobs/run-cli", post(run_cli_job))
        .route("/jobs/submit-cli", post(submit_cli_job))
        .route("/jobs/{job_id}", get(job_status))
        .route("/jobs/{job_id}/cancel", post(cancel_job))
        .route("/events", get(events))
        .with_state(state);

    let addr: SocketAddr = format!("{bind_host}:{http_port}")
        .parse()
        .with_context(|| format!("无效的 sidecar 监听地址: {bind_host}:{http_port}"))?;
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("绑定 sidecar 监听地址失败: {addr}"))?;
    println!("🚀 aios-database parse sidecar listening on http://{addr}");
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = shutdown_rx.await;
            println!("📴 aios-database parse sidecar graceful shutdown requested");
        })
        .await
        .context("sidecar HTTP 服务异常退出")
}

async fn health(
    State(state): State<ParseSidecarState>,
    headers: HeaderMap,
) -> Result<Json<SidecarEnvelope<Value>>, Response> {
    authorize(&state, &headers)?;
    Ok(Json(ok(
        "sidecar healthy",
        json!({
            "site_key": state.site_key,
            "runtime_dir": state.runtime_dir,
            "capabilities": ["health", "preview-plan", "projects-scan", "mdb-candidates", "db-file-resolve", "db-index-rebuild", "run-cli-job", "submit-cli-job", "job-status", "job-cancel", "events"]
        }),
    )))
}

async fn preview_plan(
    State(state): State<ParseSidecarState>,
    headers: HeaderMap,
    Json(payload): Json<ParsePreviewRequest>,
) -> Result<Json<SidecarEnvelope<Value>>, Response> {
    authorize(&state, &headers)?;
    match build_preview_plan(payload) {
        Ok(plan) => Ok(Json(ok("获取解析预览成功", json!(plan)))),
        Err(err) => Err(domain_error(err)),
    }
}

async fn scan_projects(
    State(state): State<ParseSidecarState>,
    headers: HeaderMap,
    Json(payload): Json<ProjectScanRequest>,
) -> Result<Json<SidecarEnvelope<Value>>, Response> {
    authorize(&state, &headers)?;
    match scan_projects_under_root(&payload.root) {
        Ok(result) => Ok(Json(ok("工程扫描完成", json!(result)))),
        Err(err) => Err(domain_error(err)),
    }
}

async fn mdb_candidates(
    State(state): State<ParseSidecarState>,
    headers: HeaderMap,
    Json(payload): Json<MdbCandidatesRequest>,
) -> Result<Json<SidecarEnvelope<Value>>, Response> {
    authorize(&state, &headers)?;
    match discover_mdb_candidates_request(payload).await {
        Ok(result) => Ok(Json(ok("MBD 候选发现完成", json!(result)))),
        Err(err) => Err(domain_error(err)),
    }
}

async fn resolve_db_file(
    State(state): State<ParseSidecarState>,
    headers: HeaderMap,
    Json(payload): Json<DbFileResolveRequest>,
) -> Result<Json<SidecarEnvelope<Value>>, Response> {
    authorize(&state, &headers)?;
    match resolve_db_file_request(payload) {
        Ok(result) => Ok(Json(ok("DB 文件解析成功", json!(result)))),
        Err(err) => Err(domain_error(err)),
    }
}

async fn rebuild_db_index(
    State(state): State<ParseSidecarState>,
    headers: HeaderMap,
    Json(payload): Json<DbIndexRebuildRequest>,
) -> Result<Json<SidecarEnvelope<Value>>, Response> {
    authorize(&state, &headers)?;
    #[cfg(feature = "sqlite-index")]
    {
        emit_event(
            &state.events_tx,
            json!({
                "type": "db_index_rebuild_started",
                "site_key": state.site_key,
            }),
        );
        match rebuild_db_index_request(payload, state.events_tx.clone(), state.site_key.clone())
            .await
        {
            Ok(result) => {
                emit_event(
                    &state.events_tx,
                    json!({
                        "type": "db_index_rebuild_done",
                        "site_key": state.site_key,
                        "summary": result,
                    }),
                );
                Ok(Json(ok("db_index 重建完成", json!(result))))
            }
            Err(err) => {
                emit_event(
                    &state.events_tx,
                    json!({
                        "type": "db_index_rebuild_failed",
                        "site_key": state.site_key,
                        "message": err.to_string(),
                    }),
                );
                Err(domain_error(err))
            }
        }
    }
    #[cfg(not(feature = "sqlite-index"))]
    {
        let _ = payload;
        Err(structured_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "SQLITE_INDEX_DISABLED",
            "当前构建未启用 sqlite-index feature",
            None,
            None,
            false,
        ))
    }
}

async fn run_cli_job(
    State(state): State<ParseSidecarState>,
    headers: HeaderMap,
    Json(payload): Json<RunCliJobRequest>,
) -> Result<Json<SidecarEnvelope<Value>>, Response> {
    authorize(&state, &headers)?;
    match run_cli_job_request(payload).await {
        Ok(result) => Ok(Json(ok("CLI 作业完成", json!(result)))),
        Err(err) => Err(domain_error(err)),
    }
}

async fn submit_cli_job(
    State(state): State<ParseSidecarState>,
    headers: HeaderMap,
    Json(payload): Json<RunCliJobRequest>,
) -> Result<Json<SidecarEnvelope<Value>>, Response> {
    authorize(&state, &headers)?;
    let job_id = Uuid::new_v4().to_string();
    let record = SidecarJobRecord {
        job_id: job_id.clone(),
        kind: "cli".to_string(),
        status: "queued".to_string(),
        exit_code: None,
        error: None,
        stdout_path: Some(payload.stdout_path.clone()),
        stderr_path: Some(payload.stderr_path.clone()),
        submitted_at_ms: now_ms(),
        started_at_ms: None,
        finished_at_ms: None,
    };
    state
        .jobs
        .lock()
        .await
        .insert(job_id.clone(), record.clone());
    emit_event(
        &state.events_tx,
        json!({
            "type": "job_submitted",
            "site_key": state.site_key,
            "job": record,
        }),
    );

    let (cancel_tx, cancel_rx) = oneshot::channel();
    state
        .job_cancels
        .lock()
        .await
        .insert(job_id.clone(), cancel_tx);
    tokio::spawn(run_submitted_cli_job(
        state.clone(),
        job_id.clone(),
        payload,
        cancel_rx,
    ));

    Ok(Json(ok(
        "CLI 作业已提交",
        json!(SubmitCliJobResponse { job_id }),
    )))
}

async fn job_status(
    State(state): State<ParseSidecarState>,
    headers: HeaderMap,
    AxumPath(job_id): AxumPath<String>,
) -> Result<Json<SidecarEnvelope<Value>>, Response> {
    authorize(&state, &headers)?;
    let jobs = state.jobs.lock().await;
    let Some(record) = jobs.get(&job_id).cloned() else {
        return Err(structured_error(
            StatusCode::NOT_FOUND,
            "JOB_NOT_FOUND",
            "sidecar job 不存在",
            None,
            Some("job_id".to_string()),
            false,
        ));
    };
    Ok(Json(ok("CLI 作业状态", json!(record))))
}

async fn cancel_job(
    State(state): State<ParseSidecarState>,
    headers: HeaderMap,
    AxumPath(job_id): AxumPath<String>,
) -> Result<Json<SidecarEnvelope<Value>>, Response> {
    authorize(&state, &headers)?;
    let cancel = state.job_cancels.lock().await.remove(&job_id);
    if let Some(cancel) = cancel {
        let _ = cancel.send(());
        update_job_record(&state, &job_id, |record| {
            if record.status == "queued" || record.status == "running" {
                record.status = "cancelling".to_string();
            }
        })
        .await;
        emit_event(
            &state.events_tx,
            json!({
                "type": "job_cancel_requested",
                "site_key": state.site_key,
                "job_id": job_id,
            }),
        );
        emit_job_stage_changed(&state, &job_id, "cancelling", "取消请求已发送", None);
        Ok(Json(ok(
            "CLI 作业取消请求已发送",
            json!({ "cancelled": true }),
        )))
    } else {
        Err(structured_error(
            StatusCode::NOT_FOUND,
            "JOB_CANCEL_UNAVAILABLE",
            "sidecar job 不存在或已结束",
            None,
            Some("job_id".to_string()),
            false,
        ))
    }
}

async fn events(
    State(state): State<ParseSidecarState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, Response> {
    authorize(&state, &headers)?;
    Ok(ws
        .on_upgrade(|mut socket| async move {
            let mut rx = state.events_tx.subscribe();
            let _ = socket
                .send(axum::extract::ws::Message::Text(
                    json!({
                        "type": "sidecar_hello",
                        "site_key": state.site_key,
                        "message": "parse sidecar event stream connected"
                    })
                    .to_string()
                    .into(),
                ))
                .await;
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        if socket
                            .send(axum::extract::ws::Message::Text(event.to_string().into()))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        })
        .into_response())
}

fn emit_event(events_tx: &broadcast::Sender<Value>, mut event: Value) {
    if let Value::Object(ref mut map) = event {
        map.insert(
            "timestamp_ms".to_string(),
            json!(chrono::Utc::now().timestamp_millis()),
        );
    }
    let _ = events_tx.send(event);
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

async fn update_job_record<F>(
    state: &ParseSidecarState,
    job_id: &str,
    update: F,
) -> Option<SidecarJobRecord>
where
    F: FnOnce(&mut SidecarJobRecord),
{
    let mut jobs = state.jobs.lock().await;
    let record = jobs.get_mut(job_id)?;
    update(record);
    Some(record.clone())
}

async fn run_submitted_cli_job(
    state: ParseSidecarState,
    job_id: String,
    payload: RunCliJobRequest,
    cancel_rx: oneshot::Receiver<()>,
) {
    if let Some(record) = update_job_record(&state, &job_id, |record| {
        record.status = "running".to_string();
        record.started_at_ms = Some(now_ms());
    })
    .await
    {
        emit_event(
            &state.events_tx,
            json!({
                "type": "job_running",
                "site_key": state.site_key,
                "job": record,
            }),
        );
        emit_event(
            &state.events_tx,
            json!({
                "type": "job_started",
                "site_key": state.site_key,
                "job": record,
            }),
        );
        emit_job_stage_changed(
            &state,
            &job_id,
            "running",
            "CLI 作业已启动",
            Some(record.clone()),
        );
    }

    let result = run_cli_job_with_cancel(payload, cancel_rx).await;
    state.job_cancels.lock().await.remove(&job_id);

    let (event_type, status, exit_code, error) = match result {
        Ok(JobExecutionOutcome::Completed(response)) if response.success => (
            "job_done",
            "succeeded".to_string(),
            response.exit_code,
            None,
        ),
        Ok(JobExecutionOutcome::Completed(response)) => (
            "job_failed",
            "failed".to_string(),
            response.exit_code,
            Some("CLI 作业返回非零退出码".to_string()),
        ),
        Ok(JobExecutionOutcome::Cancelled) => (
            "job_cancelled",
            "cancelled".to_string(),
            None,
            Some("CLI 作业已取消".to_string()),
        ),
        Err(err) => (
            "job_failed",
            "failed".to_string(),
            None,
            Some(err.to_string()),
        ),
    };

    if let Some(record) = update_job_record(&state, &job_id, |record| {
        record.status = status;
        record.exit_code = exit_code;
        record.error = error;
        record.finished_at_ms = Some(now_ms());
    })
    .await
    {
        emit_event(
            &state.events_tx,
            json!({
                "type": event_type,
                "site_key": state.site_key,
                "job": record,
            }),
        );
        emit_job_stage_changed(
            &state,
            &job_id,
            record.status.as_str(),
            "CLI 作业已结束",
            Some(record.clone()),
        );
        emit_job_log_appended(&state, &job_id, "stdout", record.stdout_path.as_deref());
        emit_job_log_appended(&state, &job_id, "stderr", record.stderr_path.as_deref());
        if event_type == "job_done" {
            emit_event(
                &state.events_tx,
                json!({
                    "type": "artifact_ready",
                    "site_key": state.site_key,
                    "job_id": job_id,
                    "artifacts": [
                        { "kind": "stdout", "path": record.stdout_path.clone() },
                        { "kind": "stderr", "path": record.stderr_path.clone() },
                    ],
                }),
            );
        }
    }
    schedule_shutdown_after_job(&state).await;
}

fn is_terminal_job_status(status: &str) -> bool {
    matches!(status, "succeeded" | "failed" | "cancelled")
}

/// 空闲看门狗：serve sidecar 在 `idle_timeout_secs` 内无任何请求且无活跃 job 时自动退出。
/// 这是 reaper / Job Object 之外的第三道保险，防止孤儿无限驻留。
fn spawn_idle_watchdog(state: &ParseSidecarState) {
    let idle_secs = state.idle_timeout_secs;
    if idle_secs == 0 {
        return;
    }
    let idle = Duration::from_secs(idle_secs);
    let last_activity = state.last_activity.clone();
    let jobs = state.jobs.clone();
    let shutdown_tx = state.shutdown_tx.clone();
    let site_key = state.site_key.clone();
    let check_interval = Duration::from_secs((idle_secs / 4).clamp(5, 60));
    task::spawn(async move {
        loop {
            tokio::time::sleep(check_interval).await;
            let elapsed = last_activity
                .lock()
                .map(|ts| ts.elapsed())
                .unwrap_or_default();
            if elapsed < idle {
                continue;
            }
            let has_active_job = {
                let guard = jobs.lock().await;
                guard
                    .values()
                    .any(|record| !is_terminal_job_status(&record.status))
            };
            if has_active_job {
                continue;
            }
            let Some(tx) = shutdown_tx.lock().await.take() else {
                return;
            };
            println!("📴 aios-database sidecar {site_key} idle {idle_secs}s 超时，自动退出");
            let _ = tx.send(());
            return;
        }
    });
}

async fn schedule_shutdown_after_job(state: &ParseSidecarState) {
    if !state.shutdown_after_job {
        return;
    }
    if !state.job_cancels.lock().await.is_empty() {
        return;
    }
    let shutdown_tx = state.shutdown_tx.clone();
    let delay = Duration::from_millis(state.shutdown_delay_ms);
    let site_key = state.site_key.clone();
    tokio::spawn(async move {
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        let Some(tx) = shutdown_tx.lock().await.take() else {
            return;
        };
        println!("📴 aios-database sidecar {site_key} shutting down after terminal job");
        let _ = tx.send(());
    });
}

fn emit_job_stage_changed(
    state: &ParseSidecarState,
    job_id: &str,
    stage: &str,
    label: &str,
    job: Option<SidecarJobRecord>,
) {
    emit_event(
        &state.events_tx,
        json!({
            "type": "stage_changed",
            "site_key": state.site_key,
            "job_id": job_id,
            "stage": stage,
            "label": label,
            "job": job,
        }),
    );
}

fn emit_job_log_appended(
    state: &ParseSidecarState,
    job_id: &str,
    stream: &str,
    path: Option<&str>,
) {
    let Some(path) = path else {
        return;
    };
    let Some(line) = last_non_empty_log_line(path) else {
        return;
    };
    emit_event(
        &state.events_tx,
        json!({
            "type": "log_appended",
            "site_key": state.site_key,
            "job_id": job_id,
            "stream": stream,
            "path": path,
            "line": line,
        }),
    );
}

fn last_non_empty_log_line(path: &str) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    content
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(500).collect())
}

enum JobExecutionOutcome {
    Completed(RunCliJobResponse),
    Cancelled,
}

async fn run_cli_job_with_cancel(
    payload: RunCliJobRequest,
    cancel_rx: oneshot::Receiver<()>,
) -> Result<JobExecutionOutcome> {
    let mut child = spawn_cli_child(payload)?;
    tokio::pin!(cancel_rx);
    tokio::select! {
        status = child.wait() => {
            let status = status.context("等待 CLI 作业失败")?;
            Ok(JobExecutionOutcome::Completed(RunCliJobResponse {
                success: status.success(),
                exit_code: status.code(),
            }))
        }
        _ = &mut cancel_rx => {
            kill_child_process_tree(&mut child).await;
            Ok(JobExecutionOutcome::Cancelled)
        }
    }
}

fn spawn_cli_child(payload: RunCliJobRequest) -> Result<Child> {
    let config_no_ext = payload.config_no_ext.trim();
    if config_no_ext.is_empty() {
        bail!("config_no_ext 不能为空");
    }
    let cwd = PathBuf::from(payload.cwd.trim());
    if !cwd.is_dir() {
        bail!("cwd 不是有效目录: {}", cwd.display());
    }
    let stdout_path = PathBuf::from(payload.stdout_path.trim());
    let stderr_path = PathBuf::from(payload.stderr_path.trim());
    if let Some(parent) = stdout_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("创建 stdout 日志目录失败: {}", parent.display()))?;
    }
    if let Some(parent) = stderr_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("创建 stderr 日志目录失败: {}", parent.display()))?;
    }
    let stdout = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&stdout_path)
        .with_context(|| format!("打开 stdout 日志失败: {}", stdout_path.display()))?;
    let stderr = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&stderr_path)
        .with_context(|| format!("打开 stderr 日志失败: {}", stderr_path.display()))?;
    let exe = std::env::current_exe().context("定位 aios-database 可执行文件失败")?;
    let mut command = Command::new(exe);
    command
        .arg("-c")
        .arg(config_no_ext)
        .args(&payload.args)
        .envs(&payload.env)
        .current_dir(cwd)
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    isolate_cli_job_process_group(&mut command);
    command.spawn().context("启动 CLI 作业失败")
}

fn isolate_cli_job_process_group(command: &mut Command) {
    #[cfg(unix)]
    {
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x00000200);
    }
}

#[cfg(unix)]
fn killpg_group(pid: u32, sig: libc::c_int) -> bool {
    let pgid = unsafe { libc::getpgid(pid as libc::pid_t) };
    if pgid <= 0 {
        return false;
    }
    unsafe { libc::killpg(pgid, sig) == 0 }
}

async fn kill_child_process_tree(child: &mut Child) {
    let Some(pid) = child.id() else {
        let _ = child.kill().await;
        return;
    };
    #[cfg(unix)]
    {
        if !killpg_group(pid, libc::SIGTERM) {
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGTERM);
            }
        }
        tokio::time::sleep(Duration::from_millis(CLI_JOB_KILL_GRACE_MS)).await;
        match child.try_wait() {
            Ok(Some(_)) => {}
            _ => {
                if !killpg_group(pid, libc::SIGKILL) {
                    unsafe {
                        libc::kill(pid as libc::pid_t, libc::SIGKILL);
                    }
                }
                let _ = child.wait().await;
            }
        }
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T"])
            .output()
            .await;
        tokio::time::sleep(Duration::from_millis(CLI_JOB_KILL_GRACE_MS)).await;
        match child.try_wait() {
            Ok(Some(_)) => {}
            _ => {
                let _ = Command::new("taskkill")
                    .args(["/PID", &pid.to_string(), "/T", "/F"])
                    .output()
                    .await;
                let _ = child.wait().await;
            }
        }
    }
}

fn authorize(state: &ParseSidecarState, headers: &HeaderMap) -> Result<(), Response> {
    // 每次请求刷新活跃时间，供 idle watchdog 判断空闲。
    if let Ok(mut ts) = state.last_activity.lock() {
        *ts = Instant::now();
    }
    let Some(expected) = state.token.as_deref().filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    let actual = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or_default();
    if actual == expected {
        Ok(())
    } else {
        Err(structured_error(
            StatusCode::UNAUTHORIZED,
            "UNAUTHORIZED",
            "sidecar authorization failed",
            None,
            None,
            false,
        ))
    }
}

fn ok<T>(message: impl Into<String>, data: T) -> SidecarEnvelope<T>
where
    T: Serialize,
{
    SidecarEnvelope {
        success: true,
        message: message.into(),
        data: Some(data),
        error: None,
    }
}

fn structured_error(
    status: StatusCode,
    code: impl Into<String>,
    message: impl Into<String>,
    detail: Option<String>,
    field: Option<String>,
    retryable: bool,
) -> Response {
    let message = message.into();
    (
        status,
        Json(SidecarEnvelope::<Value> {
            success: false,
            message: message.clone(),
            data: None,
            error: Some(SidecarError {
                code: code.into(),
                message,
                detail,
                field,
                retryable,
            }),
        }),
    )
        .into_response()
}

fn domain_error(err: anyhow::Error) -> Response {
    let message = err.to_string();
    let (status, code, field) = classify_domain_error(&message);
    structured_error(status, code, message, None, field, false)
}

fn classify_domain_error(message: &str) -> (StatusCode, &'static str, Option<String>) {
    if message.contains("项目名") && message.contains("不能为空") {
        (
            StatusCode::BAD_REQUEST,
            "INVALID_PROJECT_NAME",
            Some("project_name".to_string()),
        )
    } else if message.contains("项目路径") || message.contains("读取目录失败") {
        (
            StatusCode::BAD_REQUEST,
            "INVALID_PROJECT_PATH",
            Some("project_path".to_string()),
        )
    } else if message.contains("config_no_ext") {
        (
            StatusCode::BAD_REQUEST,
            "INVALID_CONFIG_PATH",
            Some("config_no_ext".to_string()),
        )
    } else if message.contains("cwd 不是有效目录") || message.contains("cwd") {
        (
            StatusCode::BAD_REQUEST,
            "INVALID_WORKING_DIRECTORY",
            Some("cwd".to_string()),
        )
    } else if message.contains("必须提供 db_file")
        || message.contains("目标 db 文件")
        || message.contains("db_file")
    {
        (
            StatusCode::BAD_REQUEST,
            "DB_FILE_REQUIRED",
            Some("db_file".to_string()),
        )
    } else if message.contains("无法从文件头解析") {
        (
            StatusCode::BAD_REQUEST,
            "DB_FILE_HEADER_INVALID",
            Some("manual_db_files".to_string()),
        )
    } else if message.contains("未能在任一工程路径下解析") || message.contains("未找到 dbnum")
    {
        (
            StatusCode::BAD_REQUEST,
            "DB_FILE_NOT_FOUND",
            Some("manual_db_files".to_string()),
        )
    } else if message.contains("超过") && message.contains("上限") {
        (
            StatusCode::BAD_REQUEST,
            "SCAN_LIMIT_EXCEEDED",
            Some("project_path".to_string()),
        )
    } else if message.contains("冲突") {
        (StatusCode::CONFLICT, "DBNUM_CONFLICT", None)
    } else {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "PARSE_ENGINE_FAILED",
            None,
        )
    }
}

fn build_preview_plan(payload: ParsePreviewRequest) -> Result<ManagedSiteParsePlan> {
    let project_name = payload.project_name.trim();
    if project_name.is_empty() {
        bail!("项目名不能为空");
    }
    let project_path = payload.project_path.trim();
    if project_path.is_empty() {
        bail!("项目路径不能为空");
    }

    let mut projects = normalize_projects(payload.projects, project_name, project_path)?;
    if projects.is_empty() {
        projects.push(SidecarSiteProject {
            path: canonical_project_path(project_path)?
                .to_string_lossy()
                .to_string(),
            name: project_name.to_string(),
            role: ProjectRole::Design,
            is_primary: true,
            sort_order: 0,
        });
    }
    let project_roots = project_roots(&projects)?;
    let primary_project_roots =
        primary_project_roots(&projects).unwrap_or_else(|_| project_roots.clone());
    let manual_db_nums =
        resolve_manual_db_nums(payload.manual_db_nums, payload.manual_db_files, |db_file| {
            let (dbnum, _) = resolve_dbnum_from_db_file_roots(&project_roots, db_file)?;
            Ok(dbnum)
        })?;
    let parse_db_types = normalize_parse_db_types(payload.parse_db_types);
    let force_rebuild_system_db =
        payload.force_rebuild_system_db && parse_db_types.iter().any(|v| v == "SYST");
    let mut included_db_files = resolve_included_db_files(
        &project_roots,
        &primary_project_roots,
        &manual_db_nums,
        &parse_db_types,
        force_rebuild_system_db,
        payload.auto_parse_related_dbnums,
        payload.cata_partial_parse,
    )?;
    let mut warnings = Vec::new();
    append_related_cata_from_db_index(
        &mut included_db_files,
        payload.db_index_path.as_deref(),
        &manual_db_nums,
        payload.auto_parse_related_dbnums,
        payload.cata_partial_parse,
        &mut warnings,
    );
    included_db_files.sort();
    included_db_files.dedup();
    let (entries, fact_warnings) = build_parse_plan_facts(
        &project_roots,
        &manual_db_nums,
        &parse_db_types,
        payload.auto_parse_related_dbnums,
        &included_db_files,
    );
    warnings.extend(fact_warnings);
    let auto_related_db_files = entries
        .iter()
        .filter(|entry| entry.source == "auto_related")
        .map(|entry| entry.file_name.clone())
        .collect();
    Ok(build_parse_plan_with_files(
        &manual_db_nums,
        &parse_db_types,
        force_rebuild_system_db,
        included_db_files,
        auto_related_db_files,
        entries,
        warnings,
    ))
}

/// MBD 候选发现：规范化工程组成后，离线读 SYST/GLOB/GLB 枚举 MDB 及成员 DB 文件定位状态。
async fn discover_mdb_candidates_request(
    payload: MdbCandidatesRequest,
) -> Result<crate::data_interface::mdb_candidates::MdbCandidatesResult> {
    let project_name = payload.project_name.trim();
    let project_path = payload.project_path.trim();
    let projects = normalize_projects(payload.projects, project_name, project_path)?;
    if projects.is_empty() {
        bail!("工程组成为空：请先在站点配置中添加至少一个工程路径");
    }
    let named_roots = projects
        .into_iter()
        .map(|project| (project.name, project.path))
        .collect::<Vec<_>>();
    crate::data_interface::mdb_candidates::discover_mdb_candidates_for_roots(named_roots).await
}

fn append_related_cata_from_db_index(
    included_db_files: &mut Vec<String>,
    db_index_path: Option<&str>,
    manual_db_nums: &[u32],
    auto_parse_related_dbnums: bool,
    cata_partial_parse: bool,
    warnings: &mut Vec<String>,
) {
    if !auto_parse_related_dbnums || !cata_partial_parse || manual_db_nums.is_empty() {
        return;
    }
    let Some(raw_path) = db_index_path.map(str::trim).filter(|path| !path.is_empty()) else {
        return;
    };

    #[cfg(feature = "sqlite-index")]
    {
        use crate::data_interface::db_index::DbIndexStore;

        let path = PathBuf::from(raw_path);
        if !path.is_file() {
            warnings.push(format!(
                "CATA 精确预览跳过：db_index 不存在或未就绪: {}",
                path.display()
            ));
            return;
        }
        let store = match DbIndexStore::open(&path) {
            Ok(store) => store,
            Err(err) => {
                warnings.push(format!(
                    "CATA 精确预览跳过：db_index 打开失败 {}: {}",
                    path.display(),
                    err
                ));
                return;
            }
        };
        for dbnum in store.resolve_related_closure(manual_db_nums) {
            let Some(record) = store.file_by_dbnum(dbnum) else {
                continue;
            };
            if record.db_type.eq_ignore_ascii_case("CATA") {
                included_db_files.push(record.file_name);
            }
        }
    }

    #[cfg(not(feature = "sqlite-index"))]
    {
        let _ = raw_path;
        warnings.push("CATA 精确预览跳过：当前构建未启用 sqlite-index feature".to_string());
    }
}

fn resolve_db_file_request(payload: DbFileResolveRequest) -> Result<DbFileResolveResponse> {
    let project_roots = payload
        .project_roots
        .iter()
        .map(|root| canonical_project_path(root))
        .collect::<Result<Vec<_>>>()?;
    if project_roots.is_empty() {
        bail!("project_roots 不能为空");
    }
    let db_file = payload.db_file.trim();
    if db_file.is_empty() {
        bail!("db_file 不能为空");
    }
    let (dbnum, file_name) = resolve_dbnum_from_db_file_roots(&project_roots, db_file)?;
    Ok(DbFileResolveResponse { dbnum, file_name })
}

#[cfg(feature = "sqlite-index")]
async fn rebuild_db_index_request(
    payload: DbIndexRebuildRequest,
    events_tx: broadcast::Sender<Value>,
    site_key: String,
) -> Result<DbIndexRebuildSummary> {
    use crate::data_interface::db_index::{
        DbIndexStore, ScanReport, collect_design_outbound, prescan_roots_with_progress,
    };

    let mut roots = Vec::new();
    for root in payload.roots {
        let path = canonical_project_path(&root.path)?;
        if path.exists() {
            let name = root.name.trim();
            roots.push((
                if name.is_empty() {
                    path.file_name()
                        .map(|value| value.to_string_lossy().to_string())
                        .unwrap_or_else(|| "project".to_string())
                } else {
                    name.to_string()
                },
                path,
            ));
        }
    }
    if roots.is_empty() {
        bail!("db_index roots 不能为空");
    }

    let index_path = PathBuf::from(payload.index_path);
    let mut summary = DbIndexRebuildSummary::default();
    let roots_p1 = roots.clone();
    let index_p1 = index_path.clone();
    let events_p1 = events_tx.clone();
    let site_key_p1 = site_key.clone();
    match task::spawn_blocking(move || -> Result<ScanReport> {
        let store = DbIndexStore::open(&index_p1)?;
        let mut last_emit = Instant::now() - Duration::from_secs(1);
        Ok(prescan_roots_with_progress(
            &store,
            &roots_p1,
            payload.force,
            |progress| {
                let should_emit = progress.processed_files == 1
                    || progress.processed_files % 25 == 0
                    || last_emit.elapsed() >= Duration::from_secs(1);
                if !should_emit {
                    return;
                }
                last_emit = Instant::now();
                emit_event(
                    &events_p1,
                    json!({
                        "type": "db_index_rebuild_progress",
                        "site_key": site_key_p1,
                        "phase": "prescan",
                        "project": progress.project,
                        "current_file": progress.current_file,
                        "processed_files": progress.processed_files,
                        "scanned": progress.scanned,
                        "skipped": progress.skipped,
                        "ref0_total": progress.ref0_total,
                        "errors": progress.errors,
                    }),
                );
            },
        ))
    })
    .await
    {
        Ok(Ok(report)) => {
            summary.scanned = report.scanned;
            summary.skipped = report.skipped;
            summary.db_files = report.db_files;
            summary.ref0_total = report.ref0_total;
            summary.errors += report.errors.len();
        }
        Ok(Err(err)) => return Err(err),
        Err(err) => bail!("db_index phase1 任务失败: {err}"),
    }

    let outbound = filter_design_outbound_for_manual_dbnums(
        collect_design_outbound(&roots).await,
        &payload.manual_db_nums,
    );
    if !outbound.is_empty() {
        let index_p2 = index_path.clone();
        match task::spawn_blocking(move || -> Result<usize> {
            let store = DbIndexStore::open(&index_p2)?;
            let mut edges = 0usize;
            for (src, ref0s) in &outbound {
                let mut dsts = store.resolve_dbnums(ref0s);
                dsts.retain(|dst| dst != src);
                store.record_dependencies(*src, &dsts)?;
                edges += dsts.len();
            }
            Ok(edges)
        })
        .await
        {
            Ok(Ok(edges)) => summary.dependency_edges = edges,
            Ok(Err(err)) => return Err(err),
            Err(err) => bail!("db_index phase2 任务失败: {err}"),
        }
    }

    Ok(summary)
}

#[cfg(feature = "sqlite-index")]
fn filter_design_outbound_for_manual_dbnums(
    mut outbound: Vec<(u32, Vec<u32>)>,
    manual_db_nums: &[u32],
) -> Vec<(u32, Vec<u32>)> {
    let target_dbnums = manual_db_nums.iter().copied().collect::<HashSet<_>>();
    if !target_dbnums.is_empty() {
        outbound.retain(|(src, _)| target_dbnums.contains(src));
    }
    outbound
}

async fn run_cli_job_request(payload: RunCliJobRequest) -> Result<RunCliJobResponse> {
    let config_no_ext = payload.config_no_ext.trim();
    if config_no_ext.is_empty() {
        bail!("config_no_ext 不能为空");
    }
    let cwd = PathBuf::from(payload.cwd.trim());
    if !cwd.is_dir() {
        bail!("cwd 不是有效目录: {}", cwd.display());
    }
    let stdout_path = PathBuf::from(payload.stdout_path.trim());
    let stderr_path = PathBuf::from(payload.stderr_path.trim());
    if let Some(parent) = stdout_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("创建 stdout 日志目录失败: {}", parent.display()))?;
    }
    if let Some(parent) = stderr_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("创建 stderr 日志目录失败: {}", parent.display()))?;
    }
    let stdout = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&stdout_path)
        .with_context(|| format!("打开 stdout 日志失败: {}", stdout_path.display()))?;
    let stderr = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&stderr_path)
        .with_context(|| format!("打开 stderr 日志失败: {}", stderr_path.display()))?;
    let exe = std::env::current_exe().context("定位 aios-database 可执行文件失败")?;
    let status = Command::new(exe)
        .arg("-c")
        .arg(config_no_ext)
        .args(&payload.args)
        .envs(&payload.env)
        .current_dir(cwd)
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .status()
        .await
        .context("等待 CLI 作业失败")?;
    Ok(RunCliJobResponse {
        success: status.success(),
        exit_code: status.code(),
    })
}

fn normalize_projects(
    projects: Vec<SidecarSiteProject>,
    fallback_name: &str,
    fallback_path: &str,
) -> Result<Vec<SidecarSiteProject>> {
    let mut normalized = Vec::new();
    for (idx, project) in projects.into_iter().enumerate() {
        let raw_path = project.path.trim();
        if raw_path.is_empty() {
            continue;
        }
        let canonical = canonical_project_path(raw_path)?;
        let name = if project.name.trim().is_empty() {
            canonical
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or(fallback_name)
                .to_string()
        } else {
            project.name.trim().to_string()
        };
        normalized.push(SidecarSiteProject {
            path: canonical.to_string_lossy().to_string(),
            name,
            role: project.role,
            is_primary: project.is_primary,
            sort_order: idx as u32,
        });
    }
    if normalized.is_empty() && !fallback_path.trim().is_empty() {
        let canonical = canonical_project_path(fallback_path)?;
        normalized.push(SidecarSiteProject {
            path: canonical.to_string_lossy().to_string(),
            name: fallback_name.to_string(),
            role: ProjectRole::Design,
            is_primary: true,
            sort_order: 0,
        });
    }
    Ok(normalized)
}

fn canonical_project_path(raw: &str) -> Result<PathBuf> {
    let path = PathBuf::from(raw);
    if path.as_os_str().is_empty() {
        bail!("项目路径不能为空");
    }
    fs::canonicalize(&path).with_context(|| format!("项目路径无法访问: {}", path.display()))
}

fn project_roots(projects: &[SidecarSiteProject]) -> Result<Vec<PathBuf>> {
    let mut roots = Vec::new();
    for project in projects {
        roots.push(canonical_project_path(&project.path)?);
    }
    Ok(roots)
}

fn primary_project_roots(projects: &[SidecarSiteProject]) -> Result<Vec<PathBuf>> {
    let roots = projects
        .iter()
        .filter(|project| project.is_primary || matches!(project.role, ProjectRole::Design))
        .map(|project| canonical_project_path(&project.path))
        .collect::<Result<Vec<_>>>()?;
    if roots.is_empty() {
        project_roots(projects)
    } else {
        Ok(roots)
    }
}

fn normalize_parse_db_types(values: Vec<String>) -> Vec<String> {
    let mut values = values
        .into_iter()
        .map(|value| value.trim().to_ascii_uppercase())
        .filter(|value| SUPPORTED_PARSE_DB_TYPES.contains(&value.as_str()))
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values
}

fn resolve_manual_db_nums<F>(
    manual_db_nums: Vec<u32>,
    manual_db_files: Vec<String>,
    mut resolve_db_file: F,
) -> Result<Vec<u32>>
where
    F: FnMut(&str) -> Result<u32>,
{
    let mut values = manual_db_nums;
    for db_file in manual_db_files {
        let db_file = db_file.trim();
        if db_file.is_empty() {
            continue;
        }
        values.push(resolve_db_file(db_file)?);
    }
    values.retain(|value| *value > 0);
    values.sort();
    values.dedup();
    Ok(values)
}

fn is_safe_scan_entry(entry: &fs::DirEntry) -> bool {
    let Ok(meta) = entry.metadata() else {
        return false;
    };
    if meta.file_type().is_symlink() {
        return false;
    }
    let name = entry.file_name();
    let name = name.to_string_lossy();
    !name.starts_with('.')
}

fn scan_db_file_name(
    root: &Path,
    target_dbnum: Option<u32>,
    target_types: Option<&[&str]>,
    depth: usize,
    visited: &mut usize,
    file_names: &mut Vec<String>,
) -> Result<bool> {
    if depth > SCAN_MAX_DEPTH {
        return Ok(false);
    }
    for entry in fs::read_dir(root)
        .with_context(|| format!("读取目录失败: {}", root.display()))?
        .flatten()
    {
        *visited += 1;
        if *visited > SCAN_MAX_FILES {
            bail!("项目路径扫描文件数超过 {SCAN_MAX_FILES} 上限，请缩小 project_path");
        }
        if !is_safe_scan_entry(&entry) {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            if scan_db_file_name(
                &path,
                target_dbnum,
                target_types,
                depth + 1,
                visited,
                file_names,
            )? {
                return Ok(true);
            }
            continue;
        }
        if !path.is_file() {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if target_types.is_some() && file_name.contains('.') {
            continue;
        }
        let mut file = match fs::File::open(&path) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let mut buf = [0u8; 60];
        if file.read_exact(&mut buf).is_err() {
            continue;
        }
        let db_info = parse_file_basic_info(&buf);
        if let Some(dbnum) = target_dbnum {
            if db_info.dbnum == dbnum {
                file_names.push(file_name.to_string());
                return Ok(true);
            }
        }
        if let Some(types) = target_types {
            if types.contains(&db_info.db_type.as_str()) {
                file_names.push(file_name.to_string());
            }
        }
    }
    Ok(false)
}

fn collect_db_file_names_for_types(
    root: &Path,
    target_types: &[&str],
    file_names: &mut Vec<String>,
) -> Result<()> {
    let mut visited = 0usize;
    scan_db_file_name(root, None, Some(target_types), 0, &mut visited, file_names)?;
    Ok(())
}

fn find_db_file_name_for_dbnum(root: &Path, target_dbnum: u32) -> Result<Option<String>> {
    let mut visited = 0usize;
    let mut file_names = Vec::with_capacity(1);
    scan_db_file_name(
        root,
        Some(target_dbnum),
        None,
        0,
        &mut visited,
        &mut file_names,
    )?;
    Ok(file_names.into_iter().next())
}

fn find_file_by_name(root: &Path, target: &str, depth: usize) -> Result<Option<PathBuf>> {
    if depth > SCAN_MAX_DEPTH {
        return Ok(None);
    }
    for entry in fs::read_dir(root)
        .with_context(|| format!("读取目录失败: {}", root.display()))?
        .flatten()
    {
        if !is_safe_scan_entry(&entry) {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_file_by_name(&path, target, depth + 1)? {
                return Ok(Some(found));
            }
            continue;
        }
        if path
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case(target))
        {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn resolve_dbnum_from_db_file(project_root: &Path, db_file: &str) -> Result<(u32, String)> {
    let project_root = canonical_project_path(&project_root.to_string_lossy())?;
    let direct = Path::new(db_file);
    let candidate = if direct.is_absolute() && direct.is_file() {
        fs::canonicalize(direct)
            .with_context(|| format!("项目文件无法访问: {}", direct.display()))?
    } else {
        let joined = project_root.join(db_file);
        if joined.is_file() {
            fs::canonicalize(joined)?
        } else {
            find_file_by_name(&project_root, db_file, 0)?
                .ok_or_else(|| anyhow!("项目路径下未找到 db_file={db_file}"))?
        }
    };
    if !candidate.starts_with(&project_root) {
        bail!(
            "db_file 不在工程路径下: {} (project_root={})",
            candidate.display(),
            project_root.display()
        );
    }
    let mut file = fs::File::open(&candidate)
        .with_context(|| format!("打开 db 文件失败: {}", candidate.display()))?;
    let mut buf = [0u8; 60];
    file.read_exact(&mut buf)
        .with_context(|| format!("读取 db 文件头失败: {}", candidate.display()))?;
    let info = parse_file_basic_info(&buf);
    if info.dbnum == 0 {
        bail!("无法从文件头解析 dbnum: {}", candidate.display());
    }
    let rel = candidate
        .strip_prefix(&project_root)
        .unwrap_or(&candidate)
        .to_string_lossy()
        .to_string();
    Ok((info.dbnum, rel))
}

fn resolve_dbnum_from_db_file_roots(
    project_roots: &[PathBuf],
    db_file: &str,
) -> Result<(u32, String)> {
    let mut errors = Vec::new();
    for root in project_roots {
        match resolve_dbnum_from_db_file(root, db_file) {
            Ok(found) => return Ok(found),
            Err(err) => errors.push(format!("{}: {err}", root.display())),
        }
    }
    bail!(
        "未能在任一工程路径下解析 db_file={}；{}",
        db_file,
        errors.join("；")
    )
}

fn resolve_db_info_from_db_file_roots(
    project_roots: &[PathBuf],
    db_file: &str,
) -> Result<(u32, String)> {
    let mut errors = Vec::new();
    for root in project_roots {
        let project_root = canonical_project_path(&root.to_string_lossy())?;
        let joined = project_root.join(db_file);
        let candidate = if joined.is_file() {
            fs::canonicalize(joined)?
        } else {
            find_file_by_name(&project_root, db_file, 0)?
                .ok_or_else(|| anyhow!("项目路径下未找到 db_file={db_file}"))?
        };
        if !candidate.starts_with(&project_root) {
            errors.push(format!(
                "{}: db_file 不在工程路径下: {}",
                root.display(),
                candidate.display()
            ));
            continue;
        }
        let mut file = match fs::File::open(&candidate) {
            Ok(file) => file,
            Err(err) => {
                errors.push(format!("{}: {err}", candidate.display()));
                continue;
            }
        };
        let mut buf = [0u8; 60];
        if let Err(err) = file.read_exact(&mut buf) {
            errors.push(format!("{}: {err}", candidate.display()));
            continue;
        }
        let info = parse_file_basic_info(&buf);
        if info.dbnum > 0 {
            return Ok((info.dbnum, info.db_type.trim().to_ascii_uppercase()));
        }
        errors.push(format!("{}: 无法从文件头解析 dbnum", candidate.display()));
    }
    bail!(
        "未能解析 db_file={} 的文件头；{}",
        db_file,
        errors.join("；")
    )
}

fn classify_parse_plan_source(
    dbnum: Option<u32>,
    db_type: Option<&str>,
    manual_db_nums: &[u32],
    parse_db_types: &[String],
    auto_parse_related_dbnums: bool,
) -> (&'static str, u32) {
    if dbnum.is_some_and(|value| manual_db_nums.contains(&value)) {
        return ("manual_db_num", 10);
    }
    if let Some(db_type) = db_type {
        if parse_db_types.iter().any(|value| value == db_type) {
            return ("parse_db_type", 30);
        }
        if MANDATORY_PREPARSE_DB_TYPES.contains(&db_type) {
            return ("mandatory_preparse", 20);
        }
        if REPARSE_REUSE_DB_TYPES.contains(&db_type) {
            return ("system_reuse", 25);
        }
        if auto_parse_related_dbnums && db_type == "CATA" {
            return ("auto_related", 40);
        }
    }
    ("sidecar_inferred", 90)
}

fn build_parse_plan_facts(
    project_roots: &[PathBuf],
    manual_db_nums: &[u32],
    parse_db_types: &[String],
    auto_parse_related_dbnums: bool,
    included_db_files: &[String],
) -> (Vec<ParsePlanFact>, Vec<String>) {
    let mut entries = Vec::with_capacity(included_db_files.len());
    let mut warnings = Vec::new();
    for file_name in included_db_files {
        match resolve_db_info_from_db_file_roots(project_roots, file_name) {
            Ok((dbnum, db_type)) => {
                let (source, priority) = classify_parse_plan_source(
                    Some(dbnum),
                    Some(&db_type),
                    manual_db_nums,
                    parse_db_types,
                    auto_parse_related_dbnums,
                );
                entries.push(ParsePlanFact {
                    file_name: file_name.clone(),
                    dbnum: Some(dbnum),
                    db_type: Some(db_type),
                    source: source.to_string(),
                    priority,
                });
            }
            Err(err) => {
                warnings.push(format!("无法读取解析目标事实 {file_name}: {err}"));
                entries.push(ParsePlanFact {
                    file_name: file_name.clone(),
                    dbnum: None,
                    db_type: None,
                    source: "sidecar_inferred".to_string(),
                    priority: 90,
                });
            }
        }
    }
    (entries, warnings)
}

fn resolve_included_db_files(
    project_roots: &[PathBuf],
    primary_project_roots: &[PathBuf],
    manual_db_nums: &[u32],
    parse_db_types: &[String],
    force_rebuild_system_db: bool,
    auto_parse_related_dbnums: bool,
    cata_partial_parse: bool,
) -> Result<Vec<String>> {
    let mut file_names = Vec::new();
    let has_manual_targets = !manual_db_nums.is_empty();
    let has_type_targets = !parse_db_types.is_empty();
    let include_reuse_types = force_rebuild_system_db || parse_db_types.iter().any(|v| v == "SYST");
    if include_reuse_types {
        for root in project_roots {
            collect_db_file_names_for_types(root, REPARSE_REUSE_DB_TYPES, &mut file_names)?;
        }
    }
    if has_manual_targets || has_type_targets {
        for root in project_roots {
            collect_db_file_names_for_types(root, MANDATORY_PREPARSE_DB_TYPES, &mut file_names)?;
        }
    }
    let include_desi_by_type =
        parse_db_types.iter().any(|value| value == "DESI") && manual_db_nums.is_empty();
    if include_desi_by_type {
        for root in primary_project_roots {
            collect_db_file_names_for_types(root, &["DESI"], &mut file_names)?;
        }
    }
    let extra_type_refs = parse_db_types
        .iter()
        .filter(|value| {
            value.as_str() != "DESI"
                && !REPARSE_REUSE_DB_TYPES.contains(&value.as_str())
                && !MANDATORY_PREPARSE_DB_TYPES.contains(&value.as_str())
        })
        .map(|value| value.as_str())
        .collect::<Vec<_>>();
    if !extra_type_refs.is_empty() {
        for root in project_roots {
            collect_db_file_names_for_types(root, &extra_type_refs, &mut file_names)?;
        }
    }
    for dbnum in manual_db_nums {
        let mut matched = None;
        for root in project_roots {
            if let Some(file_name) = find_db_file_name_for_dbnum(root, *dbnum)? {
                matched = Some(file_name);
                break;
            }
        }
        let file_name =
            matched.ok_or_else(|| anyhow!("项目路径下未找到 dbnum={} 对应的 db 文件", dbnum))?;
        file_names.push(file_name);
    }
    if auto_parse_related_dbnums && !cata_partial_parse {
        for root in project_roots {
            collect_db_file_names_for_types(root, &["CATA"], &mut file_names)?;
        }
    }
    file_names.sort();
    file_names.dedup();
    Ok(file_names)
}

fn build_parse_plan_with_files(
    manual_db_nums: &[u32],
    parse_db_types: &[String],
    force_rebuild_system_db: bool,
    included_db_files: Vec<String>,
    auto_related_db_files: Vec<String>,
    entries: Vec<ParsePlanFact>,
    warnings: Vec<String>,
) -> ManagedSiteParsePlan {
    let parse_type_summary = if parse_db_types.is_empty() {
        "未限制类型".to_string()
    } else {
        parse_db_types.join(" + ")
    };
    let target_summary = if included_db_files.is_empty() {
        "按项目配置全量解析".to_string()
    } else {
        included_db_files.join(", ")
    };
    let includes_system_db_files = included_db_files.iter().any(|file_name| {
        let lower = file_name.to_ascii_lowercase();
        lower.contains("syst") || lower.contains("sys")
    });
    if included_db_files.is_empty() && manual_db_nums.is_empty() && parse_db_types.is_empty() {
        return ManagedSiteParsePlan {
            mode: ManagedSiteParsePlanMode::Full,
            label: "全量解析".to_string(),
            detail: "当前没有限制 db 文件，解析时会按项目配置做全量解析。".to_string(),
            includes_system_db_files: true,
            included_db_files,
            auto_related_db_files,
            entries,
            warnings,
        };
    }
    if includes_system_db_files && force_rebuild_system_db {
        ManagedSiteParsePlan {
            mode: ManagedSiteParsePlanMode::RebuildSystem,
            label: "重建系统库".to_string(),
            detail: format!(
                "已勾选类型：{}。已开启强制重建系统库，本次会解析目标文件：{}。",
                parse_type_summary, target_summary
            ),
            includes_system_db_files,
            included_db_files,
            auto_related_db_files,
            entries,
            warnings,
        }
    } else if includes_system_db_files {
        ManagedSiteParsePlan {
            mode: ManagedSiteParsePlanMode::Bootstrap,
            label: "首次解析".to_string(),
            detail: format!(
                "已勾选类型：{}。本次会补齐系统数据，再解析目标文件：{}。",
                parse_type_summary, target_summary
            ),
            includes_system_db_files,
            included_db_files,
            auto_related_db_files,
            entries,
            warnings,
        }
    } else if !manual_db_nums.is_empty() || !included_db_files.is_empty() {
        ManagedSiteParsePlan {
            mode: ManagedSiteParsePlanMode::FastReparse,
            label: "快速重解析".to_string(),
            detail: format!(
                "已勾选类型：{}。本次复用已解析的 SYST，只解析当前目标：{}。",
                parse_type_summary, target_summary
            ),
            includes_system_db_files,
            included_db_files,
            auto_related_db_files,
            entries,
            warnings,
        }
    } else {
        ManagedSiteParsePlan {
            mode: ManagedSiteParsePlanMode::Selective,
            label: "按范围解析".to_string(),
            detail: format!(
                "已勾选类型：{}。本次按当前范围解析：{}。",
                parse_type_summary, target_summary
            ),
            includes_system_db_files,
            included_db_files,
            auto_related_db_files,
            entries,
            warnings,
        }
    }
}

fn collect_project_db_entries(
    root: &Path,
    depth: usize,
    visited: &mut usize,
    out: &mut Vec<(u32, String)>,
) -> Result<()> {
    if depth > SCAN_MAX_DEPTH {
        return Ok(());
    }
    for entry in fs::read_dir(root)
        .with_context(|| format!("读取目录失败: {}", root.display()))?
        .flatten()
    {
        *visited += 1;
        if *visited > SCAN_MAX_FILES {
            bail!("项目路径扫描文件数超过 {SCAN_MAX_FILES} 上限，请缩小工程路径");
        }
        if !is_safe_scan_entry(&entry) {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            collect_project_db_entries(&path, depth + 1, visited, out)?;
            continue;
        }
        if !path.is_file() {
            continue;
        }
        let mut file = match fs::File::open(&path) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let mut buf = [0u8; 60];
        if file.read_exact(&mut buf).is_err() {
            continue;
        }
        let db_info = parse_file_basic_info(&buf);
        if db_info.dbnum > 0 {
            out.push((db_info.dbnum, db_info.db_type.trim().to_ascii_uppercase()));
        }
    }
    Ok(())
}

fn infer_scanned_role(db_types: &HashSet<String>) -> ProjectRole {
    if db_types.contains("DESI") {
        ProjectRole::Design
    } else if db_types.contains("CATA") {
        ProjectRole::Library
    } else {
        ProjectRole::Design
    }
}

fn scan_projects_under_root(raw_root: &str) -> Result<ScanProjectsResult> {
    let root = canonical_project_path(raw_root.trim())?;
    let mut candidate_dirs = Vec::new();
    for entry in fs::read_dir(&root)
        .with_context(|| format!("读取目录失败: {}", root.display()))?
        .flatten()
    {
        if !is_safe_scan_entry(&entry) {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            let mut visited = 0usize;
            let mut entries = Vec::new();
            collect_project_db_entries(&path, 0, &mut visited, &mut entries)?;
            if !entries.is_empty() {
                candidate_dirs.push((path, entries));
            }
        }
    }
    if candidate_dirs.is_empty() {
        let mut visited = 0usize;
        let mut entries = Vec::new();
        collect_project_db_entries(&root, 0, &mut visited, &mut entries)?;
        if !entries.is_empty() {
            candidate_dirs.push((root.clone(), entries));
        }
    }
    let mut owners: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    let mut projects = Vec::new();
    for (dir, entries) in candidate_dirs {
        let canonical = fs::canonicalize(&dir).unwrap_or(dir);
        let name = match canonical
            .file_name()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
        {
            Some(name) => name.to_string(),
            None => canonical.to_string_lossy().to_string(),
        };
        let mut dbnum_set = BTreeSet::new();
        let mut type_set = HashSet::new();
        for (dbnum, db_type) in entries {
            dbnum_set.insert(dbnum);
            if !db_type.is_empty() {
                type_set.insert(db_type);
            }
        }
        for dbnum in &dbnum_set {
            let owner = owners.entry(*dbnum).or_default();
            if !owner.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
                owner.push(name.clone());
            }
        }
        let role = infer_scanned_role(&type_set);
        let mut db_types: Vec<String> = type_set.into_iter().collect();
        db_types.sort();
        projects.push(ScannedProject {
            path: canonical.to_string_lossy().to_string(),
            name,
            role,
            is_primary: false,
            sort_order: 0,
            dbnums: dbnum_set.into_iter().collect(),
            db_types,
        });
    }
    projects.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
    });
    let primary_idx = projects
        .iter()
        .position(|p| matches!(p.role, ProjectRole::Design))
        .or(if projects.is_empty() { None } else { Some(0) });
    for (idx, project) in projects.iter_mut().enumerate() {
        project.sort_order = idx as u32;
        project.is_primary = Some(idx) == primary_idx;
    }
    let conflicts = owners
        .into_iter()
        .filter(|(_, projects)| projects.len() > 1)
        .map(|(dbnum, projects)| ScannedDbnumConflict { dbnum, projects })
        .collect::<Vec<_>>();
    let has_conflict = !conflicts.is_empty();
    Ok(ScanProjectsResult {
        root: root.to_string_lossy().to_string(),
        projects,
        conflicts,
        has_conflict,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "sqlite-index")]
    #[test]
    fn filter_design_outbound_keeps_all_sources_when_manual_targets_empty() {
        let outbound = vec![(250160, vec![250193]), (250161, vec![250194])];

        let filtered = filter_design_outbound_for_manual_dbnums(outbound.clone(), &[]);

        assert_eq!(filtered, outbound);
    }

    #[cfg(feature = "sqlite-index")]
    #[test]
    fn filter_design_outbound_keeps_only_manual_target_sources() {
        let outbound = vec![
            (250160, vec![7015, 250193]),
            (250161, vec![250194]),
            (250162, vec![250195]),
        ];

        let filtered = filter_design_outbound_for_manual_dbnums(outbound, &[250160, 250162]);

        assert_eq!(
            filtered,
            vec![(250160, vec![7015, 250193]), (250162, vec![250195])]
        );
    }

    #[cfg(feature = "sqlite-index")]
    #[test]
    fn append_related_cata_from_db_index_adds_only_cata_dependencies() {
        let dir = tempfile::tempdir().expect("tempdir");
        let index_path = dir
            .path()
            .join(crate::data_interface::db_index::DB_INDEX_FILE_NAME);
        let store = crate::data_interface::db_index::DbIndexStore::open(&index_path)
            .expect("db index store");
        store
            .upsert_db_file(&db_file_record(250160, "DESI", "aps250160_0001"))
            .expect("desi file");
        store
            .upsert_db_file(&db_file_record(250193, "CATA", "aps250193_0001"))
            .expect("cata file");
        store
            .upsert_db_file(&db_file_record(250206, "DICT", "aps250206_0001"))
            .expect("dict file");
        store
            .record_dependencies(250160, &[250193, 250206])
            .expect("dependencies");

        let path_text = index_path.to_string_lossy().to_string();
        let mut included = vec!["aps250160_0001".to_string()];
        let mut warnings = Vec::new();
        append_related_cata_from_db_index(
            &mut included,
            Some(&path_text),
            &[250160],
            true,
            true,
            &mut warnings,
        );

        assert_eq!(included, vec!["aps250160_0001", "aps250193_0001"]);
        assert!(warnings.is_empty());
    }

    #[cfg(feature = "sqlite-index")]
    fn db_file_record(
        dbnum: u32,
        db_type: &str,
        file_name: &str,
    ) -> crate::data_interface::db_index::DbFileRecord {
        crate::data_interface::db_index::DbFileRecord {
            dbnum,
            db_type: db_type.to_string(),
            file_name: file_name.to_string(),
            file_path: file_name.to_string(),
            project: "TestProject".to_string(),
            latest_sesno: 1,
            fingerprint: format!("{dbnum}:1"),
        }
    }
}
