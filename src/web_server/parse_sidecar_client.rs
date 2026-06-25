use std::{
    collections::HashMap,
    net::TcpListener,
    path::{Path, PathBuf},
    process::Stdio,
    sync::OnceLock,
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use axum::http::{HeaderValue, StatusCode, header};
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
use tokio::{
    process::Command,
    sync::{Mutex, mpsc},
};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use uuid::Uuid;

use crate::web_server::models::PreviewManagedSiteParsePlanRequest;

const SIDECAR_HOST: &str = "127.0.0.1";
const SIDECAR_HEALTH_ATTEMPTS: usize = 40;
const SIDECAR_HEALTH_DELAY_MS: u64 = 100;
const DEFAULT_JOB_SIDECAR_SHUTDOWN_DELAY_MS: u64 = 10_000;
const JOB_SIDECAR_SHUTDOWN_DELAY_ENV: &str = "ADMIN_SIDECAR_JOB_SHUTDOWN_DELAY_MS";
const SIDECAR_KILL_GRACE_MS: u64 = 1500;
const SURREAL_CONN_ENV_KEYS: &[&str] =
    &["SURREAL_CONN_MODE", "SURREAL_CONN_IP", "SURREAL_CONN_PORT"];

#[derive(Debug, Clone)]
struct SidecarHandle {
    base_url: String,
    token: String,
    pid: u32,
    start_token: Option<u64>,
}

#[derive(Debug)]
pub struct SidecarProxyError {
    pub status: StatusCode,
    pub message: String,
    pub body: Value,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct DbFileResolveResponse {
    pub dbnum: u32,
    pub file_name: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct RunCliJobResponse {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub job_id: String,
}

#[derive(Debug, Clone)]
pub struct RunCliJobStatus {
    pub status: String,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Deserialize)]
struct SubmitCliJobResponse {
    job_id: String,
}

#[derive(Debug, Clone, Deserialize)]
struct SidecarJobRecord {
    status: String,
    exit_code: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DbIndexRoot {
    pub name: String,
    pub path: String,
}

fn sidecars() -> &'static Mutex<HashMap<String, SidecarHandle>> {
    static SIDECARS: OnceLock<Mutex<HashMap<String, SidecarHandle>>> = OnceLock::new();
    SIDECARS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn job_sidecar_shutdown_delay_ms() -> u64 {
    std::env::var(JOB_SIDECAR_SHUTDOWN_DELAY_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_JOB_SIDECAR_SHUTDOWN_DELAY_MS)
}

pub async fn preview_parse_plan(
    payload: PreviewManagedSiteParsePlanRequest,
) -> Result<Value, SidecarProxyError> {
    let key = preview_sidecar_key(&payload);
    let handle = ensure_sidecar(&key).await.map_err(internal_proxy_error)?;
    post_sidecar(&handle, "/parse/preview-plan", &payload).await
}

pub async fn scan_projects(root: &str) -> Result<Value, SidecarProxyError> {
    let key = format!("scan:{}", stable_key(root));
    let handle = ensure_sidecar(&key).await.map_err(internal_proxy_error)?;
    post_sidecar(&handle, "/projects/scan", &json!({ "root": root })).await
}

/// MBD 候选发现（只读）：有 site_id 时复用站点 sidecar，否则按工程组成派生独立 key。
pub async fn mdb_candidates(
    payload: crate::web_server::models::MdbCandidatesRequest,
) -> Result<Value, SidecarProxyError> {
    let key = mdb_candidates_sidecar_key(&payload);
    let handle = ensure_sidecar(&key).await.map_err(internal_proxy_error)?;
    post_sidecar(&handle, "/projects/mdb-candidates", &payload).await
}

fn mdb_candidates_sidecar_key(payload: &crate::web_server::models::MdbCandidatesRequest) -> String {
    if let Some(site_id) = payload
        .site_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return format!("site:{site_id}");
    }
    let mut material = payload.project_name.clone();
    material.push('|');
    material.push_str(&payload.project_path);
    for project in &payload.projects {
        material.push('|');
        material.push_str(&project.path);
        material.push(':');
        material.push_str(&project.name);
    }
    format!("mdb:{}", stable_key(&material))
}

pub async fn resolve_db_file(
    project_roots: Vec<String>,
    db_file: String,
) -> Result<DbFileResolveResponse, SidecarProxyError> {
    let key = format!("resolve:{}", stable_key(&project_roots.join("|")));
    let handle = ensure_sidecar(&key).await.map_err(internal_proxy_error)?;
    let value = post_sidecar(
        &handle,
        "/db-files/resolve",
        &json!({
            "project_roots": project_roots,
            "db_file": db_file,
        }),
    )
    .await?;
    serde_json::from_value(value).map_err(internal_proxy_error)
}

pub async fn rebuild_db_index(
    key: &str,
    roots: Vec<DbIndexRoot>,
    index_path: String,
    force: bool,
    manual_db_nums: Vec<u32>,
) -> Result<Value, SidecarProxyError> {
    let handle = ensure_sidecar(&format!("db-index:{}", stable_key(key)))
        .await
        .map_err(internal_proxy_error)?;
    post_sidecar_with_client(
        &handle,
        "/db-index/rebuild",
        &json!({
            "roots": roots,
            "index_path": index_path,
            "force": force,
            "manual_db_nums": manual_db_nums,
        }),
        sidecar_job_http_client().map_err(internal_proxy_error)?,
    )
    .await
}

pub async fn run_cli_job(
    key: &str,
    config_no_ext: String,
    cwd: String,
    stdout_path: String,
    stderr_path: String,
) -> Result<RunCliJobResponse, SidecarProxyError> {
    run_cli_job_with_status(
        key,
        config_no_ext,
        cwd,
        stdout_path,
        stderr_path,
        Vec::new(),
        HashMap::new(),
        |_, _| {},
    )
    .await
}

pub async fn cancel_cli_job(key: &str, job_id: &str) -> Result<Value, SidecarProxyError> {
    let handle = ensure_sidecar(&cli_job_sidecar_key(key))
        .await
        .map_err(internal_proxy_error)?;
    post_sidecar_with_client(
        &handle,
        &format!("/jobs/{job_id}/cancel"),
        &json!({}),
        sidecar_http_client().map_err(internal_proxy_error)?,
    )
    .await
}

pub fn subscribe_cli_job_events(key: String, job_id: String) -> mpsc::UnboundedReceiver<Value> {
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        if let Err(err) = stream_cli_job_events(&key, &job_id, tx).await {
            tracing::warn!(job_id, "sidecar job event stream ended: {err}");
        }
    });
    rx
}

