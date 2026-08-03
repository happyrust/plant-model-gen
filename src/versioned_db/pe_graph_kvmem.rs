//! 切片 2 第一块：把已发布的 rkyv 种子装载进临时 kv-mem SurrealDB 站点（ADR-0012）。
//!
//! 严格按 ADR 53-61 行：只接受 `pe_graph_seed_meta` 的 Ready 记录并打开其精确文件；
//! 在 `spawn_blocking` 中读入 aligned buffer，依次校验前缀/长度/SHA-256/rkyv archived
//! access/业务凭据（**不整体反序列化成原生 `Vec<Node>`**，走 `rkyv::access` 零拷贝 +
//! 逐字段取值）；每批最多 500 行 typed INSERT；kv-mem 使用最小严格 schema（`pe SCHEMAFULL`、
//! `pe_owner TYPE RELATION IN pe OUT pe ENFORCED`、仅 `(dbnum, noun)` 索引）；先写全部 `pe`
//! 再写 `pe_owner`，最后校验节点数、边数与 ref0 集合摘要。
//!
//! 本模块不接入产品 CLI；配套 `examples/seed_kvmem_bench.rs` 驱动它测装载耗时。

use std::collections::BTreeSet;
use std::path::Path;
use std::time::Instant;

use aios_core::{RefU64, SurrealQueryExt};
use anyhow::{Context, Result, anyhow, ensure};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use surrealdb::Surreal;
use surrealdb::engine::local::{Db, Mem};
use surrealdb::types::{RecordId, SurrealValue};

use crate::versioned_db::pe_graph_seed::{self, ArchivedPeGraphSeedV1, ORDER_NONE};

/// kv-mem 最小严格 schema（ADR 59 行）。
const KVMEM_SCHEMA: &str = r#"
DEFINE TABLE IF NOT EXISTS pe SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS dbnum ON TABLE pe TYPE int;
DEFINE FIELD IF NOT EXISTS owner ON TABLE pe TYPE record<pe>;
DEFINE FIELD IF NOT EXISTS noun ON TABLE pe TYPE string;
DEFINE FIELD IF NOT EXISTS name ON TABLE pe TYPE string;
DEFINE FIELD IF NOT EXISTS cata_hash ON TABLE pe TYPE option<string>;
DEFINE FIELD IF NOT EXISTS child_count ON TABLE pe TYPE int;
DEFINE TABLE IF NOT EXISTS pe_owner TYPE RELATION IN pe OUT pe ENFORCED;
DEFINE INDEX IF NOT EXISTS pe_dbnum_noun ON TABLE pe FIELDS dbnum, noun;
"#;

/// `pe` / `pe_owner` 每个 INSERT 最多 500 行（ADR 54/73 行）。
const INSERT_BATCH: usize = 500;

