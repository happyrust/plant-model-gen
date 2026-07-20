//! specs/023 M2/T6（D2 决策）：per-run 内存层级快照——生成/导出管线的 pe_owner 数据源。
//!
//! 与 `.tree`（TreeIndex）的本质差别：
//! - **数据永远来自 SurrealDB pe 表**（`children` 字段序 = 同胞序，增量提交后即最新）；
//! - 进程内按 dbnum 缓存一次加载，但**每次生成 run 开始必须显式失效**
//!   （`invalidate_pe_snapshots()`，由 `gen_all_geos_data` 入口调用）——
//!   这是本迁移要修的 §0-2/§0-3 缺陷（TreeIndex 永不失效 → 增量后静默漏元素），
//!   不允许再造一个永不失效的缓存。
//!
//! 查询语义逐条对齐 rs-core `tree_query::TreeIndex`（BFS 输出过滤 / prune_on_match /
//! include_self / max_depth、ancestors root→parent、children 过滤），保证消费面切换后
//! 结果 diff=0；额外加 visited 防环（`pe.children` 是数据字段，理论上可能脏成环，
//! arena 结构性无环的保证在这里不存在）。

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use aios_core::tool::db_tool::db1_hash;
use aios_core::tree_query::{TreeNodeMeta, TreeQueryFilter, TreeQueryOptions, is_geo_noun_hash};
use aios_core::{RefU64, RefnoEnum, SurrealQueryExt, project_primary_db};
use dashmap::DashMap;
use once_cell::sync::Lazy;
use serde::Deserialize;
use surrealdb::types::SurrealValue;

/// 读分页大小（cursor 分页，非写 chunk 口径）；可用 `AIOS_PE_SNAPSHOT_PAGE_SIZE` 覆盖。
fn snapshot_page_size() -> usize {
    std::env::var("AIOS_PE_SNAPSHOT_PAGE_SIZE")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|v| *v >= 100)
        .unwrap_or(2000)
}

#[derive(Debug, Clone)]
pub struct PeSnapshotNode {
    pub owner: RefU64,
    pub noun_hash: u32,
    pub cata_hash: Option<u64>,
    /// pe.children 字段原序（同胞顺序权威来源）
    pub children: Vec<RefU64>,
}