pub async fn run_cli_job_with_status<F>(
    key: &str,
    config_no_ext: String,
    cwd: String,
    stdout_path: String,
    stderr_path: String,
    args: Vec<String>,
    env: HashMap<String, String>,
    mut on_status: F,
) -> Result<RunCliJobResponse, SidecarProxyError>
where
    F: FnMut(&str, &RunCliJobStatus),
{
    let handle = ensure_sidecar(&cli_job_sidecar_key(key))
        .await
        .map_err(internal_proxy_error)?;
    let client = sidecar_job_http_client().map_err(internal_proxy_error)?;
    let value = post_sidecar_with_client(
        &handle,
        "/jobs/submit-cli",
        &json!({
            "config_no_ext": config_no_ext,
            "cwd": cwd,
            "stdout_path": stdout_path,
            "stderr_path": stderr_path,
            "args": args,
            "env": env,
        }),
        client.clone(),
    )
    .await?;
    let submitted =
        serde_json::from_value::<SubmitCliJobResponse>(value).map_err(internal_proxy_error)?;
    on_status(
        &submitted.job_id,
        &RunCliJobStatus {
            status: "submitted".to_string(),
            exit_code: None,
        },
    );
    let mut last_status = Some("submitted".to_string());
    loop {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let value = get_sidecar_with_client(
            &handle,
            &format!("/jobs/{}", submitted.job_id),
            client.clone(),
        )
        .await?;
        let record =
            serde_json::from_value::<SidecarJobRecord>(value).map_err(internal_proxy_error)?;
        if last_status.as_deref() != Some(record.status.as_str()) {
            on_status(
                &submitted.job_id,
                &RunCliJobStatus {
                    status: record.status.clone(),
                    exit_code: record.exit_code,
                },
            );
            last_status = Some(record.status.clone());
        }
        match record.status.as_str() {
            "succeeded" => {
                forget_cli_job_sidecar(key).await;
                return Ok(RunCliJobResponse {
                    success: true,
                    exit_code: record.exit_code,
                    job_id: submitted.job_id,
                });
            }
            "failed" | "cancelled" => {
                forget_cli_job_sidecar(key).await;
                return Ok(RunCliJobResponse {
                    success: false,
                    exit_code: record.exit_code,
                    job_id: submitted.job_id,
                });
            }
            _ => continue,
        }
    }
}

