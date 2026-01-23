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
    ensure_tree_index_exists, load_index_with_large_stack, TreeIndexManager, enable_auto_generate_tree,
    is_auto_generate_tree_enabled,
};
use aios_core::RefnoEnum;
use aios_core::query_provider::*;
use aios_core::tool::db_tool::db1_hash;
use aios_core::tree_query::{TreeQuery, TreeQueryFilter, TreeQueryOptions};
use aios_core::types::{NamedAttrMap as NamedAttMap, SPdmsElement as PE};
use anyhow::Context;
use once_cell::sync::OnceCell;
use std::sync::Arc;
use std::collections::HashSet;

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

/// 初始化查询提供者
async fn init_provider() -> anyhow::Result<Arc<dyn QueryProvider>> {
    log::info!("使用 TreeIndex 查询提供者（层级查询走 indextree）");

    let tree_dir = TreeIndexManager::with_default_dir(Vec::new())
        .tree_dir()
        .to_path_buf();

    // 检查 tree 目录是否存在
    if !tree_dir.exists() {
        // tree 目录不存在，提示用户
        print_tree_index_missing_help(&tree_dir);

        // 尝试询问用户是否自动生成
        if should_auto_generate_tree_index() {
            enable_auto_generate_tree();
            log::info!("[init_provider] 用户选择自动生成 tree 索引文件");

            // 尝试从 SurrealDB 生成 tree 索引
            if let Err(e) = generate_all_tree_indices(&tree_dir).await {
                log::warn!("[init_provider] 自动生成 tree 索引失败: {}", e);
                anyhow::bail!("Tree 索引文件不存在且自动生成失败: {}", e);
            }
        } else {
            anyhow::bail!(
                "Tree 索引目录不存在: {}\n请先运行数据库解析命令生成 tree 索引文件",
                tree_dir.display()
            );
        }
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
║  解决方案:                                                                     ║
║                                                                               ║
║  方案1: 重新解析数据库（推荐）                                                  ║
║    cargo run --bin aios-database -- --parse-db                               ║
║                                                                               ║
║  方案2: 从 SurrealDB 重建 tree 索引                                            ║
║    cargo run --bin aios-database -- --rebuild-tree-index                     ║
╚══════════════════════════════════════════════════════════════════════════════╝
"#,
        tree_dir.display()
    );
}

/// 检查是否应该自动生成 tree 索引
///
/// 在交互式终端中询问用户，非交互式环境返回 false
fn should_auto_generate_tree_index() -> bool {
    use std::io::{self, Write};

    // 如果已经启用了自动生成，直接返回 true
    if is_auto_generate_tree_enabled() {
        return true;
    }

    // 检查环境变量是否禁用交互
    if std::env::var("CI").is_ok() || std::env::var("AIOS_NON_INTERACTIVE").is_ok() {
        log::info!("[init_provider] 非交互式环境，跳过用户确认");
        return false;
    }

    print!("\n是否从 SurrealDB 自动生成 tree 索引文件? [y/N]: ");
    io::stdout().flush().ok();

    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_ok() {
        let input = input.trim().to_lowercase();
        if input == "y" || input == "yes" {
            return true;
        }
    }

    false
}

/// 从 SurrealDB 生成所有 tree 索引
async fn generate_all_tree_indices(output_dir: &std::path::Path) -> anyhow::Result<()> {
    use crate::fast_model::gen_model::tree_index_manager::{
        get_available_dbnums_from_db, generate_tree_indices_from_db,
    };

    println!("🔄 正在从 SurrealDB 获取可用的 dbnum 列表...");

    let dbnums = get_available_dbnums_from_db().await?;
    if dbnums.is_empty() {
        anyhow::bail!("SurrealDB 中没有找到任何可用的 dbnum");
    }

    println!("📋 找到 {} 个 dbnum: {:?}", dbnums.len(), dbnums);
    println!("🔧 开始生成 tree 索引文件...");

    let success_count = generate_tree_indices_from_db(&dbnums, output_dir).await?;

    println!(
        "✅ Tree 索引生成完成: 成功 {}/{} 个",
        success_count,
        dbnums.len()
    );

    if success_count == 0 {
        anyhow::bail!("没有成功生成任何 tree 索引文件");
    }

    Ok(())
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
/// let refnos = query_by_noun_all_db(&["EQUI", "FITT"]).await?;
/// ```
///
/// # 实现说明
///
/// 此函数使用 TreeIndexManager 查询全库范围内的所有实例。
pub async fn query_by_noun_all_db(nouns: &[&str]) -> anyhow::Result<Vec<RefnoEnum>> {
    if nouns.is_empty() {
        return Ok(Vec::new());
    }
    let dbnums = resolve_tree_dbnums()?;
    let manager = TreeIndexManager::with_default_dir(dbnums);
    let mut seen = HashSet::new();
    let mut refnos = Vec::new();
    for noun in nouns {
        for refno in manager.query_noun_refnos(noun, None) {
            if refno.is_valid() && seen.insert(refno) {
                refnos.push(refno);
            }
        }
    }
    Ok(refnos)
}

/// 统计指定 noun 在全库范围内的实例数量（GROUP ALL + LIMIT 1）
pub async fn count_noun_all_db(noun: &str) -> anyhow::Result<u64> {
    if noun.is_empty() {
        return Ok(0);
    }
    let dbnums = resolve_tree_dbnums()?;
    let manager = TreeIndexManager::with_default_dir(dbnums);
    let mut refnos = manager.query_noun_refnos(noun, None);
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
    let manager = TreeIndexManager::with_default_dir(dbnums);
    let mut refnos = manager.query_noun_refnos(noun, None);
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

    // 按需生成缺失的 `{dbnum}.tree`，避免因 tree 缺失导致层级查询直接返回空结果。
    let tree_dir = TreeIndexManager::with_default_dir(Vec::new())
        .tree_dir()
        .to_path_buf();

    let mut root_dbnums: Vec<(RefnoEnum, u32)> = Vec::with_capacity(refnos.len());
    let mut unique_dbnums: HashSet<u32> = HashSet::new();
    for &root in refnos {
        let dbnum = TreeIndexManager::resolve_dbnum_for_refno(root).await?;
        root_dbnums.push((root, dbnum));
        if unique_dbnums.insert(dbnum) {
            ensure_tree_index_exists(dbnum, &tree_dir)
                .await
                .with_context(|| format!("按需生成 tree 索引失败: dbnum={}", dbnum))?;
        }
    }

    // 这里直接用 TreeIndex 查询（并在大栈线程加载 `.tree`），避免依赖全局 Provider 的初始化时机。
    let noun_hashes: Option<Vec<u32>> = if nouns.is_empty() {
        None
    } else {
        Some(nouns.iter().map(|&n| db1_hash(n)).collect())
    };

    let mut out: Vec<RefnoEnum> = Vec::new();
    let mut seen: HashSet<RefnoEnum> = HashSet::new();

    for (root, dbnum) in root_dbnums {
        let index = load_index_with_large_stack(&tree_dir, dbnum)
            .with_context(|| format!("加载 TreeIndex 失败: {}/{}.tree", tree_dir.display(), dbnum))?;

        let options = TreeQueryOptions {
            include_self,
            max_depth: None,
            filter: TreeQueryFilter {
                noun_hashes: noun_hashes.clone(),
                ..Default::default()
            },
        };

        let descendants = index
            .query_descendants_bfs(root.refno(), options)
            .await
            .with_context(|| format!("TreeIndex 查询子孙节点失败: root={}", root))?;

        for r in descendants {
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
