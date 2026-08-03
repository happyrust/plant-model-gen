//! `initialization_runs`：ZoneStream 运行的管理侧持久状态（ADR-0016 D8 第一类）。
//!
//! 三类持久状态的分工：本表记录**一次运行的过程与结果**（进度、slot 状态、attempt
//! manifest、错误、指标）；ZONE 级的 Verified 证明与 dbnum 级的 Published 注册表都在
//! **目标 RocksDB** 里，不在这里，避免出现第二个事实源。
//!
//! 表落在管理端 SQLite（`deployment_sites.sqlite`），与 `managed_project_sites`、
//! `admin_tasks` 同库。

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension};

use crate::web_server::models::ManagedInitializationStatus;
use crate::web_server::wizard_handlers::open_deployment_sites_sqlite;

pub const TABLE_NAME: &str = "initialization_runs";

/// 单个 slot 的状态。崩溃后不恢复内存内容，Resume 一律从 `Empty` 重建（ADR-0016 D3）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SlotState {
    #[default]
    Empty,
    Parsing,
    Sealed,
    Generating,
    Backfilling,
}

impl SlotState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::Parsing => "parsing",
            Self::Sealed => "sealed",
            Self::Generating => "generating",
            Self::Backfilling => "backfilling",
        }
    }

    pub fn from_str_strict(value: &str) -> Option<Self> {
        match value.trim() {
            "empty" => Some(Self::Empty),
            "parsing" => Some(Self::Parsing),
            "sealed" => Some(Self::Sealed),
            "generating" => Some(Self::Generating),
            "backfilling" => Some(Self::Backfilling),
            _ => None,
        }
    }
}

/// 判断 Resume 能否继续同一个 run 的三个哈希（ADR-0016 D10）。
///
/// 刻意**不含内存预算**：预算是执行资源参数而非数据契约，纳入后「失败调大预算再 Resume」
/// 会退化成必须从头跑。预算单独记在 [`InitializationRun::memory_budget_mib`]。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunIdentity {
    /// 源文件集合摘要：canonical path + header60 + sesno + 逐文件 SHA-256。
    pub source_manifest_hash: String,
    /// 契约摘要：模式 + 契约 schema 版本 + 参与生成语义的配置。
    pub contract_hash: String,
    /// ZONE 规划摘要：目标 dbnum 序列与每个 dbnum 内的 ZONE 稳定序。
    pub zone_plan_hash: String,
}

impl RunIdentity {
    /// Resume 判等：三者全等才允许继续同一个 run。
    pub fn matches(&self, other: &Self) -> bool {
        self == other
    }
}

