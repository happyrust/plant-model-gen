//! 站点配置管理 API 处理器
//!
//! 提供读取和保存 DbOption.toml 配置的 API 接口

use axum::{extract::State, http::StatusCode, response::Json};
use rusqlite;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::net::{IpAddr, UdpSocket};
use std::path::Path;
use toml;

/// 配置名（不带 .toml 后缀，`config` crate 约定）
/// 默认 `db_options/DbOption`，可用 env `DB_OPTION_FILE` 覆盖
fn get_db_option_name() -> String {
    std::env::var("DB_OPTION_FILE").unwrap_or_else(|_| "db_options/DbOption".to_string())
}

/// 完整 TOML 文件路径（带 .toml 后缀）
fn get_db_option_path() -> String {
    format!("{}.toml", get_db_option_name())
}

/// 站点配置结构（对应 DbOption.toml 的主要配置项）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteConfig {
    // 项目设置
    pub project_path: String,
    pub included_projects: Vec<String>,
    pub project_name: String,
    pub project_code: String,
    pub module: String,

    // 位置和数据库
    pub location: String,
    pub location_dbs: Vec<u32>,

    // 数据库连接参数
    pub ip: String,
    pub user: String,
    pub password: String,
    pub port: String,

    // MQTT 配置
    pub mqtt_host: String,
    pub mqtt_port: u16,

    // 服务器配置
    pub server_release_ip: String,
    pub file_server_host: String,

    // 模型生成配置
    pub gen_model: bool,
    pub gen_mesh: bool,
    pub gen_spatial_tree: bool,
    pub apply_boolean_operation: bool,
    pub mesh_tol_ratio: f32,

    // 同步配置
    pub total_sync: bool,
    pub incr_sync: bool,
    pub sync_live: bool,

    // 允许同步推送的数据库类型列表
    pub sync_push_db_types: Vec<String>,
}

/// 获取服务器本机IP地址
#[cfg(feature = "web_server")]
pub async fn get_server_ip(
    _state: State<crate::web_server::AppState>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // 使用UdpSocket连接到外部地址，获取本地IP
    let local_ip = match get_local_ip_via_udp() {
        Ok(ip) => ip,
        Err(_) => {
            // 如果失败，返回127.0.0.1作为fallback
            "127.0.0.1".to_string()
        }
    };

    Ok(Json(json!({
        "status": "success",
        "ip": local_ip
    })))
}

/// 通过UdpSocket获取本机IP地址
fn get_local_ip_via_udp() -> Result<String, std::io::Error> {
    // 连接到一个外部地址（不需要实际连接成功）
    // 这个方法会返回用于发送数据包的网络接口的IP地址
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.connect("8.8.8.8:80")?;
    let local_addr = socket.local_addr()?;

    if let IpAddr::V4(ipv4) = local_addr.ip() {
        Ok(ipv4.to_string())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "无法获取IPv4地址",
        ))
    }
}

/// 读取当前配置
#[cfg(feature = "web_server")]
pub async fn get_site_config(
    _state: State<crate::web_server::AppState>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    match read_config() {
        Ok(config) => {
            let config_file_location = config.location.clone();
            Ok(Json(json!({
                "status": "success",
                "config": config,
                "config_file_location": config_file_location
            })))
        }
        Err(e) => Ok(Json(json!({
            "status": "error",
            "message": format!("读取配置失败: {}", e)
        }))),
    }
}

/// 获取站点信息（供其他站点查询）
/// GET /api/site/info
#[cfg(feature = "web_server")]
pub async fn get_site_info(
    _state: State<crate::web_server::AppState>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    use aios_core::get_db_option;
    
    let db_option = get_db_option();
    
    // 返回站点基本信息，格式与前端期望的一致
    Ok(Json(json!({
        "file_server_host": db_option.file_server_host,
        "mqtt_host": db_option.mqtt_host,
        "mqtt_port": db_option.mqtt_port,
        "location": db_option.location,
        "location_dbs": db_option.location_dbs,
        "project_name": db_option.project_name,
        "project_code": db_option.project_code,
    })))
}

/// 打开 SQLite 数据库并确保 site_config 表存在
fn open_site_config_sqlite() -> Result<rusqlite::Connection, Box<dyn std::error::Error>> {
    use config as cfg;

    let db_option_path = get_db_option_path();
    let db_option_name = get_db_option_name();
    let db_path = if std::path::Path::new(&db_option_path).exists() {
        let builder = cfg::Config::builder()
            .add_source(cfg::File::with_name(&db_option_name))
            .build()?;
        builder
            .get_string("deployment_sites_sqlite_path")
            .unwrap_or_else(|_| "deployment_sites.sqlite".to_string())
    } else {
        "deployment_sites.sqlite".to_string()
    };

    let mut conn = rusqlite::Connection::open(&db_path)?;

    // 创建站点配置表
    conn.execute(
        "CREATE TABLE IF NOT EXISTS site_config (
            id TEXT PRIMARY KEY DEFAULT 'default',
            project_path TEXT,
            included_projects TEXT,
            project_name TEXT,
            project_code TEXT,
            module TEXT,
            location TEXT,
            location_dbs TEXT,
            ip TEXT,
            user TEXT,
            password TEXT,
            port TEXT,
            mqtt_host TEXT,
            mqtt_port INTEGER,
            server_release_ip TEXT,
            file_server_host TEXT,
            gen_model BOOLEAN,
            gen_mesh BOOLEAN,
            gen_spatial_tree BOOLEAN,
            apply_boolean_operation BOOLEAN,
            mesh_tol_ratio REAL,
            total_sync BOOLEAN,
            incr_sync BOOLEAN,
            sync_live BOOLEAN,
            sync_push_db_types TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )",
        rusqlite::params![],
    )?;

    Ok(conn)
}

