//! 查询兼容层 - 提供与旧 API 兼容的查询函数
//!
//! 此模块提供与原 aios_core 查询函数签名完全兼容的封装，
//! 使得迁移到新的 query_provider 更加平滑。
//!
//! ## 使用方式
//!
//! ```rust,ignore
//! // 旧代码
//! use aios_core::{query_type_refnos_by_dbnum, query_multi_children_refnos};
//!
//! // 新代码 - 只需改变 import
//! use crate::fast_model::query_compat::{query_type_refnos_by_dbnum, query_multi_children_refnos};
//! ```

use crate::fast_model::gen_model::tree_index_manager::{
    TreeIndexManager, load_index_with_large_stack,
};
use crate::fast_model::query_provider;
use aios_core::RefnoEnum;
use aios_core::pdms_types::{TOTAL_NEG_NOUN_NAMES, VISBILE_GEO_NOUNS};
use aios_core::tool::db_tool::db1_hash;
use aios_core::tree_query::{TreeIndex, TreeQuery, TreeQueryFilter, TreeQueryOptions};
use aios_core::types::{NamedAttrMap as NamedAttMap, SPdmsElement as PE};
use once_cell::sync::Lazy;
use std::collections::HashSet;
use std::sync::Arc;

static VISIBLE_GEO_NOUN_HASHES: Lazy<HashSet<u32>> = Lazy::new(|| {
    VISBILE_GEO_NOUNS
        .iter()
        .map(|&name| db1_hash(name))
        .collect()
});
static NEG_GEO_NOUN_HASHES: Lazy<HashSet<u32>> = Lazy::new(|| {
    TOTAL_NEG_NOUN_NAMES
        .iter()
        .map(|&name| db1_hash(name))
        .collect()
});
static BRAN_HASH: Lazy<u32> = Lazy::new(|| db1_hash("BRAN"));
static HANG_HASH: Lazy<u32> = Lazy::new(|| db1_hash("HANG"));
const BRAN_HANG_NOUNS: &[&str] = &["BRAN", "HANG"];

async fn load_tree_index_for_refno(refno: RefnoEnum) -> anyhow::Result<Arc<TreeIndex>> {
    let tree_dir = TreeIndexManager::with_default_dir(Vec::new())
        .tree_dir()
        .to_path_buf();
    let dbnum = TreeIndexManager::resolve_dbnum_for_refno(refno)?;

    // 大栈线程加载，避免 Windows 反序列化大 `.tree` 文件时触发栈溢出。
    load_index_with_large_stack(&tree_dir, dbnum)
}

fn build_noun_hashes(nouns: &[&str]) -> Option<HashSet<u32>> {
    if nouns.is_empty() {
        None
    } else {
        Some(nouns.iter().map(|n| db1_hash(n)).collect())
    }
}

fn parse_max_depth(range_str: Option<&str>) -> Option<usize> {
    let range = range_str?;
    if range.is_empty() || range == ".." {
        return None;
    }
    if let Some((_, end)) = range.split_once("..") {
        if end.is_empty() {
            return None;
        }
        return end.parse::<usize>().ok();
    }
    range.parse::<usize>().ok()
}

async fn query_descendants_bfs(
    refno: RefnoEnum,
    noun_hashes: Option<HashSet<u32>>,
    include_self: bool,
    range_str: Option<&str>,
) -> anyhow::Result<Vec<RefnoEnum>> {
    let index = load_tree_index_for_refno(refno).await?;
    let options = TreeQueryOptions {
        include_self,
        max_depth: parse_max_depth(range_str),
        filter: TreeQueryFilter {
            noun_hashes,
            ..Default::default()
        },
        prune_on_match: false,
    };
    let descendants = index.query_descendants_bfs(refno.refno(), options).await?;
    Ok(descendants.into_iter().map(RefnoEnum::from).collect())
}

async fn query_children_filtered(
    refno: RefnoEnum,
    noun_hashes: Option<HashSet<u32>>,
) -> anyhow::Result<Vec<RefnoEnum>> {
    let index = load_tree_index_for_refno(refno).await?;
    let filter = TreeQueryFilter {
        noun_hashes,
        ..Default::default()
    };
    let children = index.query_children(refno.refno(), filter).await?;
    Ok(children.into_iter().map(RefnoEnum::from).collect())
}

fn sort_dedup_refnos(mut refnos: Vec<RefnoEnum>) -> Vec<RefnoEnum> {
    refnos.sort();
    refnos.dedup();
    refnos
}

