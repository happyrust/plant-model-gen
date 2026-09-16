//! `pe_owner` 覆盖起点与最近一次全量审计状态。

use aios_core::project_primary_db;
use serde::Deserialize;
use surrealdb::types::SurrealValue;
use tokio::sync::OnceCell;

pub const META_SOURCE_FULL_RELOAD: &str = "full_reload";
pub const META_SOURCE_REBUILD_CLI: &str = "rebuild_cli";

static PE_OWNER_META_SCHEMA_INIT: OnceCell<()> = OnceCell::const_new();

#[derive(Debug, Deserialize, SurrealValue)]
struct MetaRow {
    maintained_since_sesno: Option<i64>,
    #[serde(default)]
    bulk_state: Option<String>,
}

/// 幂等 schema；进程内成功一次后不再重复执行，失败不缓存、下次重试。
pub async fn ensure_pe_owner_version_meta_schema() -> anyhow::Result<()> {
    PE_OWNER_META_SCHEMA_INIT
        .get_or_try_init(|| async {
            let sql = r#"
DEFINE TABLE IF NOT EXISTS pe_owner_version_meta SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS dbnum ON TABLE pe_owner_version_meta TYPE int;
DEFINE FIELD IF NOT EXISTS maintained_since_sesno ON TABLE pe_owner_version_meta TYPE option<int>;
DEFINE FIELD IF NOT EXISTS source ON TABLE pe_owner_version_meta TYPE string ASSERT $value IN ['full_reload', 'rebuild_cli'];
DEFINE FIELD IF NOT EXISTS bulk_state ON TABLE pe_owner_version_meta TYPE string DEFAULT 'not_ready' ASSERT $value IN ['not_ready', 'ready'];
DEFINE FIELD IF NOT EXISTS verified_sesno ON TABLE pe_owner_version_meta TYPE option<int>;
DEFINE FIELD IF NOT EXISTS node_count ON TABLE pe_owner_version_meta TYPE option<int>;
DEFINE FIELD IF NOT EXISTS edge_count ON TABLE pe_owner_version_meta TYPE option<int>;
DEFINE FIELD IF NOT EXISTS hierarchy_hash ON TABLE pe_owner_version_meta TYPE option<string>;
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

/// 读取 dbnum 的 pe_owner 历史覆盖起点；它不代表最近一次全量审计水位。
pub async fn get_maintained_since(dbnum: u32) -> anyhow::Result<Option<u32>> {
    ensure_pe_owner_version_meta_schema().await?;
    let sql =
        format!("SELECT maintained_since_sesno, bulk_state FROM pe_owner_version_meta:{dbnum};");
    let mut response = project_primary_db().query(sql).await?.check()?;
    let rows: Vec<MetaRow> = response.take(0)?;
    Ok(rows
        .into_iter()
        .next()
        .filter(|row| row.bulk_state.as_deref() == Some("ready"))
        .and_then(|row| row.maintained_since_sesno)
        .map(|sesno| sesno.max(0) as u32))
}

pub async fn require_bulk_ready(dbnum: u32) -> anyhow::Result<()> {
    ensure_pe_owner_version_meta_schema().await?;
    let sql = format!("SELECT VALUE bulk_state FROM pe_owner_version_meta:{dbnum};");
    let mut response = project_primary_db().query(sql).await?.check()?;
    let states: Vec<Option<String>> = response.take(0)?;
    anyhow::ensure!(
        states.into_iter().flatten().next().as_deref() == Some("ready"),
        "dbnum={dbnum} pe_owner 尚未通过全量完整性审计"
    );
    Ok(())
}

/// 全量写入前调用。失败时调用方必须在修改 PE/pe_owner 前中止。
pub async fn mark_bulk_not_ready(dbnum: u32, source: &str) -> anyhow::Result<()> {
    ensure_pe_owner_version_meta_schema().await?;
    let sql = format!(
        "UPSERT pe_owner_version_meta:{dbnum} SET dbnum = {dbnum}, source = $source, \
         maintained_since_sesno = maintained_since_sesno ?? 0, bulk_state = 'not_ready', \
         verified_sesno = NONE, node_count = NONE, \
         edge_count = NONE, hierarchy_hash = NONE, updated_at = time::now();"
    );
    project_primary_db()
        .query(sql)
        .bind(("source", source.to_string()))
        .await?
        .check()?;
    Ok(())
}

/// worker 全部完成且持久数据审计通过后发布 Ready。
pub async fn publish_bulk_ready(
    dbnum: u32,
    sesno: u32,
    source: &str,
    node_count: usize,
    edge_count: usize,
    hierarchy_hash: &str,
) -> anyhow::Result<()> {
    ensure_pe_owner_version_meta_schema().await?;
    let sql = format!(
        "UPSERT pe_owner_version_meta:{dbnum} SET dbnum = {dbnum}, \
         maintained_since_sesno = {sesno}, source = $source, bulk_state = 'ready', \
         verified_sesno = {sesno}, node_count = {node_count}, edge_count = {edge_count}, \
         hierarchy_hash = $hierarchy_hash, updated_at = time::now();"
    );
    project_primary_db()
        .query(sql)
        .bind(("source", source.to_string()))
        .bind(("hierarchy_hash", hierarchy_hash.to_string()))
        .await?
        .check()?;
    Ok(())
}
