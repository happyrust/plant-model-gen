//! 模型生成专用的查询提供者
//!
//! 使用 TreeIndex 作为层级查询的数据源（PE/属性仍委托 SurrealDB）。
//!
//! # 使用示例
//!
//! ```rust,ignore
//! use crate::fast_model::query_provider::*;
//!
//! // 获取层级过滤的子孙节点
//! let descendants = get_descendants_by_types(
//!     zone_refno,
//!     &["EQUI", "PIPE"],
//!     Some(12)
//! ).await?;
//! ```

use crate::data_interface::db_meta;
use crate::fast_model::gen_model::tree_index_manager::{
    TreeIndexManager, load_index_with_large_stack,
};
use crate::versioned_db::pe_owner_snapshot::get_or_load_pe_snapshot;
use crate::versioned_db::pe_owner_tree::latest_tree_source_is_pe_owner;
use aios_core::RefnoEnum;
use aios_core::query_provider::*;
use aios_core::tool::db_tool::db1_hash;
use aios_core::tree_query::{TreeQueryFilter, TreeQueryOptions};
use aios_core::types::{NamedAttrMap as NamedAttMap, SPdmsElement as PE};
use anyhow::Context;
use async_trait::async_trait;
use once_cell::sync::OnceCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// 全局查询提供者实例
static GLOBAL_PROVIDER: OnceCell<Arc<dyn QueryProvider>> = OnceCell::new();

/// 获取用于模型生成的查询提供者
///
pub async fn get_model_query_provider() -> anyhow::Result<Arc<dyn QueryProvider>> {
    if let Some(provider) = GLOBAL_PROVIDER.get() {
        return Ok(provider.clone());
    }

    let provider = init_provider().await?;
    let _ = GLOBAL_PROVIDER.set(provider.clone());
    Ok(provider)
}

// ============================================================================
// specs/023 M2/T6：PeOwnerSnapshotProvider（层级查询走 per-run pe 快照）
// ============================================================================

/// pe_owner 快照查询提供者：层级查询走 `versioned_db::pe_owner_snapshot`
/// （per-dbnum 按需加载、run 级失效），其余（PE/属性/类型查询）委托 SurrealDB——
/// 与 `TreeIndexQueryProvider` 的委托结构完全同构。
struct PeOwnerSnapshotProvider {
    name: String,
    surreal_provider: SurrealQueryProvider,
}

impl PeOwnerSnapshotProvider {
    fn new() -> anyhow::Result<Self> {
        Ok(Self {
            name: "PeOwnerSnapshot".to_string(),
            surreal_provider: SurrealQueryProvider::new()
                .map_err(|e| anyhow::anyhow!("初始化 SurrealQueryProvider 失败: {e}"))?,
        })
    }

    /// 解析 refno 所属快照；dbnum 解析失败/快照缺该节点时返回 None（对齐
    /// TreeIndexQueryProvider::find_index 的"找不到 → 空结果"语义）。
    async fn snapshot_for(
        &self,
        refno: RefnoEnum,
    ) -> QueryResult<Option<Arc<crate::versioned_db::pe_owner_snapshot::PeDbnumSnapshot>>> {
        let Ok(dbnum) = TreeIndexManager::resolve_dbnum_for_refno(refno) else {
            return Ok(None);
        };
        let snap = get_or_load_pe_snapshot(dbnum)
            .await
            .map_err(|e| QueryError::ExecutionError(format!("加载 pe 快照失败 dbnum={dbnum}: {e}")))?;
        Ok(Some(snap))
    }

    fn build_filter(nouns: &[&str]) -> TreeQueryFilter {
        let noun_hashes = if nouns.is_empty() {
            None
        } else {
            Some(nouns.iter().map(|n| db1_hash(n)).collect())
        };
        TreeQueryFilter {
            noun_hashes,
            ..Default::default()
        }
    }