/// 按类型查询 refno (兼容旧 API)
///
/// # 参数
/// - `nouns`: 类型列表
/// - `dbnum`: 数据库编号
/// - `has_children`: 是否过滤有子节点的元素
/// - `_include_history`: 是否包含历史数据 (目前忽略,保持兼容性)
///
/// # 示例
///
/// ```rust,ignore
/// // 查询所有 SITE
/// let sites = query_type_refnos_by_dbnum(&["SITE"], 1112, None, false).await?;
///
/// // 查询有子节点的 ZONE
/// let zones = query_type_refnos_by_dbnum(&["ZONE"], 1112, Some(true), false).await?;
/// ```
pub async fn query_type_refnos_by_dbnum(
    nouns: &[&str],
    dbnum: u32,
    has_children: Option<bool>,
    _include_history: bool, // 暂时忽略历史数据参数
) -> anyhow::Result<Vec<RefnoEnum>> {
    query_provider::query_by_type(nouns, dbnum as i32, has_children).await
}

/// 批量查询多个节点的所有子节点 (兼容旧 API)
///
/// # 参数
/// - `refnos`: 父节点 refno 列表
///
/// # 返回
/// 所有父节点的子节点 refno 列表 (去重)
///
/// # 示例
///
/// ```rust,ignore
/// let children = query_multi_children_refnos(&[zone1, zone2, zone3]).await?;
/// ```
///
/// # 注意
/// **已废弃**: 请使用 `aios_core::collect_descendant_filter_ids(refnos, &[])` 代替
#[deprecated(
    since = "0.1.0",
    note = "使用 aios_core::collect_descendant_filter_ids(refnos, &[], None) 代替"
)]
pub async fn query_multi_children_refnos(refnos: &[RefnoEnum]) -> anyhow::Result<Vec<RefnoEnum>> {
    query_provider::query_multi_descendants(refnos, &[]).await
}

/// 查询使用特定 CATE 的 refno (兼容旧 API)
///
/// # 参数
/// - `cate_names`: CATE 名称列表
/// - `dbnum`: 数据库编号
/// - `_include_history`: 是否包含历史数据 (目前忽略)
///
/// # 注意
/// 这个函数需要更复杂的实现，暂时返回错误，需要后续完善
pub async fn query_use_cate_refnos_by_dbnum(
    cate_names: &[&str],
    dbnum: u32,
    _include_history: bool,
) -> anyhow::Result<Vec<RefnoEnum>> {
    // TODO: 需要实现 CATE 查询逻辑
    // 目前先调用原有的 aios_core 函数
    aios_core::query_use_cate_refnos_by_dbnum(cate_names, dbnum, _include_history).await
}

/// 获取子节点的 PE 信息 (兼容旧 API)
///
/// # 参数
/// - `refno`: 父节点 refno
///
/// # 返回
/// 子节点的完整 PE 列表
pub async fn get_children_pes(refno: RefnoEnum) -> anyhow::Result<Vec<PE>> {
    query_provider::get_children_pes(refno).await
}

/// 获取单个 PE 信息 (兼容旧 API)
pub async fn get_pe(refno: RefnoEnum) -> anyhow::Result<Option<PE>> {
    query_provider::get_pe(refno).await
}

/// 批量获取 PE 信息 (兼容旧 API)
pub async fn get_pes_batch(refnos: &[RefnoEnum]) -> anyhow::Result<Vec<PE>> {
    query_provider::get_pes_batch(refnos).await
}

/// 获取直接子节点 (兼容旧 API)
pub async fn get_children_refnos(refno: RefnoEnum) -> anyhow::Result<Vec<RefnoEnum>> {
    query_provider::get_children(refno).await
}

/// 查询可见几何子孙节点（模型生成路径：TreeIndex）
pub async fn query_visible_geo_descendants(
    refno: RefnoEnum,
    include_self: bool,
    range_str: Option<&str>,
) -> anyhow::Result<Vec<RefnoEnum>> {
    query_descendants_bfs(
        refno,
        Some(VISIBLE_GEO_NOUN_HASHES.clone()),
        include_self,
        range_str,
    )
    .await
}

/// 查询负实体几何子孙节点（模型生成路径：TreeIndex）
pub async fn query_negative_geo_descendants(
    refno: RefnoEnum,
    include_self: bool,
    range_str: Option<&str>,
) -> anyhow::Result<Vec<RefnoEnum>> {
    query_descendants_bfs(
        refno,
        Some(NEG_GEO_NOUN_HASHES.clone()),
        include_self,
        range_str,
    )
    .await
}

