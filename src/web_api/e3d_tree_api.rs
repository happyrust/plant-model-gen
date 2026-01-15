use aios_core::{RefnoEnum, RefU64, SUL_DB, SurrealQueryExt};
use axum::{
    Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::sync::Arc;
use surrealdb::types::{self as surrealdb_types, SurrealValue};

#[derive(Clone)]
pub struct E3dTreeApiState {
    pub db_manager: Arc<crate::data_interface::tidb_manager::AiosDBManager>,
}

pub fn create_e3d_tree_routes(state: E3dTreeApiState) -> Router {
    Router::new()
        .route("/api/e3d/world-root", get(get_world_root))
        .route("/api/e3d/node/{refno}", get(get_node))
        .route("/api/e3d/children/{refno}", get(get_children))
        .route("/api/e3d/ancestors/{refno}", get(get_ancestors))
        .route("/api/e3d/subtree-refnos/{refno}", get(get_subtree_refnos))
        .route("/api/e3d/visible-insts/{refno}", get(get_visible_insts))
        .route("/api/e3d/site-nodes/{refno}", get(get_site_nodes))
        .route("/api/e3d/search", post(search_nodes))
        .with_state(state)
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TreeNodeDto {
    pub refno: RefnoEnum,
    pub name: String,
    pub noun: String,
    pub owner: Option<RefnoEnum>,
    pub children_count: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NodeResponse {
    pub success: bool,
    pub node: Option<TreeNodeDto>,
    pub error_message: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChildrenResponse {
    pub success: bool,
    pub parent_refno: RefnoEnum,
    pub children: Vec<TreeNodeDto>,
    pub truncated: bool,
    pub error_message: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AncestorsResponse {
    pub success: bool,
    pub refnos: Vec<RefnoEnum>,
    pub error_message: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SubtreeRefnosResponse {
    pub success: bool,
    pub refnos: Vec<RefnoEnum>,
    pub truncated: bool,
    pub error_message: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VisibleInstsResponse {
    pub success: bool,
    pub refno: RefnoEnum,
    pub refnos: Vec<RefnoEnum>,
    pub error_message: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchRequest {
    pub keyword: String,
    pub nouns: Option<Vec<String>>,
    pub limit: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchResponse {
    pub success: bool,
    pub items: Vec<TreeNodeDto>,
    pub error_message: Option<String>,
}

// ========================
// Site Nodes API (xeokit Node 层级)
// ========================

/// AABB 包围盒
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct NodeAabb {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

/// Site Node 数据（用于前端构建 xeokit Node 层级）
#[derive(Debug, Serialize, Deserialize)]
pub struct SiteNodeDto {
    pub refno: RefnoEnum,
    pub parent: Option<RefnoEnum>,
    pub noun: String,
    pub name: Option<String>,
    pub aabb: Option<NodeAabb>,
    pub has_geo: bool,
}

/// Site Nodes API 响应
#[derive(Debug, Serialize, Deserialize)]
pub struct SiteNodesResponse {
    pub success: bool,
    pub nodes: Vec<SiteNodeDto>,
    pub total: usize,
    pub error_message: Option<String>,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct PeRow {
    pub refno: Option<RefnoEnum>,
    pub name: Option<String>,
    pub noun: Option<String>,
    pub owner: Option<RefnoEnum>,
}

#[derive(Debug, Deserialize)]
pub struct ChildrenQuery {
    pub limit: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct SubtreeQuery {
    pub include_self: Option<bool>,
    pub max_depth: Option<i32>,
    pub limit: Option<i32>,
}

async fn get_world_root(
    State(_state): State<E3dTreeApiState>,
) -> Result<Json<NodeResponse>, StatusCode> {
    let db_option = aios_core::get_db_option();
    let mdb_name = db_option.mdb_name.clone();

    let world = match aios_core::mdb::get_world_refno(mdb_name).await {
        Ok(r) => r.refno(),
        Err(e) => {
            return Ok(Json(NodeResponse {
                success: false,
                node: None,
                error_message: Some(format!("get_world_refno failed: {e}")),
            }));
        }
    };

    // pe 表可能不包含 WORL/SITE 数据；因此这里优先返回可用的根 refno + noun。
    let node = match query_node(world.into()).await {
        Ok(Some(n)) => Some(n),
        Ok(None) | Err(_) => Some(TreeNodeDto {
            refno: world.into(),
            name: "*".to_string(),
            noun: "WORL".to_string(),
            owner: None,
            children_count: None,
        }),
    };

    Ok(Json(NodeResponse {
        success: true,
        node,
        error_message: None,
    }))
}

async fn get_node(
    Path(refno): Path<RefnoEnum>,
    State(_state): State<E3dTreeApiState>,
) -> Result<Json<NodeResponse>, StatusCode> {
    let node = match query_node(refno).await {
        Ok(v) => v,
        Err(_) => None,
    };
    if node.is_none() {
        return Ok(Json(NodeResponse {
            success: false,
            node: None,
            error_message: Some("Node not found".to_string()),
        }));
    }

    Ok(Json(NodeResponse {
        success: true,
        node,
        error_message: None,
    }))
}

async fn get_children(
    Path(parent_refno): Path<RefnoEnum>,
    Query(query): Query<ChildrenQuery>,
    State(_state): State<E3dTreeApiState>,
) -> Result<Json<ChildrenResponse>, StatusCode> {
    let limit = query.limit.unwrap_or(200).clamp(1, 2000);

    let parent_type = get_type_name(parent_refno).await;

    let mut children_nodes = if parent_type == "WORL" {
        let db_option = aios_core::get_db_option();
        let mdb_name = db_option.mdb_name.clone();

        let mut eles = aios_core::get_mdb_world_site_ele_nodes(mdb_name, aios_core::DBType::DESI)
            .await
            .unwrap_or_default();
        for ele in &mut eles {
            ele.owner = parent_refno;
        }
        eles
    } else {
        aios_core::get_children_ele_nodes(parent_refno)
            .await
            .unwrap_or_default()
    };

    let truncated = (children_nodes.len() as i32) > limit;
    if children_nodes.len() > limit as usize {
        children_nodes.truncate(limit as usize);
    }

    let children: Vec<TreeNodeDto> = children_nodes
        .into_iter()
        .map(|ele| TreeNodeDto {
            refno: ele.refno,
            name: ele.name,
            noun: ele.noun,
            owner: Some(parent_refno),
            children_count: Some(i32::from(ele.children_count)),
        })
        .collect();

    Ok(Json(ChildrenResponse {
        success: true,
        parent_refno,
        children,
        truncated,
        error_message: None,
    }))
}

async fn get_ancestors(
    Path(refno): Path<RefnoEnum>,
    State(_state): State<E3dTreeApiState>,
) -> Result<Json<AncestorsResponse>, StatusCode> {
    let ancestors = match aios_core::query_ancestor_refnos(refno).await {
        Ok(v) => v,
        Err(e) => {
            return Ok(Json(AncestorsResponse {
                success: false,
                refnos: vec![],
                error_message: Some(format!("query_ancestor_refnos failed: {e}")),
            }));
        }
    };

    Ok(Json(AncestorsResponse {
        success: true,
        refnos: ancestors,
        error_message: None,
    }))
}

async fn get_subtree_refnos(
    Path(root_refno): Path<RefnoEnum>,
    Query(query): Query<SubtreeQuery>,
    State(_state): State<E3dTreeApiState>,
) -> Result<Json<SubtreeRefnosResponse>, StatusCode> {
    let include_self = query.include_self.unwrap_or(true);
    let max_depth = query.max_depth.unwrap_or(64).clamp(0, 256);
    let limit = query.limit.unwrap_or(50_000).clamp(1, 200_000) as usize;

    let range_str = if max_depth <= 0 {
        None
    } else {
        Some(format!("1..{}", max_depth))
    };

    let mut out: Vec<RefnoEnum> = if max_depth <= 0 {
        Vec::new()
    } else {
        match aios_core::collect_descendant_filter_ids(&[root_refno], &[], range_str.as_deref())
            .await
        {
            Ok(v) => v,
            Err(e) => {
                return Ok(Json(SubtreeRefnosResponse {
                    success: false,
                    refnos: vec![],
                    truncated: false,
                    error_message: Some(format!("collect_descendant_filter_ids failed: {e}")),
                }));
            }
        }
    };

    if include_self {
        out.insert(0, root_refno);
    }

    let truncated = out.len() > limit;
    if out.len() > limit {
        out.truncate(limit);
    }

    Ok(Json(SubtreeRefnosResponse {
        success: true,
        refnos: out,
        truncated,
        error_message: None,
    }))
}

async fn get_visible_insts(
    Path(refno): Path<RefnoEnum>,
    State(_state): State<E3dTreeApiState>,
) -> Result<Json<VisibleInstsResponse>, StatusCode> {
    // 1) 先拿“深度可见实例”（可能包含无几何的组节点）
    let mut candidates = match aios_core::query_deep_visible_inst_refnos(refno).await {
        Ok(v) => v,
        Err(e) => {
            return Ok(Json(VisibleInstsResponse {
                success: false,
                refno,
                refnos: vec![],
                error_message: Some(format!("query_deep_visible_inst_refnos failed: {e}")),
            }));
        }
    };

    // 兼容：如果没有子孙可见节点，至少包含自己
    if candidates.is_empty() {
        candidates.push(refno);
    }

    // 2) 优先用 instances_{dbno}.json 做“可加载几何”过滤：与前端实际加载数据保持一致。
    //    - 这可以避免 query_deep_visible_inst_refnos 返回“组节点/无几何节点”，导致前端 instances 缺失。
    //    - 若文件不存在，再回退到 inst_relate 的几何实例查询做过滤。
    fn parse_dbno(r: RefnoEnum) -> Option<u32> {
        let s = r.to_string();
        let (dbno, _) = s.split_once('_')?;
        dbno.parse::<u32>().ok()
    }

    fn collect_component_refnos(v: &serde_json::Value, out: &mut HashSet<String>) {
        let Some(obj) = v.as_object() else { return };

        // ungrouped: [{ refno, instances: [...] }, ...]
        if let Some(arr) = obj.get("ungrouped").and_then(|x| x.as_array()) {
            for item in arr {
                if let Some(r) = item.get("refno").and_then(|x| x.as_str()) {
                    out.insert(r.to_string());
                }
            }
        }

        // bran_groups / equi_groups: [{ refno, children: [{refno,...}, ...], tubings: [{refno,...}, ...] }, ...]
        for key in ["bran_groups", "equi_groups"] {
            let Some(arr) = obj.get(key).and_then(|x| x.as_array()) else { continue };
            for g in arr {
                if let Some(r) = g.get("refno").and_then(|x| x.as_str()) {
                    out.insert(r.to_string());
                }
                if let Some(children) = g.get("children").and_then(|x| x.as_array()) {
                    for c in children {
                        if let Some(r) = c.get("refno").and_then(|x| x.as_str()) {
                            out.insert(r.to_string());
                        }
                    }
                }
                if let Some(tubings) = g.get("tubings").and_then(|x| x.as_array()) {
                    for t in tubings {
                        if let Some(r) = t.get("refno").and_then(|x| x.as_str()) {
                            out.insert(r.to_string());
                        }
                    }
                }
            }
        }
    }

    let refnos = if let Some(dbno) = parse_dbno(refno) {
        let instances_path =
            std::path::Path::new("output").join("instances").join(format!("instances_{dbno}.json"));
        if let Ok(bytes) = fs::read(&instances_path) {
            if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                let mut available = HashSet::<String>::new();
                collect_component_refnos(&json, &mut available);
                let mut out = candidates
                    .iter()
                    .copied()
                    .filter(|r| available.contains(&r.to_string()))
                    .collect::<Vec<_>>();
                out.sort();
                out.dedup();
                out
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    // 如果文件过滤成功拿到结果（或文件存在但无匹配），直接返回；
    // 如果文件缺失/解析失败导致 refnos 为空，则回退到 inst_relate 几何实例过滤。
    let refnos = if !refnos.is_empty() {
        refnos
    } else {
        match crate::fast_model::export_model::model_exporter::query_geometry_instances(
            &candidates,
            true,  // enable_holes：这里只用于过滤是否存在几何实例
            false, // verbose
        )
        .await
        {
            Ok(v) => {
                let mut out = v.into_iter().map(|q| q.refno).collect::<Vec<_>>();
                out.sort();
                out.dedup();
                out
            }
            Err(e) => {
                return Ok(Json(VisibleInstsResponse {
                    success: false,
                    refno,
                    refnos: vec![],
                    error_message: Some(format!("query_geometry_instances failed: {e}")),
                }));
            }
        }
    };

    Ok(Json(VisibleInstsResponse {
        success: true,
        refno,
        refnos,
        error_message: None,
    }))
}

async fn search_nodes(
    State(_state): State<E3dTreeApiState>,
    Json(request): Json<SearchRequest>,
) -> Result<Json<SearchResponse>, StatusCode> {
    let keyword = request.keyword.trim();
    if keyword.is_empty() {
        return Ok(Json(SearchResponse {
            success: true,
            items: vec![],
            error_message: None,
        }));
    }

    let limit = request.limit.unwrap_or(50).clamp(1, 200) as usize;

    let mut items: Vec<TreeNodeDto> = Vec::new();
    if let Some(nouns) = request.nouns.as_ref() && !nouns.is_empty() {
        for noun in nouns {
            if items.len() >= limit {
                break;
            }

            let rows = match aios_core::query_noun_hierarchy(noun, Some(keyword), None).await {
                Ok(v) => v,
                Err(e) => {
                    return Ok(Json(SearchResponse {
                        success: false,
                        items: vec![],
                        error_message: Some(format!(
                            "query_noun_hierarchy failed for noun={noun}: {e}"
                        )),
                    }));
                }
            };

            for row in rows {
                if items.len() >= limit {
                    break;
                }
                items.push(TreeNodeDto {
                    refno: row.id,
                    name: row.name,
                    noun: row.noun,
                    owner: Some(row.owner),
                    children_count: row.children_cnt,
                });
            }
        }
    } else {
        let sql = r#"
            SELECT refno, name, noun, owner
            FROM pe
            WHERE refno != NONE
              AND (
                string::contains(
                    string::lowercase(name ?? ''),
                    string::lowercase($keyword)
                )
                OR string::contains(
                    string::lowercase(type::string(id)),
                    string::lowercase($keyword)
                )
              )
            LIMIT $limit
        "#;

        let mut resp = match SUL_DB
            .query(sql)
            .bind(("keyword", keyword.to_string()))
            .bind(("limit", limit as i32))
            .await
        {
            Ok(v) => v,
            Err(e) => {
                return Ok(Json(SearchResponse {
                    success: false,
                    items: vec![],
                    error_message: Some(format!("Database query error: {e}")),
                }));
            }
        };

        let rows: Vec<PeRow> = match resp.take(0) {
            Ok(v) => v,
            Err(e) => {
                return Ok(Json(SearchResponse {
                    success: false,
                    items: vec![],
                    error_message: Some(format!("Database decode error: {e}")),
                }));
            }
        };

        for row in rows {
            if items.len() >= limit {
                break;
            }
            let Some(refno) = row.refno else {
                continue;
            };
            items.push(TreeNodeDto {
                refno,
                name: row.name.unwrap_or_else(|| "UNKNOWN".to_string()),
                noun: row.noun.unwrap_or_else(|| "UNKNOWN".to_string()),
                owner: row.owner,
                children_count: None,
            });
        }
    }

    Ok(Json(SearchResponse {
        success: true,
        items,
        error_message: None,
    }))
}

async fn query_node(refno: RefnoEnum) -> anyhow::Result<Option<TreeNodeDto>> {
    let pe = aios_core::get_pe(refno).await?;
    let Some(pe) = pe else {
        return Ok(None);
    };

    Ok(Some(TreeNodeDto {
        refno: pe.refno,
        name: pe.name,
        noun: pe.noun,
        owner: Some(pe.owner),
        children_count: None,
    }))
}

async fn get_type_name(refno: RefnoEnum) -> String {
    aios_core::get_type_name(refno)
        .await
        .unwrap_or_else(|_| "UNKNOWN".to_string())
}

// ========================
// Site Nodes Handler
// ========================

/// 查询 scene_node 表的返回结构
#[derive(Debug, Deserialize, SurrealValue)]
struct SceneNodeRow {
    pub id: i64,
    pub parent: Option<i64>,
    pub has_geo: bool,
    pub is_leaf: bool,
    pub aabb_min: Option<Vec<f64>>,
    pub aabb_max: Option<Vec<f64>>,
}

/// 获取 SITE 的所有 Node 层级数据（用于前端构建 xeokit Node 层级）
async fn get_site_nodes(
    Path(site_refno): Path<RefnoEnum>,
    State(_state): State<E3dTreeApiState>,
) -> Result<Json<SiteNodesResponse>, StatusCode> {
    // 1. 获取 SITE 的所有子孙节点（通过 BFS 遍历 contains 关系）
    const MAX_DEPTH: usize = 20;
    const MAX_NODES: usize = 10000;
    const CHUNK_SIZE: usize = 500;

    let site_id = site_refno.refno().0 as i64;
    let mut all_ids: Vec<i64> = vec![site_id];
    let mut frontier: Vec<i64> = vec![site_id];
    let mut visited: std::collections::HashSet<i64> = std::collections::HashSet::new();
    visited.insert(site_id);

    for _ in 0..MAX_DEPTH {
        if frontier.is_empty() || all_ids.len() >= MAX_NODES {
            break;
        }

        let mut next_frontier: Vec<i64> = Vec::new();
        for chunk in frontier.chunks(CHUNK_SIZE) {
            let in_list = chunk
                .iter()
                .map(|id| format!("scene_node:{}", id))
                .collect::<Vec<_>>()
                .join(",");

            let sql = format!(
                "SELECT VALUE meta::id(out) FROM contains WHERE in IN [{}]",
                in_list
            );
            let children: Vec<i64> = SUL_DB.query_take(&sql, 0).await.unwrap_or_default();

            for child_id in children {
                if all_ids.len() >= MAX_NODES {
                    break;
                }
                if visited.insert(child_id) {
                    all_ids.push(child_id);
                    next_frontier.push(child_id);
                }
            }
        }
        frontier = next_frontier;
    }

    // 2. 批量查询 scene_node 详细信息（包括 aabb）
    let mut nodes: Vec<SiteNodeDto> = Vec::with_capacity(all_ids.len());

    for chunk in all_ids.chunks(500) {
        let id_list = chunk
            .iter()
            .map(|id| format!("scene_node:{}", id))
            .collect::<Vec<_>>()
            .join(",");

        // 查询 scene_node 表，同时关联 aabb 表获取包围盒
        let sql = format!(
            r#"SELECT 
                meta::id(id) as id,
                parent,
                has_geo,
                is_leaf,
                aabb.min as aabb_min,
                aabb.max as aabb_max
            FROM [{}]"#,
            id_list
        );

        let rows: Vec<SceneNodeRow> = SUL_DB.query_take(&sql, 0).await.unwrap_or_default();

        for row in rows {
            let refno = RefnoEnum::from(RefU64(row.id as u64));
            let parent = row.parent.map(|p| RefnoEnum::from(RefU64(p as u64)));

            // 获取 pe 表的 noun 和 name
            let (noun, name) = match aios_core::get_pe(refno).await {
                Ok(Some(pe)) => (pe.noun, Some(pe.name)),
                _ => ("UNKNOWN".to_string(), None),
            };

            // 构建 AABB
            let aabb = match (&row.aabb_min, &row.aabb_max) {
                (Some(min), Some(max)) if min.len() >= 3 && max.len() >= 3 => Some(NodeAabb {
                    min: [min[0], min[1], min[2]],
                    max: [max[0], max[1], max[2]],
                }),
                _ => None,
            };

            nodes.push(SiteNodeDto {
                refno,
                parent,
                noun,
                name,
                aabb,
                has_geo: row.has_geo,
            });
        }
    }

    let total = nodes.len();
    Ok(Json(SiteNodesResponse {
        success: true,
        nodes,
        total,
        error_message: None,
    }))
}