    fn descendants_options(
        nouns: &[&str],
        max_depth: Option<usize>,
        include_self: bool,
    ) -> TreeQueryOptions {
        TreeQueryOptions {
            include_self,
            max_depth,
            filter: Self::build_filter(nouns),
            prune_on_match: false,
        }
    }
}

#[async_trait]
impl HierarchyQuery for PeOwnerSnapshotProvider {
    async fn get_children(&self, refno: RefnoEnum) -> QueryResult<Vec<RefnoEnum>> {
        let Some(snap) = self.snapshot_for(refno).await? else {
            return Ok(Vec::new());
        };
        Ok(snap
            .collect_children(refno.refno(), &TreeQueryFilter::default())
            .into_iter()
            .map(RefnoEnum::from)
            .collect())
    }

    async fn get_descendants(
        &self,
        refno: RefnoEnum,
        max_depth: Option<usize>,
    ) -> QueryResult<Vec<RefnoEnum>> {
        let Some(snap) = self.snapshot_for(refno).await? else {
            return Ok(Vec::new());
        };
        let options = Self::descendants_options(&[], max_depth, false);
        Ok(snap
            .collect_descendants_bfs(refno.refno(), &options)
            .into_iter()
            .map(RefnoEnum::from)
            .collect())
    }

    async fn get_ancestors(&self, refno: RefnoEnum) -> QueryResult<Vec<RefnoEnum>> {
        let Some(snap) = self.snapshot_for(refno).await? else {
            return Ok(Vec::new());
        };
        let options = TreeQueryOptions {
            include_self: false,
            max_depth: None,
            filter: TreeQueryFilter::default(),
            prune_on_match: false,
        };
        Ok(snap
            .collect_ancestors_root_to_parent(refno.refno(), &options)
            .into_iter()
            .map(RefnoEnum::from)
            .collect())
    }

    async fn get_ancestors_of_type(
        &self,
        refno: RefnoEnum,
        nouns: &[&str],
    ) -> QueryResult<Vec<RefnoEnum>> {
        let Some(snap) = self.snapshot_for(refno).await? else {
            return Ok(Vec::new());
        };
        let options = TreeQueryOptions {
            include_self: false,
            max_depth: None,
            filter: Self::build_filter(nouns),
            prune_on_match: false,
        };
        Ok(snap
            .collect_ancestors_root_to_parent(refno.refno(), &options)
            .into_iter()
            .map(RefnoEnum::from)
            .collect())
    }

    async fn get_descendants_filtered(
        &self,
        refno: RefnoEnum,
        nouns: &[&str],
        max_depth: Option<usize>,
    ) -> QueryResult<Vec<RefnoEnum>> {
        let Some(snap) = self.snapshot_for(refno).await? else {
            return Ok(Vec::new());
        };
        let options = Self::descendants_options(nouns, max_depth, false);
        Ok(snap
            .collect_descendants_bfs(refno.refno(), &options)
            .into_iter()
            .map(RefnoEnum::from)
            .collect())
    }

    async fn get_children_pes(&self, refno: RefnoEnum) -> QueryResult<Vec<PE>> {
        self.surreal_provider.get_children_pes(refno).await
    }
}

#[async_trait]
impl TypeQuery for PeOwnerSnapshotProvider {
    async fn query_by_type(
        &self,
        nouns: &[&str],
        dbnum: i32,
        has_children: Option<bool>,
    ) -> QueryResult<Vec<RefnoEnum>> {
        self.surreal_provider
            .query_by_type(nouns, dbnum, has_children)
            .await
    }

    async fn query_by_type_name_contains(
        &self,
        nouns: &[&str],
        dbnum: i32,
        keyword: &str,
        case_sensitive: bool,
    ) -> QueryResult<Vec<RefnoEnum>> {
        self.surreal_provider
            .query_by_type_name_contains(nouns, dbnum, keyword, case_sensitive)
            .await
    }

