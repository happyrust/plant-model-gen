//! Scene Tree HTTP API
//!
//! 提供场景树初始化和查询的 HTTP 接口

use axum::{
    Router,
    extract::{Path, Query, Json as AxumJson},
    http::StatusCode,
    response::Json,
    routing::{get, post},
};
use aios_core::{RefU64, RefnoEnum};
use serde::{Deserialize, Serialize};

use crate::scene_tree;

// ========================
// 请求/响应结构
// ========================

#[derive(Debug, Deserialize)]
pub struct InitRequest {
    pub mdb_name: Option<String>,
    pub force_rebuild: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct InitByRootRequest {
    pub force_rebuild: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct InitByDbnoRequest {
    pub force_rebuild: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct InitResponse {
    pub success: bool,
    pub node_count: usize,
    pub relation_count: usize,
    pub duration_ms: u128,
    pub error_message: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct LeavesResponse {
    pub success: bool,
    pub refno: String,
    pub leaves: Vec<i64>,
    pub count: usize,
    pub error_message: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChildrenQuery {
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct ChildrenResponse {
    pub success: bool,
    pub refno: String,
    pub children: Vec<i64>,
    pub count: usize,
    pub truncated: bool,
    pub error_message: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AncestorsQuery {
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct AncestorsResponse {
    pub success: bool,
    pub refno: String,
    pub ancestors: Vec<i64>,
    pub count: usize,
    pub error_message: Option<String>,
}

// ========================
// 路由
// ========================

pub fn create_scene_tree_routes() -> Router {
    Router::new()
        .route("/api/scene-tree/init", post(init_scene_tree))
        .route("/api/scene-tree/init/{dbno}", post(init_scene_tree_by_dbno))
        .route(
            "/api/scene-tree/init-by-root/{refno}",
            post(init_scene_tree_by_root),
        )
        .route("/api/scene-tree/{refno}/leaves", get(get_leaves))
        .route("/api/scene-tree/{refno}/children", get(get_children))
        .route("/api/scene-tree/{refno}/ancestors", get(get_ancestors))
}

// ========================
// 处理函数
// ========================

/// 初始化 Scene Tree
async fn init_scene_tree(
    AxumJson(req): AxumJson<InitRequest>,
) -> Result<Json<InitResponse>, StatusCode> {
    let mdb_name = req.mdb_name.unwrap_or_else(|| "ALL".to_string());
    let force_rebuild = req.force_rebuild.unwrap_or(false);

    match scene_tree::init_scene_tree(&mdb_name, force_rebuild).await {
        Ok(result) => Ok(Json(InitResponse {
            success: true,
            node_count: result.node_count,
            relation_count: result.relation_count,
            duration_ms: result.duration_ms,
            error_message: None,
        })),
        Err(e) => Ok(Json(InitResponse {
            success: false,
            node_count: 0,
            relation_count: 0,
            duration_ms: 0,
            error_message: Some(e.to_string()),
        })),
    }
}

/// 初始化 Scene Tree（按 refno dbno 构造 WORLD=`${dbno}_0` 开始构建）
async fn init_scene_tree_by_dbno(
    Path(dbno): Path<u32>,
    AxumJson(req): AxumJson<InitByDbnoRequest>,
) -> Result<Json<InitResponse>, StatusCode> {
    let force_rebuild = req.force_rebuild.unwrap_or(false);
    match scene_tree::init_scene_tree_by_dbno(dbno, force_rebuild).await {
        Ok(result) => Ok(Json(InitResponse {
            success: true,
            node_count: result.node_count,
            relation_count: result.relation_count,
            duration_ms: result.duration_ms,
            error_message: None,
        })),
        Err(e) => Ok(Json(InitResponse {
            success: false,
            node_count: 0,
            relation_count: 0,
            duration_ms: 0,
            error_message: Some(e.to_string()),
        })),
    }
}

/// 初始化 Scene Tree（从指定 root refno 开始构建子树）
async fn init_scene_tree_by_root(
    Path(refno): Path<String>,
    AxumJson(req): AxumJson<InitByRootRequest>,
) -> Result<Json<InitResponse>, StatusCode> {
    let root_id = match parse_refno_to_u64(&refno) {
        Some(id) => id,
        None => {
            return Ok(Json(InitResponse {
                success: false,
                node_count: 0,
                relation_count: 0,
                duration_ms: 0,
                error_message: Some("Invalid refno format".to_string()),
            }));
        }
    };

    let root_refno = RefnoEnum::from(RefU64(root_id));
    let force_rebuild = req.force_rebuild.unwrap_or(false);

    match scene_tree::init_scene_tree_from_root(root_refno, force_rebuild).await {
        Ok(result) => Ok(Json(InitResponse {
            success: true,
            node_count: result.node_count,
            relation_count: result.relation_count,
            duration_ms: result.duration_ms,
            error_message: None,
        })),
        Err(e) => Ok(Json(InitResponse {
            success: false,
            node_count: 0,
            relation_count: 0,
            duration_ms: 0,
            error_message: Some(e.to_string()),
        })),
    }
}

/// 查询节点下所有未生成的几何叶子节点
async fn get_leaves(
    Path(refno): Path<String>,
) -> Result<Json<LeavesResponse>, StatusCode> {
    // 解析 refno 为 u64
    let root_id = match parse_refno_to_u64(&refno) {
        Some(id) => id as i64,
        None => {
            return Ok(Json(LeavesResponse {
                success: false,
                refno: refno.clone(),
                leaves: vec![],
                count: 0,
                error_message: Some("Invalid refno format".to_string()),
            }));
        }
    };

    match scene_tree::query_ungenerated_leaves(root_id).await {
        Ok(leaves) => {
            let count = leaves.len();
            Ok(Json(LeavesResponse {
                success: true,
                refno,
                leaves,
                count,
                error_message: None,
            }))
        }
        Err(e) => Ok(Json(LeavesResponse {
            success: false,
            refno,
            leaves: vec![],
            count: 0,
            error_message: Some(e.to_string()),
        })),
    }
}

/// 查询直属子节点
async fn get_children(
    Path(refno): Path<String>,
    Query(query): Query<ChildrenQuery>,
) -> Result<Json<ChildrenResponse>, StatusCode> {
    let root_id = match parse_refno_to_u64(&refno) {
        Some(id) => id as i64,
        None => {
            return Ok(Json(ChildrenResponse {
                success: false,
                refno: refno.clone(),
                children: vec![],
                count: 0,
                truncated: false,
                error_message: Some("Invalid refno format".to_string()),
            }));
        }
    };

    let limit = query.limit.unwrap_or(2000).clamp(1, 20000) as usize;
    match scene_tree::query_children_ids(root_id, limit + 1).await {
        Ok(mut children) => {
            let truncated = children.len() > limit;
            if truncated {
                children.truncate(limit);
            }
            let count = children.len();
            Ok(Json(ChildrenResponse {
                success: true,
                refno,
                children,
                count,
                truncated,
                error_message: None,
            }))
        }
        Err(e) => Ok(Json(ChildrenResponse {
            success: false,
            refno,
            children: vec![],
            count: 0,
            truncated: false,
            error_message: Some(e.to_string()),
        })),
    }
}

/// 查询祖先链（从直接父节点到根）
async fn get_ancestors(
    Path(refno): Path<String>,
    Query(query): Query<AncestorsQuery>,
) -> Result<Json<AncestorsResponse>, StatusCode> {
    let node_id = match parse_refno_to_u64(&refno) {
        Some(id) => id as i64,
        None => {
            return Ok(Json(AncestorsResponse {
                success: false,
                refno: refno.clone(),
                ancestors: vec![],
                count: 0,
                error_message: Some("Invalid refno format".to_string()),
            }));
        }
    };

    let limit = query.limit.unwrap_or(2000).clamp(1, 20000) as usize;
    match scene_tree::query_ancestor_ids(node_id, limit).await {
        Ok(ancestors) => {
            let count = ancestors.len();
            Ok(Json(AncestorsResponse {
                success: true,
                refno,
                ancestors,
                count,
                error_message: None,
            }))
        }
        Err(e) => Ok(Json(AncestorsResponse {
            success: false,
            refno,
            ancestors: vec![],
            count: 0,
            error_message: Some(e.to_string()),
        })),
    }
}

// ========================
// 辅助函数
// ========================

/// 解析 refno 字符串为 u64
fn parse_refno_to_u64(refno: &str) -> Option<u64> {
    let parts: Vec<&str> = refno.split('_').collect();
    if parts.len() != 2 {
        return None;
    }
    let dbno: u64 = parts[0].parse().ok()?;
    let ref_num: u64 = parts[1].parse().ok()?;
    Some((dbno << 32) | ref_num)
}
