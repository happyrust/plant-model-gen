//! 目录引用反向索引 `cata_ref_index`（ADR-0011 / P1 索引层）。
//!
//! 增量模型生成需要在**目录定义**（SCOM 及其下几何/尺寸）被改时，反查所有引用
//! 它的设计实例并纳入重生成目标（`目录反向波及闭包`，见 `CONTEXT.md`）。当前实现
//! 只把「被改 refno 自身」入桶，漏掉所有引用它的实例（最大漏判，见
//! `docs/reverse/incremental_update_vs_core_dll.md` §4.4）。
//!
//! 本模块提供该闭包的**索引层**：存 as-written 语法引用边
//! `{source_refno, source_dbnum, attribute(大写), ordinal, target_refno}`，覆盖
//! **除 OWNER/CHILDREN 层级轴与自身标识 REFNO/ID 外的全部 Ref/RefList**，随主库
//! MVCC 版本化（本表在同一 versioned 数据库内，自然获得 `VERSION AT` 历史）。它是
//! 大计划 `pe_reference_edge`（`docs/plans/2026-07-23-incremental-model-impact-closure-refactor-plan.md`
//! Q3/§5）的先行**目录子集**：列名对齐，将来收敛为其视图/改名，不重抽取。
//!
//! ## P1 分步（当前只落 step-1，见落地方案 §6）
//! - **step-1（本文件 + `reference-index` CLI）**：schema ensure、纯函数
//!   `extract_ref_edges`、`replace-by-source` SQL 构造、一跳读 API、`cata_ref_index_state`、
//!   以及 `backfill`/`audit` CLI。**不触碰增量提交热路径**，可独立 CLI 自测、随时回退。
//! - **step-2（后续）**：在 `sesno_increment::persist_pdms_increment_grouped` 的
//!   `commit_version` apply 闭包内按 changed source `replace-by-source`（与数据同事务、
//!   仿 debt 先例**不进 fingerprint**，避免改动既有锚点幂等）。
//!
//! ## 正确性红线（ADR-0011）
//! - as-written 语法边：不把 `get_or_create_scom_info` 解析烘焙进索引；多跳
//!   `SPRE→CATR→…→SCOM` 留给上层 expander 的 BFS（P2）。
//! - 删 target **不**级联（被删 SCOM 仍可反查引用者）；删 source 才清其出边。
//! - 「设计实例改自身 CATR/SPRE/PRTREF」= direct-only、不经此闭包扇出兄弟；只有
//!   「目录定义被改」才反向扇出——该语义属传播层（P2），本索引层只忠实记边。

use aios_core::{NamedAttrMap, NamedAttrValue, RefU64, SurrealQueryExt, project_primary_db};
use itertools::Itertools;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use surrealdb::types::SurrealValue;

use crate::version_management::model_impact::normalize_attribute_name;

/// 抽取器版本：写入 `cata_ref_index_state`，抽取规则变化时用于判定 backfill 过期。
pub const EXTRACTOR_VERSION: &str = "cata-ref-index-v1";

/// 不进反向索引的属性（大写规范名后比较）。
/// - 层级轴 `OWNER/CHILDREN/MEMBERS/MEMB` 由 `pe_owner` 关系边承载（ADR-0011 Q2）；
/// - 自身标识 `REFNO/ID` 不是指向他元素的引用，收录会造成自环噪声。
const EXCLUDED_ATTRS: &[&str] = &["OWNER", "CHILDREN", "MEMBERS", "MEMB", "REFNO", "ID"];

/// 一条 as-written 语法引用边。`source_refno`/`target_refno` 用规范 `"ref0_ref1"`
/// 字符串（`RefU64::to_string()`），稳定、可反查、且不依赖记录存在（删 target 仍可查）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefEdge {
    pub source_dbnum: u32,
    pub source_refno: String,
    pub attribute: String,
    pub ordinal: u32,
    pub target_refno: String,
}

impl RefEdge {
    /// 规范化排序/去重键（source 固定时用于比对）。
    fn canonical(&self) -> (String, String, u32, String) {
        (
            self.source_refno.clone(),
            self.attribute.clone(),
            self.ordinal,
            self.target_refno.clone(),
        )
    }

