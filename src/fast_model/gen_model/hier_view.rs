//! specs/023 M2：生成/导出管线的层级视图（pe_owner 快照）。
//!
//! `HierView::load(dbnums)` 预加载 per-dbnum 内存快照（`versioned_db::pe_owner_snapshot`，
//! 数据来自 pe 表，run 级失效）。加载完成后全部查询为**同步**操作。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use aios_core::pdms_types::BRAN_COMPONENT_NOUN_NAMES;
use aios_core::pe::SPdmsElement;
use aios_core::tool::db_tool::{db1_dehash, db1_hash};
use aios_core::tree_query::{TreeNodeMeta, TreeQueryFilter, TreeQueryOptions};
use aios_core::{RefU64, RefnoEnum};

use crate::data_interface::db_meta_manager::resolve_dbnum_for_refno;
use crate::versioned_db::pe_owner_snapshot::{PeDbnumSnapshot, get_or_load_pe_snapshot};

pub struct HierView {
    by_dbnum: HashMap<u32, Arc<PeDbnumSnapshot>>,
}

impl HierView {
    /// 加载层级视图。dbnums 为空时从 db_meta 取全部 dbnum。
    pub async fn load(dbnums: Vec<u32>) -> anyhow::Result<Self> {
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
        Ok(Self { by_dbnum })
    }

    /// 单 refno 场景的便捷入口（自动解析 dbnum）。
    pub async fn load_for_refno(refno: RefnoEnum) -> anyhow::Result<Self> {
        let dbnum = resolve_dbnum_for_refno(refno)?;
        Self::load(vec![dbnum]).await
    }

    pub fn is_snapshot(&self) -> bool {
        true
    }

    fn snapshot_for_refno(&self, refno: RefU64) -> Option<&Arc<PeDbnumSnapshot>> {
        if let Ok(dbnum) = resolve_dbnum_for_refno(RefnoEnum::from(refno)) {
            if let Some(snap) = self.by_dbnum.get(&dbnum) {
                if snap.contains_refno(refno) {
                    return Some(snap);
                }
            }
        }
        self.by_dbnum.values().find(|s| s.contains_refno(refno))
    }

    fn snapshots(&self) -> Vec<&Arc<PeDbnumSnapshot>> {
        self.by_dbnum.values().collect()
    }

    pub fn get_node_meta(&self, refno: RefnoEnum) -> Option<TreeNodeMeta> {
        self.snapshot_for_refno(refno.refno())
            .and_then(|s| s.node_meta(refno.refno()))
    }

    pub fn get_noun(&self, refno: RefnoEnum) -> Option<String> {
        self.get_node_meta(refno).map(|m| db1_dehash(m.noun))
    }

    pub fn contains(&self, refno: RefnoEnum) -> bool {
        self.snapshot_for_refno(refno.refno()).is_some()
    }

    pub fn query_children(&self, parent: RefnoEnum) -> Vec<RefnoEnum> {
        self.snapshot_for_refno(parent.refno())
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
            .unwrap_or_default()
    }

    pub fn query_children_filtered(&self, parent: RefnoEnum, nouns: &[&str]) -> Vec<RefnoEnum> {
        self.snapshot_for_refno(parent.refno())
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
            .unwrap_or_default()
    }

    pub fn query_descendants(&self, root: RefnoEnum, max_depth: Option<usize>) -> Vec<RefnoEnum> {
        self.snapshot_for_refno(root.refno())
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
            .unwrap_or_default()
    }

    pub fn query_descendants_filtered(
        &self,
        root: RefnoEnum,
        nouns: &[&str],
        max_depth: Option<usize>,
    ) -> Vec<RefnoEnum> {
        self.snapshot_for_refno(root.refno())
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
            .unwrap_or_default()
    }

