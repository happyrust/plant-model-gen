//! Scene Tree Parquet 导出
//!
//! 将 scene_node 数据导出为 Parquet 文件，供前端直接加载使用。

use aios_core::{SUL_DB, SurrealQueryExt};
use anyhow::Result;
use polars::prelude::*;
use serde::Deserialize;
use std::fs;
use std::path::Path;
use surrealdb::types as surrealdb_types;
use surrealdb::types::SurrealValue;

/// Scene Node 查询结果
#[derive(Debug, Clone, Deserialize, SurrealValue)]
struct SceneNodeRow {
    pub id: i64,
    pub parent: Option<i64>,
    pub has_geo: bool,
    pub is_leaf: bool,
    pub generated: bool,
    pub dbnum: i32,
    pub geo_type: Option<String>,
    pub name: String,
}

/// 导出 Scene Tree 到 Parquet 文件
///
/// # Arguments
/// * `dbnum` - 数据库编号
/// * `output_dir` - 输出目录
///
/// # Returns
/// * 导出的节点数量
pub async fn export_scene_tree_parquet(dbnum: u32, output_dir: &Path) -> Result<usize> {
    // 1. 查询指定 dbnum 的所有节点
    // 在 SurrealDB 中，scene_node.id 和 pe.id 的数字部分相同
    // 通过 record ID 直接构造关联查询
    let sql = format!(
        r#"SELECT
            record::id(id) as id,
            parent,
            has_geo,
            is_leaf,
            generated,
            dbnum,
            geo_type,
            (SELECT VALUE name FROM pe WHERE id = scene_node.id LIMIT 1)[0] ?? '' as name
        FROM scene_node
        WHERE dbnum = {}"#,
        dbnum
    );

    let rows: Vec<SceneNodeRow> = SUL_DB.query_take(&sql, 0).await?;
    if rows.is_empty() {
        println!("[scene_tree_parquet] dbnum={} 没有节点数据", dbnum);
        return Ok(0);
    }

    let node_count = rows.len();
    println!(
        "[scene_tree_parquet] 查询到 {} 个节点 (dbnum={})",
        node_count, dbnum
    );

    // 2. 构建列数据
    let ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
    let parents: Vec<Option<i64>> = rows.iter().map(|r| r.parent).collect();
    let has_geos: Vec<bool> = rows.iter().map(|r| r.has_geo).collect();
    let is_leafs: Vec<bool> = rows.iter().map(|r| r.is_leaf).collect();
    let generateds: Vec<bool> = rows.iter().map(|r| r.generated).collect();
    let dbnos: Vec<i32> = rows.iter().map(|r| r.dbnum).collect();
    let geo_types: Vec<Option<&str>> = rows.iter().map(|r| r.geo_type.as_deref()).collect();
    let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();

    // 3. 创建 DataFrame
    let df = df![
        "id" => ids,
        "parent" => parents,
        "has_geo" => has_geos,
        "is_leaf" => is_leafs,
        "generated" => generateds,
        "dbnum" => dbnos,
        "geo_type" => geo_types,
        "name" => names,
    ]?;

    // 4. 确保输出目录存在
    fs::create_dir_all(output_dir)?;

    // 5. 写入 Parquet 文件
    let file_name = format!("scene_tree_{}.parquet", dbnum);
    let file_path = output_dir.join(&file_name);

    let file = fs::File::create(&file_path)?;
    ParquetWriter::new(file).finish(&mut df.clone())?;

    println!(
        "[scene_tree_parquet] 导出完成: {} ({} 节点)",
        file_path.display(),
        node_count
    );

    Ok(node_count)
}