    /// 顺序无关的内容摘要贡献（XOR-fold 用）。
    fn digest(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(b"cata-ref-edge-v1\0");
        hasher.update(self.source_dbnum.to_le_bytes());
        hasher.update([0]);
        hasher.update(self.source_refno.as_bytes());
        hasher.update([0]);
        hasher.update(self.attribute.as_bytes());
        hasher.update([0]);
        hasher.update(self.ordinal.to_le_bytes());
        hasher.update([0]);
        hasher.update(self.target_refno.as_bytes());
        hasher.finalize().into()
    }
}

#[derive(Debug, Deserialize, SurrealValue)]
struct RefEdgeRow {
    source_dbnum: i64,
    source_refno: String,
    attribute: String,
    ordinal: i64,
    target_refno: String,
}

impl From<RefEdgeRow> for RefEdge {
    fn from(row: RefEdgeRow) -> Self {
        RefEdge {
            source_dbnum: row.source_dbnum.max(0) as u32,
            source_refno: row.source_refno,
            attribute: row.attribute,
            ordinal: row.ordinal.max(0) as u32,
            target_refno: row.target_refno,
        }
    }
}

/// `cata_ref_index_state` 行：某 dbnum 的 backfill 水位（ready 门 + 对账基线）。
#[derive(Debug, Clone, Deserialize, SurrealValue)]
pub struct RefIndexState {
    pub dbnum: i64,
    pub ready: bool,
    pub row_count: i64,
    pub checksum: String,
    #[serde(default)]
    pub extractor_version: Option<String>,
}

// ──────────────────────────────────────────────────────────────────────────
// schema
// ──────────────────────────────────────────────────────────────────────────

/// 代码内 ensure（仿 `pe_owner_tree` / `version_commit`）：`pe` 为 SCHEMALESS，
/// 反向索引同样用 NORMAL SCHEMALESS + 显式索引，随主库 MVCC 版本化。
pub async fn ensure_cata_ref_index_schema() -> anyhow::Result<()> {
    let sql = r#"
DEFINE TABLE IF NOT EXISTS cata_ref_index TYPE NORMAL SCHEMALESS;
DEFINE INDEX IF NOT EXISTS idx_crx_target ON TABLE cata_ref_index FIELDS target_refno;
DEFINE INDEX IF NOT EXISTS idx_crx_source ON TABLE cata_ref_index FIELDS source_dbnum, source_refno;

DEFINE TABLE IF NOT EXISTS cata_ref_index_state TYPE NORMAL SCHEMALESS;
"#;
    project_primary_db().query(sql).await?.check()?;
    Ok(())
}

// ──────────────────────────────────────────────────────────────────────────
// 抽取（纯函数，无 DB —— 供 backfill / audit / 未来写入接缝复用）
// ──────────────────────────────────────────────────────────────────────────

/// 从一个元素的属性 map 抽取全部 as-written 引用边（除层级/自身标识轴）。
///
/// - `RefU64Type` / `RefnoEnumType` → 单边，`ordinal = 0`；
/// - `RefU64Array`（RefList）→ 逐项，`ordinal = 数组下标`（稳定顺序）；
/// - 跳过 unset（ref0==0）与自环（target==source）目标。
pub fn extract_ref_edges(source: RefU64, source_dbnum: u32, att: &NamedAttrMap) -> Vec<RefEdge> {
    let source_key = source.to_string();
    let mut edges = Vec::new();
    let push = |edges: &mut Vec<RefEdge>, attribute: &str, ordinal: u32, target: RefU64| {
        if target.is_unset() {
            return;
        }
        let target_refno = target.to_string();
        if target_refno == source_key {
            return;
        }
        edges.push(RefEdge {
            source_dbnum,
            source_refno: source_key.clone(),
            attribute: attribute.to_string(),
            ordinal,
            target_refno,
        });
    };

    for (raw_name, value) in att.map.iter() {
        let attribute = normalize_attribute_name(raw_name);
        if attribute.is_empty() || EXCLUDED_ATTRS.contains(&attribute.as_str()) {
            continue;
        }
        match value {
            NamedAttrValue::RefU64Type(refno) => push(&mut edges, &attribute, 0, *refno),
            NamedAttrValue::RefnoEnumType(refno) => push(&mut edges, &attribute, 0, refno.refno()),
            NamedAttrValue::RefU64Array(refnos) => {
                for (idx, refno) in refnos.iter().enumerate() {
                    push(&mut edges, &attribute, idx as u32, refno.refno());
                }
            }
            _ => {}
        }
    }
    edges
}