    async fn query_by_type_multi_db(
        &self,
        nouns: &[&str],
        dbnums: &[i32],
    ) -> QueryResult<Vec<RefnoEnum>> {
        self.surreal_provider
            .query_by_type_multi_db(nouns, dbnums)
            .await
    }

    async fn get_world(&self, dbnum: i32) -> QueryResult<Option<RefnoEnum>> {
        self.surreal_provider.get_world(dbnum).await
    }

    async fn get_sites(&self, dbnum: i32) -> QueryResult<Vec<RefnoEnum>> {
        self.surreal_provider.get_sites(dbnum).await
    }

    async fn count_by_type(&self, noun: &str, dbnum: i32) -> QueryResult<usize> {
        self.surreal_provider.count_by_type(noun, dbnum).await
    }
}

#[async_trait]
impl BatchQuery for PeOwnerSnapshotProvider {
    async fn get_pes_batch(&self, refnos: &[RefnoEnum]) -> QueryResult<Vec<PE>> {
        self.surreal_provider.get_pes_batch(refnos).await
    }

    async fn get_attmaps_batch(&self, refnos: &[RefnoEnum]) -> QueryResult<Vec<NamedAttMap>> {
        self.surreal_provider.get_attmaps_batch(refnos).await
    }

    async fn get_full_names_batch(
        &self,
        refnos: &[RefnoEnum],
    ) -> QueryResult<Vec<(RefnoEnum, String)>> {
        self.surreal_provider.get_full_names_batch(refnos).await
    }
}

#[async_trait]
impl GraphQuery for PeOwnerSnapshotProvider {
    async fn query_multi_descendants(
        &self,
        refnos: &[RefnoEnum],
        nouns: &[&str],
        include_self: bool,
    ) -> QueryResult<Vec<RefnoEnum>> {
        if refnos.is_empty() {
            return Ok(Vec::new());
        }
        // 与 TreeIndexQueryProvider 一致：按输入顺序拼接每个 root 的 BFS 结果。
        let options = Self::descendants_options(nouns, None, include_self);
        let mut result = Vec::new();
        for &refno in refnos {
            let Some(snap) = self.snapshot_for(refno).await? else {
                continue;
            };
            result.extend(snap.collect_descendants_bfs(refno.refno(), &options));
        }
        Ok(result.into_iter().map(RefnoEnum::from).collect())
    }

    async fn find_shortest_path(
        &self,
        from: RefnoEnum,
        to: RefnoEnum,
    ) -> QueryResult<Vec<RefnoEnum>> {
        self.surreal_provider.find_shortest_path(from, to).await
    }

    async fn get_node_depth(&self, refno: RefnoEnum) -> QueryResult<usize> {
        let Some(snap) = self.snapshot_for(refno).await? else {
            return Ok(0);
        };
        let options = TreeQueryOptions {
            include_self: false,
            max_depth: None,
            filter: TreeQueryFilter::default(),
            prune_on_match: false,
        };
        Ok(snap
            .collect_ancestors_root_to_parent(refno.refno(), &options)
            .len())
    }
}

#[async_trait]
impl QueryProvider for PeOwnerSnapshotProvider {
    async fn get_pe(&self, refno: RefnoEnum) -> QueryResult<Option<PE>> {
        self.surreal_provider.get_pe(refno).await
    }

    async fn get_attmap(&self, refno: RefnoEnum) -> QueryResult<Option<NamedAttMap>> {
        self.surreal_provider.get_attmap(refno).await
    }

    async fn exists(&self, refno: RefnoEnum) -> QueryResult<bool> {
        self.surreal_provider.exists(refno).await
    }

    fn provider_name(&self) -> &str {
        &self.name
    }

    async fn health_check(&self) -> QueryResult<bool> {
        self.surreal_provider.health_check().await
    }
}

