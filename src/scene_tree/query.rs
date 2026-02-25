//! Scene Tree 查询方法
//!
//! 提供生成状态查询、AABB 更新等功能

use aios_core::{RefnoEnum, RefU64, SUL_DB, SurrealQueryExt};
use anyhow::Result;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use surrealdb::types as surrealdb_types;
use surrealdb::types::SurrealValue;
use std::collections::HashSet;

/// Scene Node 状态
#[derive(Debug, Serialize, Deserialize, SurrealValue)]
#[surreal(crate = "surrealdb_types")]
pub struct SceneNodeStatus {
    pub id: i64,
    pub has_geo: bool,
    pub generated: bool,
}

/// 批量查询 refnos 的生成状态
pub async fn query_generation_status(
    refnos: &[RefnoEnum],
) -> Result<Vec<SceneNodeStatus>> {
    if refnos.is_empty() {
        return Ok(vec![]);
    }

    let id_list = refnos
        .iter()
        .map(|r| format!("scene_node:{}", r.refno().0))
        .collect::<Vec<_>>()
        .join(",");

    let sql = format!(
        "SELECT VALUE {{ id: record::id(id), has_geo: has_geo, generated: generated }} FROM [{}]",
        id_list
    );

    let result: Vec<SceneNodeStatus> = SUL_DB.query_take(&sql, 0).await?;
    Ok(result)
}

/// 从 refnos 中过滤出未生成的几何节点
pub async fn filter_ungenerated_geo_nodes(
    refnos: &[RefnoEnum],
) -> Result<Vec<i64>> {
    if refnos.is_empty() {
        return Ok(vec![]);
    }

    let id_list = refnos
        .iter()
        .map(|r| format!("scene_node:{}", r.refno().0))
        .collect::<Vec<_>>()
        .join(",");

    let sql = format!(
        "SELECT VALUE record::id(id) FROM [{}] WHERE has_geo = true AND generated = false",
        id_list
    );

    let result: Vec<i64> = SUL_DB.query_take(&sql, 0).await?;
    Ok(result)
}

/// 批量标记节点为已生成
pub async fn mark_as_generated(ids: &[i64]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }

    let id_list = ids
        .iter()
        .map(|id| format!("scene_node:{}", id))
        .collect::<Vec<_>>()
        .join(",");

    let sql = format!("UPDATE [{}] SET generated = true", id_list);
    SUL_DB.query_response(&sql).await?;
    Ok(())
}

/// 查询指定节点下所有未生成的几何叶子节点
pub async fn query_ungenerated_leaves(root_id: i64) -> Result<Vec<i64>> {
    // SurrealDB 3 的图递归语法与 v2 不兼容，这里改为在 Rust 侧做 BFS（最多 20 层）。
    const MAX_DEPTH: usize = 20;
    const CHUNK_SIZE: usize = 500;

    let mut visited: HashSet<i64> = HashSet::new();
    let mut frontier: Vec<i64> = vec![root_id];
    visited.insert(root_id);

    for _ in 0..MAX_DEPTH {
        if frontier.is_empty() {
            break;
        }

        let mut next_frontier: Vec<i64> = Vec::new();
        for chunk in frontier.chunks(CHUNK_SIZE) {
            let in_list = chunk
                .iter()
                .map(|id| format!("scene_node:{}", id))
                .collect::<Vec<_>>()
                .join(",");

            // 关系表 contains 的 in/out 字段是 record<scene_node>
            let sql = format!(
                "SELECT VALUE record::id(out) FROM contains WHERE in IN [{}]",
                in_list
            );
            let children: Vec<i64> = SUL_DB.query_take(&sql, 0).await.unwrap_or_default();

            for child_id in children {
                if visited.insert(child_id) {
                    next_frontier.push(child_id);
                }
            }
        }
        frontier = next_frontier;
    }

    // 过滤子孙集合中的几何叶子（排除根本身）
    visited.remove(&root_id);
    if visited.is_empty() {
        return Ok(vec![]);
    }

    let mut result: Vec<i64> = Vec::new();
    let ids: Vec<i64> = visited.into_iter().collect();
    for chunk in ids.chunks(2000) {
        let id_list = chunk
            .iter()
            .map(|id| format!("scene_node:{}", id))
            .collect::<Vec<_>>()
            .join(",");

        let sql = format!(
            "SELECT VALUE record::id(id) FROM [{}] WHERE has_geo = true AND is_leaf = true AND generated = false",
            id_list
        );
        let mut part: Vec<i64> = SUL_DB.query_take(&sql, 0).await.unwrap_or_default();
        result.append(&mut part);
    }

    Ok(result)
}

/// 查询指定节点的直属子节点（scene_node id 列表）
pub async fn query_children_ids(parent_id: i64, limit: usize) -> Result<Vec<i64>> {
    let limit = limit.clamp(1, 20000);
    let sql = format!(
        "SELECT VALUE record::id(out) FROM contains WHERE in = scene_node:{parent_id} LIMIT {limit}"
    );
    let result: Vec<i64> = SUL_DB.query_take(&sql, 0).await.unwrap_or_default();
    Ok(result)
}

/// 查询指定节点的祖先链（从直接父节点到根）
pub async fn query_ancestor_ids(start_id: i64, limit: usize) -> Result<Vec<i64>> {
    let limit = limit.clamp(1, 20000);
    let mut out: Vec<i64> = Vec::new();
    let mut visited: HashSet<i64> = HashSet::new();

    let mut current = start_id;
    for _ in 0..limit {
        let sql = format!("SELECT VALUE parent FROM scene_node:{current} LIMIT 1");
        let parents: Vec<Option<i64>> = SUL_DB.query_take(&sql, 0).await.unwrap_or_default();
        let Some(Some(parent_id)) = parents.into_iter().next() else {
            break;
        };

        if !visited.insert(parent_id) {
            break;
        }
        out.push(parent_id);
        current = parent_id;
    }

    Ok(out)
}

/// 批量更新 scene_node 的 AABB 并标记为已生成
pub async fn update_scene_node_aabb(
    inst_aabb_map: &DashMap<RefnoEnum, String>,
) -> Result<()> {
    if inst_aabb_map.is_empty() {
        return Ok(());
    }

    let keys: Vec<_> = inst_aabb_map.iter().map(|e| *e.key()).collect();

    for chunk in keys.chunks(200) {
        let mut sql = String::new();
        for refno in chunk {
            let Some(aabb_hash) = inst_aabb_map.get(refno) else { continue };
            let id = refno.refno().0;
            sql.push_str(&format!(
                "UPDATE scene_node:{} SET aabb = aabb:⟨{}⟩, generated = true;",
                id, aabb_hash.value()
            ));
        }
        if !sql.is_empty() {
            SUL_DB.query_response(&sql).await?;
        }
    }
    Ok(())
}

/// 查询已生成的节点（替代 inst_relate_aabb 存在性检查）
pub async fn query_generated_refnos(
    refnos: &[RefnoEnum],
) -> Result<Vec<RefnoEnum>> {
    if refnos.is_empty() {
        return Ok(vec![]);
    }

    let id_list = refnos
        .iter()
        .map(|r| format!("scene_node:{}", r.refno().0))
        .collect::<Vec<_>>()
        .join(",");

    let sql = format!(
        "SELECT VALUE record::id(id) FROM [{}] WHERE generated = true",
        id_list
    );

    let ids: Vec<i64> = SUL_DB.query_take(&sql, 0).await?;
    Ok(ids.into_iter().map(|id| RefnoEnum::from(RefU64(id as u64))).collect())
}
