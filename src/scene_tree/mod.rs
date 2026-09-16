//! Scene Tree 模块
//!
//! 提供场景树的初始化、查询和更新功能。
//! 用于替代 inst_relate_aabb 表，统一管理模型生成状态和 AABB。

use aios_core::{SurrealQueryExt, project_primary_db};
use anyhow::Result;

pub mod init;
#[cfg(feature = "parquet-export")]
pub mod parquet_export;
pub mod query;
pub mod schema;

pub fn is_geo_noun(noun: &str) -> bool {
    init::is_geo_noun(noun)
}

// 重新导出常用类型和函数
pub use init::{
    SceneTreeInitResult, init_scene_tree, init_scene_tree_by_dbno, init_scene_tree_from_root,
};
#[cfg(feature = "parquet-export")]
pub use parquet_export::export_scene_tree_parquet;
pub use query::{
    SceneNodeStatus, filter_ungenerated_geo_nodes, mark_as_generated, query_ancestor_ids,
    query_children_ids, query_generated_refnos, query_generation_status, query_ungenerated_leaves,
    update_scene_node_aabb,
};
pub use schema::init_schema;

/// 检查 scene_tree 是否已初始化
pub async fn is_initialized() -> Result<bool> {
    // 用 RETURN 返回纯 int，避免 record/object 反序列化兼容问题
    let sql = "RETURN array::len((SELECT id FROM scene_node LIMIT 1));";
    let result: Vec<i64> = project_primary_db()
        .query_take(sql, 0)
        .await
        .unwrap_or_default();
    Ok(result.first().copied().unwrap_or(0) > 0)
}

/// 确保 scene_tree 已初始化（启动时调用）
pub async fn ensure_initialized() -> Result<()> {
    // 1. 初始化 Schema（幂等）
    schema::init_schema().await?;

    // 2. 检查是否已有数据
    if is_initialized().await? {
        println!("[scene_tree] 已初始化，跳过");
        return Ok(());
    }

    // 3. 执行初始化
    println!("[scene_tree] 未初始化，开始全量同步...");
    init_scene_tree("ALL", false).await?;
    Ok(())
}