/// 初始化查询提供者（specs/023 M2：`AIOS_TREE_QUERY_SOURCE` 双源开关，M4 删除 tree 分支）
async fn init_provider() -> anyhow::Result<Arc<dyn QueryProvider>> {
    if latest_tree_source_is_pe_owner() {
        log::info!("使用 PeOwnerSnapshot 查询提供者（层级查询走 pe 快照，数据源 SurrealDB）");
        return Ok(Arc::new(PeOwnerSnapshotProvider::new()?));
    }

    log::info!("使用 TreeIndex 查询提供者（层级查询走 indextree；AIOS_TREE_QUERY_SOURCE=tree 回退路径）");

    let tree_dir = TreeIndexManager::with_default_dir(Vec::new())
        .tree_dir()
        .to_path_buf();

    // 检查 tree 目录是否存在且包含 .tree 文件
    let tree_files_exist = tree_dir.exists() && has_tree_files(&tree_dir);

    if !tree_files_exist {
        // tree 目录不存在或为空，自动运行解析生成
        print_tree_index_missing_help(&tree_dir);
        println!("🔄 Tree 索引缺失，正在自动解析 PDMS 数据库生成...");

        if let Err(e) = auto_generate_tree_index_by_parse(&tree_dir).await {
            anyhow::bail!(
                "Tree 索引自动生成失败: {}\n\
                 请检查 DbOption.toml 配置是否正确，PDMS 数据库文件是否存在",
                e
            );
        }

        // 再次检查是否生成成功
        if !has_tree_files(&tree_dir) {
            anyhow::bail!("Tree 索引生成后仍无 .tree 文件，请检查解析日志");
        }

        println!("✅ Tree 索引生成完成");
    }

    // 在 Windows 上，加载/反序列化较大的 `.tree` 文件时可能触发主线程栈溢出；
    // 这里用大栈线程执行初始化，避免 `STATUS_STACK_OVERFLOW` 直接杀进程。
    let tree_dir_clone = tree_dir.clone();
    let handle = std::thread::Builder::new()
        .name("tree-index-loader".to_string())
        .stack_size(64 * 1024 * 1024)
        .spawn(move || TreeIndexQueryProvider::from_tree_dir(tree_dir_clone))
        .context("创建 tree-index-loader 线程失败")?;

    let provider = handle
        .join()
        .map_err(|_| anyhow::anyhow!("tree-index-loader 线程 panic（可能由栈溢出导致）"))??;
    Ok(Arc::new(provider))
}

/// 检查目录中是否有 .tree 文件
fn has_tree_files(tree_dir: &std::path::Path) -> bool {
    if let Ok(entries) = std::fs::read_dir(tree_dir) {
        for entry in entries.flatten() {
            if let Some(ext) = entry.path().extension() {
                if ext == "tree" {
                    return true;
                }
            }
        }
    }
    false
}

/// 打印 tree 索引缺失的帮助信息
fn print_tree_index_missing_help(tree_dir: &std::path::Path) {
    eprintln!(
        r#"
╔══════════════════════════════════════════════════════════════════════════════╗
║  ⚠️  Tree 索引目录不存在                                                       ║
╠══════════════════════════════════════════════════════════════════════════════╣
║  缺失目录: {}
╠══════════════════════════════════════════════════════════════════════════════╣
║  Tree 索引文件用于快速查询节点的层级关系（父子、祖先、子孙）。                    ║
║  该文件在解析 PDMS 数据库时自动生成。                                           ║
╠══════════════════════════════════════════════════════════════════════════════╣
║  解决方案: 运行数据库解析命令                                                   ║
║                                                                               ║
║    cargo run --bin aios-database -- -c DbOption                              ║
║                                                                               ║
║  该命令会解析 PDMS 数据库文件并自动生成 tree 索引。                              ║
╚══════════════════════════════════════════════════════════════════════════════╝
"#,
        tree_dir.display()
    );
}

