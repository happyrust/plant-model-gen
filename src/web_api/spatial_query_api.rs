use aios_core::{RefnoEnum, RefU64, SUL_DB, SurrealQueryExt};
use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::Json,
    routing::get,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use surrealdb::types::{self as surrealdb_types, SurrealValue};

use crate::fast_model::gen_model::tree_index_manager::TreeIndexManager;

/// 空间查询API状态
#[derive(Clone)]
pub struct SpatialQueryApiState {
    pub db_manager: Arc<crate::data_interface::tidb_manager::AiosDBManager>,
}

/// 创建空间查询路由
pub fn create_spatial_query_routes(state: SpatialQueryApiState) -> Router {
    Router::new()
        .route("/api/spatial/query/{refno}", get(query_spatial_node))
        .route("/api/spatial/children/{refno}", get(query_children_nodes))
        .route("/api/spatial/node-info/{refno}", get(get_node_info))
        .with_state(state)
}

// === 请求/响应结构体 ===

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SpatialNode {
    pub refno: u64,
    pub name: String,
    pub noun: String,
    pub node_type: String, // "SPACE", "ROOM", "COMPONENT"
    pub children_count: i32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SpatialQueryResponse {
    pub success: bool,
    pub node: Option<SpatialNode>,
    pub children: Vec<SpatialNode>,
    pub error_message: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NodeInfoResponse {
    pub success: bool,
    pub refno: u64,
    pub name: String,
    pub noun: String,
    pub node_type: String,
    pub owner: Option<u64>,
    pub error_message: Option<String>,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct PeRow {
    pub refno: Option<u64>,
    pub name: Option<String>,
    pub noun: Option<String>,
    pub owner: Option<u64>,
}

// === 处理函数 ===

/// 查询空间节点及其子节点
async fn query_spatial_node(
    Path(refno_str): Path<String>,
    State(state): State<SpatialQueryApiState>,
) -> Result<Json<SpatialQueryResponse>, StatusCode> {
    // 解析参考号
    let refno = match refno_str.parse::<u64>() {
        Ok(r) => r,
        Err(_) => {
            return Ok(Json(SpatialQueryResponse {
                success: false,
                node: None,
                children: vec![],
                error_message: Some("Invalid reference number format".to_string()),
            }));
        }
    };

    // 查询节点信息
    let node_query = format!(
        "SELECT refno, name, noun FROM pe WHERE refno = {} LIMIT 1",
        refno
    );

    match SUL_DB
        .query_take::<Vec<PeRow>>(&node_query, 0)
        .await
    {
        Ok(records) => {
            if records.is_empty() {
                return Ok(Json(SpatialQueryResponse {
                    success: false,
                    node: None,
                    children: vec![],
                    error_message: Some("Node not found".to_string()),
                }));
            }

            let record = &records[0];
            let name = record
                .name
                .clone()
                .unwrap_or_else(|| "Unknown".to_string());
            let noun = record
                .noun
                .clone()
                .unwrap_or_else(|| "UNKNOWN".to_string());

            // 判断节点类型
            let node_type = determine_node_type(&noun);

            // 查询子节点
            let children = query_children_by_type(&noun, refno)
                .await
                .unwrap_or_default();

            let node = SpatialNode {
                refno,
                name,
                noun,
                node_type,
                children_count: children.len() as i32,
            };

            Ok(Json(SpatialQueryResponse {
                success: true,
                node: Some(node),
                children,
                error_message: None,
            }))
        }
        Err(e) => Ok(Json(SpatialQueryResponse {
            success: false,
            node: None,
            children: vec![],
            error_message: Some(format!("Database query error: {}", e)),
        })),
    }
}

/// 查询子节点
async fn query_children_nodes(
    Path(refno_str): Path<String>,
    State(_state): State<SpatialQueryApiState>,
) -> Result<Json<Vec<SpatialNode>>, StatusCode> {
    let refno = match refno_str.parse::<u64>() {
        Ok(r) => r,
        Err(_) => return Err(StatusCode::BAD_REQUEST),
    };

    // 层级查询统一走 TreeIndex（indextree）：这里仅需要父节点 noun 做路由判断。
    let parent_refno = RefnoEnum::from(RefU64(refno));
    let parent_noun = TreeIndexManager::resolve_dbnum_for_refno(parent_refno)
        .await
        .ok()
        .and_then(|dbnum| {
            let mgr = TreeIndexManager::with_default_dir(vec![dbnum]);
            mgr.get_noun(parent_refno)
        })
        .unwrap_or_default();

    if parent_noun.is_empty() {
        return Ok(Json(vec![]));
    }

    let children = query_children_by_type(parent_noun.as_str(), refno)
        .await
        .unwrap_or_default();
    Ok(Json(children))
}

/// 获取节点详细信息
async fn get_node_info(
    Path(refno_str): Path<String>,
    State(_state): State<SpatialQueryApiState>,
) -> Result<Json<NodeInfoResponse>, StatusCode> {
    let refno = match refno_str.parse::<u64>() {
        Ok(r) => r,
        Err(_) => {
            return Ok(Json(NodeInfoResponse {
                success: false,
                refno: 0,
                name: String::new(),
                noun: String::new(),
                node_type: String::new(),
                owner: None,
                error_message: Some("Invalid reference number".to_string()),
            }));
        }
    };

    let query = format!(
        "SELECT refno, name, noun, owner FROM pe WHERE refno = {} LIMIT 1",
        refno
    );

    match SUL_DB.query_take::<Vec<PeRow>>(&query, 0).await {
        Ok(records) => {
            if let Some(record) = records.first() {
                let name = record
                    .name
                    .clone()
                    .unwrap_or_else(|| "Unknown".to_string());
                let noun = record
                    .noun
                    .clone()
                    .unwrap_or_else(|| "UNKNOWN".to_string());
                let owner = record.owner;

                let node_type = determine_node_type(&noun);
                Ok(Json(NodeInfoResponse {
                    success: true,
                    refno,
                    name,
                    noun,
                    node_type,
                    owner,
                    error_message: None,
                }))
            } else {
                Ok(Json(NodeInfoResponse {
                    success: false,
                    refno,
                    name: String::new(),
                    noun: String::new(),
                    node_type: String::new(),
                    owner: None,
                    error_message: Some("Node not found".to_string()),
                }))
            }
        }
        Err(e) => Ok(Json(NodeInfoResponse {
            success: false,
            refno,
            name: String::new(),
            noun: String::new(),
            node_type: String::new(),
            owner: None,
            error_message: Some(format!("Database error: {}", e)),
        })),
    }
}

// === 辅助函数 ===

/// 判断节点类型
fn determine_node_type(noun: &str) -> String {
    match noun {
        "FRMW" | "SBFR" => "SPACE".to_string(),
        "PANE" => "ROOM".to_string(),
        _ => "COMPONENT".to_string(),
    }
}

/// 根据父节点类型查询子节点
async fn query_children_by_type(
    parent_noun: &str,
    parent_refno: u64,
) -> anyhow::Result<Vec<SpatialNode>> {
    let parent_noun = parent_noun.trim().to_uppercase();

    let query = match parent_noun.as_str() {
        // Space -> Room (通过 room_panel_relate 关系)
        "FRMW" | "SBFR" => {
            format!(
                "SELECT pe.refno, pe.name, pe.noun FROM pe \
                 WHERE pe.refno IN (SELECT refno FROM room_panel_relate WHERE owner = {}) \
                 LIMIT 100",
                parent_refno
            )
        }
        // Room -> Component (通过 room_relate 关系)
        "PANE" => {
            format!(
                "SELECT pe.refno, pe.name, pe.noun FROM pe \
                 WHERE pe.refno IN (SELECT refno FROM room_relate WHERE owner = {}) \
                 LIMIT 100",
                parent_refno
            )
        }
        // 其他类型 -> 直接子节点：pe_owner 层级关系改走 TreeIndex。
        _ => {
            let parent_refno_enum = RefnoEnum::from(RefU64(parent_refno));
            let dbnum = TreeIndexManager::resolve_dbnum_for_refno(parent_refno_enum)
                .await
                .ok();
            let Some(dbnum) = dbnum else {
                return Ok(Vec::new());
            };
            let mgr = TreeIndexManager::with_default_dir(vec![dbnum]);
            let mut children = mgr.query_children(parent_refno_enum);
            if children.len() > 100 {
                children.truncate(100);
            }
            if children.is_empty() {
                return Ok(Vec::new());
            }
            let ids = children
                .iter()
                .map(|r| r.refno().0.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "SELECT refno, name, noun FROM pe WHERE refno IN [{}] LIMIT 100",
                ids
            )
        }
    };

    let records: Vec<PeRow> = SUL_DB.query_take(&query, 0).await?;

    let children = records
        .into_iter()
        .filter_map(|record| {
            let refno = record.refno?;
            let name = record.name.unwrap_or_else(|| "Unknown".to_string());
            let noun = record.noun.unwrap_or_else(|| "UNKNOWN".to_string());

            Some(SpatialNode {
                refno,
                name,
                noun: noun.clone(),
                node_type: determine_node_type(&noun),
                children_count: 0, // 暂时设为0,前端可以按需查询
            })
        })
        .collect();

    Ok(children)
}
