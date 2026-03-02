//! Scene Tree 表结构定义
//!
//! 定义 scene_node 和 contains 关系表的 Schema

use aios_core::{project_primary_db, SurrealQueryExt};
use anyhow::Result;

/// 初始化 Scene Tree 表结构
pub async fn init_schema() -> Result<()> {
    let sql = r#"
-- Scene Node 表
DEFINE TABLE IF NOT EXISTS scene_node SCHEMAFULL;

-- 父节点 refno (u64 编码)
DEFINE FIELD IF NOT EXISTS parent ON TABLE scene_node TYPE option<int>;

-- 包围盒引用
DEFINE FIELD IF NOT EXISTS aabb ON TABLE scene_node TYPE option<record<aabb>>;

-- 是否为几何节点（根据 noun 类型判断）
DEFINE FIELD IF NOT EXISTS has_geo ON TABLE scene_node TYPE bool DEFAULT false;

-- 是否为叶子节点（叶子节点定义：无任何子节点）
DEFINE FIELD IF NOT EXISTS is_leaf ON TABLE scene_node TYPE bool DEFAULT false;

-- 模型是否已生成
DEFINE FIELD IF NOT EXISTS generated ON TABLE scene_node TYPE bool DEFAULT false;

-- 数据库编号
DEFINE FIELD IF NOT EXISTS dbnum ON TABLE scene_node TYPE int;

-- 几何类型（用于区分正负实体）
-- 可选值: Pos, Neg, CataNeg, CataCrossNeg, Compound, CatePos, DesiPos
DEFINE FIELD IF NOT EXISTS geo_type ON TABLE scene_node TYPE option<string>;

-- 索引
DEFINE INDEX IF NOT EXISTS idx_parent ON TABLE scene_node COLUMNS parent;
DEFINE INDEX IF NOT EXISTS idx_has_geo ON TABLE scene_node COLUMNS has_geo;
DEFINE INDEX IF NOT EXISTS idx_is_leaf ON TABLE scene_node COLUMNS is_leaf;
DEFINE INDEX IF NOT EXISTS idx_dbno ON TABLE scene_node COLUMNS dbnum;
DEFINE INDEX IF NOT EXISTS idx_generated ON TABLE scene_node COLUMNS generated;
DEFINE INDEX IF NOT EXISTS idx_has_geo_generated ON TABLE scene_node COLUMNS has_geo, generated;
DEFINE INDEX IF NOT EXISTS idx_geo_type ON TABLE scene_node COLUMNS geo_type;

-- 父子关系表（Graph Edge）
DEFINE TABLE IF NOT EXISTS contains TYPE RELATION FROM scene_node TO scene_node;
"#;

    project_primary_db().query_response(sql).await?;
    println!("[scene_tree] Schema 初始化完成");
    Ok(())
}
