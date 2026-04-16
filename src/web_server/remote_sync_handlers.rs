use axum::{
    body::Body,
    extract::{OriginalUri, Path, Query},
    http::{Request, StatusCode, Uri},
    response::{Html, Json, Response},
};
use rusqlite::{OptionalExtension, types::Value as SqlValue};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{convert::TryFrom, io::ErrorKind, path::PathBuf, time::Instant};
use tokio::time::{Duration, timeout};
use tokio::{fs, net::TcpStream};
use uuid::Uuid;

use crate::web_server::site_metadata::{self, CachedMetadata, MetadataSource, SiteMetadataFile};
use tower::ServiceExt;
use tower_http::services::ServeDir;

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

/// 打开 SQLite（复用部署站点同一文件）
pub fn open_sqlite() -> Result<rusqlite::Connection, Box<dyn std::error::Error>> {
    use config as cfg;

    let cfg_name =
        std::env::var("DB_OPTION_FILE").unwrap_or_else(|_| "db_options/DbOption".to_string());
    let cfg_file = format!("{}.toml", cfg_name);
    let db_path = if std::path::Path::new(&cfg_file).exists() {
        let builder = cfg::Config::builder()
            .add_source(cfg::File::with_name(&cfg_name))
            .build()?;
        builder
            .get_string("deployment_sites_sqlite_path")
            .unwrap_or_else(|_| "deployment_sites.sqlite".to_string())
    } else {
        "deployment_sites.sqlite".to_string()
    };

    eprintln!("打开数据库: {}", db_path);

    let mut conn = rusqlite::Connection::open(&db_path)?;

    // 检查是否为 LiteFS 挂载点
    let is_litefs = db_path.starts_with("/litefs");

    if is_litefs {
        // LiteFS 下推荐设置
        eprintln!("检测到 LiteFS 环境，配置 WAL 模式");
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
    } else {
        // 本地开发环境
        eprintln!("本地开发环境");
    }
    // env 表
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
    // 尝试向已存在表添加新列（忽略错误）
    let _ = conn.execute(
        "ALTER TABLE remote_sync_envs ADD COLUMN reconnect_initial_ms INTEGER",
        rusqlite::params![],
    );
    let _ = conn.execute(
        "ALTER TABLE remote_sync_envs ADD COLUMN reconnect_max_ms INTEGER",
        rusqlite::params![],
    );
    // site 表
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
    // 日志表
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
    Ok(conn)
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
    let conn = open_sqlite().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut stmt = conn
        .prepare(
            "SELECT id, name, mqtt_host, mqtt_port, file_server_host, location, location_dbs, reconnect_initial_ms, reconnect_max_ms, created_at, updated_at FROM remote_sync_envs ORDER BY updated_at DESC",
        )
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
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
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut envs = Vec::new();
    for r in rows {
        envs.push(r.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?);
    }
    Ok(Json(json!({"status":"success","items": envs})))
}

pub async fn create_env(
    Json(req): Json<EnvCreateRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if req.name.trim().is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let conn = open_sqlite().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
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
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(json!({"status":"success","id": id})))
}