// ──────────────────────────────────────────────────────────────────────────
// replace-by-source SQL 构造（step-2 写入接缝 / backfill 复用）
// ──────────────────────────────────────────────────────────────────────────

fn sql_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "\\'"))
}

fn edge_record_id(edge: &RefEdge) -> String {
    format!(
        "cata_ref_index:[{}, {}, {}, {}]",
        edge.source_dbnum,
        sql_quote(&edge.source_refno),
        sql_quote(&edge.attribute),
        edge.ordinal
    )
}

fn edge_insert_object(edge: &RefEdge) -> String {
    format!(
        "{{ id: {id}, source_dbnum: {db}, source_refno: {src}, attribute: {attr}, ordinal: {ord}, target_refno: {tgt} }}",
        id = edge_record_id(edge),
        db = edge.source_dbnum,
        src = sql_quote(&edge.source_refno),
        attr = sql_quote(&edge.attribute),
        ord = edge.ordinal,
        tgt = sql_quote(&edge.target_refno),
    )
}

/// 单个 source 的 replace-by-source：返回 (删除语句, 插入语句集)。
///
/// 先删后插（MVCC 保旧版）。删/插**必须分属不同请求**提交：同一请求内「删同 id →
/// 重插同 id」在 versioned 引擎会撞唯一约束（见 `sesno_increment` 中 pe_owner 边先例）。
pub fn build_replace_by_source_sql(
    source_dbnum: u32,
    source_refno: &str,
    edges: &[RefEdge],
) -> (String, Vec<String>) {
    let delete = format!(
        "DELETE cata_ref_index WHERE source_dbnum = {source_dbnum} AND source_refno = {};",
        sql_quote(source_refno)
    );
    let inserts = insert_edges_sql(edges, 500);
    (delete, inserts)
}

/// 批量删除一页 source 的现有出边（backfill 幂等重跑用）。
pub fn delete_sources_sql(source_dbnum: u32, sources: &[String]) -> Option<String> {
    if sources.is_empty() {
        return None;
    }
    let list = sources.iter().map(|s| sql_quote(s)).join(", ");
    Some(format!(
        "DELETE cata_ref_index WHERE source_dbnum = {source_dbnum} AND source_refno IN [{list}];"
    ))
}

/// 分块 INSERT（每块 `chunk` 条）。
pub fn insert_edges_sql(edges: &[RefEdge], chunk: usize) -> Vec<String> {
    let chunk = chunk.max(1);
    edges
        .chunks(chunk)
        .map(|batch| {
            let rows = batch.iter().map(edge_insert_object).join(", ");
            format!("INSERT INTO cata_ref_index [{rows}];")
        })
        .collect()
}

// ──────────────────────────────────────────────────────────────────────────
// 读 API（一跳；传递 BFS/环/深度收敛归 P2 expander，ADR-0011 Q5）
// ──────────────────────────────────────────────────────────────────────────

/// 出边：给定 source 集合读其全部引用边（audit / 维护用）。
pub async fn load_outbound_references(
    source_dbnum: u32,
    sources: &[RefU64],
) -> anyhow::Result<Vec<RefEdge>> {
    if sources.is_empty() {
        return Ok(Vec::new());
    }
    let list = sources.iter().map(|r| sql_quote(&r.to_string())).join(", ");
    let sql = format!(
        "SELECT source_dbnum, source_refno, attribute, ordinal, target_refno \
         FROM cata_ref_index WHERE source_dbnum = {source_dbnum} AND source_refno IN [{list}];"
    );
    let rows: Vec<RefEdgeRow> = project_primary_db().query_take(&sql, 0).await?;
    Ok(rows.into_iter().map(RefEdge::from).collect())
}