/// 自动通过解析 PDMS 数据库生成 Tree 索引
async fn auto_generate_tree_index_by_parse(tree_dir: &std::path::Path) -> anyhow::Result<()> {
    use crate::versioned_db::database::sync_pdms;

    // 从 DbOption.toml 加载配置
    let db_option = load_db_option_for_parse()?;

    println!("📂 解析项目: {}", db_option.project_name);
    println!("📁 输出目录: {}", tree_dir.display());

    // 确保输出目录存在
    std::fs::create_dir_all(tree_dir)?;

    // 运行解析
    sync_pdms(&db_option).await?;

    Ok(())
}

/// 加载用于解析的 DbOption 配置
fn load_db_option_for_parse() -> anyhow::Result<aios_core::options::DbOption> {
    use aios_core::options::DbOption;

    // 通过环境变量或默认路径加载配置
    let config_name =
        std::env::var("DB_OPTION_FILE").unwrap_or_else(|_| "db_options/DbOption".to_string());
    let config_path = format!("{}.toml", config_name);
    if std::path::Path::new(&config_path).exists() {
        let content = std::fs::read_to_string(&config_path)?;
        let mut db_option: DbOption = toml::from_str(&content)
            .map_err(|e| anyhow::anyhow!("解析 {} 失败: {}", config_path, e))?;

        // 设置解析模式参数
        db_option.save_db = Some(false); // 不写入 SurrealDB，仅生成本地文件
        db_option.total_sync = true; // 全量解析

        Ok(db_option)
    } else {
        anyhow::bail!("未找到配置文件 {}", config_path)
    }
}

// ============================================================================
// 便捷查询函数 (替换 fast_model/query.rs 中的现有函数)
// ============================================================================

/// 查询深层子孙节点并按类型过滤
///
/// # 参数
/// - `root`: 根节点 refno
/// - `nouns`: 要过滤的类型列表
/// - `max_depth`: 最大递归深度 (已忽略，保持兼容性)
///
/// # 示例
///
/// ```rust,ignore
/// // 查询 ZONE 下所有 EQUI 和 PIPE
/// let equips = get_descendants_by_types(
///     zone_refno,
///     &["EQUI", "PIPE"],
///     Some(12)
/// ).await?;
/// ```
///
/// # 注意
/// **已废弃**: 请使用 `aios_core::collect_descendant_filter_ids(&[root], nouns)` 代替
///
/// `max_depth` 参数已被忽略，因为 `collect_descendant_filter_ids` 会查询所有深度的子孙节点。
#[deprecated(
    since = "0.1.0",
    note = "使用 aios_core::collect_descendant_filter_ids(&[root], nouns, None) 代替"
)]
pub async fn get_descendants_by_types(
    root: RefnoEnum,
    nouns: &[&str],
    _max_depth: Option<usize>, // 参数保留以保持兼容性，但已忽略
) -> anyhow::Result<Vec<RefnoEnum>> {
    let provider = get_model_query_provider().await?;
    provider
        .get_descendants_filtered(root, nouns, None)
        .await
        .map_err(Into::into)
}

/// 批量获取子节点
///
/// # 参数
/// - `refnos`: 父节点 refno 列表
///
/// # 返回
/// 所有父节点的子节点 refno 列表 (去重)
///
/// # 注意
/// **已废弃**: 请使用 `aios_core::collect_descendant_filter_ids(refnos, &[])` 代替
///
/// 此函数现在直接调用 `collect_descendant_filter_ids`，传入空的 noun 过滤器表示查询所有子节点。
#[deprecated(
    since = "0.1.0",
    note = "使用 aios_core::collect_descendant_filter_ids(refnos, &[], None) 代替"
)]
pub async fn get_children_batch(refnos: &[RefnoEnum]) -> anyhow::Result<Vec<RefnoEnum>> {
    let provider = get_model_query_provider().await?;
    provider
        .query_multi_descendants(refnos, &[], false)
        .await
        .map_err(Into::into)
}

