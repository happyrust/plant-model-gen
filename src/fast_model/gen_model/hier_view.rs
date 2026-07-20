//! specs/023 M2：生成/导出管线的层级视图（双源桥接层，M4 删除 Tree 分支后收敛为快照）。
//!
//! `HierView::load(dbnums)` 按 `AIOS_TREE_QUERY_SOURCE` 决定数据源：
//! - `pe_owner`（默认）：预加载 per-dbnum 内存快照（`versioned_db::pe_owner_snapshot`，
//!   数据来自 pe 表，run 级失效）；
//! - `tree`：旧 `.tree` 文件路径（`TreeIndexManager`），一键回退。
//!
//! 加载完成后全部查询为**同步**操作，方法面向 `TreeIndexManager` 现有签名对齐（drop-in），
//! 消费面只需把 `TreeIndexManager::with_default_dir(dbnums)` 换成 `HierView::load(dbnums).await?`。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use aios_core::tool::db_tool::{db1_dehash, db1_hash};
use aios_core::tree_query::{TreeNodeMeta, TreeQueryFilter, TreeQueryOptions};
use aios_core::{RefU64, RefnoEnum};

use crate::fast_model::gen_model::tree_index_manager::TreeIndexManager;
use crate::versioned_db::pe_owner_snapshot::{PeDbnumSnapshot, get_or_load_pe_snapshot};
use crate::versioned_db::pe_owner_tree::latest_tree_source_is_pe_owner;

pub enum HierView {
    Snapshot {
        by_dbnum: HashMap<u32, Arc<PeDbnumSnapshot>>,
    },
    Tree(TreeIndexManager),
}

impl HierView {
    /// 按开关加载层级视图。dbnums 为空时：snapshot 分支从 db_meta 取全部 dbnum；
    /// tree 分支与 `TreeIndexManager::with_default_dir(vec![])` 行为一致。
    pub async fn load(dbnums: Vec<u32>) -> anyhow::Result<Self> {
        if !latest_tree_source_is_pe_owner() {
            return Ok(Self::Tree(TreeIndexManager::with_default_dir(dbnums)));
        }
        let dbnums = if dbnums.is_empty() {
            resolve_all_dbnums()?
        } else {
            dbnums
        };
        let mut by_dbnum = HashMap::with_capacity(dbnums.len());
        for dbnum in dbnums {
            let snap = get_or_load_pe_snapshot(dbnum).await?;
            by_dbnum.insert(dbnum, snap);
        }
        Ok(Self::Snapshot { by_dbnum })
    }

    /// 单 refno 场景的便捷入口（自动解析 dbnum）。
    pub async fn load_for_refno(refno: RefnoEnum) -> anyhow::Result<Self> {
        let dbnum = TreeIndexManager::resolve_dbnum_for_refno(refno)?;
        Self::load(vec![dbnum]).await
    }

    pub fn is_snapshot(&self) -> bool {
        matches!(self, Self::Snapshot { .. })
    }

    fn snapshot_for_refno(&self, refno: RefU64) -> Option<&Arc<PeDbnumSnapshot>> {
        let Self::Snapshot { by_dbnum } = self else {
            return None;
        };
        if let Ok(dbnum) = TreeIndexManager::resolve_dbnum_for_refno(RefnoEnum::from(refno)) {
            if let Some(snap) = by_dbnum.get(&dbnum) {
                if snap.contains_refno(refno) {
                    return Some(snap);
                }
            }
        }
        by_dbnum.values().find(|s| s.contains_refno(refno))
    }

    fn snapshots(&self) -> Vec<&Arc<PeDbnumSnapshot>> {
        match self {
            Self::Snapshot { by_dbnum } => by_dbnum.values().collect(),
            Self::Tree(_) => Vec::new(),
        }
    }

    // ========================================================================
    // 节点元信息
    // ========================================================================

    pub fn get_node_meta(&self, refno: RefnoEnum) -> Option<TreeNodeMeta> {
        match self {
            Self::Snapshot { .. } => self
                .snapshot_for_refno(refno.refno())
                .and_then(|s| s.node_meta(refno.refno())),
            Self::Tree(manager) => manager.get_node_meta(refno),
        }
    }

