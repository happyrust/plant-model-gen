use axum::{
    Router,
    body::Body,
    extract::{OriginalUri, Path, Query},
    http::{Request, StatusCode, Uri},
    response::{Html, IntoResponse, Json, Response, sse::{Event, KeepAlive, Sse}},
    routing::{get, post, put, delete as axum_delete},
};
use futures::stream::StreamExt;
use once_cell::sync::Lazy;
use rusqlite::{OptionalExtension, types::Value as SqlValue};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::io::Write;
use std::path::Path as FsPath;
use std::{convert::Infallible, convert::TryFrom, io::ErrorKind, path::PathBuf, time::Instant};
use tokio::sync::Mutex as AsyncMutex;
use tokio::sync::broadcast;
use tokio::time::{Duration, timeout};
use tokio::{fs, net::TcpStream};
use tokio_stream::wrappers::BroadcastStream;
use uuid::Uuid;

use crate::web_server::admin_response;
use crate::web_server::site_metadata::{self, CachedMetadata, MetadataSource, SiteMetadataFile};
use crate::web_server::topology_handlers;
use tower::ServiceExt;
use tower_http::services::ServeDir;

// ── M3 B5 · Remote-Sync SSE 广播通道 ──
pub static REMOTE_SYNC_EVENT_TX: Lazy<broadcast::Sender<RemoteSyncEvent>> = Lazy::new(|| {
    let (tx, _) = broadcast::channel(256);
    tx
});

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RemoteSyncEvent {
    #[serde(rename = "active_task_update")]
    ActiveTaskUpdate { task: serde_json::Value },
    #[serde(rename = "failed_task_new")]
    FailedTaskNew { task: serde_json::Value },
    #[serde(rename = "site_status_change")]
    SiteStatusChange {
        site_id: String,
        detection_status: String,
        progress: Option<f64>,
    },
    #[serde(rename = "sync_completed")]
    SyncCompleted {
        site_id: String,
        file_count: u32,
    },
    #[serde(rename = "sync_failed")]
    SyncFailed {
        site_id: String,
        error: String,
    },
    #[serde(rename = "keepalive")]
    Keepalive,
}

pub fn emit_remote_sync_event(event: RemoteSyncEvent) {
    let _ = REMOTE_SYNC_EVENT_TX.send(event);
}

pub fn create_remote_sync_routes() -> Router {
    Router::new()
        .route(
            "/api/remote-sync/envs",
            get(list_envs).post(create_env),
        )
        .route(
            "/api/remote-sync/envs/{id}",
            get(get_env).put(update_env).delete(delete_env),
        )
        .route("/api/remote-sync/envs/{id}/apply", post(apply_env))
        .route("/api/remote-sync/envs/{id}/activate", post(activate_env))
        .route("/api/remote-sync/runtime/stop", post(stop_runtime))
        .route("/api/remote-sync/envs/{id}/test-mqtt", post(test_mqtt_env))
        .route("/api/remote-sync/envs/{id}/test-http", post(test_http_env))
        .route("/api/remote-sync/sites/{id}/test-http", post(test_http_site))
        .route("/api/remote-sync/runtime/status", get(runtime_status))
        .route("/api/remote-sync/runtime/config", get(runtime_config))
        .route(
            "/api/remote-sync/envs/import-from-dboption",
            post(import_env_from_dboption),
        )
        .route("/api/remote-sync/logs", get(list_logs))
        .route("/api/remote-sync/stats/daily", get(daily_stats))
        .route("/api/remote-sync/stats/flows", get(flow_stats))
        .route(
            "/api/remote-sync/envs/{id}/sites",
            get(list_sites).post(create_site),
        )
        .route(
            "/api/remote-sync/sites/{id}",
            put(update_site).delete(delete_site),
        )
        .route("/api/remote-sync/sites/{id}/metadata", get(get_site_metadata))
        .route("/api/remote-sync/sites/{id}/files/{*path}", get(serve_site_files))
        .route(
            "/api/remote-sync/topology",
            get(topology_handlers::get_topology)
                .post(topology_handlers::save_topology)
                .delete(topology_handlers::delete_topology),
        )
        .route("/api/remote-sync/sites/{id}/files", get(serve_site_files_root))
        // v2 · ROADMAP M2 · 任务队列
        .route("/api/remote-sync/tasks/active", get(list_active_tasks))
        .route("/api/remote-sync/tasks/{id}/abort", post(abort_active_task))
        // v2 · ROADMAP M2 · 失败任务
        .route(
            "/api/remote-sync/tasks/failed",
            get(list_failed_tasks).delete(cleanup_failed_tasks),
        )
        .route(
            "/api/remote-sync/tasks/failed/{id}/retry",
            post(retry_failed_task),
        )
        // v2 · ROADMAP M2 · 协同组参数配置
        .route(
            "/api/remote-sync/envs/{id}/config",
            get(get_env_config).put(update_env_config),
        )
        // v2 · ROADMAP M3 B5 · SSE 实时事件流
        .route("/api/remote-sync/events/stream", get(remote_sync_events_stream))
}

/// 远程增量环境
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteSyncEnv {
    pub id: String,
    pub name: String,
    pub mqtt_host: Option<String>,
    pub mqtt_port: Option<u16>,
    pub file_server_host: Option<String>,
    pub location: Option<String>,
    /// 逗号分隔或JSON数组（UI 层转换）
    pub location_dbs: Option<String>,
    /// 连接失败后的重连初始间隔(ms)
    pub reconnect_initial_ms: Option<u64>,
    /// 连接失败后的重连最大间隔(ms)
    pub reconnect_max_ms: Option<u64>,
    pub created_at: String,
    pub updated_at: String,
}

/// 远程站点（外部站点）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteSyncSite {
    pub id: String,
    pub env_id: String,
    pub name: String,
    pub location: Option<String>,
    pub http_host: Option<String>,
    /// 逗号分隔或JSON数组（UI 层转换）
    pub dbnums: Option<String>,
    pub notes: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteSyncLogRecord {
    pub id: String,
    pub task_id: Option<String>,
    pub env_id: Option<String>,
    pub source_env: Option<String>,
    pub target_site: Option<String>,
    pub site_id: Option<String>,
    pub direction: Option<String>,
    pub file_path: Option<String>,
    pub file_size: Option<u64>,
    pub record_count: Option<u64>,
    pub status: String,
    pub error_message: Option<String>,
    pub notes: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
struct SiteInfo {
    env_id: String,
    env_name: Option<String>,
    env_file_host: Option<String>,
    site_id: String,
    site_name: String,
    site_host: Option<String>,
}

#[derive(Debug)]
struct MetadataLoadContext {
    info: SiteInfo,
    metadata: SiteMetadataFile,
    source: MetadataSource,
    fetched_at: String,
    cache_path: Option<PathBuf>,
    warnings: Vec<String>,
    http_base: Option<String>,
    local_base: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
pub struct MetadataQuery {
    #[serde(default)]
    pub refresh: bool,
    #[serde(default)]
    pub cache_only: bool,
}

/// 页面
pub async fn remote_sync_page() -> Html<String> {
    Html(crate::web_server::remote_sync_template::render_remote_sync_page_with_sidebar())
}

static SCHEMA_INITIALIZED: std::sync::Once = std::sync::Once::new();
static REMOTE_SYNC_CONFIG_LOCK: Lazy<AsyncMutex<()>> = Lazy::new(|| AsyncMutex::new(()));
const LOCAL_FILE_SERVER_PLACEHOLDER: &str = "local://file_server_host";

#[derive(Debug)]
struct RemoteSyncRuntimeConfigUpdate {
    mqtt_host: Option<String>,
    mqtt_port: Option<u16>,
    file_server_host: Option<String>,
    location: Option<String>,
    location_dbs: Vec<u32>,
}

fn resolve_db_path() -> String {
    use config as cfg;

    let cfg_name =
        std::env::var("DB_OPTION_FILE").unwrap_or_else(|_| "db_options/DbOption".to_string());
    let cfg_file = format!("{}.toml", cfg_name);
    if std::path::Path::new(&cfg_file).exists() {
        cfg::Config::builder()
            .add_source(cfg::File::with_name(&cfg_name))
            .build()
            .ok()
            .and_then(|b| b.get_string("deployment_sites_sqlite_path").ok())
            .unwrap_or_else(|| "deployment_sites.sqlite".to_string())
    } else {
        "deployment_sites.sqlite".to_string()
    }
}

fn run_schema_migration(conn: &rusqlite::Connection) -> Result<(), Box<dyn std::error::Error>> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS remote_sync_envs (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            mqtt_host TEXT,
            mqtt_port INTEGER,
            file_server_host TEXT,
            location TEXT,
            location_dbs TEXT,
            reconnect_initial_ms INTEGER,
            reconnect_max_ms INTEGER,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )",
        rusqlite::params![],
    )?;
    let _ = conn.execute(
        "ALTER TABLE remote_sync_envs ADD COLUMN reconnect_initial_ms INTEGER",
        rusqlite::params![],
    );
    let _ = conn.execute(
        "ALTER TABLE remote_sync_envs ADD COLUMN reconnect_max_ms INTEGER",
        rusqlite::params![],
    );
    conn.execute(
        "CREATE TABLE IF NOT EXISTS remote_sync_sites (
            id TEXT PRIMARY KEY,
            env_id TEXT NOT NULL,
            name TEXT NOT NULL,
            location TEXT,
            http_host TEXT,
            dbnums TEXT,
            notes TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            FOREIGN KEY(env_id) REFERENCES remote_sync_envs(id) ON DELETE CASCADE
        )",
        rusqlite::params![],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS remote_sync_logs (
            id TEXT PRIMARY KEY,
            task_id TEXT,
            env_id TEXT,
            source_env TEXT,
            target_site TEXT,
            site_id TEXT,
            direction TEXT,
            file_path TEXT,
            file_size INTEGER,
            record_count INTEGER,
            status TEXT,
            error_message TEXT,
            notes TEXT,
            started_at TEXT,
            completed_at TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )",
        rusqlite::params![],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_remote_sync_logs_env ON remote_sync_logs(env_id)",
        rusqlite::params![],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_remote_sync_logs_status ON remote_sync_logs(status)",
        rusqlite::params![],
    )?;

    // ────────────────────────────────────────────────────────────────
    // v2 · 对应 design/collaboration-v2/ROADMAP.md 的 M2 B4。
    //   tasks: 实时活跃任务（WebSocket/SSE 推送源）
    //   failed_tasks: 失败任务队列（可重试 + 自动耗尽）
    //   env_config: 协同组级参数配置（自动检测 / 并发 / 重连 等）
    // ────────────────────────────────────────────────────────────────
    conn.execute(
        "CREATE TABLE IF NOT EXISTS remote_sync_tasks (
            task_id TEXT PRIMARY KEY,
            env_id TEXT NOT NULL,
            site_id TEXT,
            site_name TEXT,
            task_name TEXT NOT NULL,
            file_path TEXT,
            progress REAL DEFAULT 0,
            status TEXT NOT NULL,
            started_at TEXT,
            updated_at TEXT NOT NULL,
            FOREIGN KEY(env_id) REFERENCES remote_sync_envs(id) ON DELETE CASCADE
        )",
        rusqlite::params![],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_remote_sync_tasks_env ON remote_sync_tasks(env_id)",
        rusqlite::params![],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_remote_sync_tasks_status ON remote_sync_tasks(status)",
        rusqlite::params![],
    )?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS remote_sync_failed_tasks (
            id TEXT PRIMARY KEY,
            task_type TEXT NOT NULL,
            env_id TEXT NOT NULL,
            site_id TEXT,
            site_name TEXT,
            error TEXT NOT NULL,
            retry_count INTEGER NOT NULL DEFAULT 0,
            max_retries INTEGER NOT NULL DEFAULT 5,
            first_failed_at TEXT NOT NULL,
            next_retry_at TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            FOREIGN KEY(env_id) REFERENCES remote_sync_envs(id) ON DELETE CASCADE
        )",
        rusqlite::params![],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_remote_sync_failed_tasks_env ON remote_sync_failed_tasks(env_id)",
        rusqlite::params![],
    )?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS remote_sync_env_config (
            env_id TEXT PRIMARY KEY,
            auto_detect INTEGER NOT NULL DEFAULT 1,
            detect_interval INTEGER NOT NULL DEFAULT 30,
            auto_sync INTEGER NOT NULL DEFAULT 0,
            batch_size INTEGER NOT NULL DEFAULT 10,
            max_concurrent INTEGER NOT NULL DEFAULT 3,
            reconnect_initial_ms INTEGER NOT NULL DEFAULT 1000,
            reconnect_max_ms INTEGER NOT NULL DEFAULT 30000,
            enable_notifications INTEGER NOT NULL DEFAULT 1,
            log_retention_days INTEGER NOT NULL DEFAULT 30,
            updated_at TEXT NOT NULL,
            FOREIGN KEY(env_id) REFERENCES remote_sync_envs(id) ON DELETE CASCADE
        )",
        rusqlite::params![],
    )?;

    Ok(())
}