async fn stream_cli_job_events(
    key: &str,
    job_id: &str,
    tx: mpsc::UnboundedSender<Value>,
) -> Result<()> {
    let handle = ensure_sidecar(&cli_job_sidecar_key(key)).await?;
    let ws_url = handle
        .base_url
        .replacen("http://", "ws://", 1)
        .replacen("https://", "wss://", 1)
        + "/events";
    let mut request = ws_url
        .into_client_request()
        .context("创建 sidecar events WebSocket request 失败")?;
    request.headers_mut().insert(
        header::AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", handle.token))
            .context("创建 sidecar events Authorization header 失败")?,
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(request)
        .await
        .context("连接 sidecar events WebSocket 失败")?;
    while let Some(message) = socket.next().await {
        let message = message.context("读取 sidecar events WebSocket 消息失败")?;
        if !message.is_text() {
            continue;
        }
        let value = serde_json::from_str::<Value>(
            message
                .to_text()
                .context("读取 sidecar events 文本消息失败")?,
        )
        .context("解析 sidecar events JSON 失败")?;
        if !event_matches_job(&value, job_id) {
            continue;
        }
        let event_type = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if tx.send(value).is_err() {
            break;
        }
        if matches!(
            event_type.as_str(),
            "job_done" | "job_failed" | "job_cancelled"
        ) {
            break;
        }
    }
    Ok(())
}

fn event_matches_job(value: &Value, job_id: &str) -> bool {
    value
        .get("job_id")
        .and_then(Value::as_str)
        .map(|value| value == job_id)
        .unwrap_or(false)
        || value
            .get("job")
            .and_then(|job| job.get("job_id"))
            .and_then(Value::as_str)
            .map(|value| value == job_id)
            .unwrap_or(false)
}

fn preview_sidecar_key(payload: &PreviewManagedSiteParsePlanRequest) -> String {
    if let Some(site_id) = payload
        .site_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return format!("site:{site_id}");
    }
    let mut material = payload.project_name.clone();
    material.push('|');
    material.push_str(&payload.project_path);
    material.push('|');
    material.push_str(&payload.web_port.to_string());
    for project in &payload.projects {
        material.push('|');
        material.push_str(&project.path);
        material.push(':');
        material.push_str(&project.name);
    }
    format!("preview:{}", stable_key(&material))
}

fn cli_job_sidecar_key(key: &str) -> String {
    format!("job:{}", stable_key(key))
}

pub async fn forget_cli_job_sidecar(key: &str) {
    let mut guard = sidecars().lock().await;
    guard.remove(&cli_job_sidecar_key(key));
}

pub async fn shutdown_site_sidecars(site_id: &str) -> usize {
    let site_key = format!("site:{site_id}");
    let parse_key = cli_job_sidecar_key(&format!("parse:{site_id}"));
    let generate_key = cli_job_sidecar_key(&format!("generate:{site_id}"));
    let mut killed = shutdown_sidecars_by_keys(&[site_key.clone(), parse_key, generate_key]).await;
    killed += shutdown_orphan_site_sidecars(&site_key).await;
    // 顺带回收本实例归属根下“属主已死”的全类型孤儿（db-index/resolve/scan/preview/mdb 等）。
    killed += reap_dead_owner_sidecars().await;
    killed
}

async fn shutdown_sidecars_by_keys(keys: &[String]) -> usize {
    let handles = {
        let mut guard = sidecars().lock().await;
        keys.iter()
            .filter_map(|key| guard.remove(key))
            .collect::<Vec<_>>()
    };
    let mut killed = 0usize;
    for handle in handles {
        if kill_handle_process_tree(&handle).await {
            killed += 1;
        }
    }
    killed
}

async fn shutdown_orphan_site_sidecars(site_key: &str) -> usize {
    let handles = find_orphan_site_sidecars(site_key);
    let mut killed = 0usize;
    for handle in handles {
        if kill_handle_process_tree(&handle).await {
            killed += 1;
        }
    }
    killed
}

fn find_orphan_site_sidecars(site_key: &str) -> Vec<SidecarHandle> {
    let system = System::new_all();
    system
        .processes()
        .iter()
        .filter_map(|(pid, process)| {
            if !is_site_sidecar_command(process.cmd(), site_key) {
                return None;
            }
            let pid = pid.as_u32();
            Some(SidecarHandle {
                base_url: String::new(),
                token: String::new(),
                pid,
                start_token: process_start_token(pid),
            })
        })
        .collect()
}