    pub fn get_noun(&self, refno: RefnoEnum) -> Option<String> {
        self.get_node_meta(refno).map(|m| db1_dehash(m.noun))
    }

    pub fn contains(&self, refno: RefnoEnum) -> bool {
        match self {
            Self::Snapshot { .. } => self.snapshot_for_refno(refno.refno()).is_some(),
            Self::Tree(manager) => manager.contains(refno),
        }
    }

    // ========================================================================
    // 层级查询（对齐 TreeIndexManager 签名）
    // ========================================================================

    pub fn query_children(&self, parent: RefnoEnum) -> Vec<RefnoEnum> {
        match self {
            Self::Snapshot { .. } => self
                .snapshot_for_refno(parent.refno())
                .map(|s| {
                    let options = TreeQueryOptions {
                        include_self: false,
                        max_depth: Some(1),
                        filter: TreeQueryFilter::default(),
                        prune_on_match: false,
                    };
                    s.collect_descendants_bfs(parent.refno(), &options)
                        .into_iter()
                        .map(RefnoEnum::from)
                        .collect()
                })
                .unwrap_or_default(),
            Self::Tree(manager) => manager.query_children(parent),
        }
    }

    pub fn query_children_filtered(&self, parent: RefnoEnum, nouns: &[&str]) -> Vec<RefnoEnum> {
        match self {
            Self::Snapshot { .. } => self
                .snapshot_for_refno(parent.refno())
                .map(|s| {
                    let filter = TreeQueryFilter {
                        noun_hashes: Some(nouns.iter().map(|n| db1_hash(n)).collect()),
                        ..Default::default()
                    };
                    s.collect_children(parent.refno(), &filter)
                        .into_iter()
                        .map(RefnoEnum::from)
                        .collect()
                })
                .unwrap_or_default(),
            Self::Tree(manager) => manager.query_children_filtered(parent, nouns),
        }
    }

    pub fn query_descendants(&self, root: RefnoEnum, max_depth: Option<usize>) -> Vec<RefnoEnum> {
        match self {
            Self::Snapshot { .. } => self
                .snapshot_for_refno(root.refno())
                .map(|s| {
                    let options = TreeQueryOptions {
                        include_self: false,
                        max_depth,
                        filter: TreeQueryFilter::default(),
                        prune_on_match: false,
                    };
                    s.collect_descendants_bfs(root.refno(), &options)
                        .into_iter()
                        .map(RefnoEnum::from)
                        .collect()
                })
                .unwrap_or_default(),
            Self::Tree(manager) => manager.query_descendants(root, max_depth),
        }
    }

    pub fn query_descendants_filtered(
        &self,
        root: RefnoEnum,
        nouns: &[&str],
        max_depth: Option<usize>,
    ) -> Vec<RefnoEnum> {
        match self {
            Self::Snapshot { .. } => self
                .snapshot_for_refno(root.refno())
                .map(|s| {
                    let options = TreeQueryOptions {
                        include_self: false,
                        max_depth,
                        filter: TreeQueryFilter {
                            noun_hashes: Some(nouns.iter().map(|n| db1_hash(n)).collect()),
                            ..Default::default()
                        },
                        prune_on_match: false,
                    };
                    s.collect_descendants_bfs(root.refno(), &options)
                        .into_iter()
                        .map(RefnoEnum::from)
                        .collect()
                })
                .unwrap_or_default(),
            Self::Tree(manager) => manager.query_descendants_filtered(root, nouns, max_depth),
        }
    }

    pub fn query_multi_descendants_filtered(
        &self,
        roots: &[RefnoEnum],
        nouns: &[&str],
    ) -> Vec<RefnoEnum> {
        match self {
            Self::Snapshot { .. } => {
                let mut seen = HashSet::new();
                let mut out = Vec::new();
                for &root in roots {
                    for r in self.query_descendants_filtered(root, nouns, None) {
                        if seen.insert(r) {
                            out.push(r);
                        }
                    }
                }
                out
            }
            Self::Tree(manager) => manager.query_multi_descendants_filtered(roots, nouns),
        }
    }