    pub fn query_multi_descendants_filtered(
        &self,
        roots: &[RefnoEnum],
        nouns: &[&str],
    ) -> Vec<RefnoEnum> {
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

    pub fn collect_target_refnos_pruned(
        &self,
        roots: &[RefnoEnum],
        nouns: &[&str],
    ) -> Vec<RefnoEnum> {
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

    pub fn collect_target_refnos_grouped(
        &self,
        roots: &[RefnoEnum],
        nouns: &[&str],
        prune: bool,
    ) -> HashMap<u32, Vec<RefnoEnum>> {
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
            for (noun_hash, refnos) in snap.collect_descendants_bfs_grouped(root.refno(), &options)
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

    pub fn query_ancestors(&self, node: RefnoEnum) -> Vec<RefnoEnum> {
        self.snapshot_for_refno(node.refno())
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
            .unwrap_or_default()
    }

    pub fn query_ancestors_filtered(&self, node: RefnoEnum, nouns: &[&str]) -> Vec<RefnoEnum> {
        self.snapshot_for_refno(node.refno())
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
            .unwrap_or_default()
    }

    pub fn query_noun_refnos(&self, noun: &str, limit: Option<usize>) -> Vec<RefnoEnum> {
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

    pub fn query_nouns_grouped(&self, nouns: &[&str]) -> HashMap<String, Vec<RefnoEnum>> {
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

    pub fn query_visible_geo_refnos(&self) -> Vec<RefnoEnum> {
        use aios_core::pdms_types::VISBILE_GEO_NOUNS;
        let visible: HashSet<u32> = VISBILE_GEO_NOUNS.iter().map(|&n| db1_hash(n)).collect();
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

    pub fn count_by_noun(&self) -> HashMap<String, usize> {
        let mut out: HashMap<String, usize> = HashMap::new();
        for snap in self.snapshots() {
            for (hash, count) in snap.count_by_noun_hash() {
                *out.entry(db1_dehash(hash)).or_default() += count;
            }
        }
        out
    }

    pub fn total_node_count(&self) -> usize {
        self.snapshots().iter().map(|s| s.node_count()).sum()
    }

    pub fn all_refnos(&self) -> Vec<RefnoEnum> {
        let mut out = Vec::new();
        for snap in self.snapshots() {
            out.extend(snap.all_refnos().into_iter().map(RefnoEnum::from));
        }
        out
    }

    pub fn roots(&self) -> Vec<RefnoEnum> {
        let mut out = Vec::new();
        for snap in self.by_dbnum.values() {
            out.extend(snap.roots().iter().copied().map(RefnoEnum::from));
        }
        out
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

fn metas_to_min_elements(
    metas: impl Iterator<Item = TreeNodeMeta>,
    dbnum: u32,
) -> Vec<SPdmsElement> {
    metas
        .map(|meta| {
            let mut ele = SPdmsElement::default();
            ele.refno = RefnoEnum::from(meta.refno);
            ele.owner = RefnoEnum::from(meta.owner);
            ele.noun = db1_dehash(meta.noun);
            ele.dbnum = dbnum as i32;
            ele.sesno = 0;
            ele
        })
        .collect()
}

/// 收集 parent 的直接子元件（pe_owner 快照）。
pub async fn collect_children_elements_from_hierarchy(
    parent: RefnoEnum,
) -> anyhow::Result<Vec<SPdmsElement>> {
    let dbnum = resolve_dbnum_for_refno(parent)?;
    let snap = get_or_load_pe_snapshot(dbnum).await?;
    let child_u64s = snap.collect_children(parent.refno(), &TreeQueryFilter::default());
    Ok(metas_to_min_elements(
        child_u64s.into_iter().filter_map(|c| snap.node_meta(c)),
        dbnum,
    ))
}

/// 收集 BRAN 下所有 CATE/管件子孙节点（BFS + BRAN_COMPONENT 过滤）。
pub async fn collect_bran_cate_descendant_elements_from_hierarchy(
    parent: RefnoEnum,
) -> anyhow::Result<Vec<SPdmsElement>> {
    let dbnum = resolve_dbnum_for_refno(parent)?;
    let noun_hashes: HashSet<u32> = BRAN_COMPONENT_NOUN_NAMES
        .iter()
        .map(|n| db1_hash(n))
        .collect();
    let options = TreeQueryOptions {
        include_self: false,
        max_depth: None,
        filter: TreeQueryFilter {
            noun_hashes: Some(noun_hashes),
            ..Default::default()
        },
        prune_on_match: false,
    };
    let snap = get_or_load_pe_snapshot(dbnum).await?;
    let descendant_u64s = snap.collect_descendants_bfs(parent.refno(), &options);
    Ok(metas_to_min_elements(
        descendant_u64s
            .into_iter()
            .filter_map(|c| snap.node_meta(c)),
        dbnum,
    ))
}
