use std::{
    collections::HashMap,
    net::TcpListener,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, OnceLock},
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
const DEFAULT_SIDECAR_IDLE_SHUTDOWN_MS: u64 = 900_000;
const SIDECAR_IDLE_SHUTDOWN_ENV: &str = "ADMIN_SIDECAR_IDLE_SHUTDOWN_MS";
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

/// 非 job sidecar 的空闲自关闭阈值；env 可调，显式设 0 禁用
/// （specs/007-sidecar-singleflight-idle）。
fn sidecar_idle_shutdown_ms() -> u64 {
    std::env::var(SIDECAR_IDLE_SHUTDOWN_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_SIDECAR_IDLE_SHUTDOWN_MS)
}

/// 每个 sidecar key 一把 spawn 锁，保证同 key 并发 ensure 只 spawn 一个进程，
/// 同时不串行化不同 key 的拉起（健康等待可达数秒）。
///
/// 锁条目一旦创建就不移除：key 集合有限（site:/scan:/resolve:/preview:/db-index:
/// 的稳定 hash），而提前 remove 会让两个调用方各持不同锁实例，破坏单飞语义。
fn spawn_locks() -> &'static Mutex<HashMap<String, Arc<Mutex<()>>>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
    LOCKS.get_or_init(|| Mutex::new(HashMap::new()))
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
    run_cli_job_with_status(key, config_no_ext, cwd, stdout_path, stderr_path, |_, _| {}).await
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

async fn ensure_sidecar(key: &str) -> Result<SidecarHandle> {
    // 快路径：已有健康 handle 直接复用。
    if let Some(handle) = healthy_registered_sidecar(key).await {
        return Ok(handle);
    }

    // 慢路径单飞：同 key 并发只允许一个调用方 spawn，其余等待后复用。
    let key_lock = {
        let mut locks = spawn_locks().lock().await;
        locks
            .entry(key.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };
    let _spawn_guard = key_lock.lock().await;

    // double-check：等锁期间可能已有并发方完成 spawn。
    if let Some(handle) = healthy_registered_sidecar(key).await {
        return Ok(handle);
    }

    let handle = spawn_sidecar(key).await?;
    let mut guard = sidecars().lock().await;
    guard.insert(key.to_string(), handle.clone());
    Ok(handle)
}

async fn healthy_registered_sidecar(key: &str) -> Option<SidecarHandle> {
    let handle = {
        let guard = sidecars().lock().await;
        guard.get(key).cloned()
    };
    match handle {
        Some(handle) if sidecar_healthy(&handle).await => Some(handle),
        _ => None,
    }
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
    } else {
        let idle_ms = sidecar_idle_shutdown_ms();
        if idle_ms > 0 {
            command.arg("--idle-shutdown-ms").arg(idle_ms.to_string());
        }
    }

    isolate_sidecar_process_group(&mut command);
    let child = command.spawn().context("启动 aios-database sidecar 失败")?;
    let pid = child.id().unwrap_or_default();
    let handle = SidecarHandle {
        base_url: format!("http://{SIDECAR_HOST}:{port}"),
        token,
        pid,
        start_token: process_start_token(pid),
    };
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