    /// 批量 BFS 收集目标 noun refnos，匹配后剪枝（include_self=true 语义）。
    pub fn collect_target_refnos_pruned(
        &self,
        roots: &[RefnoEnum],
        nouns: &[&str],
    ) -> Vec<RefnoEnum> {
        match self {
            Self::Snapshot { .. } => {
                let noun_hashes: HashSet<u32> = nouns.iter().map(|n| db1_hash(n)).collect();
                let mut seen = HashSet::new();
                let mut out = Vec::new();
                for &root in roots {
                    let Some(snap) = self.snapshot_for_refno(root.refno()) else {
                        continue;
                    };
                    let options = TreeQueryOptions {
                        include_self: true,
                        max_depth: None,
                        filter: TreeQueryFilter {
                            noun_hashes: Some(noun_hashes.clone()),
                            ..Default::default()
                        },
                        prune_on_match: true,
                    };
                    for r in snap.collect_descendants_bfs(root.refno(), &options) {
                        if seen.insert(r) {
                            out.push(RefnoEnum::from(r));
                        }
                    }
                }
                out
            }
            Self::Tree(manager) => manager.collect_target_refnos_pruned(roots, nouns),
        }
    }

    /// 批量 BFS 收集目标 noun refnos 并按 noun_hash 分组（include_self=true 语义）。
    pub fn collect_target_refnos_grouped(
        &self,
        roots: &[RefnoEnum],
        nouns: &[&str],
        prune: bool,
    ) -> HashMap<u32, Vec<RefnoEnum>> {
        match self {
            Self::Snapshot { .. } => {
                let noun_hashes: HashSet<u32> = nouns.iter().map(|n| db1_hash(n)).collect();
                let mut grouped: HashMap<u32, Vec<RefnoEnum>> = HashMap::new();
                let mut seen = HashSet::new();
                for &root in roots {
                    let Some(snap) = self.snapshot_for_refno(root.refno()) else {
                        continue;
                    };
                    let options = TreeQueryOptions {
                        include_self: true,
                        max_depth: None,
                        filter: TreeQueryFilter {
                            noun_hashes: Some(noun_hashes.clone()),
                            ..Default::default()
                        },
                        prune_on_match: prune,
                    };
                    for (noun_hash, refnos) in
                        snap.collect_descendants_bfs_grouped(root.refno(), &options)
                    {
                        for r in refnos {
                            if seen.insert(r) {
                                grouped
                                    .entry(noun_hash)
                                    .or_default()
                                    .push(RefnoEnum::from(r));
                            }
                        }
                    }
                }
                grouped
            }
            Self::Tree(manager) => manager.collect_target_refnos_grouped(roots, nouns, prune),
        }
    }

    pub fn query_ancestors(&self, node: RefnoEnum) -> Vec<RefnoEnum> {
        match self {
            Self::Snapshot { .. } => self
                .snapshot_for_refno(node.refno())
                .map(|s| {
                    let options = TreeQueryOptions {
                        include_self: false,
                        max_depth: None,
                        filter: TreeQueryFilter::default(),
                        prune_on_match: false,
                    };
                    s.collect_ancestors_root_to_parent(node.refno(), &options)
                        .into_iter()
                        .map(RefnoEnum::from)
                        .collect()
                })
                .unwrap_or_default(),
            Self::Tree(manager) => manager.query_ancestors(node),
        }
    }

    pub fn query_ancestors_filtered(&self, node: RefnoEnum, nouns: &[&str]) -> Vec<RefnoEnum> {
        match self {
            Self::Snapshot { .. } => self
                .snapshot_for_refno(node.refno())
                .map(|s| {
                    let options = TreeQueryOptions {
                        include_self: false,
                        max_depth: None,
                        filter: TreeQueryFilter {
                            noun_hashes: Some(nouns.iter().map(|n| db1_hash(n)).collect()),
                            ..Default::default()
                        },
                        prune_on_match: false,
                    };
                    s.collect_ancestors_root_to_parent(node.refno(), &options)
                        .into_iter()
                        .map(RefnoEnum::from)
                        .collect()
                })
                .unwrap_or_default(),
            Self::Tree(manager) => manager.query_ancestors_filtered(node, nouns),
        }
    }

    // ========================================================================
    // noun 枚举 / 统计
    // ========================================================================

