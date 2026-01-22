//! TreeIndex 管理器
//!
//! 统一管理 `output/scene_tree/{dbnum}.tree` 文件的加载和查询

use crate::versioned_db::db_meta_info::DEFAULT_TREE_DIR;
use aios_core::tool::db_tool::{db1_dehash, db1_hash};
use aios_core::tree_query::{load_tree_index_from_dir, TreeIndex, TreeQueryFilter, TreeQueryOptions};
use aios_core::{RefnoEnum, RefU64};
use dashmap::DashMap;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

static TREE_INDEX_CACHE: Lazy<DashMap<(PathBuf, u32), Arc<TreeIndex>>> =
    Lazy::new(DashMap::new);

/// TreeIndex 管理器
/// 
/// 提供对指定 dbnum 的 TreeIndex 统一访问接口
pub struct TreeIndexManager {
    tree_dir: PathBuf,
    dbnums: Vec<u32>,
}

impl TreeIndexManager {
    /// 创建新的 TreeIndexManager
    /// 
    /// # Arguments
    /// * `tree_dir` - TreeIndex 文件目录 (如 "output/scene_tree")
    /// * `dbnums` - 要管理的 dbnum 列表
    pub fn new(tree_dir: impl AsRef<Path>, dbnums: Vec<u32>) -> Self {
        Self {
            tree_dir: tree_dir.as_ref().to_path_buf(),
            dbnums,
        }
    }

    /// 使用默认目录创建 Manager
    pub fn with_default_dir(dbnums: Vec<u32>) -> Self {
        Self::new(DEFAULT_TREE_DIR, dbnums)
    }

    /// 获取管理的 dbnum 列表
    pub fn dbnums(&self) -> &[u32] {
        &self.dbnums
    }

    /// 获取 TreeIndex 目录
    pub fn tree_dir(&self) -> &Path {
        &self.tree_dir
    }

    /// 加载指定 dbnum 的 TreeIndex
    pub fn load_index(&self, dbnum: u32) -> anyhow::Result<Arc<TreeIndex>> {
        let key = (self.tree_dir.clone(), dbnum);
        if let Some(entry) = TREE_INDEX_CACHE.get(&key) {
            return Ok(entry.clone());
        }
        let index = load_tree_index_from_dir(dbnum, &self.tree_dir)?;
        TREE_INDEX_CACHE.insert(key, index.clone());
        Ok(index)
    }

    /// 通过 refno 解析 dbnum
    pub async fn resolve_dbnum_for_refno(refno: RefnoEnum) -> anyhow::Result<u32> {
        if let Some(dbnum) = crate::fast_model::db_meta_cache::get_dbnum_for_refno(refno) {
            return Ok(dbnum);
        }

        let Some(pe) = aios_core::get_pe(refno).await? else {
            return Err(anyhow::anyhow!("refno {} 不存在，无法获取 dbnum", refno));
        };
        if pe.dbnum < 0 {
            return Err(anyhow::anyhow!(
                "refno {} 的 dbnum 非法: {}",
                refno,
                pe.dbnum
            ));
        }
        Ok(pe.dbnum as u32)
    }

    /// 通过 refno 加载对应 TreeIndex
    pub async fn load_index_for_refno(&self, refno: RefnoEnum) -> anyhow::Result<Arc<TreeIndex>> {
        let dbnum = Self::resolve_dbnum_for_refno(refno).await?;
        self.load_index(dbnum)
    }

    /// 查询指定 noun 类型的所有 refnos
    /// 
    /// # Arguments
    /// * `noun` - Noun 名称 (如 "BRAN", "PANE")
    /// * `limit` - 可选的数量限制
    pub fn query_noun_refnos(&self, noun: &str, limit: Option<usize>) -> Vec<RefnoEnum> {
        let target_noun_hash = db1_hash(noun);
        let mut refnos = Vec::new();

        for &dbnum in &self.dbnums {
            match self.load_index(dbnum) {
                Ok(index) => {
                    for refno in index.all_refnos() {
                        if let Some(meta) = index.node_meta(refno) {
                            if meta.noun == target_noun_hash {
                                refnos.push(RefnoEnum::from(refno));
                            }
                        }
                    }
                }
                Err(e) => {
                    log::warn!(
                        "[TreeIndexManager] 加载 TreeIndex dbnum={} 失败: {}",
                        dbnum, e
                    );
                }
            }
        }

        if let Some(l) = limit {
            if refnos.len() > l {
                refnos.truncate(l);
            }
        }

        refnos
    }