/// 单 dbnum 的内存层级快照（加载后只读，全部查询为纯内存同步操作）。
pub struct PeDbnumSnapshot {
    dbnum: u32,
    nodes: HashMap<RefU64, PeSnapshotNode>,
    roots: Vec<RefU64>,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct PeSnapshotRow {
    id: RefnoEnum,
    #[serde(default)]
    owner: Option<RefnoEnum>,
    #[serde(default)]
    noun: Option<String>,
    /// string 存储（u64 哈希可能超出 Surreal int/i64 范围，见 M0/T2）
    #[serde(default)]
    cata_hash: Option<String>,
    #[serde(default)]
    children: Option<Vec<RefnoEnum>>,
}

fn filter_matches(filter: &TreeQueryFilter, meta: &TreeNodeMeta, has_geo: bool, is_leaf: bool) -> bool {
    if let Some(f) = filter.has_geo {
        if has_geo != f {
            return false;
        }
    }
    if let Some(f) = filter.is_leaf {
        if is_leaf != f {
            return false;
        }
    }
    if let Some(hashes) = &filter.noun_hashes {
        if !hashes.contains(&meta.noun) {
            return false;
        }
    }
    true
}

impl PeDbnumSnapshot {
    pub fn dbnum(&self) -> u32 {
        self.dbnum
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn contains_refno(&self, refno: RefU64) -> bool {
        self.nodes.contains_key(&refno)
    }

    pub fn all_refnos(&self) -> Vec<RefU64> {
        self.nodes.keys().copied().collect()
    }

    pub fn roots(&self) -> &[RefU64] {
        &self.roots
    }

    fn node(&self, refno: RefU64) -> Option<&PeSnapshotNode> {
        self.nodes.get(&refno)
    }

    fn is_leaf(&self, refno: RefU64) -> bool {
        self.node(refno)
            .map(|n| n.children.is_empty())
            .unwrap_or(true)
    }

    /// 与 `TreeIndex::node_meta` 同构（noun 为 db1_hash）。
    pub fn node_meta(&self, refno: RefU64) -> Option<TreeNodeMeta> {
        self.node(refno).map(|n| TreeNodeMeta {
            refno,
            owner: n.owner,
            noun: n.noun_hash,
            cata_hash: n.cata_hash,
        })
    }

    /// 直接子节点（pe.children 字段序），带 filter——对齐 `TreeIndex::collect_children`。
    pub fn collect_children(&self, parent: RefU64, filter: &TreeQueryFilter) -> Vec<RefU64> {
        let Some(node) = self.node(parent) else {
            return Vec::new();
        };
        node.children
            .iter()
            .filter_map(|&child| {
                let meta = self.node_meta(child)?;
                let has_geo = is_geo_noun_hash(meta.noun);
                let is_leaf = self.is_leaf(child);
                filter_matches(filter, &meta, has_geo, is_leaf).then_some(child)
            })
            .collect()
    }

    /// BFS 收集子孙——语义逐行对齐 `TreeIndex::collect_descendants_bfs`
    /// （输出过滤不剪枝、prune_on_match 匹配后不展开、root 受 include_self 控制、
    /// max_depth 为展开深度上限），另加 visited 防脏数据环。
    pub fn collect_descendants_bfs(&self, root: RefU64, options: &TreeQueryOptions) -> Vec<RefU64> {
        let mut out = Vec::new();
        self.bfs_inner(root, options, |refno, _noun| out.push(refno));
        out
    }

    /// BFS 收集并按 noun_hash 分组——对齐 `TreeIndex::collect_descendants_bfs_grouped`。
    pub fn collect_descendants_bfs_grouped(
        &self,
        root: RefU64,
        options: &TreeQueryOptions,
    ) -> HashMap<u32, Vec<RefU64>> {
        let mut out: HashMap<u32, Vec<RefU64>> = HashMap::new();
        self.bfs_inner(root, options, |refno, noun| {
            out.entry(noun).or_default().push(refno)
        });
        out
    }

    fn bfs_inner(&self, root: RefU64, options: &TreeQueryOptions, mut emit: impl FnMut(RefU64, u32)) {
        if !self.nodes.contains_key(&root) {
            return;
        }
        let mut visited: HashSet<RefU64> = HashSet::new();
        visited.insert(root);
        let mut queue: VecDeque<(RefU64, usize)> = VecDeque::new();
        queue.push_back((root, 0));
        while let Some((refno, depth)) = queue.pop_front() {
            let Some(node) = self.node(refno) else {
                continue;
            };
            let meta = TreeNodeMeta {
                refno,
                owner: node.owner,
                noun: node.noun_hash,
                cata_hash: node.cata_hash,
            };
            let is_root = depth == 0;
            let has_geo = is_geo_noun_hash(node.noun_hash);
            let is_leaf = node.children.is_empty();
            let matched = !(is_root && !options.include_self)
                && filter_matches(&options.filter, &meta, has_geo, is_leaf);
            if matched {
                emit(refno, node.noun_hash);
            }
            if options.prune_on_match && matched && !is_root {
                continue;
            }
            if let Some(max_depth) = options.max_depth {
                if depth >= max_depth {
                    continue;
                }
            }
            for &child in &node.children {
                if visited.insert(child) {
                    queue.push_back((child, depth + 1));
                }
            }
        }
    }

    /// 祖先链（根→父）——对齐 `TreeIndex::collect_ancestors_root_to_parent`。
    pub fn collect_ancestors_root_to_parent(
        &self,
        node: RefU64,
        options: &TreeQueryOptions,
    ) -> Vec<RefU64> {
        let mut chain: Vec<RefU64> = Vec::new();
        let mut current = node;
        let mut depth = 0usize;
        let mut visited = HashSet::new();
        loop {
            if !visited.insert(current) {
                break;
            }
            if !(current == node && !options.include_self) {
                if let Some(meta) = self.node_meta(current) {
                    let is_leaf = self.is_leaf(current);
                    let has_geo = is_geo_noun_hash(meta.noun);
                    if filter_matches(&options.filter, &meta, has_geo, is_leaf) {
                        chain.push(current);
                    }
                    current = meta.owner;
                } else {
                    break;
                }
            } else if let Some(meta) = self.node_meta(current) {
                current = meta.owner;
            } else {
                break;
            }
            depth += 1;
            if let Some(max_depth) = options.max_depth {
                if depth >= max_depth {
                    break;
                }
            }
            if current.0 == 0 {
                break;
            }
        }
        chain.reverse();
        chain
    }

    /// 按 noun_hash 枚举（存储序不稳定，调用侧需要稳定序时自行排序——与
    /// TreeIndexManager::query_noun_refnos 的 arena 序同为"实现细节序"）。
    pub fn noun_refnos(&self, noun_hash: u32) -> Vec<RefU64> {
        self.nodes
            .iter()
            .filter_map(|(&r, n)| (n.noun_hash == noun_hash).then_some(r))
            .collect()
    }

    pub fn count_by_noun_hash(&self) -> HashMap<u32, usize> {
        let mut out: HashMap<u32, usize> = HashMap::new();
        for node in self.nodes.values() {
            *out.entry(node.noun_hash).or_default() += 1;
        }
        out
    }
}

// ============================================================================
// 进程内快照缓存（run 级失效）
// ============================================================================

static SNAPSHOT_CELLS: Lazy<DashMap<u32, Arc<tokio::sync::OnceCell<Arc<PeDbnumSnapshot>>>>> =
    Lazy::new(DashMap::new);

/// 加载（或取缓存）单 dbnum 快照。并发调用只加载一次（OnceCell 收敛）。
pub async fn get_or_load_pe_snapshot(dbnum: u32) -> anyhow::Result<Arc<PeDbnumSnapshot>> {
    let cell = SNAPSHOT_CELLS
        .entry(dbnum)
        .or_insert_with(|| Arc::new(tokio::sync::OnceCell::new()))
        .clone();
    let snap = cell
        .get_or_try_init(|| async { load_snapshot_from_db(dbnum).await.map(Arc::new) })
        .await?;
    Ok(snap.clone())
}

/// 同步取已加载快照（供 sync 闭包/循环使用；调用前须先 preload）。
pub fn try_get_cached_pe_snapshot(dbnum: u32) -> Option<Arc<PeDbnumSnapshot>> {
    SNAPSHOT_CELLS
        .get(&dbnum)
        .and_then(|cell| cell.get().cloned())
}

/// 预加载一组 dbnum 快照（sync 消费点之前调用）。
pub async fn preload_pe_snapshots(dbnums: &[u32]) -> anyhow::Result<()> {
    for &dbnum in dbnums {
        get_or_load_pe_snapshot(dbnum).await?;
    }
    Ok(())
}

/// 失效全部快照。**每次生成 run 开始必须调用**（gen_all_geos_data 入口），
/// 保证增量提交后的新 run 看到最新层级。返回被清除的 dbnum 数。
pub fn invalidate_pe_snapshots() -> usize {
    let n = SNAPSHOT_CELLS.len();
    SNAPSHOT_CELLS.clear();
    n
}

/// 失效单 dbnum 快照。
pub fn invalidate_pe_snapshot(dbnum: u32) {
    SNAPSHOT_CELLS.remove(&dbnum);
}

async fn load_snapshot_from_db(dbnum: u32) -> anyhow::Result<PeDbnumSnapshot> {
    let started = std::time::Instant::now();
    let page = snapshot_page_size();
    let mut nodes: HashMap<RefU64, PeSnapshotNode> = HashMap::new();
    let mut cursor: Option<String> = None;

    loop {
        // cursor 分页（record id 天然有序）；避免 START offset 分页在大表上的 O(N²) 扫描。
        let sql = match &cursor {
            Some(last_key) => format!(
                "SELECT id, owner, noun, cata_hash, children FROM pe \
                 WHERE dbnum = {dbnum} AND id > {last_key} ORDER BY id LIMIT {page};"
            ),
            None => format!(
                "SELECT id, owner, noun, cata_hash, children FROM pe \
                 WHERE dbnum = {dbnum} ORDER BY id LIMIT {page};"
            ),
        };
        let rows: Vec<PeSnapshotRow> = project_primary_db().query_take(&sql, 0).await?;
        let fetched = rows.len();
        if fetched == 0 {
            break;
        }
        cursor = rows.last().map(|r| r.id.to_pe_key());
        for row in rows {
            let refno = row.id.refno();
            let noun = row.noun.unwrap_or_default();
            nodes.insert(
                refno,
                PeSnapshotNode {
                    owner: row.owner.map(|o| o.refno()).unwrap_or(refno),
                    // db1_hash 同时注册 hash→name 反查（db1_dehash 依赖）
                    noun_hash: db1_hash(&noun),
                    cata_hash: row.cata_hash.and_then(|s| s.parse::<u64>().ok()),
                    children: row
                        .children
                        .unwrap_or_default()
                        .into_iter()
                        .map(|c| c.refno())
                        .collect(),
                },
            );
        }
        if fetched < page {
            break;
        }
    }

    let roots: Vec<RefU64> = nodes
        .iter()
        .filter_map(|(&r, n)| (n.owner == r || !nodes.contains_key(&n.owner)).then_some(r))
        .collect();

    log::info!(
        "[pe_snapshot] dbnum={} 加载完成: nodes={} roots={} elapsed_ms={}",
        dbnum,
        nodes.len(),
        roots.len(),
        started.elapsed().as_millis()
    );

    Ok(PeDbnumSnapshot {
        dbnum,
        nodes,
        roots,
    })
}