    pub fn query_noun_refnos(&self, noun: &str, limit: Option<usize>) -> Vec<RefnoEnum> {
        match self {
            Self::Snapshot { .. } => {
                let hash = db1_hash(noun);
                let mut out: Vec<RefnoEnum> = Vec::new();
                for snap in self.snapshots() {
                    out.extend(snap.noun_refnos(hash).into_iter().map(RefnoEnum::from));
                }
                if let Some(l) = limit {
                    if out.len() > l {
                        out.truncate(l);
                    }
                }
                out
            }
            Self::Tree(manager) => manager.query_noun_refnos(noun, limit),
        }
    }

    pub fn query_nouns_grouped(&self, nouns: &[&str]) -> HashMap<String, Vec<RefnoEnum>> {
        match self {
            Self::Snapshot { .. } => {
                let target: HashMap<u32, &str> = nouns.iter().map(|&n| (db1_hash(n), n)).collect();
                let mut out: HashMap<String, Vec<RefnoEnum>> = HashMap::new();
                for snap in self.snapshots() {
                    for r in snap.all_refnos() {
                        if let Some(meta) = snap.node_meta(r) {
                            if let Some(&name) = target.get(&meta.noun) {
                                out.entry(name.to_string())
                                    .or_default()
                                    .push(RefnoEnum::from(r));
                            }
                        }
                    }
                }
                out
            }
            Self::Tree(manager) => manager.query_nouns_grouped(nouns),
        }
    }

    pub fn query_visible_geo_refnos(&self) -> Vec<RefnoEnum> {
        match self {
            Self::Snapshot { .. } => {
                use aios_core::pdms_types::VISBILE_GEO_NOUNS;
                let visible: HashSet<u32> =
                    VISBILE_GEO_NOUNS.iter().map(|&n| db1_hash(n)).collect();
                let mut out = Vec::new();
                for snap in self.snapshots() {
                    for r in snap.all_refnos() {
                        if let Some(meta) = snap.node_meta(r) {
                            if visible.contains(&meta.noun) {
                                out.push(RefnoEnum::from(r));
                            }
                        }
                    }
                }
                out
            }
            Self::Tree(manager) => manager.query_visible_geo_refnos(),
        }
    }

    pub fn count_by_noun(&self) -> HashMap<String, usize> {
        match self {
            Self::Snapshot { .. } => {
                let mut out: HashMap<String, usize> = HashMap::new();
                for snap in self.snapshots() {
                    for (hash, count) in snap.count_by_noun_hash() {
                        *out.entry(db1_dehash(hash)).or_default() += count;
                    }
                }
                out
            }
            Self::Tree(manager) => manager.count_by_noun(),
        }
    }

    pub fn total_node_count(&self) -> usize {
        match self {
            Self::Snapshot { .. } => self.snapshots().iter().map(|s| s.node_count()).sum(),
            Self::Tree(manager) => manager.total_node_count(),
        }
    }

    pub fn all_refnos(&self) -> Vec<RefnoEnum> {
        match self {
            Self::Snapshot { .. } => {
                let mut out = Vec::new();
                for snap in self.snapshots() {
                    out.extend(snap.all_refnos().into_iter().map(RefnoEnum::from));
                }
                out
            }
            Self::Tree(manager) => manager.all_refnos(),
        }
    }

    /// 各 dbnum 的树根（snapshot：owner 自指或悬挂的节点；tree：index.roots()）。
    pub fn roots(&self) -> Vec<RefnoEnum> {
        match self {
            Self::Snapshot { by_dbnum } => {
                let mut out = Vec::new();
                for snap in by_dbnum.values() {
                    out.extend(snap.roots().iter().copied().map(RefnoEnum::from));
                }
                out
            }
            Self::Tree(manager) => {
                let mut out = Vec::new();
                for &dbnum in manager.dbnums() {
                    if let Ok(index) = manager.load_index(dbnum) {
                        out.extend(index.roots().iter().copied().map(RefnoEnum::from));
                    }
                }
                out
            }
        }
    }
}

fn resolve_all_dbnums() -> anyhow::Result<Vec<u32>> {
    use crate::data_interface::db_meta;
    db_meta().ensure_loaded()?;
    let mut dbnums = db_meta().get_all_dbnums();
    if dbnums.is_empty() {
        anyhow::bail!("db_meta_info.json 中未找到可用 dbnum（pe_owner 快照需要 dbnum 清单）");
    }
    dbnums.sort_unstable();
    Ok(dbnums)
}
