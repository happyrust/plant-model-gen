//! Scene Tree 模块
//!
//! 提供场景树的初始化、查询和更新功能。
//! 用于替代 inst_relate_aabb 表，统一管理模型生成状态和 AABB。

use aios_core::{SUL_DB, SurrealQueryExt};
use anyhow::Result;

pub mod init;
pub mod query;
pub mod schema;
pub mod parquet_export;

pub fn is_geo_noun(noun: &str) -> bool {
    init::is_geo_noun(noun)
}

// 重新导出常用类型和函数
pub use init::{init_scene_tree, init_scene_tree_by_dbno, init_scene_tree_from_root, SceneTreeInitResult};
pub use query::{
    filter_ungenerated_geo_nodes, mark_as_generated, query_generated_refnos,
    query_ancestor_ids, query_children_ids, query_generation_status, query_ungenerated_leaves,
    update_scene_node_aabb,
    SceneNodeStatus,
};
pub use schema::init_schema;
pub use parquet_export::export_scene_tree_parquet;

/// 检查 scene_tree 是否已初始化
pub async fn is_initialized() -> Result<bool> {
    // 使用 VALUE 提取 count 值
    let sql = "SELECT VALUE count() FROM scene_node GROUP ALL";
    let result: Vec<i64> = SUL_DB.query_take(sql, 0).await.unwrap_or_default();
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
