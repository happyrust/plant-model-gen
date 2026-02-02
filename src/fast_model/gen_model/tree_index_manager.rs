//! TreeIndex 管理器
//!
//! 统一管理 `output/scene_tree/{dbnum}.tree` 文件的加载和查询
//!
//! ## 重要说明：从 refno 获取 dbnum
//!
//! 本模块提供了标准的 `resolve_dbnum_for_refno()` 方法来从 refno 解析 dbnum。
//!
//! ⚠️ **不要使用以下错误方法**：
//! - ❌ 字符串分割：`refno.to_string().split_once('_')` - 不可靠，会将 "25688_36110" 错误解析为 dbnum=25688
//! - ❌ 直接取高位：`refno.refno().get_0()` - 依赖内部实现，不够健壮
//!
//! ✅ **正确用法**：
//! ```rust
//! use crate::fast_model::gen_model::tree_index_manager::TreeIndexManager;
//! let dbnum = TreeIndexManager::resolve_dbnum_for_refno(refno).await?;
//! ```

use crate::versioned_db::db_meta_info::DEFAULT_TREE_DIR;
use aios_core::tool::db_tool::{db1_dehash, db1_hash};
use aios_core::tree_query::{load_tree_index_from_dir, TreeIndex, TreeQuery, TreeQueryFilter, TreeQueryOptions};
use aios_core::pe::SPdmsElement;
use aios_core::{RefnoEnum, RefU64};
use dashmap::DashMap;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

static TREE_INDEX_CACHE: Lazy<DashMap<(PathBuf, u32), Arc<TreeIndex>>> =
    Lazy::new(DashMap::new);

/// 全局开关：是否允许自动生成缺失的 tree 索引文件
static AUTO_GENERATE_TREE_ENABLED: AtomicBool = AtomicBool::new(false);

/// 启用自动生成缺失的 tree 索引文件
pub fn enable_auto_generate_tree() {
    AUTO_GENERATE_TREE_ENABLED.store(true, Ordering::Relaxed);
    log::info!("[TreeIndexManager] 已启用自动生成 tree 索引文件");
}

/// 禁用自动生成缺失的 tree 索引文件
pub fn disable_auto_generate_tree() {
    AUTO_GENERATE_TREE_ENABLED.store(false, Ordering::Relaxed);
}

/// 检查是否启用了自动生成
pub fn is_auto_generate_tree_enabled() -> bool {
    AUTO_GENERATE_TREE_ENABLED.load(Ordering::Relaxed)
}

/// 从全局缓存中尝试获取已加载的 TreeIndex（不会触发磁盘读取/反序列化）。
pub fn try_get_cached_index(tree_dir: impl AsRef<Path>, dbnum: u32) -> Option<Arc<TreeIndex>> {
    let key = (tree_dir.as_ref().to_path_buf(), dbnum);
    TREE_INDEX_CACHE.get(&key).map(|v| v.clone())
}

/// 在大栈线程中加载 TreeIndex（避免 Windows 上反序列化大 `.tree` 文件时触发栈溢出）。
///
/// - 若缓存命中，直接返回缓存结果（不创建线程）。
/// - 若缓存未命中，则在 64MB 栈线程中执行 `load_index` 并写入缓存。
pub fn load_index_with_large_stack(
    tree_dir: impl AsRef<Path>,
    dbnum: u32,
) -> anyhow::Result<Arc<TreeIndex>> {
    if let Some(cached) = try_get_cached_index(tree_dir.as_ref(), dbnum) {
        return Ok(cached);
    }

    let tree_dir = tree_dir.as_ref().to_path_buf();
    let handle = std::thread::Builder::new()
        .name(format!("tree-index-loader-{}", dbnum))
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let manager = TreeIndexManager::new(&tree_dir, vec![dbnum]);
            manager.load_index(dbnum)
        })?;

    handle
        .join()
        .map_err(|_| anyhow::anyhow!("tree-index-loader 线程 panic（可能由栈溢出导致）"))?
}

/// Tree 索引缺失错误，包含友好的提示信息
#[derive(Debug)]
pub struct TreeIndexMissingError {
    pub dbnum: u32,
    pub tree_dir: PathBuf,
    pub tree_file_path: PathBuf,
}