/// 保存配置到 SQLite
fn save_config_to_sqlite(config: &SiteConfig) -> Result<(), Box<dyn std::error::Error>> {
    let conn = open_site_config_sqlite()?;
    let now = chrono::Utc::now().to_rfc3339();

    let location_dbs_json = serde_json::to_string(&config.location_dbs)?;
    let included_projects_json = serde_json::to_string(&config.included_projects)?;
    let sync_push_db_types_json = serde_json::to_string(&config.sync_push_db_types)?;

    conn.execute(
        "INSERT OR REPLACE INTO site_config (
            id, project_path, included_projects, project_name, project_code, module,
            location, location_dbs, ip, user, password, port,
            mqtt_host, mqtt_port, server_release_ip, file_server_host,
            gen_model, gen_mesh, gen_spatial_tree, apply_boolean_operation, mesh_tol_ratio,
            total_sync, incr_sync, sync_live, sync_push_db_types,
            created_at, updated_at
        ) VALUES (
            'default', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
            ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24,
            COALESCE((SELECT created_at FROM site_config WHERE id = 'default'), ?25), ?25
        )",
        rusqlite::params![
            config.project_path,
            included_projects_json,
            config.project_name,
            config.project_code,
            config.module,
            config.location,
            location_dbs_json,
            config.ip,
            config.user,
            config.password,
            config.port,
            config.mqtt_host,
            config.mqtt_port as i64,
            config.server_release_ip,
            config.file_server_host,
            config.gen_model,
            config.gen_mesh,
            config.gen_spatial_tree,
            config.apply_boolean_operation,
            config.mesh_tol_ratio,
            config.total_sync,
            config.incr_sync,
            config.sync_live,
            sync_push_db_types_json,
            now,
        ],
    )?;

    Ok(())
}

/// 从 SQLite 读取配置
fn load_config_from_sqlite() -> Result<SiteConfig, Box<dyn std::error::Error>> {
    let conn = open_site_config_sqlite()?;

    let mut stmt = conn.prepare(
        "SELECT project_path, included_projects, project_name, project_code, module,
                location, location_dbs, ip, user, password, port,
                mqtt_host, mqtt_port, server_release_ip, file_server_host,
                gen_model, gen_mesh, gen_spatial_tree, apply_boolean_operation, mesh_tol_ratio,
                total_sync, incr_sync, sync_live, sync_push_db_types
         FROM site_config WHERE id = 'default'",
    )?;

    let config_result = stmt.query_row([], |row| {
        let included_projects_json: String = row.get(1)?;
        let location_dbs_json: String = row.get(6)?;
        let sync_push_db_types_json: String = row.get(23)?;

        Ok(SiteConfig {
            project_path: row.get(0)?,
            included_projects: serde_json::from_str(&included_projects_json).unwrap_or_default(),
            project_name: row.get(2)?,
            project_code: row.get(3)?,
            module: row.get(4)?,
            location: row.get(5)?,
            location_dbs: serde_json::from_str(&location_dbs_json).unwrap_or_default(),
            ip: row.get(7)?,
            user: row.get(8)?,
            password: row.get(9)?,
            port: row.get(10)?,
            mqtt_host: row.get(11)?,
            mqtt_port: row.get::<_, i64>(12)? as u16,
            server_release_ip: row.get(13)?,
            file_server_host: row.get(14)?,
            gen_model: row.get(15)?,
            gen_mesh: row.get(16)?,
            gen_spatial_tree: row.get(17)?,
            apply_boolean_operation: row.get(18)?,
            mesh_tol_ratio: row.get(19)?,
            total_sync: row.get(20)?,
            incr_sync: row.get(21)?,
            sync_live: row.get(22)?,
            sync_push_db_types: serde_json::from_str(&sync_push_db_types_json).unwrap_or_default(),
        })
    });

    match config_result {
        Ok(config) => Ok(config),
        Err(rusqlite::Error::QueryReturnedNoRows) => Err("配置不存在于 SQLite 数据库中".into()),
        Err(e) => Err(e.into()),
    }
}

