use once_cell::sync::Lazy;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::data_interface::tidb_manager::AiosDBManager;

pub struct RuntimeState {
    pub env_id: String,
    pub mgr: Arc<AiosDBManager>,
    /// 中继模式（`sync_relay_mode = true`）：本运行态没有 SurrealDB，也不跑 watch-incremental；
    /// 只做源 db 文件的接收 / 校验与中继广播（`relay_sync`）。`runtime/status` 原样透出。
    pub relay: bool,
    pub watcher_handle: Option<tokio::task::JoinHandle<()>>,
    pub mqtt_handle: Option<tokio::task::JoinHandle<()>>,
}

pub static REMOTE_RUNTIME: Lazy<RwLock<Option<RuntimeState>>> = Lazy::new(|| RwLock::new(None));

/// 停止当前运行态（如存在）
pub async fn stop_runtime() {
    let mut guard = REMOTE_RUNTIME.write().await;
    if let Some(state) = guard.as_mut() {
        if let Some(h) = state.watcher_handle.take() {
            h.abort();
        }
        if let Some(h) = state.mqtt_handle.take() {
            h.abort();
        }
    }
    *guard = None;
}

/// 使用当前 DbOption 配置启动 watcher + mqtt
///
/// `sync_relay_mode = true`（中继模式）时不过 SurrealDB 硬闸、不跑 watch-incremental：
/// `AiosDBManager::init_form_config` 只收集目录并建 `PdmsWatcher`，不需要数据库；MQTT 订阅照常起，
/// 收到的文件在 `mqtt_file_sync` 里 clone + 校验 + 记 SQLite 台账；广播由 `relay_sync` 轮询负责
/// （SQLite 水位 + e3d-io 判变更），它在这里顶替 `run_watch_incremental` 的位置。
pub async fn start_runtime(env_id: String) -> anyhow::Result<()> {
    let (init_ms, max_ms) = query_backoff_ms(&env_id).unwrap_or((1000, 30_000));
    // 开关从 DB_OPTION_FILE 指向的 toml 读：activate 刚写过该文件，而 get_db_option_ext() 拿不到扩展键。
    let relay = crate::options::current_sync_relay_mode()?;
    let mgr = Arc::new(AiosDBManager::init_form_config().await?);
    if relay {
        log::info!(
            "remote runtime: sync_relay_mode = true，中继模式——跳过 SurrealDB 初始化与 watch-incremental"
        );
    } else {
        // 非中继：原样硬闸，连不上 SurrealDB 整个 activate 失败。
        crate::fast_model::utils::ensure_surreal_init().await?;
    }
    let db_option_ext = crate::options::get_db_option_ext();
    let requested_dbnums = db_option_ext
        .inner
        .manual_db_nums
        .clone()
        .unwrap_or_default();

    let watcher_handle = if relay {
        #[cfg(feature = "relay-sync")]
        {
            // 中继广播轮询：SQLite 水位 + e3d-io 判变更，周期取 remote_sync_env_config.detect_interval。
            let options = crate::version_management::relay_sync::RelaySyncOptions {
                env_id: env_id.clone(),
                requested_dbnums,
                interval_secs: None,
                once: false,
            };
            Some(tokio::spawn(async move {
                if let Err(error) =
                    crate::version_management::relay_sync::run_relay_watch(options).await
                {
                    log::error!("remote runtime relay-sync 退出: {error:#}");
                }
            }))
        }
        #[cfg(not(feature = "relay-sync"))]
        {
            let _ = &requested_dbnums;
            anyhow::bail!(
                "sync_relay_mode = true 需要以 --features web_server,relay-sync 构建的 web_server（当前二进制没有 e3d-io，做不了收包校验与变更判定）"
            );
        }
    } else {
        // 与 CLI/sync_live 共用唯一轮询实现。
        Some(tokio::spawn(async move {
            if let Err(error) =
                crate::version_management::watch_incremental::run_watch_incremental(
                    &db_option_ext,
                    crate::version_management::watch_incremental::WatchIncrementalOptions {
                        requested_dbnums,
                        ..Default::default()
                    },
                )
                .await
            {
                log::error!("remote runtime watch-incremental 退出: {error:#}");
            }
        }))
    };

    // 启动 MQTT 订阅（仅在 mqtt feature 启用时）
    let mqtt_handle = {
        #[cfg(feature = "mqtt")]
        {
            crate::data_interface::mqtt_file_sync::initialize_file_index(&mgr.watcher).await?;
            let watcher_arc = mgr.watcher.clone();
            Some(tokio::spawn(async move {
                crate::data_interface::mqtt_file_sync::poll_sync_e3d_mqtt_events_with_backoff(
                    watcher_arc,
                    init_ms,
                    max_ms,
                )
                .await;
            }))
        }
        #[cfg(not(feature = "mqtt"))]
        {
            let _ = (init_ms, max_ms);
            None
        }
    };

    let mut guard = REMOTE_RUNTIME.write().await;
    *guard = Some(RuntimeState {
        env_id,
        mgr,
        relay,
        watcher_handle,
        mqtt_handle,
    });
    Ok(())
}

fn query_backoff_ms(env_id: &str) -> Option<(u64, u64)> {
    // 复用 handlers 中的配置文件约定
    use config as cfg;
    let cfg_name =
        std::env::var("DB_OPTION_FILE").unwrap_or_else(|_| "db_options/DbOption".to_string());
    let cfg_file = format!("{}.toml", cfg_name);
    let db_path = if std::path::Path::new(&cfg_file).exists() {
        cfg::Config::builder()
            .add_source(cfg::File::with_name(&cfg_name))
            .build()
            .ok()
            .and_then(|b| b.get_string("deployment_sites_sqlite_path").ok())
            .unwrap_or_else(|| "deployment_sites.sqlite".to_string())
    } else {
        "deployment_sites.sqlite".to_string()
    };
    let conn = rusqlite::Connection::open(&db_path).ok()?;
    let mut stmt = conn
        .prepare("SELECT reconnect_initial_ms, reconnect_max_ms FROM remote_sync_envs WHERE id = ?1 LIMIT 1")
        .ok()?;
    let mut rows = stmt.query(rusqlite::params![env_id]).ok()?;
    let row = rows.next().ok()??;
    let init: Option<i64> = row.get(0).ok().flatten();
    let max: Option<i64> = row.get(1).ok().flatten();
    Some((init.unwrap_or(1000) as u64, max.unwrap_or(30_000) as u64))
}