/// 打开 SQLite 连接。Schema migration 仅在进程生命周期内执行一次。
pub fn open_sqlite() -> Result<rusqlite::Connection, Box<dyn std::error::Error>> {
    let db_path = resolve_db_path();
    let conn = rusqlite::Connection::open(&db_path)?;

    let is_litefs = db_path.starts_with("/litefs");
    if is_litefs {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
    }

    let mut migration_error: Option<String> = None;
    SCHEMA_INITIALIZED.call_once(|| {
        eprintln!("remote_sync: 首次打开 {}，执行 schema migration", db_path);
        if let Err(e) = run_schema_migration(&conn) {
            migration_error = Some(format!("schema migration 失败: {}", e));
        }
    });
    if let Some(err) = migration_error {
        return Err(err.into());
    }

    Ok(conn)
}

fn normalize_optional_text(value: Option<String>) -> Option<String> {
    value.and_then(|item| {
        let trimmed = item.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn parse_u32_csv(value: Option<String>) -> Vec<u32> {
    value.unwrap_or_default()
        .split(|c| c == ',' || c == ' ')
        .filter_map(|token| token.trim().parse::<u32>().ok())
        .collect()
}

fn load_env_runtime_config_update(env_id: &str) -> Result<RemoteSyncRuntimeConfigUpdate, String> {
    let conn = open_sqlite().map_err(|e| format!("打开协同组存储失败: {}", e))?;
    let mut stmt = conn
        .prepare(
            "SELECT mqtt_host, mqtt_port, file_server_host, location, location_dbs
             FROM remote_sync_envs
             WHERE id = ?1
             LIMIT 1",
        )
        .map_err(|e| format!("读取协同组失败: {}", e))?;

    stmt.query_row(rusqlite::params![env_id], |row| {
        Ok(RemoteSyncRuntimeConfigUpdate {
            mqtt_host: normalize_optional_text(row.get::<_, Option<String>>(0)?),
            mqtt_port: row
                .get::<_, Option<i64>>(1)?
                .and_then(|value| u16::try_from(value).ok()),
            file_server_host: normalize_optional_text(row.get::<_, Option<String>>(2)?),
            location: normalize_optional_text(row.get::<_, Option<String>>(3)?),
            location_dbs: parse_u32_csv(row.get::<_, Option<String>>(4)?),
        })
    })
    .optional()
    .map_err(|e| format!("读取协同组失败: {}", e))?
    .ok_or_else(|| "协同组不存在".to_string())
}

fn find_first_section_offset(content: &str) -> Option<usize> {
    let mut offset = 0usize;
    for segment in content.split_inclusive('\n') {
        if segment.trim().starts_with('[') {
            return Some(offset);
        }
        offset += segment.len();
    }
    None
}

fn find_root_key_line_range(content: &str, key: &str) -> Option<(usize, usize)> {
    let mut offset = 0usize;
    for segment in content.split_inclusive('\n') {
        let trimmed = segment.trim();
        if trimmed.starts_with('[') {
            break;
        }
        if trimmed.is_empty() || trimmed.starts_with('#') {
            offset += segment.len();
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix(key) {
            if rest.trim_start().starts_with('=') {
                return Some((offset, offset + segment.len()));
            }
        }
        offset += segment.len();
    }
    None
}

fn replace_or_insert_root_line(content: &mut String, key: &str, line: Option<String>) {
    if let Some((start, end)) = find_root_key_line_range(content, key) {
        match line {
            Some(next_line) => {
                let replacement = if next_line.ends_with('\n') {
                    next_line
                } else {
                    format!("{}\n", next_line)
                };
                content.replace_range(start..end, &replacement);
            }
            None => {
                content.replace_range(start..end, "");
            }
        }
        return;
    }

    let Some(next_line) = line else {
        return;
    };
    let insert_at = find_first_section_offset(content).unwrap_or(content.len());
    let prefix = if insert_at > 0 && !content[..insert_at].ends_with('\n') {
        "\n"
    } else {
        ""
    };
    let suffix = if next_line.ends_with('\n') { "" } else { "\n" };
    content.insert_str(insert_at, &format!("{}{}{}", prefix, next_line, suffix));
}

fn escape_toml_basic_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn set_root_string_key(content: &mut String, key: &str, value: Option<&str>) {
    let rendered = format!(
        r#"{key} = "{}""#,
        escape_toml_basic_string(value.unwrap_or_default())
    );
    replace_or_insert_root_line(content, key, Some(rendered));
}

fn set_root_number_key<T: std::fmt::Display>(content: &mut String, key: &str, value: Option<T>) {
    replace_or_insert_root_line(content, key, value.map(|number| format!("{key} = {number}")));
}

fn set_root_u32_array_key(content: &mut String, key: &str, values: &[u32]) {
    let joined = values
        .iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    replace_or_insert_root_line(content, key, Some(format!("{key} = [{}]", joined)));
}

fn atomic_write_config(path: &FsPath, content: &str) -> Result<(), String> {
    let parent = path.parent().unwrap_or_else(|| FsPath::new("."));
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("DbOption.toml");
    let temp_path = parent.join(format!(".{}.{}.tmp", file_name, Uuid::new_v4()));

    let write_result = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&temp_path)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        if let Ok(metadata) = std::fs::metadata(path) {
            let _ = std::fs::set_permissions(&temp_path, metadata.permissions());
        }
        std::fs::rename(&temp_path, path)?;
        Ok(())
    })();

    if let Err(err) = write_result {
        let _ = std::fs::remove_file(&temp_path);
        return Err(format!("写入当前配置失败: {}", err));
    }
    Ok(())
}

fn write_env_to_runtime_config(env_id: &str) -> Result<(), String> {
    let update = load_env_runtime_config_update(env_id)?;
    let cfg_name =
        std::env::var("DB_OPTION_FILE").unwrap_or_else(|_| "db_options/DbOption".to_string());
    let cfg_file = format!("{}.toml", cfg_name);
    let path = FsPath::new(&cfg_file);
    if !path.exists() {
        return Err(format!("{} 不存在，无法写入当前配置。", cfg_file));
    }

    let mut content =
        std::fs::read_to_string(path).map_err(|e| format!("读取当前配置失败: {}", e))?;

    set_root_string_key(&mut content, "mqtt_host", update.mqtt_host.as_deref());
    set_root_number_key(&mut content, "mqtt_port", update.mqtt_port);
    set_root_string_key(
        &mut content,
        "file_server_host",
        update.file_server_host.as_deref(),
    );
    set_root_string_key(&mut content, "location", update.location.as_deref());
    set_root_u32_array_key(&mut content, "location_dbs", &update.location_dbs);

    atomic_write_config(path, &content)
}

// ===== Envs =====

#[derive(Debug, Deserialize)]
pub struct EnvCreateRequest {
    pub name: String,
    pub mqtt_host: Option<String>,
    pub mqtt_port: Option<u16>,
    pub file_server_host: Option<String>,
    pub location: Option<String>,
    pub location_dbs: Option<String>,
    pub reconnect_initial_ms: Option<u64>,
    pub reconnect_max_ms: Option<u64>,
}

pub async fn list_envs() -> Result<Json<serde_json::Value>, StatusCode> {
    let conn = open_sqlite().map_err(|e| {
        eprintln!("list_envs: 打开数据库失败: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let mut stmt = conn
        .prepare(
            "SELECT id, name, mqtt_host, mqtt_port, file_server_host, location, location_dbs, reconnect_initial_ms, reconnect_max_ms, created_at, updated_at FROM remote_sync_envs ORDER BY updated_at DESC",
        )
        .map_err(|e| {
            eprintln!("list_envs: SQL prepare 失败: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    let rows = stmt
        .query_map([], |row| {
            Ok(RemoteSyncEnv {
                id: row.get(0)?,
                name: row.get(1)?,
                mqtt_host: row.get(2)?,
                mqtt_port: row.get(3).ok(),
                file_server_host: row.get(4)?,
                location: row.get(5)?,
                location_dbs: row.get(6)?,
                reconnect_initial_ms: row
                    .get::<_, Option<i64>>(7)
                    .ok()
                    .flatten()
                    .map(|v| v as u64),
                reconnect_max_ms: row
                    .get::<_, Option<i64>>(8)
                    .ok()
                    .flatten()
                    .map(|v| v as u64),
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
            })
        })
        .map_err(|e| {
            eprintln!("list_envs: 查询失败: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    let mut envs = Vec::new();
    for r in rows {
        envs.push(r.map_err(|e| {
            eprintln!("list_envs: 行解析失败: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?);
    }
    Ok(Json(json!({"status":"success","items": envs})))
}

pub async fn create_env(
    Json(req): Json<EnvCreateRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if req.name.trim().is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let conn = open_sqlite().map_err(|e| {
        eprintln!("create_env: 打开数据库失败: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO remote_sync_envs (id, name, mqtt_host, mqtt_port, file_server_host, location, location_dbs, reconnect_initial_ms, reconnect_max_ms, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
        rusqlite::params![
            id,
            req.name,
            req.mqtt_host,
            req.mqtt_port.map(|x| x as i64),
            req.file_server_host,
            req.location,
            req.location_dbs,
            req.reconnect_initial_ms.map(|v| v as i64),
            req.reconnect_max_ms.map(|v| v as i64),
            now,
        ],
    )
    .map_err(|e| {
        eprintln!("create_env: 插入失败: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(Json(json!({"status":"success","id": id})))
}

pub async fn get_env(Path(id): Path<String>) -> Result<Json<serde_json::Value>, StatusCode> {
    let conn = open_sqlite().map_err(|e| {
        eprintln!("get_env: 打开数据库失败: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let mut stmt = conn
        .prepare("SELECT id, name, mqtt_host, mqtt_port, file_server_host, location, location_dbs, reconnect_initial_ms, reconnect_max_ms, created_at, updated_at FROM remote_sync_envs WHERE id = ?1 LIMIT 1")
        .map_err(|e| {
            eprintln!("get_env: SQL prepare 失败: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    let mut rows = stmt
        .query(rusqlite::params![id])
        .map_err(|e| {
            eprintln!("get_env: 查询失败: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    if let Some(row) = rows.next().map_err(|e| {
        eprintln!("get_env: 行读取失败: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })? {
        let env = RemoteSyncEnv {
            id: row.get(0).unwrap_or_default(),
            name: row.get(1).unwrap_or_default(),
            mqtt_host: row.get(2).ok(),
            mqtt_port: row
                .get::<_, Option<i64>>(3)
                .ok()
                .flatten()
                .map(|v| v as u16),
            file_server_host: row.get(4).ok(),
            location: row.get(5).ok(),
            location_dbs: row.get(6).ok(),
            reconnect_initial_ms: row
                .get::<_, Option<i64>>(7)
                .ok()
                .flatten()
                .map(|v| v as u64),
            reconnect_max_ms: row
                .get::<_, Option<i64>>(8)
                .ok()
                .flatten()
                .map(|v| v as u64),
            created_at: row.get(9).unwrap_or_default(),
            updated_at: row.get(10).unwrap_or_default(),
        };
        Ok(Json(json!({"status":"success","item": env})))
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

pub async fn update_env(
    Path(id): Path<String>,
    Json(req): Json<EnvCreateRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let conn = open_sqlite().map_err(|e| {
        eprintln!("update_env: 打开数据库失败: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let now = chrono::Utc::now().to_rfc3339();
    let changed = conn
        .execute(
            "UPDATE remote_sync_envs SET name = ?2, mqtt_host = ?3, mqtt_port = ?4, file_server_host = ?5, location = ?6, location_dbs = ?7, reconnect_initial_ms = ?8, reconnect_max_ms = ?9, updated_at = ?10 WHERE id = ?1",
            rusqlite::params![
                id,
                req.name,
                req.mqtt_host,
                req.mqtt_port.map(|x| x as i64),
                req.file_server_host,
                req.location,
                req.location_dbs,
                req.reconnect_initial_ms.map(|v| v as i64),
                req.reconnect_max_ms.map(|v| v as i64),
                now,
            ],
        )
        .map_err(|e| {
            eprintln!("update_env: 更新失败: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    if changed > 0 {
        Ok(Json(json!({"status":"success"})))
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

pub async fn delete_env(Path(id): Path<String>) -> Result<Json<serde_json::Value>, StatusCode> {
    // 如果正在运行的协同组正是要删除的，先停止运行时
    {
        let guard = crate::web_server::remote_runtime::REMOTE_RUNTIME.read().await;
        if let Some(state) = guard.as_ref() {
            if state.env_id == id {
                drop(guard);
                crate::web_server::remote_runtime::stop_runtime().await;
                eprintln!("delete_env: 已停止正在运行的协同组 {}", id);
            }
        }
    }

    let conn = open_sqlite().map_err(|e| {
        eprintln!("delete_env: 打开数据库失败: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    // 先删 sites 和 logs（ON DELETE CASCADE 并非总是启用，这里显式处理）
    let _ = conn.execute(
        "DELETE FROM remote_sync_sites WHERE env_id = ?1",
        rusqlite::params![id.as_str()],
    );
    let _ = conn.execute(
        "DELETE FROM remote_sync_logs WHERE env_id = ?1",
        rusqlite::params![id.as_str()],
    );
    let changed = conn
        .execute(
            "DELETE FROM remote_sync_envs WHERE id = ?1",
            rusqlite::params![id],
        )
        .map_err(|e| {
            eprintln!("delete_env: 删除协同组失败: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    if changed > 0 {
        Ok(Json(json!({"status":"success"})))
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

// ===== Sites =====

#[derive(Debug, Deserialize)]
pub struct SiteCreateRequest {
    pub name: String,
    pub location: Option<String>,
    pub http_host: Option<String>,
    pub dbnums: Option<String>,
    pub notes: Option<String>,
}

fn validate_http_host(host: &Option<String>) -> Result<(), String> {
    let Some(h) = host.as_deref() else {
        return Ok(());
    };
    let h = h.trim();
    if h.is_empty() {
        return Ok(());
    }
    if h.chars().any(char::is_whitespace) {
        return Err("http_host 不能包含空格或换行".into());
    }
    if h.starts_with('/') || h.starts_with("./") || h.starts_with("../") {
        return Err("http_host 只允许 HTTP/HTTPS 地址".into());
    }
    if h.starts_with("file://") {
        return Err("http_host 不支持 file:// 本地路径".into());
    }

    let parsed = reqwest::Url::parse(h).map_err(|_| "http_host 格式不合法，仅支持 HTTP/HTTPS 地址".to_string())?;
    match parsed.scheme() {
        "http" | "https" => {}
        _ => return Err("http_host 格式不合法，仅支持 HTTP/HTTPS 地址".into()),
    }
    if parsed.host_str().map(|value| value.trim().is_empty()).unwrap_or(true) {
        return Err("http_host 缺少主机名".into());
    }
    Ok(())
}

pub async fn list_sites(Path(env_id): Path<String>) -> Result<Json<serde_json::Value>, StatusCode> {
    let conn = open_sqlite().map_err(|e| {
        eprintln!("list_sites: 打开数据库失败: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let mut stmt = conn
        .prepare("SELECT id, env_id, name, location, http_host, dbnums, notes, created_at, updated_at FROM remote_sync_sites WHERE env_id = ?1 ORDER BY created_at DESC")
        .map_err(|e| {
            eprintln!("list_sites: SQL prepare 失败: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    let rows = stmt
        .query_map(rusqlite::params![env_id], |row| {
            Ok(RemoteSyncSite {
                id: row.get(0)?,
                env_id: row.get(1)?,
                name: row.get(2)?,
                location: row.get(3)?,
                http_host: row.get(4)?,
                dbnums: row.get(5)?,
                notes: row.get(6)?,
                created_at: row.get(7)?,
                updated_at: row.get(8)?,
            })
        })
        .map_err(|e| {
            eprintln!("list_sites: 查询失败: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    let mut items = Vec::new();
    for r in rows {
        items.push(r.map_err(|e| {
            eprintln!("list_sites: 行解析失败: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?);
    }
    Ok(Json(json!({"status":"success","items": items})))
}

pub async fn create_site(
    Path(env_id): Path<String>,
    Json(req): Json<SiteCreateRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if req.name.trim().is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    if let Err(msg) = validate_http_host(&req.http_host) {
        return Ok(Json(json!({"status":"failed","message": msg})));
    }
    let conn = open_sqlite().map_err(|e| {
        eprintln!("create_site: 打开数据库失败: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO remote_sync_sites (id, env_id, name, location, http_host, dbnums, notes, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
        rusqlite::params![
            id,
            env_id,
            req.name,
            req.location,
            req.http_host,
            req.dbnums,
            req.notes,
            now,
        ],
    )
    .map_err(|e| {
        eprintln!("create_site: 插入失败: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(Json(json!({"status":"success","id": id})))
}

pub async fn update_site(
    Path(site_id): Path<String>,
    Json(req): Json<SiteCreateRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if let Err(msg) = validate_http_host(&req.http_host) {
        return Ok(Json(json!({"status":"failed","message": msg})));
    }
    let conn = open_sqlite().map_err(|e| {
        eprintln!("update_site: 打开数据库失败: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let now = chrono::Utc::now().to_rfc3339();
    let changed = conn
        .execute(
            "UPDATE remote_sync_sites SET name = ?2, location = ?3, http_host = ?4, dbnums = ?5, notes = ?6, updated_at = ?7 WHERE id = ?1",
            rusqlite::params![
                site_id,
                req.name,
                req.location,
                req.http_host,
                req.dbnums,
                req.notes,
                now,
            ],
        )
        .map_err(|e| {
            eprintln!("update_site: 更新失败: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    if changed > 0 {
        Ok(Json(json!({"status":"success"})))
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

pub async fn delete_site(
    Path(site_id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let conn = open_sqlite().map_err(|e| {
        eprintln!("delete_site: 打开数据库失败: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let changed = conn
        .execute(
            "DELETE FROM remote_sync_sites WHERE id = ?1",
            rusqlite::params![site_id],
        )
        .map_err(|e| {
            eprintln!("delete_site: 删除失败: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    if changed > 0 {
        Ok(Json(json!({"status":"success"})))
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

// ===== 应用到运行时（写入 DbOption.toml） =====

pub async fn apply_env(Path(id): Path<String>) -> Result<Json<serde_json::Value>, StatusCode> {
    let _guard = REMOTE_SYNC_CONFIG_LOCK.lock().await;
    match write_env_to_runtime_config(&id) {
        Ok(()) => Ok(action_success(
            "已写入配置文件。部分运行期组件需重启或重新加载配置后生效。",
            serde_json::Map::from_iter([("env_id".to_string(), json!(id))]),
        )),
        Err(message) => Ok(action_failed(
            message,
            serde_json::Map::from_iter([("env_id".to_string(), json!(id))]),
        )),
    }
}

/// 激活环境（即时生效）：写入 DbOption.toml 并在 WebUI 进程内重启 watcher + MQTT
pub async fn activate_env(Path(id): Path<String>) -> Result<Json<serde_json::Value>, StatusCode> {
    let _guard = REMOTE_SYNC_CONFIG_LOCK.lock().await;
    if let Err(message) = write_env_to_runtime_config(&id) {
        return Ok(action_failed(
            message,
            serde_json::Map::from_iter([("env_id".to_string(), json!(id))]),
        ));
    }

    // 停止当前运行态
    crate::web_server::remote_runtime::stop_runtime().await;
    // 启动新的运行态（使用最新 DbOption）
    match crate::web_server::remote_runtime::start_runtime(id.clone()).await {
        Ok(_) => Ok(action_success(
            "已写入配置文件并启动 watcher + MQTT 订阅。",
            serde_json::Map::from_iter([("env_id".to_string(), json!(id))]),
        )),
        Err(e) => Ok(action_failed(
            format!("启动运行态失败: {}", e),
            serde_json::Map::from_iter([("env_id".to_string(), json!(id))]),
        )),
    }
}

/// 停止运行时（终止 watcher + MQTT）
pub async fn stop_runtime() -> Result<Json<serde_json::Value>, StatusCode> {
    use crate::web_server::remote_runtime::REMOTE_RUNTIME;
    let current_env_id = REMOTE_RUNTIME
        .read()
        .await
        .as_ref()
        .map(|state| state.env_id.clone());
    crate::web_server::remote_runtime::stop_runtime().await;
    Ok(action_success(
        "已停止运行时 watcher + MQTT",
        serde_json::Map::from_iter([("env_id".to_string(), json!(current_env_id))]),
    ))
}

fn checked_at_now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn action_response(
    status: &str,
    message: impl Into<String>,
    extras: serde_json::Map<String, serde_json::Value>,
) -> Json<serde_json::Value> {
    let mut payload = serde_json::Map::from_iter([
        ("status".to_string(), json!(status)),
        ("message".to_string(), json!(message.into())),
    ]);
    payload.extend(extras);
    Json(serde_json::Value::Object(payload))
}

fn action_success(
    message: impl Into<String>,
    extras: serde_json::Map<String, serde_json::Value>,
) -> Json<serde_json::Value> {
    action_response("success", message, extras)
}

fn action_failed(
    message: impl Into<String>,
    extras: serde_json::Map<String, serde_json::Value>,
) -> Json<serde_json::Value> {
    action_response("failed", message, extras)
}

fn ok_diagnostic(
    message: impl Into<String>,
    extras: serde_json::Map<String, serde_json::Value>,
) -> Json<serde_json::Value> {
    let mut payload = serde_json::Map::from_iter([
        ("status".to_string(), json!("success")),
        ("message".to_string(), json!(message.into())),
        ("checked_at".to_string(), json!(checked_at_now())),
    ]);
    payload.extend(extras);
    Json(serde_json::Value::Object(payload))
}

fn failed_diagnostic(
    message: impl Into<String>,
    extras: serde_json::Map<String, serde_json::Value>,
) -> Json<serde_json::Value> {
    let mut payload = serde_json::Map::from_iter([
        ("status".to_string(), json!("failed")),
        ("message".to_string(), json!(message.into())),
        ("checked_at".to_string(), json!(checked_at_now())),
    ]);
    payload.extend(extras);
    Json(serde_json::Value::Object(payload))
}

/// 测试环境 MQTT 连接（TCP 可达性）
pub async fn test_mqtt_env(Path(id): Path<String>) -> Result<Json<serde_json::Value>, StatusCode> {
    let (host, port) = {
        let conn = open_sqlite().map_err(|e| { eprintln!("remote_sync: {}", e); StatusCode::INTERNAL_SERVER_ERROR })?;
        let mut stmt = conn
            .prepare("SELECT mqtt_host, mqtt_port FROM remote_sync_envs WHERE id = ?1 LIMIT 1")
            .map_err(|e| { eprintln!("remote_sync: {}", e); StatusCode::INTERNAL_SERVER_ERROR })?;
        let mut rows = stmt
            .query(rusqlite::params![id])
            .map_err(|e| { eprintln!("remote_sync: {}", e); StatusCode::INTERNAL_SERVER_ERROR })?;
        let row = match rows.next() {
            Ok(Some(r)) => r,
            _ => return Err(StatusCode::NOT_FOUND),
        };
        let host: String = row
            .get::<_, Option<String>>(0)
            .ok()
            .flatten()
            .unwrap_or_default();
        let port = row
            .get::<_, Option<i64>>(1)
            .ok()
            .flatten()
            .and_then(|v| u16::try_from(v).ok());
        (host, port)
    };
    if host.is_empty() {
        return Ok(failed_diagnostic(
            "未配置 mqtt_host",
            serde_json::Map::new(),
        ));
    }
    let Some(port) = port else {
        return Ok(failed_diagnostic(
            "未配置 mqtt_port",
            serde_json::Map::new(),
        ));
    };

    let addr = format!("{}:{}", host, port);
    let start = Instant::now();
    let result = timeout(
        Duration::from_secs(3),
        tokio::net::TcpStream::connect(&addr),
    )
    .await;
    match result {
        Ok(Ok(_)) => Ok(ok_diagnostic(
            "MQTT 连接可达",
            serde_json::Map::from_iter([
                ("addr".to_string(), json!(addr)),
                (
                    "latency_ms".to_string(),
                    json!(start.elapsed().as_millis() as u64),
                ),
            ]),
        )),
        Ok(Err(e)) => Ok(failed_diagnostic(
            format!("连接失败: {}", e),
            serde_json::Map::from_iter([("addr".to_string(), json!(addr))]),
        )),
        Err(_) => Ok(failed_diagnostic(
            "连接超时",
            serde_json::Map::from_iter([("addr".to_string(), json!(addr))]),
        )),
    }
}

/// 测试环境文件服务地址（HTTP 可达性）
pub async fn test_http_env(Path(id): Path<String>) -> Result<Json<serde_json::Value>, StatusCode> {
    let url: String = {
        let conn = open_sqlite().map_err(|e| { eprintln!("remote_sync: {}", e); StatusCode::INTERNAL_SERVER_ERROR })?;
        let mut stmt = conn
            .prepare("SELECT file_server_host FROM remote_sync_envs WHERE id = ?1 LIMIT 1")
            .map_err(|e| { eprintln!("remote_sync: {}", e); StatusCode::INTERNAL_SERVER_ERROR })?;
        let mut rows = stmt
            .query(rusqlite::params![id])
            .map_err(|e| { eprintln!("remote_sync: {}", e); StatusCode::INTERNAL_SERVER_ERROR })?;
        let row = match rows.next() {
            Ok(Some(r)) => r,
            _ => return Err(StatusCode::NOT_FOUND),
        };
        row.get::<_, Option<String>>(0)
            .ok()
            .flatten()
            .unwrap_or_default()
    };
    if url.is_empty() {
        return Ok(failed_diagnostic(
            "未配置 file_server_host",
            serde_json::Map::new(),
        ));
    }

    if site_metadata::is_local_path_hint(&url) {
        let path = site_metadata::normalize_local_base(&url);
        let start = Instant::now();
        return match fs::metadata(&path).await {
            Ok(_) => Ok(ok_diagnostic(
                "本地文件服务目录可达",
                serde_json::Map::from_iter([
                    ("url".to_string(), json!(LOCAL_FILE_SERVER_PLACEHOLDER)),
                    (
                        "latency_ms".to_string(),
                        json!(start.elapsed().as_millis() as u64),
                    ),
                ]),
            )),
            Err(err) => Ok(failed_diagnostic(
                format!("本地文件服务目录不可达: {}", err),
                serde_json::Map::from_iter([(
                    "url".to_string(),
                    json!(LOCAL_FILE_SERVER_PLACEHOLDER),
                )]),
            )),
        };
    }

    if !site_metadata::is_http_url(&url) {
        return Ok(failed_diagnostic(
            "file_server_host 既不是 HTTP 地址也不是本地路径",
            serde_json::Map::from_iter([("url".to_string(), json!(url))]),
        ));
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| { eprintln!("remote_sync: {}", e); StatusCode::INTERNAL_SERVER_ERROR })?;
    let start = Instant::now();
    match client.get(&url).send().await {
        Ok(resp) => {
            let status = resp.status();
            let extras = serde_json::Map::from_iter([
                ("url".to_string(), json!(url)),
                ("code".to_string(), json!(status.as_u16())),
                (
                    "latency_ms".to_string(),
                    json!(start.elapsed().as_millis() as u64),
                ),
            ]);
            if status.is_success() {
                Ok(ok_diagnostic("文件服务可达", extras))
            } else {
                Ok(failed_diagnostic(
                    format!("文件服务返回异常状态: {}", status),
                    extras,
                ))
            }
        }
        Err(e) => Ok(failed_diagnostic(
            format!("请求失败: {}", e),
            serde_json::Map::from_iter([("url".to_string(), json!(url))]),
        )),
    }
}

/// 测试外部站点 metadata.json 可达性
pub async fn test_http_site(
    Path(site_id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let conn = open_sqlite().map_err(|e| { eprintln!("remote_sync: {}", e); StatusCode::INTERNAL_SERVER_ERROR })?;
    let info = load_site_info(&conn, &site_id)?;
    drop(conn);

    let url = info
        .site_host
        .clone()
        .filter(|value| site_metadata::is_http_url(value))
        .or_else(|| {
            info.env_file_host
                .clone()
                .filter(|value| site_metadata::is_http_url(value))
        });
    let Some(url) = url else {
        return Ok(failed_diagnostic(
            "未配置可用的 HTTP/HTTPS 站点地址",
            serde_json::Map::new(),
        ));
    };

    if !site_metadata::is_http_url(&url) {
        return Ok(failed_diagnostic(
            "http_host 只允许 HTTP/HTTPS 地址",
            serde_json::Map::new(),
        ));
    }

    let metadata_url = site_metadata::metadata_url(&url);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| { eprintln!("remote_sync: {}", e); StatusCode::INTERNAL_SERVER_ERROR })?;
    let start = Instant::now();
    match client.get(&metadata_url).send().await {
        Ok(resp) => {
            let status = resp.status();
            let extras = serde_json::Map::from_iter([
                ("url".to_string(), json!(metadata_url)),
                ("code".to_string(), json!(status.as_u16())),
                (
                    "latency_ms".to_string(),
                    json!(start.elapsed().as_millis() as u64),
                ),
            ]);
            if status.is_success() {
                Ok(ok_diagnostic("metadata.json 可达", extras))
            } else {
                Ok(failed_diagnostic(
                    format!("metadata.json 返回异常状态: {}", status),
                    extras,
                ))
            }
        }
        Err(e) => Ok(failed_diagnostic(
            format!("请求失败: {}", e),
            serde_json::Map::from_iter([("url".to_string(), json!(metadata_url))]),
        )),
    }
}

/// 运行时状态查询
pub async fn runtime_status() -> Result<Json<serde_json::Value>, StatusCode> {
    use crate::data_interface::db_model::MQTT_CONNECT_STATUS;
    use crate::web_server::remote_runtime::REMOTE_RUNTIME;
    let guard = REMOTE_RUNTIME.read().await;
    let active = guard.is_some();
    let env_id = guard.as_ref().map(|s| s.env_id.clone());
    let mqtt_connected = {
        let l = MQTT_CONNECT_STATUS.lock().await;
        l.clone()
    };
    Ok(Json(json!({
        "status":"success",
        "active": active,
        "env_id": env_id,
        "mqtt_connected": mqtt_connected,
    })))
}

/// 运行时 DbOption 简要配置（只读）
pub async fn runtime_config() -> Result<Json<serde_json::Value>, StatusCode> {
    fn normalize_text(value: Option<String>) -> Option<String> {
        value.and_then(|item| {
            let trimmed = item.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
    }

    fn parse_root_value(content: &str, key: &str) -> Option<String> {
        for segment in content.split_inclusive('\n') {
            let trimmed = segment.trim();
            if trimmed.starts_with('#') || trimmed.starts_with('[') {
                if trimmed.starts_with('[') {
                    break;
                }
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix(key) {
                if !rest.trim_start().starts_with('=') {
                    continue;
                }
                return trimmed
                    .split_once('=')
                    .map(|(_, value)| value.trim().to_string());
            }
        }
        None
    }

    fn parse_string_value(content: &str, key: &str) -> Option<String> {
        let raw = parse_root_value(content, key)?;
        let trimmed = raw.trim();
        if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
            return normalize_text(Some(trimmed[1..trimmed.len() - 1].to_string()));
        }
        normalize_text(Some(trimmed.to_string()))
    }

    fn parse_u16_value(content: &str, key: &str) -> Option<u16> {
        parse_root_value(content, key)?.trim().parse::<u16>().ok()
    }

    fn parse_bool_value(content: &str, key: &str) -> Option<bool> {
        match parse_root_value(content, key)?.trim() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        }
    }

    fn parse_u32_array_value(content: &str, key: &str) -> Option<Vec<u32>> {
        let raw = parse_root_value(content, key)?;
        let trimmed = raw.trim();
        if !(trimmed.starts_with('[') && trimmed.ends_with(']')) {
            return None;
        }
        let inner = &trimmed[1..trimmed.len() - 1];
        if inner.trim().is_empty() {
            return Some(Vec::new());
        }
        Some(
            inner
                .split(',')
                .filter_map(|item| item.trim().parse::<u32>().ok())
                .collect(),
        )
    }

    let cfg_name =
        std::env::var("DB_OPTION_FILE").unwrap_or_else(|_| "db_options/DbOption".to_string());
    let cfg_file = format!("{}.toml", cfg_name);

    let opt = aios_core::get_db_option();
    let fallback_location_dbs: Vec<u32> = opt.location_dbs.clone().unwrap_or_default();

    let mut mqtt_host = normalize_text(Some(opt.mqtt_host.clone()));
    let mut mqtt_port = Some(opt.mqtt_port);
    let mut file_server_host = normalize_text(Some(opt.file_server_host.clone()));
    let mut location = normalize_text(Some(opt.location.clone()));
    let mut location_dbs = fallback_location_dbs;
    let mut sync_live = opt.sync_live.unwrap_or(false);

    if let Ok(content) = std::fs::read_to_string(&cfg_file) {
        mqtt_host = parse_string_value(&content, "mqtt_host").or(mqtt_host);
        mqtt_port = parse_u16_value(&content, "mqtt_port").or(mqtt_port);
        file_server_host = parse_string_value(&content, "file_server_host").or(file_server_host);
        location = parse_string_value(&content, "location").or(location);
        location_dbs = parse_u32_array_value(&content, "location_dbs").unwrap_or(location_dbs);
        sync_live = parse_bool_value(&content, "sync_live").unwrap_or(sync_live);
    }

    Ok(Json(json!({
        "status":"success",
        "source": cfg_file,
        "config": {
            "mqtt_host": mqtt_host,
            "mqtt_port": mqtt_port,
            "file_server_host": file_server_host,
            "location": location,
            "location_dbs": location_dbs,
            "sync_live": sync_live,
        }
    })))
}

/// 从配置文件导入/生成一个环境
pub async fn import_env_from_dboption() -> Result<Json<serde_json::Value>, StatusCode> {
    let opt = aios_core::get_db_option();
    let conn = match open_sqlite() {
        Ok(conn) => conn,
        Err(e) => {
            return Ok(action_failed(
                format!("打开协同组存储失败: {}", e),
                serde_json::Map::from_iter([("id".to_string(), serde_json::Value::Null)]),
            ));
        }
    };
    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let name = format!(
        "导入环境 - {}",
        chrono::Local::now().format("%Y%m%d_%H%M%S")
    );
    let location_dbs_str = opt.location_dbs.as_ref().map(|v| {
        v.iter()
            .map(|x| x.to_string())
            .collect::<Vec<_>>()
            .join(",")
    });
    if let Err(e) = conn.execute(
        "INSERT INTO remote_sync_envs (id, name, mqtt_host, mqtt_port, file_server_host, location, location_dbs, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
        rusqlite::params![
            id.clone(),
            name,
            opt.mqtt_host,
            (opt.mqtt_port as i64),
            opt.file_server_host,
            opt.location,
            location_dbs_str,
            now,
        ],
    ) {
        return Ok(action_failed(
            format!("从当前配置导入协同组失败: {}", e),
            serde_json::Map::from_iter([("id".to_string(), json!(id))]),
        ));
    }
    Ok(action_success(
        "已从当前配置导入协同组",
        serde_json::Map::from_iter([("id".to_string(), json!(id))]),
    ))
}

#[derive(Debug, Deserialize)]
pub struct LogQueryParams {
    pub env_id: Option<String>,
    pub target_site: Option<String>,
    pub site_id: Option<String>,
    pub status: Option<String>,
    pub direction: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub keyword: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

pub async fn list_logs(
    Query(params): Query<LogQueryParams>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let conn = open_sqlite().map_err(|e| { eprintln!("remote_sync: {}", e); StatusCode::INTERNAL_SERVER_ERROR })?;

    let limit = params.limit.unwrap_or(50).min(500);
    let offset = params.offset.unwrap_or(0);

    let mut conditions: Vec<String> = Vec::new();
    let mut values: Vec<SqlValue> = Vec::new();

    if let Some(env_id) = params.env_id.filter(|s| !s.is_empty()) {
        conditions.push("env_id = ?".to_string());
        values.push(SqlValue::Text(env_id));
    }
    if let Some(site_id) = params.site_id.filter(|s| !s.is_empty()) {
        conditions.push("site_id = ?".to_string());
        values.push(SqlValue::Text(site_id));
    }
    if let Some(target_site) = params.target_site.filter(|s| !s.is_empty()) {
        conditions.push("target_site = ?".to_string());
        values.push(SqlValue::Text(target_site));
    }
    if let Some(status) = params.status.filter(|s| !s.is_empty()) {
        conditions.push("status = ?".to_string());
        values.push(SqlValue::Text(status));
    }
    if let Some(direction) = params.direction.filter(|s| !s.is_empty()) {
        conditions.push("direction = ?".to_string());
        values.push(SqlValue::Text(direction));
    }
    if let Some(start) = params.start.filter(|s| !s.is_empty()) {
        conditions.push("created_at >= ?".to_string());
        values.push(SqlValue::Text(start));
    }
    if let Some(end) = params.end.filter(|s| !s.is_empty()) {
        conditions.push("created_at <= ?".to_string());
        values.push(SqlValue::Text(end));
    }
    if let Some(keyword) = params.keyword.filter(|s| !s.trim().is_empty()) {
        let pattern = format!("%{}%", keyword.trim());
        conditions.push("file_path LIKE ?".to_string());
        values.push(SqlValue::Text(pattern));
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", conditions.join(" AND "))
    };

    let count_sql = format!("SELECT COUNT(*) FROM remote_sync_logs{}", where_clause);
    let total: i64 = conn
        .prepare(&count_sql)
        .map_err(|e| { eprintln!("remote_sync: {}", e); StatusCode::INTERNAL_SERVER_ERROR })?
        .query_row(
            rusqlite::params_from_iter(values.clone().into_iter()),
            |row| row.get(0),
        )
        .map_err(|e| { eprintln!("remote_sync: {}", e); StatusCode::INTERNAL_SERVER_ERROR })?;

    let mut list_params = values.clone();
    list_params.push(SqlValue::Integer(limit as i64));
    list_params.push(SqlValue::Integer(offset as i64));

    let list_sql = format!(
        "SELECT id, task_id, env_id, source_env, target_site, site_id, direction, file_path,
                file_size, record_count, status, error_message, notes, started_at,
                completed_at, created_at, updated_at
         FROM remote_sync_logs{}
         ORDER BY created_at DESC
         LIMIT ? OFFSET ?",
        where_clause
    );

    let mut stmt = conn
        .prepare(&list_sql)
        .map_err(|e| { eprintln!("remote_sync: {}", e); StatusCode::INTERNAL_SERVER_ERROR })?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(list_params.into_iter()), |row| {
            Ok(RemoteSyncLogRecord {
                id: row.get(0)?,
                task_id: row.get(1).ok(),
                env_id: row.get(2).ok(),
                source_env: row.get(3).ok(),
                target_site: row.get(4).ok(),
                site_id: row.get(5).ok(),
                direction: row.get(6).ok(),
                file_path: row.get(7).ok(),
                file_size: row
                    .get::<_, Option<i64>>(8)
                    .ok()
                    .flatten()
                    .and_then(|v| u64::try_from(v).ok()),
                record_count: row
                    .get::<_, Option<i64>>(9)
                    .ok()
                    .flatten()
                    .and_then(|v| u64::try_from(v).ok()),
                status: row.get(10)?,
                error_message: row.get(11).ok(),
                notes: row.get(12).ok(),
                started_at: row.get(13).ok(),
                completed_at: row.get(14).ok(),
                created_at: row.get(15)?,
                updated_at: row.get(16)?,
            })
        })
        .map_err(|e| { eprintln!("remote_sync: {}", e); StatusCode::INTERNAL_SERVER_ERROR })?;

    let mut items = Vec::new();
    for row in rows {
        items.push(row.map_err(|e| { eprintln!("remote_sync: {}", e); StatusCode::INTERNAL_SERVER_ERROR })?);
    }

    Ok(Json(json!({
        "status": "success",
        "items": items,
        "total": total,
        "limit": limit,
        "offset": offset,
    })))
}

#[derive(Debug, Deserialize)]
pub struct DailyStatsQuery {
    pub env_id: Option<String>,
    pub target_site: Option<String>,
    pub days: Option<u32>,
}

pub async fn daily_stats(
    Query(params): Query<DailyStatsQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let conn = open_sqlite().map_err(|e| { eprintln!("remote_sync: {}", e); StatusCode::INTERNAL_SERVER_ERROR })?;

    let days = params.days.unwrap_or(7).max(1).min(90);
    let start_time = (chrono::Utc::now() - chrono::Duration::days(days as i64)).to_rfc3339();

    let mut conditions: Vec<String> = vec!["created_at >= ?".to_string()];
    let mut values: Vec<SqlValue> = vec![SqlValue::Text(start_time)];

    if let Some(env_id) = params.env_id.filter(|s| !s.is_empty()) {
        conditions.push("env_id = ?".to_string());
        values.push(SqlValue::Text(env_id));
    }
    if let Some(target_site) = params.target_site.filter(|s| !s.is_empty()) {
        conditions.push("target_site = ?".to_string());
        values.push(SqlValue::Text(target_site));
    }

    let where_clause = format!(" WHERE {}", conditions.join(" AND "));

    let sql = format!(
        "SELECT substr(created_at, 1, 10) AS day,
                COUNT(*) AS total,
                SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END) AS completed,
                SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END) AS failed,
                SUM(COALESCE(record_count, 0)) AS record_count,
                SUM(CASE WHEN status = 'completed' THEN COALESCE(file_size, 0) ELSE 0 END) AS total_bytes
         FROM remote_sync_logs
         {}
         GROUP BY day
         ORDER BY day DESC
         LIMIT ?",
        where_clause
    );

    values.push(SqlValue::Integer(days as i64));

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| { eprintln!("remote_sync: {}", e); StatusCode::INTERNAL_SERVER_ERROR })?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(values.into_iter()), |row| {
            Ok(json!({
                "day": row.get::<_, String>(0)?,
                "total": row.get::<_, i64>(1).unwrap_or(0),
                "completed": row.get::<_, i64>(2).unwrap_or(0),
                "failed": row.get::<_, i64>(3).unwrap_or(0),
                "record_count": row.get::<_, i64>(4).unwrap_or(0),
                "total_bytes": row.get::<_, i64>(5).unwrap_or(0),
            }))
        })
        .map_err(|e| { eprintln!("remote_sync: {}", e); StatusCode::INTERNAL_SERVER_ERROR })?;

    let mut items = Vec::new();
    for row in rows {
        items.push(row.map_err(|e| { eprintln!("remote_sync: {}", e); StatusCode::INTERNAL_SERVER_ERROR })?);
    }

    Ok(Json(json!({
        "status": "success",
        "items": items,
    })))
}

#[derive(Debug, Deserialize)]
pub struct FlowStatsQuery {
    pub env_id: Option<String>,
    pub limit: Option<usize>,
}

pub async fn flow_stats(
    Query(params): Query<FlowStatsQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let conn = open_sqlite().map_err(|e| { eprintln!("remote_sync: {}", e); StatusCode::INTERNAL_SERVER_ERROR })?;

    let limit = params.limit.unwrap_or(20).max(1).min(200);

    let mut conditions: Vec<String> = Vec::new();
    let mut values: Vec<SqlValue> = Vec::new();

    if let Some(env_id) = params.env_id.filter(|s| !s.is_empty()) {
        conditions.push("env_id = ?".to_string());
        values.push(SqlValue::Text(env_id));
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", conditions.join(" AND "))
    };

    let sql = format!(
        "SELECT COALESCE(env_id, '') AS env_id,
                COALESCE(target_site, '') AS target_site,
                COALESCE(direction, '') AS direction,
                COUNT(*) AS total,
                SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END) AS completed,
                SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END) AS failed,
                SUM(COALESCE(record_count, 0)) AS record_count,
                SUM(CASE WHEN status = 'completed' THEN COALESCE(file_size, 0) ELSE 0 END) AS total_bytes
         FROM remote_sync_logs
         {}
         GROUP BY env_id, target_site, direction
         ORDER BY total_bytes DESC, total DESC
         LIMIT ?",
        where_clause
    );

    values.push(SqlValue::Integer(limit as i64));

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| { eprintln!("remote_sync: {}", e); StatusCode::INTERNAL_SERVER_ERROR })?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(values.into_iter()), |row| {
            Ok(json!({
                "env_id": row.get::<_, String>(0).unwrap_or_default(),
                "target_site": row.get::<_, String>(1).unwrap_or_default(),
                "direction": row.get::<_, String>(2).unwrap_or_default(),
                "total": row.get::<_, i64>(3).unwrap_or(0),
                "completed": row.get::<_, i64>(4).unwrap_or(0),
                "failed": row.get::<_, i64>(5).unwrap_or(0),
                "record_count": row.get::<_, i64>(6).unwrap_or(0),
                "total_bytes": row.get::<_, i64>(7).unwrap_or(0),
            }))
        })
        .map_err(|e| { eprintln!("remote_sync: {}", e); StatusCode::INTERNAL_SERVER_ERROR })?;

    let mut items = Vec::new();
    for row in rows {
        items.push(row.map_err(|e| { eprintln!("remote_sync: {}", e); StatusCode::INTERNAL_SERVER_ERROR })?);
    }

    Ok(Json(json!({
        "status": "success",
        "items": items,
    })))
}

pub async fn get_site_metadata(
    Path(site_id): Path<String>,
    Query(query): Query<MetadataQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let context = load_site_metadata(&site_id, query.refresh, query.cache_only).await?;
    let MetadataLoadContext {
        info,
        metadata,
        source,
        fetched_at,
        cache_path,
        warnings,
        http_base,
        local_base,
    } = context;

    let cache_path_str = cache_path
        .as_ref()
        .map(|path| path.to_string_lossy().to_string());
    let local_base_str = local_base
        .as_ref()
        .map(|path| path.to_string_lossy().to_string());

    Ok(Json(json!({
        "status": "success",
        "source": source.as_str(),
        "fetched_at": fetched_at,
        "entry_count": metadata.entries.len(),
        "cache_path": cache_path_str,
        "http_base": http_base,
        "local_base": local_base_str,
        "warnings": warnings,
        "env": {
            "id": info.env_id,
            "name": info.env_name,
            "file_host": info.env_file_host,
        },
        "site": {
            "id": info.site_id,
            "name": info.site_name,
            "host": info.site_host,
        },
        "metadata": metadata,
    })))
}

pub async fn serve_site_files(
    Path((site_id, requested_path)): Path<(String, String)>,
    OriginalUri(original_uri): OriginalUri,
    req: Request<Body>,
) -> Result<Response, StatusCode> {
    serve_site_files_impl(site_id, requested_path, original_uri, req).await
}

pub async fn serve_site_files_root(
    Path(site_id): Path<String>,
    OriginalUri(original_uri): OriginalUri,
    req: Request<Body>,
) -> Result<Response, StatusCode> {
    serve_site_files_impl(site_id, String::new(), original_uri, req).await
}

async fn serve_site_files_impl(
    site_id: String,
    requested_path: String,
    original_uri: Uri,
    mut req: Request<Body>,
) -> Result<Response, StatusCode> {
    let conn = open_sqlite().map_err(|e| { eprintln!("remote_sync: {}", e); StatusCode::INTERNAL_SERVER_ERROR })?;
    let info = load_site_info(&conn, &site_id)?;
    drop(conn);

    let local_base = resolve_local_base(&info).ok_or(StatusCode::NOT_FOUND)?;

    let mut service = ServeDir::new(local_base);

    let mut new_path = if requested_path.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", requested_path)
    };
    if let Some(query) = original_uri.query() {
        new_path.push('?');
        new_path.push_str(query);
    }
    let new_uri: Uri = new_path
        .parse()
        .map_err(|e| { eprintln!("remote_sync: {}", e); StatusCode::INTERNAL_SERVER_ERROR })?;
    *req.uri_mut() = new_uri;

    let response = service
        .oneshot(req)
        .await
        .map_err(|e| { eprintln!("remote_sync: {}", e); StatusCode::INTERNAL_SERVER_ERROR })?;
    Ok(response.map(Body::new))
}

async fn load_site_metadata(
    site_id: &str,
    refresh: bool,
    cache_only: bool,
) -> Result<MetadataLoadContext, StatusCode> {
    let conn = open_sqlite().map_err(|e| { eprintln!("remote_sync: {}", e); StatusCode::INTERNAL_SERVER_ERROR })?;
    let info = load_site_info(&conn, site_id)?;
    drop(conn);

    let local_base = resolve_local_base(&info);

    let http_base = info
        .site_host
        .as_deref()
        .filter(site_metadata::is_http_url_ref)
        .map(|s| s.to_string())
        .or_else(|| {
            info.env_file_host
                .as_deref()
                .filter(site_metadata::is_http_url_ref)
                .map(|s| s.to_string())
        });

    let mut warnings = Vec::new();
    let mut source = MetadataSource::Unknown;
    let mut fetched_at = site_metadata::timestamp_now();
    let mut cache_path: Option<PathBuf> = None;
    let mut metadata_opt: Option<SiteMetadataFile> = None;

    let mut cached: Option<CachedMetadata> =
        match site_metadata::read_cache(Some(info.env_id.as_str()), Some(info.site_id.as_str()))
            .await
        {
            Ok(cache) => Some(cache),
            Err(err) => {
                let is_not_found = err
                    .downcast_ref::<std::io::Error>()
                    .map(|io_err| io_err.kind() == ErrorKind::NotFound)
                    .unwrap_or(false);
                if !is_not_found {
                    warnings.push(format!("读取缓存失败: {}", err));
                }
                None
            }
        };

    if let Some(base) = &local_base {
        match site_metadata::read_local_metadata(base).await {
            Ok(mut metadata) => {
                fill_metadata_defaults(&mut metadata, &info);
                fetched_at = site_metadata::timestamp_now();
                source = MetadataSource::LocalPath;
                match site_metadata::write_cache(
                    metadata.env_id.as_deref(),
                    metadata.site_id.as_deref(),
                    &metadata,
                )
                .await
                {
                    Ok(path) => {
                        cache_path = Some(path);
                    }
                    Err(err) => warnings.push(format!("写入本地缓存失败: {}", err)),
                }
                metadata_opt = Some(metadata);
            }
            Err(err) => {
                warnings.push(format!("读取本地元数据失败: {}", err));
            }
        }
    }

    if metadata_opt.is_none() && !cache_only {
        if let Some(http_base) = &http_base {
            if refresh || cached.is_none() {
                match site_metadata::fetch_remote_metadata(http_base).await {
                    Ok(mut metadata) => {
                        fill_metadata_defaults(&mut metadata, &info);
                        fetched_at = site_metadata::timestamp_now();
                        source = MetadataSource::RemoteHttp;
                        match site_metadata::write_cache(
                            metadata.env_id.as_deref(),
                            metadata.site_id.as_deref(),
                            &metadata,
                        )
                        .await
                        {
                            Ok(path) => {
                                cache_path = Some(path);
                            }
                            Err(err) => warnings.push(format!("写入缓存失败: {}", err)),
                        }
                        metadata_opt = Some(metadata);
                    }
                    Err(err) => warnings.push(format!("拉取远程元数据失败: {}", err)),
                }
            }
        }
    }

    if metadata_opt.is_none() {
        if let Some(cache) = cached.take() {
            fetched_at = cache.cached_at.clone();
            source = MetadataSource::Cache;
            cache_path = Some(site_metadata::metadata_cache_path(
                Some(info.env_id.as_str()),
                Some(info.site_id.as_str()),
            ));
            metadata_opt = Some(cache.metadata);
        }
    }

    let metadata = metadata_opt
        .map(|mut data| {
            fill_metadata_defaults(&mut data, &info);
            data
        })
        .unwrap_or_else(|| {
            let mut data = SiteMetadataFile::default();
            fill_metadata_defaults(&mut data, &info);
            data
        });

    Ok(MetadataLoadContext {
        info,
        metadata,
        source,
        fetched_at,
        cache_path,
        warnings,
        http_base,
        local_base,
    })
}

fn load_site_info(conn: &rusqlite::Connection, site_id: &str) -> Result<SiteInfo, StatusCode> {
    let mut stmt_site = conn
        .prepare("SELECT env_id, name, http_host FROM remote_sync_sites WHERE id = ?1 LIMIT 1")
        .map_err(|e| { eprintln!("remote_sync: {}", e); StatusCode::INTERNAL_SERVER_ERROR })?;
    let site_row = stmt_site
        .query_row([site_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .optional()
        .map_err(|e| { eprintln!("remote_sync: {}", e); StatusCode::INTERNAL_SERVER_ERROR })?;
    let (env_id, site_name, site_host) = match site_row {
        Some(row) => row,
        None => return Err(StatusCode::NOT_FOUND),
    };

    let mut stmt_env = conn
        .prepare("SELECT name, file_server_host FROM remote_sync_envs WHERE id = ?1 LIMIT 1")
        .map_err(|e| { eprintln!("remote_sync: {}", e); StatusCode::INTERNAL_SERVER_ERROR })?;
    let env_row = stmt_env
        .query_row([env_id.as_str()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .optional()
        .map_err(|e| { eprintln!("remote_sync: {}", e); StatusCode::INTERNAL_SERVER_ERROR })?;
    let (env_name, env_file_host) = match env_row {
        Some((name, host)) => (Some(name), host),
        None => (None, None),
    };

    Ok(SiteInfo {
        env_id,
        env_name,
        env_file_host,
        site_id: site_id.to_string(),
        site_name,
        site_host,
    })
}

fn resolve_local_base(info: &SiteInfo) -> Option<PathBuf> {
    info.env_file_host
        .as_deref()
        .filter(site_metadata::is_local_path_hint_ref)
        .map(site_metadata::normalize_local_base)
}

fn fill_metadata_defaults(metadata: &mut SiteMetadataFile, info: &SiteInfo) {
    if metadata.env_id.is_none() {
        metadata.env_id = Some(info.env_id.clone());
    }
    if metadata.env_name.is_none() {
        metadata.env_name = info.env_name.clone();
    }
    if metadata.site_id.is_none() {
        metadata.site_id = Some(info.site_id.clone());
    }
    if metadata.site_name.is_none() {
        metadata.site_name = Some(info.site_name.clone());
    }
    if metadata.site_http_host.is_none() {
        metadata.site_http_host = info
            .site_host
            .clone()
            .or_else(|| info.env_file_host.clone());
    }
    if metadata.generated_at.is_empty() {
        metadata.generated_at = site_metadata::timestamp_now();
    }
}

// ============================================================================
// Helper functions for topology management
// ============================================================================

/// 列出所有环境（用于拓扑配置）
pub async fn list_all_envs() -> anyhow::Result<Vec<RemoteSyncEnv>> {
    let conn = open_sqlite().map_err(|e| anyhow::anyhow!("打开数据库失败: {}", e))?;
    let mut stmt = conn.prepare(
        "SELECT id, name, mqtt_host, mqtt_port, file_server_host, location, location_dbs, 
                reconnect_initial_ms, reconnect_max_ms, created_at, updated_at 
         FROM remote_sync_envs 
         ORDER BY created_at DESC",
    )?;

    let envs = stmt
        .query_map([], |row| {
            Ok(RemoteSyncEnv {
                id: row.get(0)?,
                name: row.get(1)?,
                mqtt_host: row.get(2)?,
                mqtt_port: row.get(3)?,
                file_server_host: row.get(4)?,
                location: row.get(5)?,
                location_dbs: row.get(6)?,
                reconnect_initial_ms: row.get(7)?,
                reconnect_max_ms: row.get(8)?,
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(envs)
}

/// 列出所有站点（用于拓扑配置）
pub async fn list_all_sites() -> anyhow::Result<Vec<RemoteSyncSite>> {
    let conn = open_sqlite().map_err(|e| anyhow::anyhow!("打开数据库失败: {}", e))?;
    let mut stmt = conn.prepare(
        "SELECT id, env_id, name, location, http_host, dbnums, notes, created_at, updated_at 
         FROM remote_sync_sites 
         ORDER BY created_at DESC",
    )?;

    let sites = stmt
        .query_map([], |row| {
            Ok(RemoteSyncSite {
                id: row.get(0)?,
                env_id: row.get(1)?,
                name: row.get(2)?,
                location: row.get(3)?,
                http_host: row.get(4)?,
                dbnums: row.get(5)?,
                notes: row.get(6)?,
                created_at: row.get(7)?,
                updated_at: row.get(8)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(sites)
}

/// 创建或更新环境
pub async fn create_or_update_env(env: &RemoteSyncEnv) -> anyhow::Result<()> {
    let conn = open_sqlite().map_err(|e| anyhow::anyhow!("打开数据库失败: {}", e))?;
    conn.execute(
        "INSERT OR REPLACE INTO remote_sync_envs 
         (id, name, mqtt_host, mqtt_port, file_server_host, location, location_dbs, 
          reconnect_initial_ms, reconnect_max_ms, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        rusqlite::params![
            &env.id,
            &env.name,
            &env.mqtt_host,
            &env.mqtt_port,
            &env.file_server_host,
            &env.location,
            &env.location_dbs,
            &env.reconnect_initial_ms,
            &env.reconnect_max_ms,
            &env.created_at,
            &env.updated_at,
        ],
    )?;
    Ok(())
}

/// 创建或更新站点
pub async fn create_or_update_site(site: &RemoteSyncSite) -> anyhow::Result<()> {
    let conn = open_sqlite().map_err(|e| anyhow::anyhow!("打开数据库失败: {}", e))?;
    conn.execute(
        "INSERT OR REPLACE INTO remote_sync_sites 
         (id, env_id, name, location, http_host, dbnums, notes, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            &site.id,
            &site.env_id,
            &site.name,
            &site.location,
            &site.http_host,
            &site.dbnums,
            &site.notes,
            &site.created_at,
            &site.updated_at,
        ],
    )?;
    Ok(())
}

/// 删除所有环境
pub async fn delete_all_envs() -> anyhow::Result<()> {
    let conn = open_sqlite().map_err(|e| anyhow::anyhow!("打开数据库失败: {}", e))?;
    conn.execute("DELETE FROM remote_sync_envs", [])?;
    Ok(())
}

/// 删除所有站点
pub async fn delete_all_sites() -> anyhow::Result<()> {
    let conn = open_sqlite().map_err(|e| anyhow::anyhow!("打开数据库失败: {}", e))?;
    conn.execute("DELETE FROM remote_sync_sites", [])?;
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════
// v2 · ROADMAP M2 · Task Queue / Failed Tasks / Env Config
// 契约见 design/collaboration-v2/ROADMAP.md 和 ui/admin/src/types/collaboration.ts
// ═══════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteSyncActiveTask {
    pub task_id: String,
    pub env_id: String,
    pub site_id: Option<String>,
    pub site_name: Option<String>,
    pub task_name: String,
    pub file_path: Option<String>,
    pub progress: f64,
    pub status: String,
    pub started_at: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteSyncFailedTask {
    pub id: String,
    pub task_type: String,
    pub env_id: String,
    pub site_id: Option<String>,
    pub site_name: Option<String>,
    pub site: String,
    pub error: String,
    pub retry_count: i64,
    pub max_retries: i64,
    pub first_failed_at: String,
    pub next_retry_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteSyncEnvConfig {
    pub auto_detect: bool,
    pub detect_interval: i64,
    pub auto_sync: bool,
    pub batch_size: i64,
    pub max_concurrent: i64,
    pub reconnect_initial_ms: i64,
    pub reconnect_max_ms: i64,
    pub enable_notifications: bool,
    pub log_retention_days: i64,
}

impl Default for RemoteSyncEnvConfig {
    fn default() -> Self {
        Self {
            auto_detect: true,
            detect_interval: 30,
            auto_sync: false,
            batch_size: 10,
            max_concurrent: 3,
            reconnect_initial_ms: 1000,
            reconnect_max_ms: 30000,
            enable_notifications: true,
            log_retention_days: 30,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ListActiveTasksQuery {
    pub env_id: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct ListFailedTasksQuery {
    pub env_id: Option<String>,
    pub status: Option<String>,
    pub exhausted: Option<bool>,
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct CleanupFailedTasksQuery {
    pub exhausted: Option<bool>,
    pub env_id: Option<String>,
}

// ────────────────────────────────────────────────────────────────
// B1 · Active Task APIs
// ────────────────────────────────────────────────────────────────

pub async fn list_active_tasks(
    Query(params): Query<ListActiveTasksQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let conn = open_sqlite().map_err(|e| {
        eprintln!("[list_active_tasks] open_sqlite failed: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "数据库连接失败" })),
        )
    })?;

    let limit = params.limit.unwrap_or(50).clamp(1, 500);
    let (query, bind_env) = match &params.env_id {
        Some(_) => (
            "SELECT task_id, env_id, site_id, site_name, task_name, file_path, progress, status, started_at, updated_at
             FROM remote_sync_tasks WHERE env_id = ?1 ORDER BY updated_at DESC LIMIT ?2"
                .to_string(),
            true,
        ),
        None => (
            "SELECT task_id, env_id, site_id, site_name, task_name, file_path, progress, status, started_at, updated_at
             FROM remote_sync_tasks ORDER BY updated_at DESC LIMIT ?1"
                .to_string(),
            false,
        ),
    };

    let mut stmt = conn.prepare(&query).map_err(|e| {
        eprintln!("[list_active_tasks] prepare failed: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "查询准备失败" })),
        )
    })?;

    let rows: Vec<RemoteSyncActiveTask> = if bind_env {
        stmt.query_map(rusqlite::params![params.env_id.as_ref().unwrap(), limit], map_active_row)
    } else {
        stmt.query_map(rusqlite::params![limit], map_active_row)
    }
    .map_err(|e| {
        eprintln!("[list_active_tasks] query_map failed: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "查询失败" })),
        )
    })?
    .filter_map(|r| r.ok())
    .collect();

    Ok(Json(
        json!({ "status": "success", "items": rows, "total": rows.len() }),
    ))
}

fn map_active_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RemoteSyncActiveTask> {
    Ok(RemoteSyncActiveTask {
        task_id: row.get(0)?,
        env_id: row.get(1)?,
        site_id: row.get(2)?,
        site_name: row.get(3)?,
        task_name: row.get(4)?,
        file_path: row.get(5)?,
        progress: row.get::<_, Option<f64>>(6)?.unwrap_or(0.0),
        status: row.get(7)?,
        started_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

pub async fn abort_active_task(
    Path(task_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let conn = open_sqlite().map_err(|e| {
        eprintln!("[abort_active_task] open_sqlite failed: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "数据库连接失败" })),
        )
    })?;
    let now = chrono::Utc::now().to_rfc3339();
    let affected = conn
        .execute(
            "UPDATE remote_sync_tasks SET status = 'Cancelled', updated_at = ?1 WHERE task_id = ?2",
            rusqlite::params![now, task_id],
        )
        .map_err(|e| {
            eprintln!("[abort_active_task] update failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "更新失败" })),
            )
        })?;
    if affected == 0 {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "任务不存在" })),
        ));
    }
    Ok(Json(
        json!({ "status": "success", "task_id": task_id, "new_status": "Cancelled" }),
    ))
}

// ────────────────────────────────────────────────────────────────
// B2 · Failed Task APIs
// ────────────────────────────────────────────────────────────────

pub async fn list_failed_tasks(
    Query(params): Query<ListFailedTasksQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let conn = open_sqlite().map_err(|e| {
        eprintln!("[list_failed_tasks] open_sqlite failed: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "数据库连接失败" })),
        )
    })?;

    let limit = params.limit.unwrap_or(100).clamp(1, 500);

    let mut clauses: Vec<String> = Vec::new();
    let mut binds: Vec<SqlValue> = Vec::new();
    if let Some(env_id) = &params.env_id {
        clauses.push("env_id = ?".into());
        binds.push(SqlValue::Text(env_id.clone()));
    }
    match params.status.as_deref() {
        Some("pending") => clauses.push("retry_count < max_retries".into()),
        Some("exhausted") => clauses.push("retry_count >= max_retries".into()),
        _ => {}
    }
    if params.exhausted == Some(true) && !clauses.iter().any(|c| c.contains("retry_count")) {
        clauses.push("retry_count >= max_retries".into());
    }

    let where_sql = if clauses.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", clauses.join(" AND "))
    };
    let sql = format!(
        "SELECT id, task_type, env_id, site_id, site_name, error, retry_count, max_retries,
         first_failed_at, next_retry_at, created_at, updated_at
         FROM remote_sync_failed_tasks{} ORDER BY updated_at DESC LIMIT ?",
        where_sql
    );

    binds.push(SqlValue::Integer(limit));

    let mut stmt = conn.prepare(&sql).map_err(|e| {
        eprintln!("[list_failed_tasks] prepare failed: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "查询准备失败" })),
        )
    })?;

    let param_refs: Vec<&dyn rusqlite::ToSql> = binds
        .iter()
        .map(|v| v as &dyn rusqlite::ToSql)
        .collect();

    let rows: Vec<RemoteSyncFailedTask> = stmt
        .query_map(param_refs.as_slice(), map_failed_row)
        .map_err(|e| {
            eprintln!("[list_failed_tasks] query_map failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "查询失败" })),
            )
        })?
        .filter_map(|r| r.ok())
        .collect();

    Ok(Json(
        json!({ "status": "success", "items": rows, "total": rows.len() }),
    ))
}

fn map_failed_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RemoteSyncFailedTask> {
    let site_name: Option<String> = row.get(4)?;
    let site_id: Option<String> = row.get(3)?;
    Ok(RemoteSyncFailedTask {
        id: row.get(0)?,
        task_type: row.get(1)?,
        env_id: row.get(2)?,
        site: site_name
            .clone()
            .or_else(|| site_id.clone())
            .unwrap_or_else(|| "-".to_string()),
        site_id,
        site_name,
        error: row.get(5)?,
        retry_count: row.get(6)?,
        max_retries: row.get(7)?,
        first_failed_at: row.get(8)?,
        next_retry_at: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
    })
}

pub async fn retry_failed_task(
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let conn = open_sqlite().map_err(|e| {
        eprintln!("[retry_failed_task] open_sqlite failed: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "数据库连接失败" })),
        )
    })?;
    let existing: Option<(i64, i64)> = conn
        .query_row(
            "SELECT retry_count, max_retries FROM remote_sync_failed_tasks WHERE id = ?1",
            rusqlite::params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| {
            eprintln!("[retry_failed_task] query failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "查询失败" })),
            )
        })?;

    let (retry_count, max_retries) = existing.ok_or((
        StatusCode::NOT_FOUND,
        Json(json!({ "error": "失败任务不存在" })),
    ))?;

    if retry_count >= max_retries {
        return Err((
            StatusCode::CONFLICT,
            Json(json!({
                "error": "已耗尽重试次数，不能再触发重试",
                "retry_count": retry_count,
                "max_retries": max_retries
            })),
        ));
    }

    let now = chrono::Utc::now();
    let next_retry = now + chrono::Duration::seconds(60);
    let now_str = now.to_rfc3339();
    let next_str = next_retry.to_rfc3339();

    conn.execute(
        "UPDATE remote_sync_failed_tasks
         SET retry_count = retry_count + 1, next_retry_at = ?2, updated_at = ?3
         WHERE id = ?1",
        rusqlite::params![id, next_str, now_str],
    )
    .map_err(|e| {
        eprintln!("[retry_failed_task] update failed: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "更新失败" })),
        )
    })?;

    Ok(Json(json!({
        "status": "success",
        "id": id,
        "retry_count": retry_count + 1,
        "max_retries": max_retries,
        "next_retry_at": next_str
    })))
}

pub async fn cleanup_failed_tasks(
    Query(params): Query<CleanupFailedTasksQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let conn = open_sqlite().map_err(|e| {
        eprintln!("[cleanup_failed_tasks] open_sqlite failed: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "数据库连接失败" })),
        )
    })?;

    let only_exhausted = params.exhausted.unwrap_or(true);
    let affected = match (&params.env_id, only_exhausted) {
        (Some(env_id), true) => conn.execute(
            "DELETE FROM remote_sync_failed_tasks WHERE env_id = ?1 AND retry_count >= max_retries",
            rusqlite::params![env_id],
        ),
        (Some(env_id), false) => conn.execute(
            "DELETE FROM remote_sync_failed_tasks WHERE env_id = ?1",
            rusqlite::params![env_id],
        ),
        (None, true) => conn.execute(
            "DELETE FROM remote_sync_failed_tasks WHERE retry_count >= max_retries",
            rusqlite::params![],
        ),
        (None, false) => conn.execute("DELETE FROM remote_sync_failed_tasks", rusqlite::params![]),
    }
    .map_err(|e| {
        eprintln!("[cleanup_failed_tasks] delete failed: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "清理失败" })),
        )
    })?;

    Ok(Json(
        json!({ "status": "success", "cleaned": affected }),
    ))
}

// ────────────────────────────────────────────────────────────────
// B3 · Env Config APIs
// ────────────────────────────────────────────────────────────────

pub async fn get_env_config(
    Path(env_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let conn = open_sqlite().map_err(|e| {
        eprintln!("[get_env_config] open_sqlite failed: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "数据库连接失败" })),
        )
    })?;

    let cfg: Option<RemoteSyncEnvConfig> = conn
        .query_row(
            "SELECT auto_detect, detect_interval, auto_sync, batch_size, max_concurrent,
                    reconnect_initial_ms, reconnect_max_ms, enable_notifications, log_retention_days
             FROM remote_sync_env_config WHERE env_id = ?1",
            rusqlite::params![env_id],
            |row| {
                Ok(RemoteSyncEnvConfig {
                    auto_detect: row.get::<_, i64>(0)? != 0,
                    detect_interval: row.get(1)?,
                    auto_sync: row.get::<_, i64>(2)? != 0,
                    batch_size: row.get(3)?,
                    max_concurrent: row.get(4)?,
                    reconnect_initial_ms: row.get(5)?,
                    reconnect_max_ms: row.get(6)?,
                    enable_notifications: row.get::<_, i64>(7)? != 0,
                    log_retention_days: row.get(8)?,
                })
            },
        )
        .optional()
        .map_err(|e| {
            eprintln!("[get_env_config] query failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "查询失败" })),
            )
        })?;

    let cfg = cfg.unwrap_or_default();
    Ok(Json(serde_json::to_value(&cfg).unwrap_or_default()))
}

pub async fn update_env_config(
    Path(env_id): Path<String>,
    Json(cfg): Json<RemoteSyncEnvConfig>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let conn = open_sqlite().map_err(|e| {
        eprintln!("[update_env_config] open_sqlite failed: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "数据库连接失败" })),
        )
    })?;

    let env_exists: bool = conn
        .query_row(
            "SELECT 1 FROM remote_sync_envs WHERE id = ?1",
            rusqlite::params![env_id],
            |_| Ok(true),
        )
        .optional()
        .map_err(|e| {
            eprintln!("[update_env_config] env lookup failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "查询失败" })),
            )
        })?
        .unwrap_or(false);

    if !env_exists {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "协同组不存在" })),
        ));
    }

    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO remote_sync_env_config
         (env_id, auto_detect, detect_interval, auto_sync, batch_size, max_concurrent,
          reconnect_initial_ms, reconnect_max_ms, enable_notifications, log_retention_days, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT(env_id) DO UPDATE SET
           auto_detect = excluded.auto_detect,
           detect_interval = excluded.detect_interval,
           auto_sync = excluded.auto_sync,
           batch_size = excluded.batch_size,
           max_concurrent = excluded.max_concurrent,
           reconnect_initial_ms = excluded.reconnect_initial_ms,
           reconnect_max_ms = excluded.reconnect_max_ms,
           enable_notifications = excluded.enable_notifications,
           log_retention_days = excluded.log_retention_days,
           updated_at = excluded.updated_at",
        rusqlite::params![
            env_id,
            if cfg.auto_detect { 1 } else { 0 },
            cfg.detect_interval,
            if cfg.auto_sync { 1 } else { 0 },
            cfg.batch_size,
            cfg.max_concurrent,
            cfg.reconnect_initial_ms,
            cfg.reconnect_max_ms,
            if cfg.enable_notifications { 1 } else { 0 },
            cfg.log_retention_days,
            now,
        ],
    )
    .map_err(|e| {
        eprintln!("[update_env_config] upsert failed: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "写入配置失败" })),
        )
    })?;

    Ok(Json(json!({ "status": "success", "env_id": env_id })))
}

// ── M3 B5 · SSE 事件流 handler ──

/// GET /api/remote-sync/events/stream
///
/// 返回 Server-Sent Events 流，前端 useCollaborationStream.ts 消费。
/// 事件类型：active_task_update / failed_task_new / site_status_change /
///           sync_completed / sync_failed / keepalive
async fn remote_sync_events_stream() -> Sse<impl futures::stream::Stream<Item = Result<Event, Infallible>>> {
    let rx = REMOTE_SYNC_EVENT_TX.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|result| async move {
        match result {
            Ok(event) => match serde_json::to_string(&event) {
                Ok(json) => Some(Ok::<_, Infallible>(Event::default().data(json).event("message"))),
                Err(e) => {
                    eprintln!("[remote_sync_sse] serialize error: {}", e);
                    None
                }
            },
            Err(e) => {
                eprintln!("[remote_sync_sse] broadcast recv error: {}", e);
                None
            }
        }
    });
    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keepalive"),
    )
}

