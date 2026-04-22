//! 异地协同功能相关的 SQLite schema 迁移（幂等）
//!
//! 这些迁移用于把 plant-model-gen 的 `deployment_sites.sqlite` 对齐到
//! 从 web-server 汇入的异地协同 handler 所需的 schema 超集。
//!
//! 调用时机：`start_web_server` 启动时调用一次，也可以在 handler 首次访问前惰性调用。
//!
//! 设计：
//! - 所有 DDL 使用 `CREATE TABLE IF NOT EXISTS` 或 `ALTER TABLE ADD COLUMN`（容忍重复）。
//! - SQLite 没有 `ALTER TABLE ADD COLUMN IF NOT EXISTS`，因此用 `PRAGMA table_info`
//!   先查存在性再决定是否 ALTER。
//! - 所有 migration 失败只打 warn，不阻塞服务启动（避免 schema 老旧的站点启动失败）。
//!
//! 引入于 Phase 1.6（详见 `docs/plans/2026-04-22-phase-1-execution-checklist.md`）。

use rusqlite::Connection;

/// SQLite 数据库路径（与 site_config_handlers / sync_control_handlers 保持一致）
fn resolve_db_path() -> String {
    // 优先读 DbOption.toml 中的 deployment_sites_sqlite_path
    use config as cfg;
    let cfg_name =
        std::env::var("DB_OPTION_FILE").unwrap_or_else(|_| "db_options/DbOption".to_string());
    let cfg_file = format!("{}.toml", cfg_name);

    if std::path::Path::new(&cfg_file).exists() {
        if let Ok(builder) = cfg::Config::builder()
            .add_source(cfg::File::with_name(&cfg_name))
            .build()
        {
            if let Ok(path) = builder.get_string("deployment_sites_sqlite_path") {
                return path;
            }
        }
    }
    "deployment_sites.sqlite".to_string()
}

/// 检查某张表里是否已有指定列
fn column_exists(conn: &Connection, table: &str, column: &str) -> rusqlite::Result<bool> {
    let sql = format!("PRAGMA table_info({})", table);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for name in rows {
        if name? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

/// 幂等确保异地协同 schema 就位
///
/// 对齐项（与 web-server 版 deployment_sites.sqlite 一致）：
/// 1. `remote_sync_sites` 追加 3 列：`master_mqtt_host`、`master_mqtt_port`、`master_location`
/// 2. 新建表 `node_config(location TEXT PK, is_master BOOL, updated_at)`
pub fn ensure_collab_schema() {
    let db_path = resolve_db_path();
    let conn = match Connection::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            log::warn!(
                "⚠️  [collab-migrate] 打开 SQLite 失败，跳过 schema 对齐: {} (path={})",
                e,
                db_path
            );
            return;
        }
    };

    // 1. remote_sync_sites 追加 master_* 3 列
    let existing_table = conn
        .query_row(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='remote_sync_sites'",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok();

    if existing_table.is_some() {
        for (col, col_type) in [
            ("master_mqtt_host", "TEXT"),
            ("master_mqtt_port", "INTEGER"),
            ("master_location", "TEXT"),
        ] {
            match column_exists(&conn, "remote_sync_sites", col) {
                Ok(true) => {
                    log::debug!("✓ [collab-migrate] remote_sync_sites.{} 已存在", col);
                }
                Ok(false) => {
                    let ddl = format!(
                        "ALTER TABLE remote_sync_sites ADD COLUMN {} {}",
                        col, col_type
                    );
                    match conn.execute(&ddl, []) {
                        Ok(_) => log::info!("✅ [collab-migrate] 已追加列 remote_sync_sites.{}", col),
                        Err(e) => log::warn!(
                            "⚠️  [collab-migrate] 追加列 remote_sync_sites.{} 失败: {}",
                            col,
                            e
                        ),
                    }
                }
                Err(e) => log::warn!(
                    "⚠️  [collab-migrate] 检查列 remote_sync_sites.{} 失败: {}",
                    col,
                    e
                ),
            }
        }
    }

    // 2. node_config 表（主从节点切换）
    if let Err(e) = conn.execute(
        "CREATE TABLE IF NOT EXISTS node_config (\n            location TEXT PRIMARY KEY,\n            is_master BOOLEAN NOT NULL DEFAULT 0,\n            updated_at TEXT DEFAULT CURRENT_TIMESTAMP\n        )",
        [],
    ) {
        log::warn!("⚠️  [collab-migrate] 创建 node_config 表失败: {}", e);
    } else {
        log::debug!("✓ [collab-migrate] node_config 表就绪");
    }

    log::info!("🎯 [collab-migrate] 异地协同 schema 对齐完成 (path={})", db_path);
}