#[derive(Debug, Clone)]
pub struct InitializationRun {
    pub run_id: String,
    pub site_id: String,
    pub status: ManagedInitializationStatus,
    pub identity: RunIdentity,
    /// 本次运行的目标 dbnum，按执行顺序（升序）。
    pub target_dbnums: Vec<u32>,
    pub slot_a: SlotState,
    pub slot_b: SlotState,
    /// 当前 ZONE 的 attempt manifest；重试时按它删除独占行后重放（ADR-0016 D7）。
    pub attempt_manifest_json: Option<String>,
    pub memory_budget_mib: u32,
    pub last_error: Option<String>,
    /// 完整指标 JSON（带 schema version / mode / 各阶段耗时）。
    pub metrics_json: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub fn ensure_schema() -> Result<()> {
    let conn = open_deployment_sites_sqlite()
        .map_err(|err| anyhow::anyhow!("打开管理端 SQLite 失败: {err}"))?;
    ensure_schema_with_conn(&conn)
}

pub fn ensure_schema_with_conn(conn: &Connection) -> Result<()> {
    conn.execute_batch(&format!(
        r#"
        CREATE TABLE IF NOT EXISTS {TABLE_NAME} (
            run_id TEXT PRIMARY KEY,
            site_id TEXT NOT NULL,
            status TEXT NOT NULL,
            source_manifest_hash TEXT NOT NULL,
            contract_hash TEXT NOT NULL,
            zone_plan_hash TEXT NOT NULL,
            target_dbnums TEXT NOT NULL DEFAULT '[]',
            slot_a_state TEXT NOT NULL DEFAULT 'empty',
            slot_b_state TEXT NOT NULL DEFAULT 'empty',
            attempt_manifest_json TEXT,
            memory_budget_mib INTEGER NOT NULL DEFAULT 4096,
            last_error TEXT,
            metrics_json TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_{TABLE_NAME}_site ON {TABLE_NAME}(site_id, created_at DESC);
        "#
    ))
    .with_context(|| format!("创建 {TABLE_NAME} 表失败"))?;
    Ok(())
}

fn row_to_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<InitializationRun> {
    let invalid = |column: &str, value: &str| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("{TABLE_NAME}.{column} 取值 `{value}` 无法识别"),
            )),
        )
    };

    let status_raw: String = row.get("status")?;
    let status = ManagedInitializationStatus::from_str_strict(&status_raw)
        .ok_or_else(|| invalid("status", &status_raw))?;
    let slot_a_raw: String = row.get("slot_a_state")?;
    let slot_a = SlotState::from_str_strict(&slot_a_raw)
        .ok_or_else(|| invalid("slot_a_state", &slot_a_raw))?;
    let slot_b_raw: String = row.get("slot_b_state")?;
    let slot_b = SlotState::from_str_strict(&slot_b_raw)
        .ok_or_else(|| invalid("slot_b_state", &slot_b_raw))?;
    let target_dbnums_raw: String = row.get("target_dbnums")?;
    let target_dbnums = serde_json::from_str::<Vec<u32>>(&target_dbnums_raw)
        .map_err(|_| invalid("target_dbnums", &target_dbnums_raw))?;

    Ok(InitializationRun {
        run_id: row.get("run_id")?,
        site_id: row.get("site_id")?,
        status,
        identity: RunIdentity {
            source_manifest_hash: row.get("source_manifest_hash")?,
            contract_hash: row.get("contract_hash")?,
            zone_plan_hash: row.get("zone_plan_hash")?,
        },
        target_dbnums,
        slot_a,
        slot_b,
        attempt_manifest_json: row.get("attempt_manifest_json")?,
        memory_budget_mib: row.get::<_, i64>("memory_budget_mib")? as u32,
        last_error: row.get("last_error")?,
        metrics_json: row.get("metrics_json")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
    })
}

pub fn upsert(run: &InitializationRun) -> Result<()> {
    let conn = open_deployment_sites_sqlite()
        .map_err(|err| anyhow::anyhow!("打开管理端 SQLite 失败: {err}"))?;
    ensure_schema_with_conn(&conn)?;
    conn.execute(
        &format!(
            "INSERT OR REPLACE INTO {TABLE_NAME} (
                run_id, site_id, status, source_manifest_hash, contract_hash, zone_plan_hash,
                target_dbnums, slot_a_state, slot_b_state, attempt_manifest_json,
                memory_budget_mib, last_error, metrics_json, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)"
        ),
        rusqlite::params![
            &run.run_id,
            &run.site_id,
            run.status.as_str(),
            &run.identity.source_manifest_hash,
            &run.identity.contract_hash,
            &run.identity.zone_plan_hash,
            serde_json::to_string(&run.target_dbnums)?,
            run.slot_a.as_str(),
            run.slot_b.as_str(),
            &run.attempt_manifest_json,
            run.memory_budget_mib as i64,
            &run.last_error,
            &run.metrics_json,
            &run.created_at,
            &run.updated_at,
        ],
    )
    .with_context(|| format!("写入 {TABLE_NAME} 失败 (run_id={})", run.run_id))?;
    Ok(())
}

/// 取站点最近一次运行；Resume 与管理页摘要都以它为准。
pub fn latest_for_site(site_id: &str) -> Result<Option<InitializationRun>> {
    let conn = open_deployment_sites_sqlite()
        .map_err(|err| anyhow::anyhow!("打开管理端 SQLite 失败: {err}"))?;
    ensure_schema_with_conn(&conn)?;
    let run = conn
        .query_row(
            &format!(
                "SELECT * FROM {TABLE_NAME} WHERE site_id = ?1 ORDER BY created_at DESC LIMIT 1"
            ),
            [site_id],
            row_to_run,
        )
        .optional()
        .with_context(|| format!("读取 {TABLE_NAME} 失败 (site_id={site_id})"))?;
    Ok(run)
}

/// 站点当前的初始化状态；没有任何运行记录时为 `NotStarted`。
pub fn status_for_site(site_id: &str) -> Result<ManagedInitializationStatus> {
    Ok(latest_for_site(site_id)?
        .map(|run| run.status)
        .unwrap_or(ManagedInitializationStatus::NotStarted))
}