/// 保存配置
#[cfg(feature = "web_server")]
pub async fn save_site_config(
    state: State<crate::web_server::AppState>,
    Json(config): Json<SiteConfig>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    log::info!("📝 [站点配置] 开始保存配置...");
    log::info!("📝 [站点配置] 收到的配置: location={}, mqtt_host={}, location_dbs={:?}", 
        config.location, config.mqtt_host, config.location_dbs);
    
    // 1. 先保存到 SQLite
    if let Err(e) = save_config_to_sqlite(&config) {
        log::error!("❌ [站点配置] 保存到 SQLite 失败: {}", e);
        return Ok(Json(json!({
            "status": "error",
            "message": format!("保存配置到 SQLite 失败: {}", e)
        })));
    }
    log::info!("✅ [站点配置] 已保存到 SQLite");

    // 2. 再保存到 DbOption.toml
    match write_config(&config) {
        Ok(_) => {
            log::info!("✅ [站点配置] 已保存到 DbOption.toml");
            
            // 验证保存结果：重新读取 TOML 文件
            if let Ok(content) = std::fs::read_to_string(&get_db_option_path()) {
                if let Ok(toml_value) = toml::from_str::<toml::Value>(&content) {
                    let saved_location = toml_value.get("location")
                        .and_then(|v| v.as_str())
                        .unwrap_or("(未找到)");
                    let saved_mqtt_host = toml_value.get("mqtt_host")
                        .and_then(|v| v.as_str())
                        .unwrap_or("(未找到)");
                    log::info!("🔍 [站点配置] 验证保存结果: location={}, mqtt_host={}", 
                        saved_location, saved_mqtt_host);
                }
            }
            
            // B5 / Phase 10 · 触发 graceful shutdown，由进程级 supervisor 拉起新进程。
            //
            // sender 由 `mod.rs::start_web_server_with_config` 在启动 axum 前
            // 写入；这里 take 一次就消耗掉，多次 save 期间只有第一次生效，避免
            // 重复 send 导致 panic（oneshot::Sender::send 是 self）。
            //
            // shutdown 触发后 axum 不再接受新连接，本响应仍能正常返回（同一连接
            // 已建立），随后 in-flight 请求处理完毕，进程退出，supervisor 拉起。
            let triggered = trigger_graceful_shutdown(&state).await;
            let message = if triggered {
                "配置已保存到 SQLite 和 DbOption.toml；已触发 graceful shutdown，supervisor 将自动重启 web_server 以使新配置生效"
            } else {
                "配置已保存到 SQLite 和 DbOption.toml，但 graceful shutdown 已被触发过或未启用，请手动重启服务器"
            };

            Ok(Json(json!({
                "status": "success",
                "graceful_shutdown_triggered": triggered,
                "message": message,
            })))
        }
        Err(e) => {
            log::error!("❌ [站点配置] 保存到 DbOption.toml 失败: {}", e);
            Ok(Json(json!({
                "status": "error",
                "message": format!("保存配置到 DbOption.toml 失败: {}", e)
            })))
        }
    }
}

/// 重启服务器（B5 / Phase 10 · 真实 graceful shutdown 触发）
///
/// 调用流程：
/// 1. take 走 `AppState::shutdown_tx` 中的 oneshot sender
/// 2. `send(())` 通知 `axum::serve(...).with_graceful_shutdown(...)`
/// 3. axum 停接新连接，in-flight 请求（包含本次响应）走完后进程退出
/// 4. 进程级 supervisor（systemd / nssm / pm2）自动拉起新进程
///
/// 与 `save_site_config` 内置触发的区别：本端点是"无变更前提下手动 restart"，
/// 用于热更新代码 / 升级配置之外的运维动作（如清缓存、重连后端服务）。
#[cfg(feature = "web_server")]
pub async fn restart_server(
    state: State<crate::web_server::AppState>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let triggered = trigger_graceful_shutdown(&state).await;
    let message = if triggered {
        "已触发 graceful shutdown，supervisor 将拉起新进程；本响应返回后立即进入退出流程"
    } else {
        "graceful shutdown sender 不存在或已被触发，请手动重启服务器"
    };
    Ok(Json(json!({
        "status": "success",
        "graceful_shutdown_triggered": triggered,
        "message": message,
    })))
}

/// 触发 graceful shutdown，返回是否真正触发（false = 已被触发过或未启用）。
///
/// 仅在写盘等"改变运行时事实"的成功路径上调用。take 走 sender 后立即 send，
/// 失败（receiver 已关闭）只记日志、不报错，让上层成功路径不被基础设施异常拖死。
async fn trigger_graceful_shutdown(state: &State<crate::web_server::AppState>) -> bool {
    let mut slot = state.shutdown_tx.lock().await;
    if let Some(tx) = slot.take() {
        match tx.send(()) {
            Ok(()) => {
                log::warn!("📴 [站点配置] 已触发 graceful shutdown，supervisor 将拉起新进程");
                true
            }
            Err(()) => {
                log::warn!("📴 [站点配置] graceful shutdown receiver 已关闭，无法触发");
                false
            }
        }
    } else {
        log::warn!("📴 [站点配置] graceful shutdown sender 不存在（可能已被触发或未启用）");
        false
    }
}

/// 「理论上可热改」的 DbOption 字段白名单（B6 · Phase 11）
///
/// 标记规则：不影响已建立的连接 / 已加载的索引 / 已订阅的 MQTT 的字段。
///
/// ⚠️ 本 Phase 11 (最小可用版) 不真正应用这些字段——`aios_core::get_db_option`
/// 内部为 `OnceCell::get_or_init`，不可变。白名单仅用于响应分类提示
/// 「这些不一定要重启」vs「这些一定要重启」。
///
/// 后续 rs-core 改动将 OnceCell → RwLock<Arc<DbOption>> 后，
/// 即可一键升级为真正的热加载（详见 docs/plans/2026-04-26-sprint-b-phase11-b6-reload.md §6）。
///
/// ⚠️ 注：此白名单需与 `aios_core::options::DbOption` 字段定义保持同步，
/// 新增字段时按是否影响运行态决定是否纳入。
const HOT_RELOADABLE_KEYS: &[&str] = &[
    "enable_log",
    "mesh_tol_ratio",
    "gen_model",
    "gen_spatial_tree",
    "load_spatial_tree",
    "apply_boolean_operation",
    "build_cate_relate",
    "sync_chunk_size",
    "parse_channel_capacity",
    "parse_mode",
    "incr_sync",
    "total_sync",
];