/// 单个 dbnum 分片装载的分阶段耗时（用于 bench 观测，不落审计）。
#[derive(Debug, Clone, Default)]
pub struct KvMemLoadStats {
    pub dbnum: u32,
    pub sesno: u32,
    pub node_count: usize,
    pub edge_count: usize,
    pub read_validate_ms: u128,
    pub insert_pe_ms: u128,
    pub insert_owner_ms: u128,
    pub verify_ms: u128,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct SeedMetaRow {
    state: String,
    #[serde(default)]
    sesno: Option<i64>,
    #[serde(default)]
    file_name: Option<String>,
    #[serde(default)]
    payload_sha256: Option<String>,
    #[serde(default)]
    node_count: Option<i64>,
    #[serde(default)]
    edge_count: Option<i64>,
}

/// kv-mem `pe` 行（ADR 60 行：dbnum、owner、noun、name、cata_hash、child_count）。
/// noun/name/cata_hash 是任意字符串，必须以 typed value 绑定，禁止拼接未转义 SQL。
#[derive(Debug, Clone, SurrealValue)]
struct KvPeRow {
    id: RecordId,
    dbnum: i64,
    owner: RecordId,
    noun: String,
    name: String,
    cata_hash: Option<String>,
    child_count: i64,
}

/// 创建一个空的 kv-mem 站点并建立最小严格 schema。每次初始化模型生成只创建一个站点。
pub async fn create_kvmem_site() -> Result<Surreal<Db>> {
    let db = Surreal::new::<Mem>(())
        .await
        .context("创建 kv-mem 站点失败")?;
    db.use_ns("gen_kvmem")
        .use_db("cache")
        .await
        .context("kv-mem use ns/db 失败")?;
    db.query(KVMEM_SCHEMA)
        .await
        .context("定义 kv-mem schema 失败")?
        .check()
        .context("执行 kv-mem schema 失败")?;
    Ok(db)
}

/// 把某 dbnum 的已发布 Ready 种子装载进给定 kv-mem 站点。
///
/// `expected_sesno == 0` 时跳过 sesno 凭据比对（bench 场景无法预知当前初始化 sesno）；
/// 生产切片 2 装载时应传入当前初始化 sesno 做严格校验。
pub async fn load_dbnum_into_kvmem(
    db: &Surreal<Db>,
    tree_dir: &Path,
    dbnum: u32,
    expected_sesno: u32,
) -> Result<KvMemLoadStats> {
    // 1) 只接受持久库中该 dbnum 的 Ready 元数据，取精确文件名与凭据。
    let meta: Option<SeedMetaRow> = aios_core::project_primary_db()
        .query_take(
            &format!(
                "SELECT state, sesno, file_name, payload_sha256, node_count, edge_count \
                 FROM ONLY pe_graph_seed_meta:{dbnum} LIMIT 1;"
            ),
            0,
        )
        .await
        .with_context(|| format!("dbnum={dbnum} 读取 pe_graph_seed_meta 失败"))?;
    let meta = meta.ok_or_else(|| anyhow!("dbnum={dbnum} 无 pe_graph_seed_meta 记录"))?;
    ensure!(
        meta.state == "ready",
        "dbnum={dbnum} 种子未 Ready(state={})，禁止装载",
        meta.state
    );
    let file_name = meta
        .file_name
        .ok_or_else(|| anyhow!("dbnum={dbnum} Ready 元数据缺 file_name"))?;
    let payload_sha256 = meta
        .payload_sha256
        .ok_or_else(|| anyhow!("dbnum={dbnum} Ready 元数据缺 payload_sha256"))?;
    let meta_sesno = meta.sesno.unwrap_or(0).max(0) as u32;
    let meta_node_count = meta.node_count.unwrap_or(0).max(0) as usize;
    let meta_edge_count = meta.edge_count.unwrap_or(0).max(0) as usize;
    if expected_sesno != 0 {
        ensure!(
            meta_sesno == expected_sesno,
            "dbnum={dbnum} 种子 sesno={meta_sesno} 与期望 {expected_sesno} 不一致"
        );
    }

    let path = pe_graph_seed::seed_file_path(tree_dir, &file_name);

    // 2) spawn_blocking：读入 aligned buffer → 校验前缀/长度/SHA → archived access →
    //    业务凭据（dbnum/node_count/edge_count/ref0 摘要）→ 逐字段转成 typed 行。
    let read_start = Instant::now();
    let (pe_rows, edges) = {
        let path = path.clone();
        let payload_sha256 = payload_sha256.clone();
        tokio::task::spawn_blocking(move || -> Result<(Vec<KvPeRow>, Vec<(u64, u32, u64)>)> {
            let payload = pe_graph_seed::read_seed_payload_aligned(&path, &payload_sha256)?;
            let archived =
                rkyv::access::<ArchivedPeGraphSeedV1, rkyv::rancor::Error>(payload.as_slice())
                    .map_err(|e| anyhow!("rkyv archived access 失败: {e:?}"))?;

            ensure!(
                archived.dbnum.to_native() == dbnum,
                "种子 dbnum={} 与请求 {dbnum} 不一致",
                archived.dbnum.to_native()
            );
            let node_count = archived.node_count.to_native() as usize;
            let edge_count = archived.edge_count.to_native() as usize;
            ensure!(
                node_count == meta_node_count,
                "种子 node_count={node_count} 与元数据 {meta_node_count} 不一致"
            );
            ensure!(
                edge_count == meta_edge_count,
                "种子 edge_count={edge_count} 与元数据 {meta_edge_count} 不一致"
            );

            let node_refnos: BTreeSet<u64> =
                archived.nodes.iter().map(|n| n.refno.to_native()).collect();
            let mut pe_rows: Vec<KvPeRow> = Vec::with_capacity(node_refnos.len());
            let mut edges: Vec<(u64, u32, u64)> = Vec::with_capacity(edge_count);
            let mut ref0s: BTreeSet<u32> = BTreeSet::new();
            for node in archived.nodes.iter() {
                let refno = node.refno.to_native();
                let owner = node.owner.to_native();
                let order = node.order.to_native();
                let child_count = node.child_count.to_native();
                pe_rows.push(KvPeRow {
                    id: RefU64(refno).to_pe_thing(),
                    dbnum: dbnum as i64,
                    owner: RefU64(owner).to_pe_thing(),
                    noun: node.noun.as_str().to_string(),
                    name: node.name.as_str().to_string(),
                    cata_hash: node.cata_hash.as_ref().map(|s| s.as_str().to_string()),
                    child_count: child_count as i64,
                });
                if owner != refno && order != ORDER_NONE && node_refnos.contains(&owner) {
                    edges.push((owner, order, refno));
                }
                let ref0 = (refno >> 32) as u32;
                if ref0 != 0 && ref0 != 0x8000_0001 {
                    ref0s.insert(ref0);
                }
            }
            ensure!(
                edges.len() == edge_count,
                "种子边数重建={} 与声明 {edge_count} 不一致",
                edges.len()
            );
            // ref0 集合摘要凭据（与 pe_graph_seed::ref0_set_hash 同口径）。
            let mut hasher = Sha256::new();
            for ref0 in &ref0s {
                hasher.update(ref0.to_le_bytes());
            }
            ensure!(
                hex::encode(hasher.finalize()) == archived.ref0_set_hash.as_str(),
                "种子 ref0 集合摘要校验失败"
            );
            Ok((pe_rows, edges))
        })
        .await
        .context("kv-mem 装载读取任务失败")??
    };
    let read_validate_ms = read_start.elapsed().as_millis();
    let node_count = pe_rows.len();
    let edge_count = edges.len();

    // 3) 先写全部 pe（typed，500/批），再写 pe_owner。
    //    pe_owner 行只含 record id（无任意字符串），用字面量安全拼接。
    let pe_start = Instant::now();
    for chunk in pe_rows.chunks(INSERT_BATCH) {
        db.query("INSERT INTO pe $rows;")
            .bind(("rows", chunk.to_vec()))
            .await
            .context("kv-mem 写入 pe 失败")?
            .check()
            .context("kv-mem 执行 pe INSERT 失败")?;
    }
    let insert_pe_ms = pe_start.elapsed().as_millis();

    let owner_start = Instant::now();
    for chunk in edges.chunks(INSERT_BATCH) {
        let rows_sql = chunk
            .iter()
            .map(|(owner, order, child)| {
                let owner_key = RefU64(*owner).to_pe_key();
                format!(
                    "{{ id: pe_owner:[{owner_key}, {order}], in: {}, out: {owner_key} }}",
                    RefU64(*child).to_pe_key()
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        db.query(format!("INSERT RELATION INTO pe_owner [{rows_sql}];"))
            .await
            .context("kv-mem 写入 pe_owner 失败")?
            .check()
            .context("kv-mem 执行 pe_owner INSERT 失败")?;
    }
    let insert_owner_ms = owner_start.elapsed().as_millis();

    // 4) 校验节点数、边数（ref0 摘要已在 archived 凭据阶段核对）。
    let verify_start = Instant::now();
    let pe_cnt: Option<i64> = db
        .query_take("SELECT VALUE count() FROM pe GROUP ALL;", 0)
        .await
        .context("kv-mem 统计 pe 行数失败")?;
    let owner_cnt: Option<i64> = db
        .query_take("SELECT VALUE count() FROM pe_owner GROUP ALL;", 0)
        .await
        .context("kv-mem 统计 pe_owner 行数失败")?;
    let pe_cnt = pe_cnt.unwrap_or(0);
    let owner_cnt = owner_cnt.unwrap_or(0);
    ensure!(
        pe_cnt as usize == node_count,
        "kv-mem pe 行数={pe_cnt} 与种子 {node_count} 不一致"
    );
    ensure!(
        owner_cnt as usize == edge_count,
        "kv-mem pe_owner 边数={owner_cnt} 与种子 {edge_count} 不一致"
    );
    let verify_ms = verify_start.elapsed().as_millis();

    Ok(KvMemLoadStats {
        dbnum,
        sesno: meta_sesno,
        node_count,
        edge_count,
        read_validate_ms,
        insert_pe_ms,
        insert_owner_ms,
        verify_ms,
    })
}