impl std::fmt::Display for TreeIndexMissingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            r#"
╔══════════════════════════════════════════════════════════════════════════════╗
║  ❌ Tree 索引文件不存在                                                        ║
╠══════════════════════════════════════════════════════════════════════════════╣
║  缺失文件: {tree_file}
║  dbnum: {dbnum}
╠══════════════════════════════════════════════════════════════════════════════╣
║  Tree 索引文件用于快速查询节点的层级关系（父子、祖先、子孙）。                    ║
║  该文件在解析 PDMS 数据库时自动生成。                                           ║
╠══════════════════════════════════════════════════════════════════════════════╣
║  解决方案:                                                                     ║
║                                                                               ║
║  方案1: 重新解析数据库（推荐）                                                  ║
║    cargo run --bin aios-database -- --parse-db                               ║
║                                                                               ║
║  方案2: 从 SurrealDB 重建 tree 索引                                            ║
║    cargo run --bin aios-database -- --rebuild-tree-index                     ║
║                                                                               ║
║  方案3: 启用自动生成（在程序中调用）                                            ║
║    aios_database::fast_model::gen_model::tree_index_manager::enable_auto_generate_tree();
╚══════════════════════════════════════════════════════════════════════════════╝
"#,
            tree_file = self.tree_file_path.display(),
            dbnum = self.dbnum
        )
    }
}