/// 「文件值与运行时永远不一致」的字段白名单（C3 · 修 G3 baseline 误报）
///
/// 这些字段在 `aios_core::get_db_option()` 内部会被环境变量覆盖（如
/// `SURREAL_CONN_MODE` / `SURREAL_CONN_IP` / `SURREAL_CONN_PORT` 影响 `surrealdb`），
/// 因此运行时副本与 TOML 文件原值天然不一致。
///
/// 在 reload diff 时把这类字段单独归到 `static_changed_keys_env`，避免它们
/// 把 baseline（用户没有人为改动）也错报成「需要重启」。
///
/// 触发 `requires_restart` 仅看 `static_changed_keys_user` 是否非空。
///
/// ⚠️ 注：扩展时同步 `rs-core/src/lib.rs` 中 env override 的字段列表。
const ENV_OVERRIDABLE_KEYS: &[&str] = &["surrealdb"];

/// 字段级 diff（hot vs static）
///
/// 通过 serde_json 做 key-by-key 比较；任一字段值变化按 HOT_RELOADABLE_KEYS
/// 白名单分流到 hot / static 两组。
fn diff_db_option(
    current: &aios_core::options::DbOption,
    new: &aios_core::options::DbOption,
) -> (Vec<String>, Vec<String>) {
    let cur_v = serde_json::to_value(current).unwrap_or(json!({}));
    let new_v = serde_json::to_value(new).unwrap_or(json!({}));
    let cur_obj = cur_v.as_object().cloned().unwrap_or_default();
    let new_obj = new_v.as_object().cloned().unwrap_or_default();

    let mut hot = Vec::new();
    let mut stat = Vec::new();
    let all_keys: std::collections::BTreeSet<_> =
        cur_obj.keys().chain(new_obj.keys()).cloned().collect();

    for k in all_keys {
        if cur_obj.get(&k) != new_obj.get(&k) {
            if HOT_RELOADABLE_KEYS.contains(&k.as_str()) {
                hot.push(k);
            } else {
                stat.push(k);
            }
        }
    }
    (hot, stat)
}

/// 重载站点配置（B6 · Phase 11 最小可用版）
///
/// 行为：重新读 `${DB_OPTION_FILE}.toml` → 解析 → 与当前内存 `DbOption` 做字段 diff
/// → 返回 hot / static 分类清单 + `requires_restart` 标志。
///
/// **本版本不真正应用配置变更**——`aios_core::get_db_option` 为 OnceCell，
/// 全局静态不可变。本端点的语义是「字段变更检测 + 用户告知」，让前端
/// SettingsView 能清楚展示「我改了哪些字段、哪些必须重启」。
///
/// 升级路径见 `docs/plans/2026-04-26-sprint-b-phase11-b6-reload.md §6`。
#[cfg(feature = "web_server")]
pub async fn reload_site_config(
    _state: State<crate::web_server::AppState>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let toml_path = get_db_option_path();

    // 1. 读文件
    let content = match fs::read_to_string(&toml_path) {
        Ok(c) => c,
        Err(e) => {
            log::error!("❌ [站点配置] reload 读取 {} 失败: {}", toml_path, e);
            return Ok(Json(json!({
                "status": "error",
                "message": format!("读取 {} 失败: {}", toml_path, e),
                "hot_changed_keys": [],
                "static_changed_keys": [],
                "requires_restart": false,
                "actions": ["read_failed"],
            })));
        }
    };

    // 2. 解析 toml → DbOption
    let new_option: aios_core::options::DbOption = match toml::from_str(&content) {
        Ok(o) => o,
        Err(e) => {
            log::error!("❌ [站点配置] reload 解析 {} 失败: {}", toml_path, e);
            return Ok(Json(json!({
                "status": "error",
                "message": format!("解析 {} 失败: {}", toml_path, e),
                "hot_changed_keys": [],
                "static_changed_keys": [],
                "requires_restart": false,
                "actions": ["parse_failed"],
            })));
        }
    };

    // 3. 字段 diff
    let current = aios_core::get_db_option();
    let (hot_changed, static_changed) = diff_db_option(current, &new_option);

    // C3 · 修 G3 baseline 误报：把 static_changed 拆成 user / env 两组。
    //
    // env 覆盖字段（如 `surrealdb`）会因 `aios_core::get_db_option()` 内部 env override
    // 让运行时副本永远 ≠ 文件副本，**与用户是否改了 TOML 无关**。这部分不应触发
    // requires_restart，否则用户每次 reload 都会看到误报「需要重启」。
    let (static_changed_env, static_changed_user): (Vec<String>, Vec<String>) = static_changed
        .iter()
        .cloned()
        .partition(|k| ENV_OVERRIDABLE_KEYS.contains(&k.as_str()));

    let requires_restart = !static_changed_user.is_empty();
    let no_user_change = hot_changed.is_empty() && static_changed_user.is_empty();

    let actions: Vec<&str> = if no_user_change {
        vec!["no_change"]
    } else if requires_restart {
        vec!["manual_restart_required"]
    } else {
        vec!["log_only"]
    };

    let message = if no_user_change {
        if static_changed_env.is_empty() {
            "配置文件与当前运行时完全一致，无需操作".to_string()
        } else {
            format!(
                "配置文件与当前运行时一致（{} 项 env 覆盖字段除外，属预期差异）",
                static_changed_env.len()
            )
        }
    } else if requires_restart {
        format!(
            "检测到 {} 项用户改动的静态字段（需重启）+ {} 项可热改字段{}；当前版本统一以重启生效",
            static_changed_user.len(),
            hot_changed.len(),
            if static_changed_env.is_empty() {
                String::new()
            } else {
                format!("（另 {} 项 env 覆盖字段已忽略）", static_changed_env.len())
            }
        )
    } else {
        format!(
            "检测到 {} 项可热改字段；当前版本仍需手动重启才能真正生效（OnceCell 限制）",
            hot_changed.len()
        )
    };

    log::info!(
        "ℹ️  [站点配置] reload diff: hot={:?} static_user={:?} static_env={:?} requires_restart={}",
        hot_changed,
        static_changed_user,
        static_changed_env,
        requires_restart
    );

    Ok(Json(json!({
        "status": "success",
        "message": message,
        "hot_changed_keys": hot_changed,
        // 向后兼容：保留 user + env 合并后的 static_changed_keys
        "static_changed_keys": static_changed,
        // C3 · 修 G3 baseline 误报：新增分类字段
        "static_changed_keys_user": static_changed_user,
        "static_changed_keys_env": static_changed_env,
        "requires_restart": requires_restart,
        "actions": actions,
    })))
}

