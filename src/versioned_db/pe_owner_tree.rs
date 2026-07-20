//! specs/023 M0/T1：latest（不带 sesno）层级查询原语层 `PeOwnerTreeStore`。
//!
//! 目标：替代 `TreeIndexManager` 的 `.tree` 文件路径——层级查询全部改走 SurrealDB 3.1
//! **图遍历 / 递归 idiom**（语法依据 `D:\work\plant-code\surrealdb`（dev-3.1）
//! `language-tests/tests/language/graph|idiom` 实测用例），数据源永远是库内最新态：
//!
//! - children（同胞有序）：`SELECT VALUE in FROM <owner><-pe_owner ORDER BY id;`
//!   （边 id = `pe_owner:[<owner>, <order>]`，`ORDER BY id` 保同胞顺序，specs/023 契约）
//! - descendants（子孙收集）：`<root>.{..N+collect}<-pe_owner<-pe`
//!   （递归 idiom，BFS 邻近序 + visited 去重防环；引擎递归上限 256，`recursion_limits` 实测）
//! - ancestors（祖先链）：`<node>.{..N+collect}->pe_owner->pe`，边缺失回退
//!   `<node>.{..N+collect}(.owner)`（owner 记录链接递归，不依赖边完整性）
//! - 批量子节点（BFS/剪枝用，无序）：`SELECT VALUE {{ p: id, kids: <-pe_owner<-pe, ch: children }} FROM [...]`
//!
//! 铁律：
//! - **禁止 `pe_owner:[..]..[..]` id 区间扫**（specs/023 research C3：VERSION 下静默返回当前态，
//!   latest 同样统一图遍历，不开这个口子）；
//! - **禁止 WHERE 全表扫做层级查询**（noun 枚举/计数是表级统计，不属层级查询，见文件尾注释）；
//! - 边缺失（存量站点未重灌/未 rebuild-pe-owner）一律回退 `pe.children` 字段点查，
//!   与版本路径 FR-008 双源结构同构。
//!
//! **前置条件（D5）**：递归主路径（`query_descendants`）在"部分节点有边、部分没有"的
//! 混合态下会在缺边节点处静默截断子树——回退判定只看根级结果是否为空。因此站点切换
//! 到本原语前必须通过 `scripts/smoke/pe_owner_children_audit.ps1` 审计（不绿先
//! `model-version rebuild-pe-owner`）；逐层 BFS 类接口（`children_batch` /
//! `collect_target_refnos_*`）为每节点独立回退，不受此限。
//!
//! 本模块 M0 阶段纯新增，不改变任何现有调用路径；M1/M2 起逐域替换 `TreeIndexManager` 消费面。

use std::collections::{HashMap, HashSet};

use aios_core::tool::db_tool::{db1_dehash, db1_hash};
use aios_core::{RefnoEnum, SurrealQueryExt, project_primary_db};
use serde::Deserialize;
use surrealdb::types::SurrealValue;

/// 引擎单次递归 idiom 的深度硬上限（surrealdb dev-3.1 `recursion_limits` 实测：
/// 显式上界 ≤256 且超深时**截断**；无上界 `{..}` 超深会直接报错——因此一律显式带上界）。
pub const MAX_RECURSE_DEPTH: usize = 256;

/// M1 双源开关（计划 M4 删除）：latest 树查询数据源选择。
///
/// `AIOS_TREE_QUERY_SOURCE=pe_owner`（默认，含未设置/未知值）| `tree`（一键回退旧
/// TreeIndex `.tree` 文件路径）。只影响 latest（不带 sesno）层级查询；versioned
/// 分支（specs/023）与 `resolve_dbnum_for_refno`（db_meta 驱动）不受此开关控制。
pub fn latest_tree_source_is_pe_owner() -> bool {
    match std::env::var("AIOS_TREE_QUERY_SOURCE") {
        Ok(v) => !v.trim().eq_ignore_ascii_case("tree"),
        Err(_) => true,
    }
}

/// 批量点查/展开的分片大小（对齐 sesno_increment `exec_statements` 粒度）。
const CHUNK: usize = 500;

