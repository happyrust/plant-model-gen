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

#[derive(Debug, Clone)]
struct SidecarHandle {
    base_url: String,
    token: String,
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
    let handle = ensure_sidecar(&format!("job:{}", stable_key(key)))
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
    let handle = ensure_sidecar(&format!("job:{}", stable_key(key)))
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
                return Ok(RunCliJobResponse {
                    success: true,
                    exit_code: record.exit_code,
                    job_id: submitted.job_id,
                });
            }
            "failed" | "cancelled" => {
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
    let handle = ensure_sidecar(&format!("job:{}", stable_key(key))).await?;
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

fn stable_key(value: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    hex::encode(&hasher.finalize()[..8])
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

    let _child = command.spawn().context("启动 aios-database sidecar 失败")?;
    let handle = SidecarHandle {
        base_url: format!("http://{SIDECAR_HOST}:{port}"),
        token,
    };
    wait_for_sidecar_health(&handle).await?;
    Ok(handle)
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