/// 验证配置（检查路径是否存在、数据库连接是否有效等）
#[cfg(feature = "web_server")]
pub async fn validate_site_config(
    _state: State<crate::web_server::AppState>,
    Json(config): Json<SiteConfig>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let mut errors = Vec::new();

    // 验证项目路径
    if !Path::new(&config.project_path).exists() {
        errors.push(format!("项目路径不存在: {}", config.project_path));
    }

    // 验证 IP 格式
    if config.ip.parse::<std::net::IpAddr>().is_err() && config.ip != "localhost" {
        errors.push(format!("无效的 IP 地址: {}", config.ip));
    }

    // 验证端口号
    if config.port.parse::<u16>().is_err() {
        errors.push(format!("无效的端口号: {}", config.port));
    }

    if config.mqtt_port == 0 {
        errors.push("MQTT 端口不能为 0".to_string());
    }

    // 验证 location_dbs 不为空
    if config.location_dbs.is_empty() {
        errors.push("location_dbs 不能为空".to_string());
    }

    if errors.is_empty() {
        Ok(Json(json!({
            "status": "success",
            "message": "配置验证通过"
        })))
    } else {
        Ok(Json(json!({
            "status": "error",
            "message": "配置验证失败",
            "errors": errors
        })))
    }
}

/// 从 DbOption.toml 读取配置（优先），如果为空或不存在，则从 SQLite 读取
/// 注意：直接读取 TOML 文件，避免使用 get_db_option() 缓存，确保获取最新值
fn read_config() -> anyhow::Result<SiteConfig> {
    log::debug!("📖 [站点配置] 开始读取配置...");
    let config_file = get_db_option_path();
    log::debug!("📖 [站点配置] 配置文件路径: {}", config_file);

    // 优先从 DbOption.toml 读取配置
    let mut config_from_toml: Option<SiteConfig> = None;

    // 直接从 TOML 文件读取所有字段（避免缓存问题）
    if let Ok(content) = std::fs::read_to_string(&config_file) {
        if let Ok(toml_value) = toml::from_str::<toml::Value>(&content) {
            // 辅助函数：从 TOML 值中获取字符串
            let get_string = |key: &str| -> String {
                toml_value.get(key)
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default()
            };
            
            // 辅助函数：从 TOML 值中获取整数
            let get_u16 = |key: &str, default: u16| -> u16 {
                toml_value.get(key)
                    .and_then(|v| v.as_integer())
                    .map(|i| i as u16)
                    .unwrap_or(default)
            };
            
            // 辅助函数：从 TOML 值中获取布尔值
            let get_bool = |key: &str| -> bool {
                toml_value.get(key)
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
            };
            
            // 辅助函数：从 TOML 值中获取浮点数 (f32)
            let get_f32 = |key: &str, default: f32| -> f32 {
                toml_value.get(key)
                    .and_then(|v| v.as_float())
                    .map(|f| f as f32)
                    .unwrap_or(default)
            };
            
            // 读取 included_projects 数组
            let included_projects = if let Some(array) = toml_value
                .get("included_projects")
                .and_then(|v| v.as_array())
            {
                array
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            } else {
                Vec::new()
            };
            
            // 读取 location_dbs 数组
            let location_dbs = if let Some(array) = toml_value
                .get("location_dbs")
                .and_then(|v| v.as_array())
            {
                array
                    .iter()
                    .filter_map(|v| {
                        v.as_integer()
                            .map(|i| i as u32)
                            .or_else(|| v.as_str().and_then(|s| s.parse::<u32>().ok()))
                    })
                    .collect()
            } else {
                Vec::new()
            };
            
            // 读取 sync_push_db_types 数组
            let sync_push_db_types = if let Some(array) = toml_value
                .get("sync_push_db_types")
                .and_then(|v| v.as_array())
            {
                array
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            } else {
                vec!["DESI".to_string()]
            };
            
            // 构建配置对象（直接从 TOML 文件读取，不使用缓存）
            let location = get_string("location");
            let mqtt_host = get_string("mqtt_host");
            log::debug!("📖 [站点配置] 从 TOML 读取: location={}, mqtt_host={}, location_dbs={:?}", 
                location, mqtt_host, location_dbs);
            
            config_from_toml = Some(SiteConfig {
                project_path: get_string("project_path"),
                included_projects,
                project_name: get_string("project_name"),
                project_code: get_string("project_code"),
                module: get_string("module"),
                location,
                location_dbs,
                ip: get_string("ip"),
                user: get_string("user"),
                password: get_string("password"),
                port: get_string("port"),
                mqtt_host,
                mqtt_port: get_u16("mqtt_port", 1883),
                server_release_ip: get_string("server_release_ip"),
                file_server_host: get_string("file_server_host"),
                gen_model: get_bool("gen_model"),
                gen_mesh: get_bool("gen_mesh"),
                gen_spatial_tree: get_bool("gen_spatial_tree"),
                apply_boolean_operation: get_bool("apply_boolean_operation"),
                mesh_tol_ratio: get_f32("mesh_tol_ratio", 3.0),
                total_sync: get_bool("total_sync"),
                incr_sync: get_bool("incr_sync"),
                sync_live: get_bool("sync_live"),
                sync_push_db_types,
            });
        }
    }
    
    // 用于后续逻辑的变量
    let location_dbs = config_from_toml.as_ref().map(|c| c.location_dbs.clone()).unwrap_or_default();

    // 如果 DbOption.toml 中有配置且 location_dbs 不为空，优先使用
    if let Some(ref config) = config_from_toml {
        if !config.location_dbs.is_empty() {
            return Ok(config.clone());
        }
    }

    // 如果 DbOption.toml 中没有配置或 location_dbs 为空，尝试从 SQLite 读取
    match load_config_from_sqlite() {
        Ok(sqlite_config) => {
            // 合并配置：DbOption.toml 的字段优先，SQLite 作为后备
            Ok(SiteConfig {
                project_path: config_from_toml
                    .as_ref()
                    .map(|c| c.project_path.clone())
                    .unwrap_or_else(|| sqlite_config.project_path),
                included_projects: config_from_toml
                    .as_ref()
                    .map(|c| c.included_projects.clone())
                    .unwrap_or_else(|| sqlite_config.included_projects),
                project_name: config_from_toml
                    .as_ref()
                    .map(|c| c.project_name.clone())
                    .unwrap_or_else(|| sqlite_config.project_name),
                project_code: config_from_toml
                    .as_ref()
                    .map(|c| c.project_code.clone())
                    .unwrap_or_else(|| sqlite_config.project_code),
                module: config_from_toml
                    .as_ref()
                    .map(|c| c.module.clone())
                    .unwrap_or_else(|| sqlite_config.module),
                location: config_from_toml
                    .as_ref()
                    .map(|c| c.location.clone())
                    .unwrap_or_else(|| sqlite_config.location),
                location_dbs: if !location_dbs.is_empty() {
                    location_dbs
                } else {
                    sqlite_config.location_dbs
                },
                ip: config_from_toml
                    .as_ref()
                    .map(|c| c.ip.clone())
                    .unwrap_or_else(|| sqlite_config.ip),
                user: config_from_toml
                    .as_ref()
                    .map(|c| c.user.clone())
                    .unwrap_or_else(|| sqlite_config.user),
                password: config_from_toml
                    .as_ref()
                    .map(|c| c.password.clone())
                    .unwrap_or_else(|| sqlite_config.password),
                port: config_from_toml
                    .as_ref()
                    .map(|c| c.port.clone())
                    .unwrap_or_else(|| sqlite_config.port),
                mqtt_host: config_from_toml
                    .as_ref()
                    .map(|c| c.mqtt_host.clone())
                    .unwrap_or_else(|| sqlite_config.mqtt_host),
                mqtt_port: config_from_toml
                    .as_ref()
                    .map(|c| c.mqtt_port)
                    .unwrap_or(sqlite_config.mqtt_port),
                server_release_ip: config_from_toml
                    .as_ref()
                    .map(|c| c.server_release_ip.clone())
                    .unwrap_or_else(|| sqlite_config.server_release_ip),
                file_server_host: config_from_toml
                    .as_ref()
                    .map(|c| c.file_server_host.clone())
                    .unwrap_or_else(|| sqlite_config.file_server_host),
                gen_model: config_from_toml
                    .as_ref()
                    .map(|c| c.gen_model)
                    .unwrap_or(sqlite_config.gen_model),
                gen_mesh: config_from_toml
                    .as_ref()
                    .map(|c| c.gen_mesh)
                    .unwrap_or(sqlite_config.gen_mesh),
                gen_spatial_tree: config_from_toml
                    .as_ref()
                    .map(|c| c.gen_spatial_tree)
                    .unwrap_or(sqlite_config.gen_spatial_tree),
                apply_boolean_operation: config_from_toml
                    .as_ref()
                    .map(|c| c.apply_boolean_operation)
                    .unwrap_or(sqlite_config.apply_boolean_operation),
                mesh_tol_ratio: config_from_toml
                    .as_ref()
                    .map(|c| c.mesh_tol_ratio)
                    .unwrap_or(sqlite_config.mesh_tol_ratio),
                total_sync: config_from_toml
                    .as_ref()
                    .map(|c| c.total_sync)
                    .unwrap_or(sqlite_config.total_sync),
                incr_sync: config_from_toml
                    .as_ref()
                    .map(|c| c.incr_sync)
                    .unwrap_or(sqlite_config.incr_sync),
                sync_live: config_from_toml
                    .as_ref()
                    .map(|c| c.sync_live)
                    .unwrap_or(sqlite_config.sync_live),
                sync_push_db_types: config_from_toml
                    .as_ref()
                    .map(|c| c.sync_push_db_types.clone())
                    .filter(|v| !v.is_empty())
                    .unwrap_or(sqlite_config.sync_push_db_types),
            })
        }
        Err(_) => {
            // SQLite 中也没有，使用从 TOML 文件读取的配置（即使 location_dbs 为空）
            // 如果 TOML 也没有配置，返回默认空配置
            Ok(config_from_toml.unwrap_or_else(|| SiteConfig {
                project_path: String::new(),
                included_projects: Vec::new(),
                project_name: String::new(),
                project_code: String::new(),
                module: String::new(),
                location: String::new(),
                location_dbs: Vec::new(),
                ip: String::new(),
                user: String::new(),
                password: String::new(),
                port: String::new(),
                mqtt_host: String::new(),
                mqtt_port: 1883,
                server_release_ip: String::new(),
                file_server_host: String::new(),
                gen_model: false,
                gen_mesh: false,
                gen_spatial_tree: false,
                apply_boolean_operation: false,
                mesh_tol_ratio: 3.0,
                total_sync: false,
                incr_sync: false,
                sync_live: false,
                sync_push_db_types: vec!["DESI".to_string()],
            }))
        }
    }
}

