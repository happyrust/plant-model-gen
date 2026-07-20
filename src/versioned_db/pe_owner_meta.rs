//! specs/023：pe_owner 历史可信分界元记录（`pe_owner_version_meta`）。
//!
//! 每 dbnum 一条 `pe_owner_version_meta:<dbnum>`，记录"自哪个 sesno（含）起
//! pe_owner 边由解析链路持续维护"。读侧（e3d_tree_api 版本分支）据此在
//! pe_owner 主路径与 pe.children 回退路径之间选择（FR-008）：
//! `requested_sesno >= maintained_since_sesno` → pe_owner；否则 → pe.children。
//!
//! 写入时机（data-model.md 状态迁移）：
//! - 全量重灌完成：UPSERT 重置（source=`full_reload`）
//! - 存量重建 CLI：UPSERT（source=`rebuild_cli`）
//! - **增量不写 meta**：增量只维护"本批变更过的 owner"的边；若站点曾在旧二进制下跑过
//!   增量（边未维护、已陈旧），首个新增量并不能修复陈旧边，此时打可信标记会产生
//!   静默错误历史。可信起点只能由全量重灌或重建 CLI 建立。

use aios_core::project_primary_db;
use serde::Deserialize;
use surrealdb::types::SurrealValue;
use tokio::sync::OnceCell;

pub const META_SOURCE_FULL_RELOAD: &str = "full_reload";
pub const META_SOURCE_REBUILD_CLI: &str = "rebuild_cli";

static PE_OWNER_META_SCHEMA_INIT: OnceCell<()> = OnceCell::const_new();

#[derive(Debug, Deserialize, SurrealValue)]
struct MetaRow {
    maintained_since_sesno: i64,
}

/// 幂等 schema；进程内成功一次后不再重复执行，失败不缓存、下次重试。
pub async fn ensure_pe_owner_version_meta_schema() -> anyhow::Result<()> {
    PE_OWNER_META_SCHEMA_INIT
        .get_or_try_init(|| async {
            let sql = r#"
DEFINE TABLE IF NOT EXISTS pe_owner_version_meta SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS dbnum ON TABLE pe_owner_version_meta TYPE int;
DEFINE FIELD IF NOT EXISTS maintained_since_sesno ON TABLE pe_owner_version_meta TYPE int;
DEFINE FIELD IF NOT EXISTS source ON TABLE pe_owner_version_meta TYPE string ASSERT $value IN ['full_reload', 'rebuild_cli'];
DEFINE FIELD IF NOT EXISTS updated_at ON TABLE pe_owner_version_meta TYPE datetime DEFAULT time::now();
"#;
            project_primary_db()
                .query(sql)
                .await
                .map_err(|e| anyhow::anyhow!("定义 pe_owner_version_meta schema 失败: {e}"))?
                .check()
                .map_err(|e| anyhow::anyhow!("pe_owner_version_meta schema 语句执行失败: {e}"))?;
            Ok::<(), anyhow::Error>(())
        })
        .await?;
    Ok(())
}

/// 读取 dbnum 的 pe_owner 可信起点 sesno；记录缺失返回 None（读侧应回退 pe.children）。
pub async fn get_maintained_since(dbnum: u32) -> anyhow::Result<Option<u32>> {
    ensure_pe_owner_version_meta_schema().await?;
    let sql = format!("SELECT maintained_since_sesno FROM pe_owner_version_meta:{dbnum};");
    let mut response = project_primary_db().query(sql).await?.check()?;
    let rows: Vec<MetaRow> = response.take(0)?;
    Ok(rows
        .into_iter()
        .next()
        .map(|row| row.maintained_since_sesno.max(0) as u32))
}

/// UPSERT 可信起点（full_reload / rebuild_cli 语义：重置分界）。
pub async fn upsert_maintained_since(dbnum: u32, sesno: u32, source: &str) -> anyhow::Result<()> {
    ensure_pe_owner_version_meta_schema().await?;
    let sql = format!(
        "UPSERT pe_owner_version_meta:{dbnum} SET dbnum = {dbnum}, \
         maintained_since_sesno = {sesno}, source = $source, updated_at = time::now();"
    );
    project_primary_db()
        .query(sql)
        .bind(("source", source.to_string()))
        .await?
        .check()?;
    Ok(())
}