pub async fn get_env(Path(id): Path<String>) -> Result<Json<serde_json::Value>, StatusCode> {
    let conn = open_sqlite().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut stmt = conn
        .prepare("SELECT id, name, mqtt_host, mqtt_port, file_server_host, location, location_dbs, reconnect_initial_ms, reconnect_max_ms, created_at, updated_at FROM remote_sync_envs WHERE id = ?1 LIMIT 1")
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut rows = stmt
        .query(rusqlite::params![id])
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if let Some(row) = rows.next().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)? {
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
    let conn = open_sqlite().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
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
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if changed > 0 {
        Ok(Json(json!({"status":"success"})))
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

pub async fn delete_env(Path(id): Path<String>) -> Result<Json<serde_json::Value>, StatusCode> {
    let conn = open_sqlite().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    // 先删 sites（ON DELETE CASCADE 并非总是启用，这里显式处理）
    let _ = conn.execute(
        "DELETE FROM remote_sync_sites WHERE env_id = ?1",
        rusqlite::params![id.as_str()],
    );
    let changed = conn
        .execute(
            "DELETE FROM remote_sync_envs WHERE id = ?1",
            rusqlite::params![id],
        )
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
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

pub async fn list_sites(Path(env_id): Path<String>) -> Result<Json<serde_json::Value>, StatusCode> {
    let conn = open_sqlite().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut stmt = conn
        .prepare("SELECT id, env_id, name, location, http_host, dbnums, notes, created_at, updated_at FROM remote_sync_sites WHERE env_id = ?1 ORDER BY created_at DESC")
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
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
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut items = Vec::new();
    for r in rows {
        items.push(r.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?);
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
    let conn = open_sqlite().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
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
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(json!({"status":"success","id": id})))
}

pub async fn update_site(
    Path(site_id): Path<String>,
    Json(req): Json<SiteCreateRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let conn = open_sqlite().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
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
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if changed > 0 {
        Ok(Json(json!({"status":"success"})))
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

pub async fn delete_site(
    Path(site_id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let conn = open_sqlite().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let changed = conn
        .execute(
            "DELETE FROM remote_sync_sites WHERE id = ?1",
            rusqlite::params![site_id],
        )
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if changed > 0 {
        Ok(Json(json!({"status":"success"})))
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

// ===== 应用到运行时（写入 DbOption.toml） =====

pub async fn apply_env(Path(id): Path<String>) -> Result<Json<serde_json::Value>, StatusCode> {
    let conn = match open_sqlite() {
        Ok(conn) => conn,
        Err(e) => {
            return Ok(action_failed(
                format!("打开协同组存储失败: {}", e),
                serde_json::Map::from_iter([("env_id".to_string(), json!(id))]),
            ));
        }
    };
    let mut stmt = match conn
        .prepare("SELECT name, mqtt_host, mqtt_port, file_server_host, location, location_dbs FROM remote_sync_envs WHERE id = ?1 LIMIT 1")
    {
        Ok(stmt) => stmt,
        Err(e) => {
            return Ok(action_failed(
                format!("读取协同组失败: {}", e),
                serde_json::Map::from_iter([("env_id".to_string(), json!(id))]),
            ));
        }
    };
    let mut rows = match stmt.query(rusqlite::params![id.clone()]) {
        Ok(rows) => rows,
        Err(e) => {
            return Ok(action_failed(
                format!("查询协同组失败: {}", e),
                serde_json::Map::from_iter([("env_id".to_string(), json!(id))]),
            ));
        }
    };
    let row = match rows.next() {
        Ok(Some(r)) => r,
        Ok(None) => {
            return Ok(action_failed(
                "协同组不存在",
                serde_json::Map::from_iter([("env_id".to_string(), json!(id))]),
            ));
        }
        Err(e) => {
            return Ok(action_failed(
                format!("读取协同组失败: {}", e),
                serde_json::Map::from_iter([("env_id".to_string(), json!(id))]),
            ));
        }
    };

    let mqtt_host: Option<String> = row.get(1).ok();
    let mqtt_port_opt: Option<i64> = row.get(2).ok().flatten();
    let file_server_host: Option<String> = row.get(3).ok();
    let location: Option<String> = row.get(4).ok();
    let location_dbs: Option<String> = row.get(5).ok();

    let cfg_name =
        std::env::var("DB_OPTION_FILE").unwrap_or_else(|_| "db_options/DbOption".to_string());
    let cfg_file = format!("{}.toml", cfg_name);
    let path = std::path::Path::new(&cfg_file);
    if !path.exists() {
        return Ok(action_failed(
            format!("{} 不存在，无法写入当前配置。", cfg_file),
            serde_json::Map::from_iter([("env_id".to_string(), json!(id))]),
        ));
    }

    let mut content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(e) => {
            return Ok(action_failed(
                format!("读取当前配置失败: {}", e),
                serde_json::Map::from_iter([("env_id".to_string(), json!(id))]),
            ));
        }
    };

    // 替换/插入字符串键
    fn set_str(content: &mut String, key: &str, val: &str) {
        let line = format!("{} = \"{}\"", key, val);
        let re = regex_like_find(content, key);
        if let Some((start, end)) = re {
            content.replace_range(start..end, &format!("{}\n", line));
        } else {
            content.push_str(&format!("\n{}\n", line));
        }
    }
    // 替换/插入数值键
    fn set_num<T: std::fmt::Display>(content: &mut String, key: &str, v: T) {
        let line = format!("{} = {}", key, v);
        let re = regex_like_find(content, key);
        if let Some((start, end)) = re {
            content.replace_range(start..end, &format!("{}\n", line));
        } else {
            content.push_str(&format!("\n{}\n", line));
        }
    }
    // 替换/插入数组键（u32 列表）
    fn set_u32_array(content: &mut String, key: &str, vals: &[u32]) {
        let joined = vals
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let line = format!("{} = [{}]", key, joined);
        let re = regex_like_find(content, key);
        if let Some((start, end)) = re {
            content.replace_range(start..end, &format!("{}\n", line));
        } else {
            content.push_str(&format!("\n{}\n", line));
        }
    }
    // 在不引入正则依赖的前提下，做一个简单的 key 定位（行级）
    fn regex_like_find(s: &str, key: &str) -> Option<(usize, usize)> {
        let mut off = 0usize;
        for line in s.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with(key) && trimmed.contains('=') {
                let start = off;
                let end = off + line.len();
                return Some((start, end));
            }
            off += line.len() + 1; // +1 for newline
        }
        None
    }

    if let Some(h) = mqtt_host.as_deref() {
        set_str(&mut content, "mqtt_host", h);
    }
    if let Some(p) = mqtt_port_opt {
        set_num(&mut content, "mqtt_port", p);
    }
    if let Some(fs) = file_server_host.as_deref() {
        set_str(&mut content, "file_server_host", fs);
    }
    if let Some(loc) = location.as_deref() {
        set_str(&mut content, "location", loc);
    }

    if let Some(dbs_str) = location_dbs.as_deref() {
        let vals: Vec<u32> = dbs_str
            .split(|c| c == ',' || c == ' ')
            .filter_map(|t| t.trim().parse::<u32>().ok())
            .collect();
        if !vals.is_empty() {
            set_u32_array(&mut content, "location_dbs", &vals);
        }
    }

    if let Err(e) = std::fs::write(path, content) {
        return Ok(action_failed(
            format!("写入当前配置失败: {}", e),
            serde_json::Map::from_iter([("env_id".to_string(), json!(id))]),
        ));
    }

    Ok(action_success(
        "已写入配置文件。部分运行期组件需重启或重新加载配置后生效。",
        serde_json::Map::from_iter([("env_id".to_string(), json!(id))]),
    ))
}

/// 激活环境（即时生效）：写入 DbOption.toml 并在 WebUI 进程内重启 watcher + MQTT
pub async fn activate_env(Path(id): Path<String>) -> Result<Json<serde_json::Value>, StatusCode> {
    // 先复用 apply_env 的写入逻辑
    let _ = match apply_env(Path(id.clone())).await {
        Ok(response) => response,
        Err(status) => {
            return Ok(action_failed(
                format!("写入当前配置失败: {}", status),
                serde_json::Map::from_iter([("env_id".to_string(), json!(id))]),
            ));
        }
    };

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
        let conn = open_sqlite().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let mut stmt = conn
            .prepare("SELECT mqtt_host, mqtt_port FROM remote_sync_envs WHERE id = ?1 LIMIT 1")
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let mut rows = stmt
            .query(rusqlite::params![id])
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
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
        let conn = open_sqlite().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let mut stmt = conn
            .prepare("SELECT file_server_host FROM remote_sync_envs WHERE id = ?1 LIMIT 1")
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let mut rows = stmt
            .query(rusqlite::params![id])
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
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
                    ("url".to_string(), json!(path.to_string_lossy().to_string())),
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
                    json!(path.to_string_lossy().to_string()),
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
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
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
    let conn = open_sqlite().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let info = load_site_info(&conn, &site_id)?;
    drop(conn);

    let url = info
        .site_host
        .clone()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            info.env_file_host
                .clone()
                .filter(|value| !value.trim().is_empty())
        });
    let Some(url) = url else {
        return Ok(failed_diagnostic(
            "未配置可用的站点 HTTP 地址",
            serde_json::Map::new(),
        ));
    };

    if site_metadata::is_local_path_hint(&url) {
        let metadata_path =
            site_metadata::metadata_file_path(&site_metadata::normalize_local_base(&url));
        let start = Instant::now();
        return match fs::metadata(&metadata_path).await {
            Ok(_) => Ok(ok_diagnostic(
                "metadata.json 可达",
                serde_json::Map::from_iter([
                    (
                        "url".to_string(),
                        json!(metadata_path.to_string_lossy().to_string()),
                    ),
                    ("code".to_string(), json!(200)),
                    (
                        "latency_ms".to_string(),
                        json!(start.elapsed().as_millis() as u64),
                    ),
                ]),
            )),
            Err(err) => Ok(failed_diagnostic(
                format!("metadata.json 不可达: {}", err),
                serde_json::Map::from_iter([(
                    "url".to_string(),
                    json!(metadata_path.to_string_lossy().to_string()),
                )]),
            )),
        };
    }

    if !site_metadata::is_http_url(&url) {
        return Ok(failed_diagnostic(
            "http_host 既不是 HTTP 地址也不是本地路径",
            serde_json::Map::from_iter([("url".to_string(), json!(url))]),
        ));
    }

    let metadata_url = site_metadata::metadata_url(&url);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
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
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('#') || trimmed.starts_with('[') {
                continue;
            }
            if trimmed.starts_with(key) && trimmed.contains('=') {
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
    let conn = open_sqlite().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

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
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .query_row(
            rusqlite::params_from_iter(values.clone().into_iter()),
            |row| row.get(0),
        )
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

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
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
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
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut items = Vec::new();
    for row in rows {
        items.push(row.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?);
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
    let conn = open_sqlite().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

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
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
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
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut items = Vec::new();
    for row in rows {
        items.push(row.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?);
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
    let conn = open_sqlite().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

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
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
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
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut items = Vec::new();
    for row in rows {
        items.push(row.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?);
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
    let conn = open_sqlite().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
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
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    *req.uri_mut() = new_uri;

    let response = service
        .oneshot(req)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(response.map(Body::new))
}

async fn load_site_metadata(
    site_id: &str,
    refresh: bool,
    cache_only: bool,
) -> Result<MetadataLoadContext, StatusCode> {
    let conn = open_sqlite().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
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
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let site_row = stmt_site
        .query_row([site_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .optional()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let (env_id, site_name, site_host) = match site_row {
        Some(row) => row,
        None => return Err(StatusCode::NOT_FOUND),
    };

    let mut stmt_env = conn
        .prepare("SELECT name, file_server_host FROM remote_sync_envs WHERE id = ?1 LIMIT 1")
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let env_row = stmt_env
        .query_row([env_id.as_str()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .optional()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
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
    info.site_host
        .as_deref()
        .filter(site_metadata::is_local_path_hint_ref)
        .map(site_metadata::normalize_local_base)
        .or_else(|| {
            info.env_file_host
                .as_deref()
                .filter(site_metadata::is_local_path_hint_ref)
                .map(site_metadata::normalize_local_base)
        })
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