/// 单节点层级元信息（latest 态；对齐 `TreeIndex::node_meta` 的消费面）。
#[derive(Debug, Clone)]
pub struct PeNodeMeta {
    pub refno: RefnoEnum,
    pub owner: Option<RefnoEnum>,
    /// noun 名称（pe.noun 字段原值，如 "BRAN"）
    pub noun: String,
    /// `db1_hash(noun)`，对齐 TreeIndex 消费面的 hash 分组语义
    pub noun_hash: u32,
    /// pe.cata_hash 字段（M0/T2 落地后有值；缺失 = None，消费侧回退 attmap 计算）
    pub cata_hash: Option<u64>,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct PeMetaRow {
    id: RefnoEnum,
    #[serde(default)]
    noun: Option<String>,
    #[serde(default)]
    owner: Option<RefnoEnum>,
    /// cata_hash 以 string 存储（u64 哈希可能超出 Surreal int/i64 范围）
    #[serde(default)]
    cata_hash: Option<String>,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct KidsRow {
    p: RefnoEnum,
    /// 边路径子节点（无序，仅用于集合语义的 BFS 展开）
    #[serde(default)]
    kids: Vec<RefnoEnum>,
    /// pe.children 字段回退
    #[serde(default)]
    ch: Option<Vec<RefnoEnum>>,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct CountRow {
    p: RefnoEnum,
    #[serde(default)]
    n: Option<i64>,
    #[serde(default)]
    ch: Option<Vec<RefnoEnum>>,
}

/// pe_owner 图查询原语层。
///
/// 层级查询（children/ancestors/descendants/…）是纯图操作，不需要 dbnum；
/// noun 枚举/计数等表级统计按构造时传入的 dbnums 收敛范围（依赖 D3 `idx_pe_dbnum_noun` 索引）。
pub struct PeOwnerTreeStore {
    dbnums: Vec<u32>,
}

impl PeOwnerTreeStore {
    pub fn new(dbnums: Vec<u32>) -> Self {
        Self { dbnums }
    }

    pub fn dbnums(&self) -> &[u32] {
        &self.dbnums
    }

    // ========================================================================
    // children（同胞有序）
    // ========================================================================

    /// 查询直接子节点（同胞顺序 = 边 id `[owner, order]` 升序）。
    ///
    /// 边缺失回退 `pe.children` 字段（同时天然覆盖"确无子节点"情形）。
    pub async fn query_children(parent: RefnoEnum) -> anyhow::Result<Vec<RefnoEnum>> {
        let parent_key = parent.to_pe_key();
        let sql = format!("SELECT VALUE in FROM {parent_key}<-pe_owner ORDER BY id;");
        let kids: Vec<RefnoEnum> = project_primary_db().query_take(&sql, 0).await?;
        if !kids.is_empty() {
            return Ok(kids);
        }
        Self::children_field_fallback(parent).await
    }

    async fn children_field_fallback(parent: RefnoEnum) -> anyhow::Result<Vec<RefnoEnum>> {
        let sql = format!("SELECT VALUE children FROM {};", parent.to_pe_key());
        let rows: Vec<Option<Vec<RefnoEnum>>> = project_primary_db().query_take(&sql, 0).await?;
        Ok(rows.into_iter().flatten().next().unwrap_or_default())
    }

    /// 查询直接子节点并按 noun 过滤（保持同胞顺序）。
    pub async fn query_children_filtered(
        parent: RefnoEnum,
        nouns: &[&str],
    ) -> anyhow::Result<Vec<RefnoEnum>> {
        let children = Self::query_children(parent).await?;
        if children.is_empty() {
            return Ok(children);
        }
        let wanted: HashSet<u32> = nouns.iter().map(|n| db1_hash(n)).collect();
        let metas = Self::fetch_node_metas(&children).await?;
        Ok(children
            .into_iter()
            .filter(|r| {
                metas
                    .get(r)
                    .map(|m| wanted.contains(&m.noun_hash))
                    .unwrap_or(false)
            })
            .collect())
    }

    /// 批量统计直接子节点数量（children_count 展示用）。
    ///
    /// 边计数优先（`count(<-pe_owner)`），为 0 时回退 `pe.children` 长度。
    pub async fn query_children_counts(
        refnos: &[RefnoEnum],
    ) -> anyhow::Result<HashMap<RefnoEnum, usize>> {
        let mut out = HashMap::with_capacity(refnos.len());
        for chunk in refnos.chunks(CHUNK) {
            let keys = chunk
                .iter()
                .map(|r| r.to_pe_key())
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                "SELECT VALUE {{ p: id, n: count(<-pe_owner), ch: children }} FROM [{keys}];"
            );
            let rows: Vec<CountRow> = project_primary_db().query_take(&sql, 0).await?;
            for row in rows {
                let edge_cnt = row.n.unwrap_or(0).max(0) as usize;
                let cnt = if edge_cnt > 0 {
                    edge_cnt
                } else {
                    row.ch.map(|c| c.len()).unwrap_or(0)
                };
                out.insert(row.p, cnt);
            }
        }
        Ok(out)
    }

    // ========================================================================
    // ancestors（祖先链）
    // ========================================================================

    /// 查询祖先链，返回顺序与 TreeIndex 一致：根→父，不含自身。
    ///
    /// 走 `(.owner)` 记录链接递归 idiom（单条查询）：owner 字段由 PE 写入路径恒维护，
    /// **不依赖 pe_owner 边完整性**，祖先链又是唯一路径——比边遍历更稳。
    /// `+collect` 去重防环（根节点 owner 自指靠 visited 去重终止）。
    pub async fn query_ancestors(node: RefnoEnum) -> anyhow::Result<Vec<RefnoEnum>> {
        let key = node.to_pe_key();
        let sql = format!("RETURN {key}.{{..{MAX_RECURSE_DEPTH}+collect}}(.owner);");
        let mut chain: Vec<RefnoEnum> = project_primary_db().query_take(&sql, 0).await?;
        // 根节点 owner 自指会把自身收进链；剔除后反转为 根→父。
        chain.retain(|r| *r != node);
        chain.reverse();
        Ok(chain)
    }

    /// 祖先链按 noun 过滤（保持根→父顺序）。
    pub async fn query_ancestors_filtered(
        node: RefnoEnum,
        nouns: &[&str],
    ) -> anyhow::Result<Vec<RefnoEnum>> {
        let chain = Self::query_ancestors(node).await?;
        if chain.is_empty() {
            return Ok(chain);
        }
        let wanted: HashSet<u32> = nouns.iter().map(|n| db1_hash(n)).collect();
        let metas = Self::fetch_node_metas(&chain).await?;
        Ok(chain
            .into_iter()
            .filter(|r| {
                metas
                    .get(r)
                    .map(|m| wanted.contains(&m.noun_hash))
                    .unwrap_or(false)
            })
            .collect())
    }

    // ========================================================================
    // descendants（子孙收集）
    // ========================================================================

    /// 查询全部子孙（不含自身），BFS 邻近序。
    ///
    /// 主路径：单条递归 idiom `<root>.{..N+collect}<-pe_owner<-pe`；
    /// 边缺失回退：`pe.children` 字段逐层 BFS 批查（chunk 500）。
    pub async fn query_descendants(
        root: RefnoEnum,
        max_depth: Option<usize>,
    ) -> anyhow::Result<Vec<RefnoEnum>> {
        let depth = max_depth
            .unwrap_or(MAX_RECURSE_DEPTH)
            .clamp(1, MAX_RECURSE_DEPTH);
        let root_key = root.to_pe_key();
        let sql = format!("RETURN {root_key}.{{..{depth}+collect}}<-pe_owner<-pe;");
        let via_edges: Vec<RefnoEnum> = project_primary_db()
            .query_take(&sql, 0)
            .await
            .unwrap_or_default();
        if !via_edges.is_empty() {
            return Ok(via_edges);
        }
        // 区分"确无子孙"与"边缺失"：children 字段非空才进入回退 BFS。
        if Self::children_field_fallback(root).await?.is_empty() {
            return Ok(Vec::new());
        }
        Self::descendants_via_children_field(&[root], Some(depth)).await
    }

    /// 查询子孙并按 noun 过滤（不含自身）。
    ///
    /// 注意：**过滤发生在收集之后**（先图收集再按 meta 过滤），不能用中途过滤
    /// `<-pe_owner<-(pe WHERE ...)`——那是剪枝语义，会在中间层 noun 不匹配时误断整条链。
    pub async fn query_descendants_filtered(
        root: RefnoEnum,
        nouns: &[&str],
        max_depth: Option<usize>,
    ) -> anyhow::Result<Vec<RefnoEnum>> {
        let all = Self::query_descendants(root, max_depth).await?;
        Self::filter_by_nouns(all, nouns).await
    }

    /// 批量多根子孙收集 + noun 过滤（跨根去重，保持发现顺序）。
    pub async fn query_multi_descendants_filtered(
        roots: &[RefnoEnum],
        nouns: &[&str],
    ) -> anyhow::Result<Vec<RefnoEnum>> {
        let mut seen: HashSet<RefnoEnum> = HashSet::new();
        let mut all: Vec<RefnoEnum> = Vec::new();
        for &root in roots {
            for r in Self::query_descendants(root, None).await? {
                if seen.insert(r) {
                    all.push(r);
                }
            }
        }
        Self::filter_by_nouns(all, nouns).await
    }

    async fn filter_by_nouns(
        refnos: Vec<RefnoEnum>,
        nouns: &[&str],
    ) -> anyhow::Result<Vec<RefnoEnum>> {
        if refnos.is_empty() || nouns.is_empty() {
            return Ok(refnos);
        }
        let wanted: HashSet<u32> = nouns.iter().map(|n| db1_hash(n)).collect();
        let metas = Self::fetch_node_metas(&refnos).await?;
        Ok(refnos
            .into_iter()
            .filter(|r| {
                metas
                    .get(r)
                    .map(|m| wanted.contains(&m.noun_hash))
                    .unwrap_or(false)
            })
            .collect())
    }

    /// `pe.children` 字段逐层 BFS（边缺失回退路径；集合语义、跨层去重）。
    async fn descendants_via_children_field(
        roots: &[RefnoEnum],
        max_depth: Option<usize>,
    ) -> anyhow::Result<Vec<RefnoEnum>> {
        let depth_cap = max_depth.unwrap_or(MAX_RECURSE_DEPTH);
        let mut visited: HashSet<RefnoEnum> = roots.iter().copied().collect();
        let mut frontier: Vec<RefnoEnum> = roots.to_vec();
        let mut out: Vec<RefnoEnum> = Vec::new();
        let mut level = 0usize;
        while !frontier.is_empty() && level < depth_cap {
            level += 1;
            let kids_map = Self::children_batch(&frontier).await?;
            let mut next: Vec<RefnoEnum> = Vec::new();
            for parent in &frontier {
                if let Some(kids) = kids_map.get(parent) {
                    for &kid in kids {
                        if visited.insert(kid) {
                            out.push(kid);
                            next.push(kid);
                        }
                    }
                }
            }
            frontier = next;
        }
        Ok(out)
    }

    /// 批量取直接子节点（无序集合语义）：边优先、`pe.children` 字段回退，chunk 500。
    pub async fn children_batch(
        parents: &[RefnoEnum],
    ) -> anyhow::Result<HashMap<RefnoEnum, Vec<RefnoEnum>>> {
        let mut out: HashMap<RefnoEnum, Vec<RefnoEnum>> = HashMap::with_capacity(parents.len());
        for chunk in parents.chunks(CHUNK) {
            let keys = chunk
                .iter()
                .map(|r| r.to_pe_key())
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                "SELECT VALUE {{ p: id, kids: <-pe_owner<-pe, ch: children }} FROM [{keys}];"
            );
            let rows: Vec<KidsRow> = project_primary_db().query_take(&sql, 0).await?;
            for row in rows {
                let kids = if !row.kids.is_empty() {
                    row.kids
                } else {
                    row.ch.unwrap_or_default()
                };
                out.insert(row.p, kids);
            }
        }
        Ok(out)
    }

    // ========================================================================
    // 目标收集（剪枝 / 分组）——生成管线入口查询
    // ========================================================================

    /// 批量 BFS 收集目标 noun refnos，匹配后剪枝（不再深入其子树）。
    ///
    /// 与 `TreeIndexManager::collect_target_refnos_pruned` 语义一致（include_self=true）。
    /// 剪枝无法用单条递归 idiom 表达（中途过滤是"断链"语义），因此在 Rust 侧逐层
    /// BFS + 批量 meta 判定，展开走 `children_batch`（边优先/字段回退）。
    pub async fn collect_target_refnos_pruned(
        roots: &[RefnoEnum],
        nouns: &[&str],
    ) -> anyhow::Result<Vec<RefnoEnum>> {
        let grouped = Self::collect_target_refnos_grouped(roots, nouns, true).await?;
        let mut out = Vec::new();
        for (_, refnos) in grouped {
            out.extend(refnos);
        }
        Ok(out)
    }

    /// 批量 BFS 收集目标 noun refnos 并按 noun_hash 分组（include_self=true）。
    ///
    /// `prune=true`：命中目标后不再展开其子树。
    pub async fn collect_target_refnos_grouped(
        roots: &[RefnoEnum],
        nouns: &[&str],
        prune: bool,
    ) -> anyhow::Result<HashMap<u32, Vec<RefnoEnum>>> {
        let wanted: HashSet<u32> = nouns.iter().map(|n| db1_hash(n)).collect();
        let mut grouped: HashMap<u32, Vec<RefnoEnum>> = HashMap::new();
        let mut visited: HashSet<RefnoEnum> = HashSet::new();
        let mut frontier: Vec<RefnoEnum> = Vec::new();
        for &root in roots {
            if visited.insert(root) {
                frontier.push(root);
            }
        }
        let mut level = 0usize;
        while !frontier.is_empty() && level <= MAX_RECURSE_DEPTH {
            level += 1;
            let metas = Self::fetch_node_metas(&frontier).await?;
            let mut expand: Vec<RefnoEnum> = Vec::new();
            for &node in &frontier {
                let matched = metas
                    .get(&node)
                    .map(|m| wanted.contains(&m.noun_hash))
                    .unwrap_or(false);
                if matched {
                    let hash = metas.get(&node).map(|m| m.noun_hash).unwrap_or_default();
                    grouped.entry(hash).or_default().push(node);
                    if prune {
                        continue;
                    }
                }
                expand.push(node);
            }
            if expand.is_empty() {
                break;
            }
            let kids_map = Self::children_batch(&expand).await?;
            let mut next: Vec<RefnoEnum> = Vec::new();
            for parent in &expand {
                if let Some(kids) = kids_map.get(parent) {
                    for &kid in kids {
                        if visited.insert(kid) {
                            next.push(kid);
                        }
                    }
                }
            }
            frontier = next;
        }
        Ok(grouped)
    }

    // ========================================================================
    // 节点元信息
    // ========================================================================

    /// 批量点查节点元信息（chunk 500）。
    pub async fn fetch_node_metas(
        refnos: &[RefnoEnum],
    ) -> anyhow::Result<HashMap<RefnoEnum, PeNodeMeta>> {
        let mut out = HashMap::with_capacity(refnos.len());
        for chunk in refnos.chunks(CHUNK) {
            let keys = chunk
                .iter()
                .map(|r| r.to_pe_key())
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!("SELECT id, noun, owner, cata_hash FROM [{keys}];");
            let rows: Vec<PeMetaRow> = project_primary_db().query_take(&sql, 0).await?;
            for row in rows {
                let noun = row.noun.unwrap_or_default();
                let noun_hash = db1_hash(&noun);
                out.insert(
                    row.id,
                    PeNodeMeta {
                        refno: row.id,
                        owner: row.owner,
                        noun,
                        noun_hash,
                        cata_hash: row.cata_hash.and_then(|s| s.parse::<u64>().ok()),
                    },
                );
            }
        }
        Ok(out)
    }

    /// 单节点元信息。
    pub async fn get_node_meta(refno: RefnoEnum) -> anyhow::Result<Option<PeNodeMeta>> {
        Ok(Self::fetch_node_metas(&[refno]).await?.remove(&refno))
    }

    /// 节点 noun 名称。
    pub async fn get_noun(refno: RefnoEnum) -> anyhow::Result<Option<String>> {
        Ok(Self::get_node_meta(refno).await?.map(|m| m.noun))
    }

    /// 节点是否存在（pe 行点查）。
    pub async fn contains(refno: RefnoEnum) -> anyhow::Result<bool> {
        let sql = format!("SELECT VALUE id FROM {};", refno.to_pe_key());
        let rows: Vec<RefnoEnum> = project_primary_db().query_take(&sql, 0).await?;
        Ok(!rows.is_empty())
    }

    // ========================================================================
    // 表级统计（非层级查询）——依赖 D3 `idx_pe_dbnum_noun` 索引
    // ========================================================================
    //
    // noun 枚举/计数/全量 refno 列表不是图操作（没有遍历起点），仍需 dbnum+noun
    // 维度的表级访问。D3 决策：`DEFINE INDEX idx_pe_dbnum_noun ON TABLE pe FIELDS dbnum, noun;`
    // 性能不达标时按计划降级为 per-run 快照统计（D2）。

    /// 幂等定义 pe 表 (dbnum, noun) 二级索引（D3）。
    ///
    /// 注意：存量大表上建索引有一次性构建成本，调用方（CLI/运维脚本）自行择机执行，
    /// 本模块任何查询都不隐式触发。
    pub async fn ensure_pe_dbnum_noun_index() -> anyhow::Result<()> {
        project_primary_db()
            .query("DEFINE INDEX IF NOT EXISTS idx_pe_dbnum_noun ON TABLE pe FIELDS dbnum, noun;")
            .await?
            .check()?;
        Ok(())
    }

    /// 按 noun 枚举 refnos（范围 = 构造时 dbnums；dbnums 为空 = 不限库）。
    pub async fn query_noun_refnos(
        &self,
        noun: &str,
        limit: Option<usize>,
    ) -> anyhow::Result<Vec<RefnoEnum>> {
        let mut out = Vec::new();
        let noun_escaped = noun.replace('\'', "\\'");
        let scopes: Vec<Option<u32>> = if self.dbnums.is_empty() {
            vec![None]
        } else {
            self.dbnums.iter().map(|d| Some(*d)).collect()
        };
        for scope in scopes {
            if let Some(l) = limit {
                if out.len() >= l {
                    break;
                }
            }
            let where_clause = match scope {
                Some(dbnum) => format!("WHERE dbnum = {dbnum} AND noun = '{noun_escaped}'"),
                None => format!("WHERE noun = '{noun_escaped}'"),
            };
            let limit_clause = limit
                .map(|l| format!(" LIMIT {}", (l - out.len()).max(1)))
                .unwrap_or_default();
            let sql = format!("SELECT VALUE id FROM pe {where_clause}{limit_clause};");
            let rows: Vec<RefnoEnum> = project_primary_db().query_take(&sql, 0).await?;
            out.extend(rows);
        }
        if let Some(l) = limit {
            out.truncate(l);
        }
        Ok(out)
    }

    /// 按 noun 统计数量（GROUP BY noun；范围 = 构造时 dbnums）。
    pub async fn count_by_noun(&self) -> anyhow::Result<HashMap<String, usize>> {
        #[derive(Debug, Deserialize, SurrealValue)]
        struct NounCountRow {
            noun: Option<String>,
            count: i64,
        }
        let mut out: HashMap<String, usize> = HashMap::new();
        let scopes: Vec<Option<u32>> = if self.dbnums.is_empty() {
            vec![None]
        } else {
            self.dbnums.iter().map(|d| Some(*d)).collect()
        };
        for scope in scopes {
            let where_clause = scope
                .map(|dbnum| format!("WHERE dbnum = {dbnum} "))
                .unwrap_or_default();
            let sql = format!("SELECT noun, count() AS count FROM pe {where_clause}GROUP BY noun;");
            let rows: Vec<NounCountRow> = project_primary_db().query_take(&sql, 0).await?;
            for row in rows {
                *out.entry(row.noun.unwrap_or_default()).or_default() += row.count.max(0) as usize;
            }
        }
        Ok(out)
    }

    /// noun_hash 分组版本（对齐 TreeIndex 消费面）。
    pub async fn count_by_noun_hash(&self) -> anyhow::Result<HashMap<u32, usize>> {
        Ok(self
            .count_by_noun()
            .await?
            .into_iter()
            .map(|(noun, cnt)| (db1_hash(&noun), cnt))
            .collect())
    }

    /// 可见几何 noun 的全部 refnos（Parquet 导出等消费面）。
    pub async fn query_visible_geo_refnos(&self) -> anyhow::Result<Vec<RefnoEnum>> {
        use aios_core::pdms_types::VISBILE_GEO_NOUNS;
        let mut out = Vec::new();
        for noun in VISBILE_GEO_NOUNS.iter() {
            out.extend(self.query_noun_refnos(noun, None).await?);
        }
        Ok(out)
    }
}

/// noun_hash → noun 名称（分组结果转译用，与 TreeIndex 消费面同源）。
pub fn noun_hash_to_name(hash: u32) -> String {
    db1_dehash(hash)
}