/// 查询指定类型的节点
///
/// # 参数
/// - `nouns`: 类型列表
/// - `dbnum`: 数据库编号
/// - `has_children`: 是否过滤有子节点的元素
///
/// # 示例
///
/// ```rust,ignore
/// // 查询 1112 数据库中所有 ZONE
/// let zones = query_by_type(&["ZONE"], 1112, None).await?;
///
/// // 查询有子节点的 ZONE
/// let parent_zones = query_by_type(&["ZONE"], 1112, Some(true)).await?;
/// ```
pub async fn query_by_type(
    nouns: &[&str],
    dbnum: i32,
    has_children: Option<bool>,
) -> anyhow::Result<Vec<RefnoEnum>> {
    let provider = get_model_query_provider().await?;
    provider
        .query_by_type(nouns, dbnum, has_children)
        .await
        .map_err(Into::into)
}

/// 按 Noun 全库查询（Full Noun 模式专用）
///
/// 直接按 Noun 类型查询全库范围内的所有实例，不加 dbnum 或 refno 层级约束。
///
/// # 参数
/// - `nouns`: Noun 类型列表（如 ["EQUI", "FITT", "BOX"]）
///
/// # 返回
/// 全库范围内所有匹配 Noun 的 refno 列表
///
/// # 示例
///
/// ```rust,ignore
/// // 查询全库所有 EQUI 和 FITT
/// let refnos = query_by_noun_all_db(&["EQUI", "FITT"])?;
/// ```
///
/// # 实现说明
///
/// specs/023 M2：双源——pe_owner（默认，HierView 快照）| tree（TreeIndexManager 回退）。
pub async fn query_by_noun_all_db(nouns: &[&str]) -> anyhow::Result<Vec<RefnoEnum>> {
    if nouns.is_empty() {
        return Ok(Vec::new());
    }
    let dbnums = resolve_tree_dbnums()?;
    let view = crate::fast_model::gen_model::hier_view::HierView::load(dbnums).await?;
    let mut seen = HashSet::new();
    let mut refnos = Vec::new();
    for noun in nouns {
        for refno in view.query_noun_refnos(noun, None) {
            if refno.is_valid() && seen.insert(refno) {
                refnos.push(refno);
            }
        }
    }
    Ok(refnos)
}

/// 统计指定 noun 在全库范围内的实例数量
pub async fn count_noun_all_db(noun: &str) -> anyhow::Result<u64> {
    if noun.is_empty() {
        return Ok(0);
    }
    let dbnums = resolve_tree_dbnums()?;
    let view = crate::fast_model::gen_model::hier_view::HierView::load(dbnums).await?;
    let mut refnos = view.query_noun_refnos(noun, None);
    refnos.retain(|r| r.is_valid());
    Ok(refnos.len() as u64)
}

/// 根据分页参数获取指定 noun 的 refno 列表
pub async fn query_noun_page_all_db(
    noun: &str,
    start: usize,
    limit: usize,
) -> anyhow::Result<Vec<RefnoEnum>> {
    if noun.is_empty() || limit == 0 {
        return Ok(Vec::new());
    }
    let dbnums = resolve_tree_dbnums()?;
    let view = crate::fast_model::gen_model::hier_view::HierView::load(dbnums).await?;
    let mut refnos = view.query_noun_refnos(noun, None);
    refnos.retain(|r| r.is_valid());
    if start >= refnos.len() {
        return Ok(Vec::new());
    }
    let end = (start + limit).min(refnos.len());
    Ok(refnos[start..end].to_vec())
}

fn resolve_tree_dbnums() -> anyhow::Result<Vec<u32>> {
    db_meta().ensure_loaded()?;
    let mut dbnums = db_meta().get_all_dbnums();
    if dbnums.is_empty() {
        anyhow::bail!("db_meta_info.json 中未找到可用 dbnum");
    }
    dbnums.sort_unstable();
    Ok(dbnums)
}

/// 批量获取 PE 信息
///
/// # 参数
/// - `refnos`: refno 列表
///
/// # 返回
/// PE 列表 (保持顺序，如果某个 refno 不存在则跳过)
pub async fn get_pes_batch(refnos: &[RefnoEnum]) -> anyhow::Result<Vec<PE>> {
    let provider = get_model_query_provider().await?;
    provider.get_pes_batch(refnos).await.map_err(Into::into)
}