fn is_site_sidecar_command(cmd: &[std::ffi::OsString], site_key: &str) -> bool {
    let args = cmd
        .iter()
        .map(|part| part.to_string_lossy())
        .collect::<Vec<_>>();
    let has_serve = args.iter().any(|arg| arg.as_ref() == "serve");
    let has_site_key = args
        .windows(2)
        .any(|pair| pair[0].as_ref() == "--site-key" && pair[1].as_ref() == site_key);
    has_serve && has_site_key
}

fn stable_key(value: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    hex::encode(&hasher.finalize()[..8])
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SidecarOwnerMarker {
    owner_pid: u32,
    owner_start_token: Option<u64>,
    sidecar_pid: u32,
    sidecar_start_token: Option<u64>,
    bind_port: u16,
    key: String,
    created_at: String,
}

/// 本 web_server 实例的 sidecar 归属根：`<cwd>/runtime/admin_sidecars`。
/// 所有清理动作都只针对 `--runtime-dir` 落在此根下的 sidecar，避免误杀其它仓库/release。
fn admin_sidecars_root() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_default()
        .join("runtime")
        .join("admin_sidecars")
}

fn write_sidecar_owner_marker(runtime_dir: &Path, key: &str, port: u16, handle: &SidecarHandle) {
    let marker = SidecarOwnerMarker {
        owner_pid: std::process::id(),
        owner_start_token: process_start_token(std::process::id()),
        sidecar_pid: handle.pid,
        sidecar_start_token: handle.start_token,
        bind_port: port,
        key: key.to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    if let Ok(json) = serde_json::to_vec_pretty(&marker) {
        let _ = std::fs::write(runtime_dir.join("owner.json"), json);
    }
}

fn normalize_path_str(path: &Path) -> String {
    let value = path.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        value.to_lowercase()
    } else {
        value
    }
}

fn path_is_under_normalized_root(path: &Path, root_norm: &str) -> bool {
    let path_norm = normalize_path_str(path);
    if path_norm == root_norm {
        return true;
    }
    let root_prefix = if root_norm.ends_with('/') {
        root_norm.to_string()
    } else {
        format!("{root_norm}/")
    };
    path_norm.starts_with(&root_prefix)
}

fn process_is_aios_database(process: &sysinfo::Process) -> bool {
    if let Some(name) = process
        .exe()
        .and_then(|exe| exe.file_name())
        .and_then(|name| name.to_str())
    {
        if name.eq_ignore_ascii_case(aios_database_exe_name()) {
            return true;
        }
    }
    let name = process.name().to_string_lossy();
    name.eq_ignore_ascii_case(aios_database_exe_name())
        || name.eq_ignore_ascii_case("aios-database")
}

/// 扫描进程表，找出 `aios-database serve` 且 `--runtime-dir` 落在本实例归属根下的 sidecar。
fn find_owned_serve_sidecars(root: &Path) -> Vec<SidecarHandle> {
    let root_norm = normalize_path_str(root);
    let system = System::new_all();
    system
        .processes()
        .iter()
        .filter_map(|(pid, process)| {
            if !process_is_aios_database(process) {
                return None;
            }
            let args = process
                .cmd()
                .iter()
                .map(|part| part.to_string_lossy().to_string())
                .collect::<Vec<_>>();
            if !args.iter().any(|arg| arg == "serve") {
                return None;
            }
            let runtime_dir = args
                .windows(2)
                .find_map(|pair| (pair[0] == "--runtime-dir").then(|| pair[1].clone()))?;
            if !path_is_under_normalized_root(Path::new(&runtime_dir), &root_norm) {
                return None;
            }
            let pid = pid.as_u32();
            Some(SidecarHandle {
                base_url: String::new(),
                token: String::new(),
                pid,
                start_token: process_start_token(pid),
            })
        })
        .collect()
}

/// 启动期 reaper：清理上一轮本实例残留的 `aios-database serve`。
///
/// 新实例启动时内存注册表必为空，因此归属根下任何仍在运行的 serve sidecar
/// 都是上一轮残留，可安全终止（杀前校验 PID + start-token）。
pub async fn reap_orphan_sidecars_on_startup() -> usize {
    let root = admin_sidecars_root();
    let handles = find_owned_serve_sidecars(&root);
    let scanned = handles.len();
    let mut killed = 0usize;
    for handle in handles {
        if kill_handle_process_tree(&handle).await {
            killed += 1;
        }
    }
    if scanned > 0 {
        tracing::warn!(
            phase = "startup",
            scope_root = %root.display(),
            scanned,
            killed,
            "sidecar reaper: 清理上一轮残留 aios-database serve"
        );
    }
    killed
}