/// 将配置写入 DbOption.toml（保留原有格式、注释、缩进和空行）
fn write_config(config: &SiteConfig) -> anyhow::Result<()> {
    let config_path_owned = get_db_option_path();
    let config_path = config_path_owned.as_str();

    // 读取现有配置文件
    let existing_content = fs::read_to_string(config_path)?;

    // 创建新的配置内容（保留注释、缩进和空行）
    let mut new_content = String::new();

    for line in existing_content.lines() {
        let trimmed = line.trim();

        // 跳过空行和注释行，直接保留
        if trimmed.is_empty() || trimmed.starts_with('#') {
            new_content.push_str(line);
            new_content.push('\n');
            continue;
        }

        // 提取原始缩进
        let indent = &line[..line.len() - line.trim_start().len()];

        // 检测配置项并替换（保留原始缩进和行内注释）
        let (key, rest) = if let Some(pos) = trimmed.find('=') {
            let key = trimmed[..pos].trim();
            let rest = &trimmed[pos..];
            (key, rest)
        } else {
            // 不是键值对，直接保留
            new_content.push_str(line);
            new_content.push('\n');
            continue;
        };

        // 提取行内注释
        let inline_comment = if let Some(comment_pos) = rest.find('#') {
            // 确保 # 不在字符串内
            let before_comment = &rest[..comment_pos];
            let quote_count = before_comment.matches('"').count();
            if quote_count % 2 == 0 {
                Some(&rest[comment_pos..])
            } else {
                None
            }
        } else {
            None
        };

        // 根据键名替换值
        let new_line = match key {
            "project_path" => {
                // 转义 Windows 路径中的反斜杠，以便在 TOML 中正确保存
                // 兼容配置文件中的各种格式：D:/path, D:\path, D:\\path
                // 无论输入格式如何，统一转换为 Windows 格式并转义
                let escaped_path = escape_toml_string(&config.project_path);
                format_config_line(
                    indent,
                    key,
                    &format!("\"{}\"", escaped_path),
                    inline_comment,
                )
            }
            "included_projects" => format_config_line(
                indent,
                key,
                &format_toml_array(&config.included_projects),
                inline_comment,
            ),
            "project_name" => format_config_line(
                indent,
                key,
                &format!("\"{}\"", config.project_name),
                inline_comment,
            ),
            "project_code" => format_config_line(
                indent,
                key,
                &format!("'{}'", config.project_code),
                inline_comment,
            ),
            "module" if !trimmed.contains("gen_model") => format_config_line(
                indent,
                key,
                &format!("\"{}\"", config.module),
                inline_comment,
            ),
            "location" => format_config_line(
                indent,
                key,
                &format!("\"{}\"", config.location),
                inline_comment,
            ),
            "location_dbs" => format_config_line(
                indent,
                key,
                &format_toml_array_u32(&config.location_dbs),
                inline_comment,
            ),
            "ip" => format_config_line(indent, key, &format!("\"{}\"", config.ip), inline_comment),
            "user" => {
                format_config_line(indent, key, &format!("\"{}\"", config.user), inline_comment)
            }
            "password" => format_config_line(
                indent,
                key,
                &format!("\"{}\"", config.password),
                inline_comment,
            ),
            "port"
                if !trimmed.contains("mqtt_port")
                    && !trimmed.contains("v_port")
                    && !trimmed.contains("kv_port") =>
            {
                format_config_line(indent, key, &format!("\"{}\"", config.port), inline_comment)
            }
            "mqtt_host" => format_config_line(
                indent,
                key,
                &format!("\"{}\"", config.mqtt_host),
                inline_comment,
            ),
            "mqtt_port" => {
                format_config_line(indent, key, &config.mqtt_port.to_string(), inline_comment)
            }
            "server_release_ip" => format_config_line(
                indent,
                key,
                &format!("\"{}\"", config.server_release_ip),
                inline_comment,
            ),
            "file_server_host" => format_config_line(
                indent,
                key,
                &format!("\"{}\"", config.file_server_host),
                inline_comment,
            ),
            "gen_model" => {
                format_config_line(indent, key, &config.gen_model.to_string(), inline_comment)
            }
            "gen_mesh" => {
                format_config_line(indent, key, &config.gen_mesh.to_string(), inline_comment)
            }
            "gen_spatial_tree" => format_config_line(
                indent,
                key,
                &config.gen_spatial_tree.to_string(),
                inline_comment,
            ),
            "apply_boolean_operation" => format_config_line(
                indent,
                key,
                &config.apply_boolean_operation.to_string(),
                inline_comment,
            ),
            "mesh_tol_ratio" => format_config_line(
                indent,
                key,
                &config.mesh_tol_ratio.to_string(),
                inline_comment,
            ),
            "total_sync" => {
                format_config_line(indent, key, &config.total_sync.to_string(), inline_comment)
            }
            "incr_sync" => {
                format_config_line(indent, key, &config.incr_sync.to_string(), inline_comment)
            }
            "sync_live" => {
                format_config_line(indent, key, &config.sync_live.to_string(), inline_comment)
            }
            "sync_push_db_types" => format_config_line(
                indent,
                key,
                &format_toml_array(&config.sync_push_db_types),
                inline_comment,
            ),
            _ => {
                // 不在更新列表中的配置项，保持原样
                line.to_string()
            }
        };

        new_content.push_str(&new_line);
        new_content.push('\n');
    }

    // 写回文件
    fs::write(config_path, new_content)?;

    Ok(())
}