    /// 按多个 noun 类型分组查询 refnos
    /// 
    /// # Arguments
    /// * `nouns` - Noun 名称列表
    /// 
    /// # Returns
    /// 按 noun 名称分组的 refnos 映射
    pub fn query_nouns_grouped(&self, nouns: &[&str]) -> HashMap<String, Vec<RefnoEnum>> {
        // 构建目标 noun hash 集合
        let target_hashes: HashMap<u32, &str> = nouns
            .iter()
            .map(|&n| (db1_hash(n), n))
            .collect();

        // 按 noun hash 分组收集 refnos
        let mut result: HashMap<String, Vec<RefnoEnum>> = HashMap::new();

        for &dbnum in &self.dbnums {
            match self.load_index(dbnum) {
                Ok(index) => {
                    for refno in index.all_refnos() {
                        if let Some(meta) = index.node_meta(refno) {
                            if let Some(&noun_name) = target_hashes.get(&meta.noun) {
                                result
                                    .entry(noun_name.to_string())
                                    .or_default()
                                    .push(RefnoEnum::from(refno));
                            }
                        }
                    }
                }
                Err(e) => {
                    log::warn!(
                        "[TreeIndexManager] 加载 TreeIndex dbnum={} 失败: {}",
                        dbnum, e
                    );
                }
            }
        }

        result
    }

    /// 获取所有节点的 refnos
    pub fn all_refnos(&self) -> Vec<RefnoEnum> {
        let mut refnos = Vec::new();

        for &dbnum in &self.dbnums {
            match self.load_index(dbnum) {
                Ok(index) => {
                    refnos.extend(index.all_refnos().into_iter().map(RefnoEnum::from));
                }
                Err(e) => {
                    log::warn!(
                        "[TreeIndexManager] 加载 TreeIndex dbnum={} 失败: {}",
                        dbnum, e
                    );
                }
            }
        }

        refnos
    }

    /// 统计各 noun 类型的数量
    pub fn count_by_noun(&self) -> HashMap<String, usize> {
        use aios_core::tool::db_tool::db1_dehash;
        
        let mut counts: HashMap<u32, usize> = HashMap::new();

        for &dbnum in &self.dbnums {
            match self.load_index(dbnum) {
                Ok(index) => {
                    for refno in index.all_refnos() {
                        if let Some(meta) = index.node_meta(refno) {
                            *counts.entry(meta.noun).or_default() += 1;
                        }
                    }
                }
                Err(e) => {
                    log::warn!(
                        "[TreeIndexManager] 加载 TreeIndex dbnum={} 失败: {}",
                        dbnum, e
                    );
                }
            }
        }

        // 转换 hash -> 名称
        counts
            .into_iter()
            .map(|(hash, count)| (db1_dehash(hash), count))
            .collect()
    }

    /// 获取节点总数
    pub fn total_node_count(&self) -> usize {
        let mut count = 0;

        for &dbnum in &self.dbnums {
            match self.load_index(dbnum) {
                Ok(index) => {
                    count += index.node_count();
                }
                Err(e) => {
                    log::warn!(
                        "[TreeIndexManager] 加载 TreeIndex dbnum={} 失败: {}",
                        dbnum, e
                    );
                }
            }
        }

        count
    }

    // ============================================================================
    // 层级查询方法
    // ============================================================================

    /// 查询指定节点的所有子孙节点
    /// 
    /// # Arguments
    /// * `root` - 根节点
    /// * `max_depth` - 可选的最大深度限制
    pub fn query_descendants(&self, root: RefnoEnum, max_depth: Option<usize>) -> Vec<RefnoEnum> {
        let refno = root.refno();
        for &dbnum in &self.dbnums {
            if let Ok(index) = self.load_index(dbnum) {
                if index.contains_refno(refno) {
                    let options = TreeQueryOptions {
                        include_self: false,
                        max_depth,
                        filter: TreeQueryFilter::default(),
                    };
                    return index
                        .collect_descendants_bfs(refno, &options)
                        .into_iter()
                        .map(RefnoEnum::from)
                        .collect();
                }
            }
        }
        Vec::new()
    }

    /// 查询指定节点的子孙节点，按 noun 类型过滤
    /// 
    /// # Arguments
    /// * `root` - 根节点
    /// * `nouns` - 要过滤的 noun 名称列表
    /// * `max_depth` - 可选的最大深度限制
    pub fn query_descendants_filtered(
        &self,
        root: RefnoEnum,
        nouns: &[&str],
        max_depth: Option<usize>,
    ) -> Vec<RefnoEnum> {
        let refno = root.refno();
        let noun_hashes: Vec<u32> = nouns.iter().map(|n| db1_hash(n)).collect();
        
        for &dbnum in &self.dbnums {
            if let Ok(index) = self.load_index(dbnum) {
                if index.contains_refno(refno) {
                    let options = TreeQueryOptions {
                        include_self: false,
                        max_depth,
                        filter: TreeQueryFilter {
                            noun_hashes: Some(noun_hashes),
                            ..Default::default()
                        },
                    };
                    return index
                        .collect_descendants_bfs(refno, &options)
                        .into_iter()
                        .map(RefnoEnum::from)
                        .collect();
                }
            }
        }
        Vec::new()
    }