impl std::error::Error for TreeIndexMissingError {}

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
    ///
    /// 如果 tree 文件不存在且启用了自动生成，会尝试从 SurrealDB 重建
    pub fn load_index(&self, dbnum: u32) -> anyhow::Result<Arc<TreeIndex>> {
        let key = (self.tree_dir.clone(), dbnum);
        if let Some(entry) = TREE_INDEX_CACHE.get(&key) {
            return Ok(entry.clone());
        }

        // 检查 tree 文件是否存在
        let tree_file_path = self.tree_dir.join(format!("{}.tree", dbnum));
        if !tree_file_path.exists() {
            // 检查目录是否存在
            if !self.tree_dir.exists() {
                return Err(TreeIndexMissingError {
                    dbnum,
                    tree_dir: self.tree_dir.clone(),
                    tree_file_path,
                }.into());
            }

            // 如果启用了自动生成，尝试生成
            if is_auto_generate_tree_enabled() {
                log::info!("[TreeIndexManager] Tree 索引文件不存在，尝试从 SurrealDB 重建: dbnum={}", dbnum);
                // 使用 tokio runtime 执行异步生成。
                //
                // 注意：load_index 可能在非 tokio 线程里被调用（例如 tree-index-loader-* 线程），
                // 这时 Handle::current() 会 panic。这里用 try_current + 兜底 runtime，保证稳定性。
                let tree_dir = self.tree_dir.clone();
                let result = match tokio::runtime::Handle::try_current() {
                    Ok(handle) => tokio::task::block_in_place(|| {
                        handle.block_on(async { generate_tree_index_from_db(dbnum, &tree_dir).await })
                    }),
                    Err(_) => {
                        let rt = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()?;
                        rt.block_on(async { generate_tree_index_from_db(dbnum, &tree_dir).await })
                    }
                };

                match result {
                    Ok(_) => {
                        log::info!("[TreeIndexManager] 已成功生成 tree 索引文件: dbnum={}", dbnum);
                    }
                    Err(e) => {
                        log::warn!("[TreeIndexManager] 自动生成 tree 索引失败: {}", e);
                        return Err(TreeIndexMissingError {
                            dbnum,
                            tree_dir: self.tree_dir.clone(),
                            tree_file_path,
                        }.into());
                    }
                }
            } else {
                return Err(TreeIndexMissingError {
                    dbnum,
                    tree_dir: self.tree_dir.clone(),
                    tree_file_path,
                }.into());
            }
        }

        let index = load_tree_index_from_dir(dbnum, &self.tree_dir)?;
        TREE_INDEX_CACHE.insert(key, index.clone());
        Ok(index)
    }

    /// 检查指定 dbnum 的 tree 文件是否存在
    pub fn tree_file_exists(&self, dbnum: u32) -> bool {
        let tree_file_path = self.tree_dir.join(format!("{}.tree", dbnum));
        tree_file_path.exists()
    }

    /// 获取缺失的 tree 文件列表
    pub fn get_missing_tree_files(&self) -> Vec<u32> {
        self.dbnums
            .iter()
            .filter(|&&dbnum| !self.tree_file_exists(dbnum))
            .copied()
            .collect()
    }

    /// 通过 refno 解析 dbnum
    ///
    /// **重要说明**：这是从 refno 获取 dbnum 的标准方法。
    ///
    /// ⚠️ **不要使用以下错误方法**：
    /// - ❌ `refno.to_string().split_once('_')` - 字符串分割不可靠
    /// - ❌ `refno.refno().get_0()` - 依赖内部实现细节，不够健壮
    ///
    /// ✅ **正确用法**：
    /// ```rust
    /// let dbnum = TreeIndexManager::resolve_dbnum_for_refno(refno).await?;
    /// ```
    ///
    /// **查询优先级（cache-only）**：
    /// 1. DbMetaManager (db_meta_info.json) - 最快，纯内存查询
    /// 2. db_meta_cache - 内存缓存
    ///
    /// 约定：不回退到 SurrealDB 查询（SurrealDB 仅作为“生成完成后的一键备份落库”目的地）。
    pub async fn resolve_dbnum_for_refno(refno: RefnoEnum) -> anyhow::Result<u32> {
        // 优先使用 DbMetaManager 的快速查询（通过 db_meta_info.json）。
        //
        // 注意：该映射需要先 ensure_loaded；否则 get_dbnum_by_refno 会因未加载而返回 None，
        // 进而误报“无法从缓存推导 refno 的 dbnum”。
        use crate::data_interface::db_meta;
        let _ = db_meta().ensure_loaded();
        if let Some(dbnum) = db_meta().get_dbnum_by_refno(refno) {
            return Ok(dbnum);
        }

        // 其次尝试从旧的缓存获取
        if let Some(dbnum) = crate::fast_model::db_meta_cache::get_dbnum_for_refno(refno) {
            return Ok(dbnum);
        }

        anyhow::bail!(
            "无法从缓存推导 refno 的 dbnum（cache-only 不回退 SurrealDB）：refno={}\n\
             处理建议：\n\
             - 先生成 output/scene_tree/db_meta_info.json（例如 parse-db/生成 tree 阶段会产出）\n\
             - 或确认当前运行目录/配置指向了正确的输出目录",
            refno
        )
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

    /// 仅基于 TreeIndex 查询“直接子节点元素列表”（不访问 SurrealDB）。
    ///
    /// 用途：
    /// - BRAN/HANG 生成路径中收集子元件（管件）集合
    /// - cache-only 模式下的过滤/分组查询
    ///
    /// 注意：
    /// - TreeIndex 不包含 name/status/lock 等运行期字段，这里仅构造满足生成流水线所需的最小 SPdmsElement：
    ///   refno/owner/noun/dbnum/sesno（其余字段保持默认值）。
    pub async fn collect_children_elements_from_tree(
        parent: RefnoEnum,
    ) -> anyhow::Result<Vec<SPdmsElement>> {
        let dbnum = Self::resolve_dbnum_for_refno(parent).await?;
        let manager = TreeIndexManager::with_default_dir(vec![dbnum]);
        let index = manager.load_index(dbnum)?;

        let parent_u64 = parent.refno();
        let child_u64s = index.query_children(parent_u64, TreeQueryFilter::default()).await?;

        let mut out: Vec<SPdmsElement> = Vec::with_capacity(child_u64s.len());
        for child in child_u64s {
            let Some(meta) = index.node_meta(child) else {
                continue;
            };
            let mut ele = SPdmsElement::default();
            ele.refno = RefnoEnum::from(meta.refno);
            ele.owner = RefnoEnum::from(meta.owner);
            ele.noun = db1_dehash(meta.noun);
            ele.dbnum = dbnum as i32;
            ele.sesno = 0;
            // name/status_code/lock/deleted/... 保持默认值（空/false/None）
            out.push(ele);
        }

        Ok(out)
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

// ============================================================================
// 从 SurrealDB 生成 tree 索引文件
// ============================================================================

/// 从 SurrealDB 生成指定 dbnum 的 tree 索引文件
///
/// 该函数查询 SurrealDB 中的 pe 表，获取所有节点的层级关系，
/// 然后生成 tree 索引文件保存到指定目录。
///
/// # Arguments
/// * `dbnum` - 数据库编号
/// * `output_dir` - 输出目录 (如 "output/scene_tree")
pub async fn generate_tree_index_from_db(dbnum: u32, output_dir: &Path) -> anyhow::Result<()> {
    use aios_core::{SUL_DB, SurrealQueryExt};
    use crate::versioned_db::tree_export::{export_tree_file, TreeNodeMeta};
    use aios_core::db::DbBasicData;
    use std::collections::HashMap;
    use surrealdb::types::SurrealValue;

    log::info!("[generate_tree_index] 开始从 SurrealDB 生成 tree 索引: dbnum={}", dbnum);

    // 查询指定 dbnum 的所有节点
    #[derive(Debug, serde::Deserialize, SurrealValue)]
    struct PeRow {
        refno: Option<u64>,
        owner: Option<u64>,
        noun: Option<String>,
        cata_hash: Option<u64>,
    }

    let sql = format!(
        "SELECT refno, owner, noun, cata_hash FROM pe WHERE dbnum = {}",
        dbnum
    );

    let rows: Vec<PeRow> = SUL_DB.query_take(&sql, 0).await?;

    if rows.is_empty() {
        anyhow::bail!("dbnum={} 在 SurrealDB 中没有找到任何节点", dbnum);
    }

    log::info!("[generate_tree_index] 查询到 {} 个节点", rows.len());

    // 构建 tree_nodes HashMap
    let mut tree_nodes: HashMap<RefU64, TreeNodeMeta> = HashMap::new();

    for row in rows {
        let Some(refno_val) = row.refno else { continue };
        let refno = RefU64(refno_val);

        let owner = row.owner.map(RefU64).unwrap_or(refno);
        let noun = row.noun.as_deref().unwrap_or("UNKNOWN");
        let noun_hash = db1_hash(noun);

        tree_nodes.insert(refno, TreeNodeMeta {
            refno,
            owner,
            noun: noun_hash,
            cata_hash: row.cata_hash,
        });
    }

    // 创建 DbBasicData (仅用于兼容 export_tree_file 签名)
    let db_basic = DbBasicData::default();

    // 确保输出目录存在
    std::fs::create_dir_all(output_dir)?;

    // 从 SurrealDB 构建时没有 children_map，使用空 map（顺序不保证）
    // 注意：正确的 tree 应该从 PDMS 解析时生成，这里仅作为 fallback
    let children_map: HashMap<RefU64, Vec<RefU64>> = HashMap::new();

    // 导出 tree 文件
    export_tree_file(dbnum, &db_basic, &tree_nodes, &children_map, output_dir)?;

    log::info!(
        "[generate_tree_index] 成功生成 tree 索引文件: {}/{}.tree ({} 节点)",
        output_dir.display(),
        dbnum,
        tree_nodes.len()
    );

    Ok(())
}

/// 确保指定 dbnum 的 `{dbnum}.tree` 文件存在；若缺失则从 SurrealDB 生成。
pub async fn ensure_tree_index_exists(dbnum: u32, output_dir: &Path) -> anyhow::Result<()> {
    let tree_path = output_dir.join(format!("{}.tree", dbnum));
    if tree_path.is_file() {
        return Ok(());
    }

    log::info!(
        "[tree_index] 缺失 tree 索引文件，开始按需生成: {}",
        tree_path.display()
    );
    generate_tree_index_from_db(dbnum, output_dir).await
}

/// 批量生成多个 dbnum 的 tree 索引文件
pub async fn generate_tree_indices_from_db(dbnums: &[u32], output_dir: &Path) -> anyhow::Result<usize> {
    let mut success_count = 0;

    for &dbnum in dbnums {
        match generate_tree_index_from_db(dbnum, output_dir).await {
            Ok(_) => success_count += 1,
            Err(e) => log::warn!("[generate_tree_index] dbnum={} 生成失败: {}", dbnum, e),
        }
    }

    Ok(success_count)
}

/// 从 SurrealDB 获取所有可用的 dbnum 列表
pub async fn get_available_dbnums_from_db() -> anyhow::Result<Vec<u32>> {
    use aios_core::{SUL_DB, SurrealQueryExt};

    let sql = "SELECT DISTINCT dbnum FROM pe WHERE dbnum != NONE";
    let dbnums: Vec<i32> = SUL_DB.query_take(sql, 0).await?;

    Ok(dbnums.into_iter().filter(|&d| d > 0).map(|d| d as u32).collect())
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