/// 转义 TOML 字符串中的反斜杠（用于 Windows 路径）
/// 在 TOML 字符串中，反斜杠需要转义为双反斜杠
/// 只在 Windows 平台上进行转义
fn escape_toml_string(s: &str) -> String {
    #[cfg(target_os = "windows")]
    {
        s.replace('\\', "\\\\")
    }
    #[cfg(not(target_os = "windows"))]
    {
        s.to_string()
    }
}

/// 格式化配置行（保留缩进和行内注释）
fn format_config_line(
    indent: &str,
    key: &str,
    value: &str,
    inline_comment: Option<&str>,
) -> String {
    if let Some(comment) = inline_comment {
        format!("{}{} = {} {}", indent, key, value, comment)
    } else {
        format!("{}{} = {}", indent, key, value)
    }
}

/// 格式化字符串数组为 TOML 格式
fn format_toml_array(arr: &[String]) -> String {
    if arr.is_empty() {
        return "[]".to_string();
    }
    let items: Vec<String> = arr.iter().map(|s| format!("\"{}\"", s)).collect();
    format!("[{}]", items.join(", "))
}

/// 格式化 u32 数组为 TOML 格式
fn format_toml_array_u32(arr: &[u32]) -> String {
    if arr.is_empty() {
        return "[]".to_string();
    }
    let items: Vec<String> = arr.iter().map(|n| n.to_string()).collect();
    format!("[{}]", items.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_toml_array() {
        assert_eq!(format_toml_array(&[]), "[]");
        assert_eq!(format_toml_array(&[String::from("test")]), "[\"test\"]");
        assert_eq!(
            format_toml_array(&[String::from("a"), String::from("b"), String::from("c")]),
            "[\"a\", \"b\", \"c\"]"
        );
    }

    #[test]
    fn test_format_toml_array_u32() {
        assert_eq!(format_toml_array_u32(&[]), "[]");
        assert_eq!(format_toml_array_u32(&[123]), "[123]");
        assert_eq!(format_toml_array_u32(&[1, 2, 3]), "[1, 2, 3]");
    }

    #[test]
    fn test_format_config_line() {
        // 无注释
        assert_eq!(
            format_config_line("", "key", "\"value\"", None),
            "key = \"value\""
        );

        // 有缩进
        assert_eq!(
            format_config_line("    ", "key", "\"value\"", None),
            "    key = \"value\""
        );

        // 有行内注释
        assert_eq!(
            format_config_line("", "key", "\"value\"", Some("# comment")),
            "key = \"value\" # comment"
        );

        // 有缩进和注释
        assert_eq!(
            format_config_line("  ", "key", "123", Some("# number")),
            "  key = 123 # number"
        );
    }
}