/// 获取单个 PE 信息
///
/// # 参数
/// - `refno`: PE 的 refno
///
/// # 返回
/// PE 信息，如果不存在返回 None
pub async fn get_pe(refno: RefnoEnum) -> anyhow::Result<Option<PE>> {
    let provider = get_model_query_provider().await?;
    provider.get_pe(refno).await.map_err(Into::into)
}

/// 获取直接子节点
///
/// # 参数
/// - `refno`: 父节点 refno
///
/// # 返回
/// 子节点的 refno 列表
pub async fn get_children(refno: RefnoEnum) -> anyhow::Result<Vec<RefnoEnum>> {
    let provider = get_model_query_provider().await?;
    provider.get_children(refno).await.map_err(Into::into)
}

/// 查询所有祖先节点
///
/// # 参数
/// - `refno`: 子节点 refno
///
/// # 返回
/// 祖先节点 refno 列表 (从直接父节点到根节点)
pub async fn get_ancestors(refno: RefnoEnum) -> anyhow::Result<Vec<RefnoEnum>> {
    let provider = get_model_query_provider().await?;
    provider.get_ancestors(refno).await.map_err(Into::into)
}

/// 查询特定类型的祖先
///
/// # 参数
/// - `refno`: 子节点 refno
/// - `nouns`: 要过滤的类型列表
///
/// # 返回
/// 匹配类型的祖先节点 refno 列表
pub async fn get_ancestors_of_type(
    refno: RefnoEnum,
    nouns: &[&str],
) -> anyhow::Result<Vec<RefnoEnum>> {
    let provider = get_model_query_provider().await?;
    provider
        .get_ancestors_of_type(refno, nouns)
        .await
        .map_err(Into::into)
}

/// 获取子节点的完整 PE 信息
///
/// # 参数
/// - `refno`: 父节点 refno
///
/// # 返回
/// 子节点的完整 PE 列表
pub async fn get_children_pes(refno: RefnoEnum) -> anyhow::Result<Vec<PE>> {
    let provider = get_model_query_provider().await?;
    provider.get_children_pes(refno).await.map_err(Into::into)
}

/// 批量获取属性映射
///
/// # 参数
/// - `refnos`: refno 列表
///
/// # 返回
/// NamedAttMap 列表
pub async fn get_attmaps_batch(refnos: &[RefnoEnum]) -> anyhow::Result<Vec<NamedAttMap>> {
    let provider = get_model_query_provider().await?;
    provider.get_attmaps_batch(refnos).await.map_err(Into::into)
}

/// 多起点、多类型的深层子孙查询
///
/// # 参数
/// - `refnos`: 起点节点列表
/// - `nouns`: 要过滤的类型列表
///
/// # 返回
/// 匹配条件的 refno 列表
///
/// # 注意
/// **已废弃**: 请直接使用 `aios_core::collect_descendant_filter_ids(refnos, nouns)` 代替
///
/// 此函数现在直接调用 `collect_descendant_filter_ids`，未来版本将移除。
#[deprecated(
    since = "0.1.0",
    note = "使用 aios_core::collect_descendant_filter_ids(refnos, nouns, None) 代替"
)]
pub async fn query_multi_descendants(
    refnos: &[RefnoEnum],
    nouns: &[&str],
) -> anyhow::Result<Vec<RefnoEnum>> {
    query_multi_descendants_with_self(refnos, nouns, false).await
}