/// 查询深度可见实例（模型生成路径：TreeIndex）
pub async fn query_deep_visible_inst_refnos(refno: RefnoEnum) -> anyhow::Result<Vec<RefnoEnum>> {
    let index = load_tree_index_for_refno(refno).await?;
    let Some(meta) = index.get_node_meta(refno.refno()).await? else {
        return Ok(Vec::new());
    };

    let owner_hash = if meta.owner.is_unset() {
        None
    } else {
        index.get_node_meta(meta.owner).await?.map(|m| m.noun)
    };

    if owner_hash == Some(*BRAN_HASH) || owner_hash == Some(*HANG_HASH) {
        return Ok(vec![refno]);
    }

    if meta.noun == *BRAN_HASH || meta.noun == *HANG_HASH {
        return Ok(sort_dedup_refnos(
            query_children_filtered(refno, None).await?,
        ));
    }

    let mut out = query_visible_geo_descendants(refno, true, Some("..")).await?;
    let bran_hang_roots =
        query_descendants_bfs(refno, build_noun_hashes(BRAN_HANG_NOUNS), true, Some("..")).await?;

    for bran_hang_root in bran_hang_roots {
        out.push(bran_hang_root);
        out.extend(query_children_filtered(bran_hang_root, None).await?);
    }

    Ok(sort_dedup_refnos(out))
}

/// 查询深度负实例（模型生成路径：TreeIndex）
pub async fn query_deep_neg_inst_refnos(refno: RefnoEnum) -> anyhow::Result<Vec<RefnoEnum>> {
    query_filter_deep_children(refno, &TOTAL_NEG_NOUN_NAMES).await
}

/// 查询深层子孙节点 (兼容旧 API)
pub async fn query_deep_children_refnos(refno: RefnoEnum) -> anyhow::Result<Vec<RefnoEnum>> {
    query_descendants_bfs(refno, None, true, Some("..")).await
}

/// 查询过滤后的深层子孙 (兼容旧 API)
///
/// # 注意
/// **已废弃**: 请使用 `aios_core::collect_descendant_filter_ids(&[refno], nouns)` 代替
#[deprecated(
    since = "0.1.0",
    note = "使用 aios_core::collect_descendant_filter_ids(&[refno], nouns, None) 代替"
)]
pub async fn query_filter_deep_children(
    refno: RefnoEnum,
    nouns: &[&str],
) -> anyhow::Result<Vec<RefnoEnum>> {
    query_descendants_bfs(refno, build_noun_hashes(nouns), true, Some("..")).await
}

/// 查询祖先节点 (兼容旧 API)
pub async fn query_ancestor_refnos(refno: RefnoEnum) -> anyhow::Result<Vec<RefnoEnum>> {
    query_provider::get_ancestors(refno).await
}

/// 查询特定类型的祖先 (兼容旧 API)
pub async fn query_filter_ancestors(
    refno: RefnoEnum,
    nouns: &[&str],
) -> anyhow::Result<Vec<RefnoEnum>> {
    let index = load_tree_index_for_refno(refno).await?;
    let options = TreeQueryOptions {
        include_self: false,
        max_depth: None,
        filter: TreeQueryFilter {
            noun_hashes: build_noun_hashes(nouns),
            ..Default::default()
        },
        prune_on_match: false,
    };
    let ancestors = index
        .query_ancestors_root_to_parent(refno.refno(), options)
        .await?;
    Ok(ancestors.into_iter().map(RefnoEnum::from).collect())
}

/// 查询过滤后的深层子孙属性（模型生成路径：TreeIndex -> SurrealDB）
pub async fn query_filter_deep_children_atts(
    refno: RefnoEnum,
    nouns: &[&str],
) -> anyhow::Result<Vec<NamedAttMap>> {
    let refnos = query_filter_deep_children(refno, nouns).await?;
    query_provider::get_attmaps_batch(&refnos).await
}

/// 查询直接子节点（带类型过滤，模型生成路径：TreeIndex）
pub async fn collect_children_filter_ids(
    refno: RefnoEnum,
    nouns: &[&str],
) -> anyhow::Result<Vec<RefnoEnum>> {
    query_children_filtered(refno, build_noun_hashes(nouns)).await
}

// ============================================================================
// 便捷的重导出宏
// ============================================================================

/// 使用此宏可以快速替换所有查询导入
///
/// # 示例
///
/// ```rust,ignore
/// // 在文件开头添加
/// use crate::fast_model::query_compat::*;
///
/// // 然后所有查询函数就会自动使用新的 query_provider
/// ```

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_query_type_compatibility() {
        // 测试兼容性函数是否可以正常调用
        // 需要数据库连接才能运行
    }
}