fn read_owner_marker(runtime_dir: &Path) -> Option<SidecarOwnerMarker> {
    let bytes = std::fs::read(runtime_dir.join("owner.json")).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// 属主 web_server 是否仍存活（PID + start-token 双重校验，规避 PID 复用）。
fn owner_process_alive(owner_pid: u32, owner_start_token: Option<u64>) -> bool {
    if owner_pid == 0 {
        return false;
    }
    match (owner_start_token, process_start_token(owner_pid)) {
        (Some(expected), Some(actual)) => expected == actual,
        (None, Some(_)) => true,
        _ => false,
    }
}

/// 回收“属主已死”的孤儿 sidecar：覆盖本实例归属根下的所有 key 类型
/// （site/job/db-index/resolve/scan/preview/mdb）。
///
/// 通过 owner.json 判定属主存活：属主仍在运行的 sidecar 一律跳过，
/// 因此不会误杀其它存活实例正在使用的共享 sidecar。
async fn reap_dead_owner_sidecars() -> usize {
    let root = admin_sidecars_root();
    let root_norm = normalize_path_str(&root);
    let candidates: Vec<(SidecarHandle, PathBuf)> = {
        let system = System::new_all();
        system
            .processes()
            .iter()
            .filter_map(|(pid, process)| {
                if !process_is_aios_database(process) {
                    return None;
                }
                let args = process
                    .cmd()
                    .iter()
                    .map(|part| part.to_string_lossy().to_string())
                    .collect::<Vec<_>>();
                if !args.iter().any(|arg| arg == "serve") {
                    return None;
                }
                let runtime_dir = args
                    .windows(2)
                    .find_map(|pair| (pair[0] == "--runtime-dir").then(|| pair[1].clone()))?;
                if !path_is_under_normalized_root(Path::new(&runtime_dir), &root_norm) {
                    return None;
                }
                let pid = pid.as_u32();
                Some((
                    SidecarHandle {
                        base_url: String::new(),
                        token: String::new(),
                        pid,
                        start_token: process_start_token(pid),
                    },
                    PathBuf::from(runtime_dir),
                ))
            })
            .collect()
    };
    let mut killed = 0usize;
    for (handle, runtime_dir) in candidates {
        let owner_alive = match read_owner_marker(&runtime_dir) {
            Some(marker) => owner_process_alive(marker.owner_pid, marker.owner_start_token),
            // 无 owner.json：旧版/未知来源的归属根孤儿，按孤儿回收。
            None => false,
        };
        if owner_alive {
            continue;
        }
        if kill_handle_process_tree(&handle).await {
            killed += 1;
        }
    }
    if killed > 0 {
        tracing::warn!(
            phase = "dead-owner",
            scope_root = %root.display(),
            killed,
            "sidecar reaper: 回收无存活属主的孤儿"
        );
    }
    killed
}

/// 退出期回收：尽力终止内存注册表中的全部 sidecar（覆盖所有 key 类型）。
pub async fn shutdown_all_sidecars() -> usize {
    let handles = {
        let mut guard = sidecars().lock().await;
        guard.drain().map(|(_, handle)| handle).collect::<Vec<_>>()
    };
    let mut killed = 0usize;
    for handle in handles {
        if kill_handle_process_tree(&handle).await {
            killed += 1;
        }
    }
    if killed > 0 {
        tracing::warn!(phase = "shutdown", killed, "sidecar reaper: 退出期回收");
    }
    killed
}

/// 让 sidecar 在父进程（web_server）死亡时被 OS 一并带走。
/// Unix 走 `PR_SET_PDEATHSIG`；Windows 由 spawn 后的 Job Object 绑定负责（见 `assign_sidecar_to_job`）。
fn bind_sidecar_parent_death(command: &mut Command) {
    #[cfg(unix)]
    {
        // SAFETY: pre_exec 在 fork 出的子进程内、exec 之前执行；prctl 设置父死信号。
        unsafe {
            command.pre_exec(|| {
                let rc = libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL as libc::c_ulong);
                if rc != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    #[cfg(not(unix))]
    {
        let _ = command;
    }
}

/// Windows：把 sidecar 子进程加入本实例的 Job Object（KILL_ON_JOB_CLOSE）。
fn assign_sidecar_to_job(child: &tokio::process::Child) {
    #[cfg(windows)]
    {
        if let Some(handle) = child.raw_handle() {
            win_job::assign_current_job(handle);
        }
    }
    #[cfg(not(windows))]
    {
        let _ = child;
    }
}

async fn ensure_sidecar(key: &str) -> Result<SidecarHandle> {
    {
        let guard = sidecars().lock().await;
        if let Some(handle) = guard.get(key) {
            if sidecar_healthy(handle).await {
                return Ok(handle.clone());
            }
        }
    }

    let handle = spawn_sidecar(key).await?;
    let mut guard = sidecars().lock().await;
    guard.insert(key.to_string(), handle.clone());
    Ok(handle)
}

async fn spawn_sidecar(key: &str) -> Result<SidecarHandle> {
    let port = allocate_local_port()?;
    let token = Uuid::new_v4().to_string();
    let runtime_dir = sidecar_runtime_dir(key)?;
    std::fs::create_dir_all(&runtime_dir)
        .with_context(|| format!("创建 sidecar runtime 目录失败: {}", runtime_dir.display()))?;

    let mut command = aios_database_command()?;
    command
        .arg("serve")
        .arg("--site-key")
        .arg(key)
        .arg("--bind-host")
        .arg(SIDECAR_HOST)
        .arg("--http-port")
        .arg(port.to_string())
        .arg("--runtime-dir")
        .arg(&runtime_dir)
        .arg("--token")
        .arg(&token)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for key in SURREAL_CONN_ENV_KEYS {
        command.env_remove(key);
    }
    if key.starts_with("job:") {
        command
            .arg("--shutdown-after-job")
            .arg("--shutdown-delay-ms")
            .arg(job_sidecar_shutdown_delay_ms().to_string());
    }

    isolate_sidecar_process_group(&mut command);
    bind_sidecar_parent_death(&mut command);
    let child = command.spawn().context("启动 aios-database sidecar 失败")?;
    assign_sidecar_to_job(&child);
    let pid = child.id().unwrap_or_default();
    let handle = SidecarHandle {
        base_url: format!("http://{SIDECAR_HOST}:{port}"),
        token,
        pid,
        start_token: process_start_token(pid),
    };
    write_sidecar_owner_marker(&runtime_dir, key, port, &handle);
    wait_for_sidecar_health(&handle).await?;
    Ok(handle)
}

fn isolate_sidecar_process_group(command: &mut Command) {
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

fn process_start_token(pid: u32) -> Option<u64> {
    if pid == 0 {
        return None;
    }
    let target = Pid::from_u32(pid);
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[target]),
        true,
        ProcessRefreshKind::nothing(),
    );
    system.process(target).map(|proc| proc.start_time())
}

fn same_sidecar_process(handle: &SidecarHandle) -> bool {
    if handle.pid == 0 {
        return false;
    }
    match (handle.start_token, process_start_token(handle.pid)) {
        (Some(expected), Some(actual)) => expected == actual,
        (None, Some(_)) => true,
        _ => false,
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

async fn kill_handle_process_tree(handle: &SidecarHandle) -> bool {
    if !same_sidecar_process(handle) {
        return false;
    }
    #[cfg(unix)]
    {
        if !killpg_group(handle.pid, libc::SIGTERM) {
            unsafe {
                libc::kill(handle.pid as libc::pid_t, libc::SIGTERM);
            }
        }
        tokio::time::sleep(Duration::from_millis(SIDECAR_KILL_GRACE_MS)).await;
        if same_sidecar_process(handle) {
            if !killpg_group(handle.pid, libc::SIGKILL) {
                unsafe {
                    libc::kill(handle.pid as libc::pid_t, libc::SIGKILL);
                }
            }
        }
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &handle.pid.to_string(), "/T"])
            .output()
            .await;
        tokio::time::sleep(Duration::from_millis(SIDECAR_KILL_GRACE_MS)).await;
        if same_sidecar_process(handle) {
            let _ = Command::new("taskkill")
                .args(["/PID", &handle.pid.to_string(), "/T", "/F"])
                .output()
                .await;
        }
    }
    true
}

fn allocate_local_port() -> Result<u16> {
    let listener = TcpListener::bind((SIDECAR_HOST, 0)).context("分配 sidecar 本地端口失败")?;
    Ok(listener.local_addr()?.port())
}

fn sidecar_runtime_dir(key: &str) -> Result<PathBuf> {
    let safe = key
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>();
    Ok(std::env::current_dir()
        .context("获取当前工作目录失败")?
        .join("runtime")
        .join("admin_sidecars")
        .join(safe))
}

fn aios_database_command() -> Result<Command> {
    if let Ok(path) = std::env::var("ADMIN_AIOS_DATABASE_BINARY") {
        let path = PathBuf::from(path.trim());
        if path.exists() {
            return Ok(Command::new(path));
        }
        bail!(
            "ADMIN_AIOS_DATABASE_BINARY 指向的文件不存在: {}",
            path.display()
        );
    }

    let current = std::env::current_exe().context("获取当前 web_server 可执行文件失败")?;
    let parent = current
        .parent()
        .ok_or_else(|| anyhow!("无法定位当前 web_server 所在目录"))?;
    let sibling = parent.join(aios_database_exe_name());
    if sibling.exists() {
        return Ok(Command::new(sibling));
    }

    let repo = std::env::current_dir().context("获取当前工作目录失败")?;
    for candidate in [
        repo.join("target")
            .join("debug")
            .join(aios_database_exe_name()),
        repo.join("target")
            .join("release")
            .join(aios_database_exe_name()),
    ] {
        if candidate.exists() {
            return Ok(Command::new(candidate));
        }
    }

    if admin_allow_cargo_fallback() {
        let mut command = Command::new("cargo");
        command
            .arg("run")
            .arg("--features")
            .arg("web_server")
            .arg("--bin")
            .arg("aios-database")
            .arg("--");
        return Ok(command);
    }

    bail!(
        "未找到 aios-database 二进制；请配置 ADMIN_AIOS_DATABASE_BINARY 或设置 ADMIN_ALLOW_CARGO_RUN=1"
    )
}

fn aios_database_exe_name() -> &'static str {
    if cfg!(windows) {
        "aios-database.exe"
    } else {
        "aios-database"
    }
}

fn admin_allow_cargo_fallback() -> bool {
    std::env::var("ADMIN_ALLOW_CARGO_RUN")
        .map(|value| matches!(value.trim(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

async fn wait_for_sidecar_health(handle: &SidecarHandle) -> Result<()> {
    for _ in 0..SIDECAR_HEALTH_ATTEMPTS {
        if sidecar_healthy(handle).await {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(SIDECAR_HEALTH_DELAY_MS)).await;
    }
    bail!(
        "等待 aios-database sidecar 健康检查超时: {}",
        handle.base_url
    )
}

async fn sidecar_healthy(handle: &SidecarHandle) -> bool {
    let Ok(client) = sidecar_http_client() else {
        return false;
    };
    client
        .get(format!("{}/health", handle.base_url))
        .bearer_auth(&handle.token)
        .send()
        .await
        .map(|response| response.status().is_success())
        .unwrap_or(false)
}

async fn post_sidecar<T>(
    handle: &SidecarHandle,
    path: &str,
    payload: &T,
) -> Result<Value, SidecarProxyError>
where
    T: Serialize + ?Sized,
{
    let client = sidecar_http_client().map_err(internal_proxy_error)?;
    post_sidecar_with_client(handle, path, payload, client).await
}

async fn post_sidecar_with_client<T>(
    handle: &SidecarHandle,
    path: &str,
    payload: &T,
    client: Client,
) -> Result<Value, SidecarProxyError>
where
    T: Serialize + ?Sized,
{
    let response = client
        .post(format!("{}{}", handle.base_url, path))
        .bearer_auth(&handle.token)
        .json(payload)
        .send()
        .await
        .map_err(internal_proxy_error)?;
    let status = StatusCode::from_u16(response.status().as_u16())
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let value = response
        .json::<Value>()
        .await
        .map_err(internal_proxy_error)?;
    if status.is_success() {
        Ok(value.get("data").cloned().unwrap_or(Value::Null))
    } else {
        Err(SidecarProxyError {
            status,
            message: envelope_message(&value),
            body: value,
        })
    }
}

async fn get_sidecar_with_client(
    handle: &SidecarHandle,
    path: &str,
    client: Client,
) -> Result<Value, SidecarProxyError> {
    let response = client
        .get(format!("{}{}", handle.base_url, path))
        .bearer_auth(&handle.token)
        .send()
        .await
        .map_err(internal_proxy_error)?;
    let status = StatusCode::from_u16(response.status().as_u16())
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let value = response
        .json::<Value>()
        .await
        .map_err(internal_proxy_error)?;
    if status.is_success() {
        Ok(value.get("data").cloned().unwrap_or(Value::Null))
    } else {
        Err(SidecarProxyError {
            status,
            message: envelope_message(&value),
            body: value,
        })
    }
}

fn sidecar_http_client() -> Result<Client> {
    Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(30))
        .build()
        .context("创建 sidecar HTTP client 失败")
}

fn sidecar_job_http_client() -> Result<Client> {
    Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(24 * 60 * 60))
        .build()
        .context("创建 sidecar job HTTP client 失败")
}

fn envelope_message(value: &Value) -> String {
    value
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .or_else(|| value.get("message").and_then(Value::as_str))
        .unwrap_or("aios-database sidecar 请求失败")
        .to_string()
}

fn internal_proxy_error(err: impl std::fmt::Display) -> SidecarProxyError {
    SidecarProxyError {
        status: StatusCode::SERVICE_UNAVAILABLE,
        message: err.to_string(),
        body: json!({
            "success": false,
            "message": err.to_string(),
            "error": {
                "code": "SIDECAR_UNAVAILABLE",
                "message": err.to_string(),
                "retryable": true
            }
        }),
    }
}

/// Windows Job Object 绑定：本实例创建一个 `KILL_ON_JOB_CLOSE` 的 Job，
/// 每个 sidecar 子进程加入该 Job；web_server 进程退出（含崩溃/强杀）时
/// Job 句柄随之关闭，OS 自动终止全部已分配 sidecar。
#[cfg(windows)]
mod win_job {
    use std::os::windows::io::RawHandle;
    use std::sync::OnceLock;

    type Handle = *mut core::ffi::c_void;

    #[repr(C)]
    struct JobObjectBasicLimitInformation {
        per_process_user_time_limit: i64,
        per_job_user_time_limit: i64,
        limit_flags: u32,
        minimum_working_set_size: usize,
        maximum_working_set_size: usize,
        active_process_limit: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }

    #[repr(C)]
    struct IoCounters {
        read_operation_count: u64,
        write_operation_count: u64,
        other_operation_count: u64,
        read_transfer_count: u64,
        write_transfer_count: u64,
        other_transfer_count: u64,
    }

    #[repr(C)]
    struct JobObjectExtendedLimitInformation {
        basic_limit_information: JobObjectBasicLimitInformation,
        io_info: IoCounters,
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory_used: usize,
        peak_job_memory_used: usize,
    }

    const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x0000_2000;
    const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS: i32 = 9;

    unsafe extern "system" {
        fn CreateJobObjectW(
            lp_job_attributes: *mut core::ffi::c_void,
            lp_name: *const u16,
        ) -> Handle;
        fn SetInformationJobObject(
            h_job: Handle,
            job_object_information_class: i32,
            lp_job_object_information: *mut core::ffi::c_void,
            cb_job_object_information_length: u32,
        ) -> i32;
        fn AssignProcessToJobObject(h_job: Handle, h_process: Handle) -> i32;
    }

    fn job_handle() -> Option<Handle> {
        static JOB: OnceLock<usize> = OnceLock::new();
        let raw = JOB.get_or_init(|| unsafe { create_kill_on_close_job() as usize });
        let handle = *raw as Handle;
        if handle.is_null() { None } else { Some(handle) }
    }

    unsafe fn create_kill_on_close_job() -> Handle {
        let job = unsafe { CreateJobObjectW(std::ptr::null_mut(), std::ptr::null()) };
        if job.is_null() {
            tracing::warn!("CreateJobObjectW 失败，sidecar 将依赖 reaper/idle 超时回收");
            return std::ptr::null_mut();
        }
        let mut info: JobObjectExtendedLimitInformation = unsafe { std::mem::zeroed() };
        info.basic_limit_information.limit_flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = unsafe {
            SetInformationJobObject(
                job,
                JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS,
                &mut info as *mut _ as *mut core::ffi::c_void,
                std::mem::size_of::<JobObjectExtendedLimitInformation>() as u32,
            )
        };
        if ok == 0 {
            tracing::warn!(
                "SetInformationJobObject(KILL_ON_JOB_CLOSE) 失败，父死自动回收可能不生效"
            );
        }
        job
    }

    pub(super) fn assign_current_job(process: RawHandle) {
        let Some(job) = job_handle() else {
            return;
        };
        let rc = unsafe { AssignProcessToJobObject(job, process as Handle) };
        if rc == 0 {
            tracing::warn!("AssignProcessToJobObject 失败，将依赖 reaper/idle 超时回收 sidecar");
        }
    }
}
