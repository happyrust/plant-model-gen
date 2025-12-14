use aios_core::{RefnoEnum, SUL_DB};
use axum::{
    Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
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
              AND string::contains(
                    string::lowercase(name ?? ''),
                    string::lowercase($keyword)
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