    /// 批量查询多个根节点的子孙节点，按 noun 类型过滤
    /// 
    /// # Arguments
    /// * `roots` - 根节点列表
    /// * `nouns` - 要过滤的 noun 名称列表
    pub fn query_multi_descendants_filtered(
        &self,
        roots: &[RefnoEnum],
        nouns: &[&str],
    ) -> Vec<RefnoEnum> {
        let noun_hashes: Vec<u32> = nouns.iter().map(|n| db1_hash(n)).collect();
        let mut result = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for &root in roots {
            let refno = root.refno();
            for &dbnum in &self.dbnums {
                if let Ok(index) = self.load_index(dbnum) {
                    if index.contains_refno(refno) {
                        let options = TreeQueryOptions {
                            include_self: false,
                            max_depth: None,
                            filter: TreeQueryFilter {
                                noun_hashes: Some(noun_hashes.clone()),
                                ..Default::default()
                            },
                        };
                        for desc in index.collect_descendants_bfs(refno, &options) {
                            if seen.insert(desc) {
                                result.push(RefnoEnum::from(desc));
                            }
                        }
                        break;
                    }
                }
            }
        }

        result
    }

    /// 查询指定节点的直接子节点
    pub fn query_children(&self, parent: RefnoEnum) -> Vec<RefnoEnum> {
        let refno = parent.refno();
        for &dbnum in &self.dbnums {
            if let Ok(index) = self.load_index(dbnum) {
                if index.contains_refno(refno) {
                    let options = TreeQueryOptions {
                        include_self: false,
                        max_depth: Some(1),
                        filter: TreeQueryFilter::default(),
                    };
                    return index
                        .collect_descendants_bfs(refno, &options)
                        .into_iter()
                        .map(RefnoEnum::from)
                        .collect();
                }
            }
        }
        Vec::new()
    }

    /// 查询指定节点的直接子节点，按 noun 类型过滤
    pub fn query_children_filtered(&self, parent: RefnoEnum, nouns: &[&str]) -> Vec<RefnoEnum> {
        let refno = parent.refno();
        let noun_hashes: Vec<u32> = nouns.iter().map(|n| db1_hash(n)).collect();
        
        for &dbnum in &self.dbnums {
            if let Ok(index) = self.load_index(dbnum) {
                if index.contains_refno(refno) {
                    let options = TreeQueryOptions {
                        include_self: false,
                        max_depth: Some(1),
                        filter: TreeQueryFilter {
                            noun_hashes: Some(noun_hashes),
                            ..Default::default()
                        },
                    };
                    return index
                        .collect_descendants_bfs(refno, &options)
                        .into_iter()
                        .map(RefnoEnum::from)
                        .collect();
                }
            }
        }
        Vec::new()
    }

    /// 查询指定节点的祖先节点链（从根到父）
    pub fn query_ancestors(&self, node: RefnoEnum) -> Vec<RefnoEnum> {
        let refno = node.refno();
        for &dbnum in &self.dbnums {
            if let Ok(index) = self.load_index(dbnum) {
                if index.contains_refno(refno) {
                    let options = TreeQueryOptions {
                        include_self: false,
                        max_depth: None,
                        filter: TreeQueryFilter::default(),
                    };
                    return index
                        .collect_ancestors_root_to_parent(refno, &options)
                        .into_iter()
                        .map(RefnoEnum::from)
                        .collect();
                }
            }
        }
        Vec::new()
    }

    /// 查询指定节点的祖先节点，按 noun 类型过滤
    pub fn query_ancestors_filtered(&self, node: RefnoEnum, nouns: &[&str]) -> Vec<RefnoEnum> {
        let refno = node.refno();
        let noun_hashes: Vec<u32> = nouns.iter().map(|n| db1_hash(n)).collect();
        
        for &dbnum in &self.dbnums {
            if let Ok(index) = self.load_index(dbnum) {
                if index.contains_refno(refno) {
                    let options = TreeQueryOptions {
                        include_self: false,
                        max_depth: None,
                        filter: TreeQueryFilter {
                            noun_hashes: Some(noun_hashes),
                            ..Default::default()
                        },
                    };
                    return index
                        .collect_ancestors_root_to_parent(refno, &options)
                        .into_iter()
                        .map(RefnoEnum::from)
                        .collect();
                }
            }
        }
        Vec::new()
    }

    /// 获取节点的元信息
    pub fn get_node_meta(&self, refno: RefnoEnum) -> Option<aios_core::tree_query::TreeNodeMeta> {
        let r = refno.refno();
        for &dbnum in &self.dbnums {
            if let Ok(index) = self.load_index(dbnum) {
                if let Some(meta) = index.node_meta(r) {
                    return Some(meta);
                }
            }
        }
        None
    }

    /// 获取节点的 noun 名称
    pub fn get_noun(&self, refno: RefnoEnum) -> Option<String> {
        self.get_node_meta(refno).map(|meta| db1_dehash(meta.noun))
    }

    /// 检查节点是否存在
    pub fn contains(&self, refno: RefnoEnum) -> bool {
        let r = refno.refno();
        for &dbnum in &self.dbnums {
            if let Ok(index) = self.load_index(dbnum) {
                if index.contains_refno(r) {
                    return true;
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tree_index_manager_creation() {
        let manager = TreeIndexManager::with_default_dir(vec![1112]);
        assert_eq!(manager.dbnums(), &[1112]);
    }
}