/// 多起点、多类型的深层子孙查询（支持 include_self 参数）
///
/// # 参数
/// - `refnos`: 起点节点列表
/// - `nouns`: 要过滤的类型列表
/// - `include_self`: 是否包含起点节点本身（如果符合类型过滤条件）
///
/// # 返回
/// 匹配条件的 refno 列表
pub async fn query_multi_descendants_with_self(
    refnos: &[RefnoEnum],
    nouns: &[&str],
    include_self: bool,
) -> anyhow::Result<Vec<RefnoEnum>> {
    if refnos.is_empty() {
        return Ok(Vec::new());
    }

    let mut root_dbnums: Vec<(RefnoEnum, u32)> = Vec::with_capacity(refnos.len());
    for &root in refnos {
        let dbnum = TreeIndexManager::resolve_dbnum_for_refno(root)?;
        root_dbnums.push((root, dbnum));
    }

    let noun_hashes: Option<HashSet<u32>> = if nouns.is_empty() {
        None
    } else {
        Some(nouns.iter().map(|&n| db1_hash(n)).collect())
    };
    let options = TreeQueryOptions {
        include_self,
        max_depth: None,
        filter: TreeQueryFilter {
            noun_hashes,
            ..Default::default()
        },
        prune_on_match: false,
    };

    let mut out: Vec<RefnoEnum> = Vec::new();
    let mut seen: HashSet<RefnoEnum> = HashSet::new();

    // specs/023 M2：pe_owner 主路径走 per-dbnum 快照（数据源 SurrealDB，增量后新鲜）。
    if latest_tree_source_is_pe_owner() {
        let mut snap_cache: HashMap<
            u32,
            Arc<crate::versioned_db::pe_owner_snapshot::PeDbnumSnapshot>,
        > = HashMap::new();
        for (root, dbnum) in root_dbnums {
            let snap = match snap_cache.get(&dbnum) {
                Some(s) => s.clone(),
                None => {
                    let s = get_or_load_pe_snapshot(dbnum)
                        .await
                        .with_context(|| format!("加载 pe 快照失败 dbnum={dbnum}"))?;
                    snap_cache.insert(dbnum, s.clone());
                    s
                }
            };
            for r in snap.collect_descendants_bfs(root.refno(), &options) {
                let r = RefnoEnum::from(r);
                if r.is_valid() && seen.insert(r) {
                    out.push(r);
                }
            }
        }
        return Ok(out);
    }

    // tree 回退：cache-only 语义——tree 文件缺失直接报错，不自动生成。
    let tree_dir = TreeIndexManager::with_default_dir(Vec::new())
        .tree_dir()
        .to_path_buf();
    let mut index_cache: HashMap<u32, Arc<aios_core::tree_query::TreeIndex>> = HashMap::new();

    for (root, dbnum) in root_dbnums {
        let index = match index_cache.get(&dbnum) {
            Some(idx) => idx.clone(),
            None => {
                let idx = load_index_with_large_stack(&tree_dir, dbnum).with_context(|| {
                    format!("加载 TreeIndex 失败: {}/{}.tree", tree_dir.display(), dbnum)
                })?;
                index_cache.insert(dbnum, idx.clone());
                idx
            }
        };

        for r in index.collect_descendants_bfs(root.refno(), &options) {
            let r = RefnoEnum::from(r);
            if r.is_valid() && seen.insert(r) {
                out.push(r);
            }
        }
    }

    Ok(out)
}

// ============================================================================
// 诊断和调试函数
// ============================================================================

/// 获取当前使用的查询提供者名称
///
/// 用于调试和日志输出
pub async fn get_provider_name() -> String {
    match get_model_query_provider().await {
        Ok(provider) => provider.provider_name().to_string(),
        Err(_) => "未初始化".to_string(),
    }
}

/// 健康检查
///
/// 检查数据库连接是否正常
pub async fn health_check() -> anyhow::Result<bool> {
    let provider = get_model_query_provider().await?;
    provider.health_check().await.map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_provider_initialization() {
        let provider = get_model_query_provider().await;
        assert!(provider.is_ok());
    }

    #[tokio::test]
    async fn test_provider_name() {
        let name = get_provider_name().await;
        assert!(!name.is_empty());
        println!("当前查询提供者: {}", name);
    }
}