/// 入边：反查「谁引用了这些 target」。一跳分页原语（P2 expander 消费）。
/// `families` 为空表示不按属性族过滤（收录全属性、传播期再滤，ADR-0011 Q2）。
pub async fn load_inbound_references(
    targets: &[RefU64],
    families: Option<&[String]>,
    limit: usize,
) -> anyhow::Result<Vec<RefEdge>> {
    if targets.is_empty() {
        return Ok(Vec::new());
    }
    let list = targets.iter().map(|r| sql_quote(&r.to_string())).join(", ");
    let family_clause = match families {
        Some(fams) if !fams.is_empty() => {
            let fam_list = fams.iter().map(|f| sql_quote(f)).join(", ");
            format!(" AND attribute IN [{fam_list}]")
        }
        _ => String::new(),
    };
    let limit = limit.clamp(1, 100_000);
    let sql = format!(
        "SELECT source_dbnum, source_refno, attribute, ordinal, target_refno \
         FROM cata_ref_index WHERE target_refno IN [{list}]{family_clause} LIMIT {limit};"
    );
    let rows: Vec<RefEdgeRow> = project_primary_db().query_take(&sql, 0).await?;
    Ok(rows.into_iter().map(RefEdge::from).collect())
}

// ──────────────────────────────────────────────────────────────────────────
// state（ready 门 + 对账基线）
// ──────────────────────────────────────────────────────────────────────────

pub async fn read_state(dbnum: u32) -> anyhow::Result<Option<RefIndexState>> {
    let sql = format!(
        "SELECT dbnum, ready, row_count, checksum, extractor_version \
         FROM cata_ref_index_state:[{dbnum}];"
    );
    let rows: Vec<RefIndexState> = project_primary_db().query_take(&sql, 0).await?;
    Ok(rows.into_iter().next())
}

pub async fn write_state(
    dbnum: u32,
    ready: bool,
    row_count: usize,
    checksum: &str,
) -> anyhow::Result<()> {
    let sql = format!(
        "UPSERT cata_ref_index_state:[{dbnum}] SET dbnum = {dbnum}, ready = {ready}, \
         row_count = {row_count}, checksum = {checksum}, extractor_version = {version}, \
         backfilled_at = time::now();",
        checksum = sql_quote(checksum),
        version = sql_quote(EXTRACTOR_VERSION),
    );
    project_primary_db().query(sql).await?.check()?;
    Ok(())
}

#[derive(Debug, Deserialize, SurrealValue)]
struct CountRow {
    count: i64,
}

/// 该 dbnum 索引里的全部出边行数（含可能的孤儿 source，用于 audit 孤儿检测）。
///
/// 用规范 `SELECT count() … GROUP ALL`（返回 `[{count}]`），而非 `SELECT VALUE count()`
/// 的标量投影——后者在 fork/标准引擎间行为不一（标准引擎仍返回 `[{count}]`），
/// 用带字段的结构体反序列化两侧都稳。
pub async fn count_index_rows(dbnum: u32) -> anyhow::Result<usize> {
    let sql =
        format!("SELECT count() FROM cata_ref_index WHERE source_dbnum = {dbnum} GROUP ALL;");
    let rows: Vec<CountRow> = project_primary_db().query_take(&sql, 0).await?;
    Ok(rows
        .into_iter()
        .next()
        .map(|row| row.count.max(0) as usize)
        .unwrap_or(0))
}

// ──────────────────────────────────────────────────────────────────────────
// 顺序无关内容摘要（backfill 写入 / audit 对账复现）
// ──────────────────────────────────────────────────────────────────────────

/// XOR-fold 累加器：对边集做顺序无关的稳定摘要，backfill 与 audit 两侧独立复算一致。
#[derive(Debug, Default, Clone)]
pub struct EdgeDigest {
    acc: [u8; 32],
    count: usize,
}

impl EdgeDigest {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn absorb(&mut self, edge: &RefEdge) {
        let digest = edge.digest();
        for (slot, byte) in self.acc.iter_mut().zip(digest.iter()) {
            *slot ^= *byte;
        }
        self.count += 1;
    }

    pub fn absorb_all<'a>(&mut self, edges: impl IntoIterator<Item = &'a RefEdge>) {
        for edge in edges {
            self.absorb(edge);
        }
    }

    pub fn count(&self) -> usize {
        self.count
    }

    pub fn checksum(&self) -> String {
        hex::encode(self.acc)
    }
}

/// 同一 source 的边集是否内容一致（audit 差分用；忽略行内顺序）。
pub fn edges_equal_ignoring_order(left: &[RefEdge], right: &[RefEdge]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut left_keys = left.iter().map(RefEdge::canonical).collect::<Vec<_>>();
    let mut right_keys = right.iter().map(RefEdge::canonical).collect::<Vec<_>>();
    left_keys.sort();
    right_keys.sort();
    left_keys == right_keys
}
