//! Review Embed URL API
//!
//! Provides the embed URL for 3D review interface.
//! This is the Platform's API that external systems (like Model Center) call.

use axum::{
    Router,
    extract::Json,
    http::StatusCode,
    routing::post,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

#[cfg(feature = "web_server")]
use aios_core::SUL_DB;

#[cfg(feature = "web_server")]
use surrealdb::types::{self as surrealdb_types, SurrealValue};

#[cfg(feature = "web_server")]
use super::jwt_auth::{create_token, generate_form_id};


// ============================================================================
// Configuration
// ============================================================================

/// 平台配置
#[derive(Clone, Debug)]
pub struct PlatformConfig {
    /// 前端相对路径
    pub frontend_relative_path: String,
}

impl Default for PlatformConfig {
    fn default() -> Self {
        Self {
            // plant3d-web 前端的校审页面路径
            frontend_relative_path: "/review/3d-view".to_string(),
        }
    }
}

impl PlatformConfig {
    /// 从配置文件加载
    pub fn from_config_file() -> Self {
        // 目前只有前端路径配置，直接使用默认值
        Self::default()
    }
}

// ============================================================================
// Request/Response Structs
// ============================================================================

/// 嵌入地址请求 (来自外部系统)
#[derive(Debug, Deserialize)]
pub struct EmbedUrlRequest {
    pub project_id: String,
    pub user_id: String,
    /// 外部传入的 token (用于验证请求合法性)
    pub token: Option<String>,
    #[serde(default)]
    pub extra_parameters: Option<serde_json::Value>,
}

/// 嵌入地址响应
#[derive(Debug, Serialize)]
pub struct EmbedUrlResponse {
    pub code: i32,
    pub message: String,
    pub data: Option<EmbedUrlData>,
}

#[derive(Debug, Serialize)]
pub struct EmbedUrlData {
    /// 前端相对路径
    pub relative_path: String,
    /// 平台生成的访问 token
    pub token: String,
    /// 查询参数
    pub query: EmbedUrlQuery,
}

#[derive(Debug, Serialize)]
pub struct EmbedUrlQuery {
    /// 平台生成的提资单 ID (UUID)
    pub form_id: String,
    /// 是否为审核人员
    pub is_reviewer: bool,
}

/// 缓存预加载请求 (我们调用模型中心)
#[derive(Debug, Deserialize)]
pub struct CachePreloadRequest {
    pub project_id: String,
    pub initiator: String,
}

/// 缓存预加载响应
#[derive(Debug, Serialize)]
pub struct CachePreloadResponse {
    pub success: bool,
    pub message: String,
    pub task_id: Option<String>,
}

/// 校审流程同步请求
#[derive(Debug, Deserialize)]
pub struct SyncWorkflowRequest {
    pub form_id: String,
    pub token: String,
    pub action: String,
    pub actor: WorkflowActor,
    pub next_step: Option<WorkflowNextStep>,
    pub comments: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct WorkflowActor {
    pub id: String,
    pub name: String,
    pub roles: String,
}

#[derive(Debug, Deserialize)]
pub struct WorkflowNextStep {
    pub assignee_id: String,
    pub name: String,
    pub roles: String,
}

/// 校审流程同步响应
#[derive(Debug, Serialize)]
pub struct SyncWorkflowResponse {
    pub code: i32,
    pub message: String,
    pub data: Option<SyncWorkflowData>,
}

#[derive(Debug, Serialize, Default)]
pub struct SyncWorkflowData {
    /// 当前用户选择进行编校审的所有模型清单
    pub models: Vec<String>,
    /// 审批意见
    pub opinions: Vec<WorkflowOpinion>,
    /// 附件（云线、截图等）
    pub attachments: Vec<WorkflowAttachment>,
}

#[derive(Debug, Serialize)]
pub struct WorkflowOpinion {
    /// 对应意见产生的模型 (refno 列表)
    pub model: Vec<String>,
    /// 审批节点类型: sj, jd, sh, pz
    pub node: String,
    /// 意见审批节点对应的顺序
    pub order: i32,
    /// 审批节点人员
    pub author: String,
    /// 总体意见文本
    pub opinion: String,
    /// 意见创建日期 (ISO8601)
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct WorkflowAttachment {
    /// 对应云线产生的模型 (refno 列表)
    pub model: Vec<String>,
    /// 文件 ID
    pub id: String,
    /// attachment 类型: markup (云线), file (文件)
    pub r#type: String,
    /// 对应下载地址
    pub download_url: String,
    /// 文件名称/描述
    pub description: String,
    /// 文件后缀
    pub file_ext: String,
}

// ============================================================================
// Axum Handlers
// ============================================================================

lazy_static::lazy_static! {
    static ref PLATFORM_CONFIG: PlatformConfig = PlatformConfig::from_config_file();
}

#[cfg(feature = "web_server")]
pub fn create_model_center_routes() -> Router {
    Router::new()
        // 我们提供的接口 (外部调用我们)
        .route("/api/review/embed-url", post(get_embed_url))
        .route("/api/review/workflow/sync", post(sync_workflow_handler))
        // 代理接口 (我们调用模型中心) - 用于触发模型中心的缓存
        .route("/api/review/cache/preload", post(preload_cache))
}

/// 获取嵌入地址 - 平台提供给外部的接口
#[cfg(feature = "web_server")]
async fn get_embed_url(
    Json(request): Json<EmbedUrlRequest>,
) -> impl IntoResponse {
    info!("Embed URL request: project_id={}, user_id={}", request.project_id, request.user_id);
    
    // 1. 生成新的 form_id (UUID)
    let form_id = generate_form_id();
    
    // 2. 生成 JWT token
    match create_token(&request.project_id, &request.user_id, &form_id, None) {
        Ok((token, _expires_at)) => {
            // 3. 判断是否为审核人员 (可以根据 extra_parameters 或其他逻辑判断)
            let is_reviewer = request.extra_parameters
                .as_ref()
                .and_then(|p| p.get("is_reviewer"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            
            info!("Generated form_id={}, token_len={}", form_id, token.len());
            
            // 4. 返回响应
            (StatusCode::OK, Json(EmbedUrlResponse {
                code: 200,
                message: "ok".to_string(),
                data: Some(EmbedUrlData {
                    relative_path: PLATFORM_CONFIG.frontend_relative_path.clone(),
                    token,
                    query: EmbedUrlQuery {
                        form_id,
                        is_reviewer,
                    },
                }),
            }))
        }
        Err(e) => {
            warn!("Token generation failed: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(EmbedUrlResponse {
                code: -1,
                message: format!("Token generation failed: {}", e),
                data: None,
            }))
        }
    }
}

/// 触发缓存预加载 - 代理到模型中心
#[cfg(feature = "web_server")]
async fn preload_cache(
    Json(request): Json<CachePreloadRequest>,
) -> impl IntoResponse {
    info!("Cache preload request: project_id={}, initiator={}", request.project_id, request.initiator);
    
    // 这里可以调用模型中心的缓存接口
    // 目前返回一个占位响应
    (StatusCode::OK, Json(CachePreloadResponse {
        success: true,
        message: "Cache preload request accepted".to_string(),
        task_id: Some(format!("cache_{}", request.project_id)),
    }))
}

/// 同步校审流程信息
#[cfg(feature = "web_server")]
async fn sync_workflow_handler(
    Json(request): Json<SyncWorkflowRequest>,
) -> impl IntoResponse {
    info!(
        "Sync workflow request: form_id={}, action={}, actor_id={}, actor_role={}", 
        request.form_id, request.action, request.actor.id, request.actor.roles
    );
    
    if let Some(ref next) = request.next_step {
        info!(
            "Next step: assignee={}, name={}, roles={}", 
            next.assignee_id, next.name, next.roles
        );
    }
    
    // 查询该 form_id 关联的数据
    let data = match query_workflow_data(&request.form_id).await {
        Ok(d) => d,
        Err(e) => {
            warn!("Failed to query workflow data for form_id={}: {}", request.form_id, e);
            SyncWorkflowData::default()
        }
    };
    
    info!(
        "Returning workflow data: models={}, opinions={}, attachments={}",
        data.models.len(), data.opinions.len(), data.attachments.len()
    );
    
    (StatusCode::OK, Json(SyncWorkflowResponse {
        code: 200,
        message: "success".to_string(),
        data: Some(data),
    }))
}

// ============================================================================
// Database Query Functions
// ============================================================================

/// 查询表单关联的所有模型 refno
#[cfg(feature = "web_server")]
async fn query_workflow_models(form_id: &str) -> anyhow::Result<Vec<String>> {
    let sql = r#"
        SELECT model_refno FROM review_form_model 
        WHERE form_id = $form_id
    "#;
    
    let mut response = SUL_DB
        .query(sql)
        .bind(("form_id", form_id.to_string()))
        .await?;
    
    #[derive(Debug, serde::Deserialize, SurrealValue)]
    struct ModelRow {
        model_refno: Option<String>,
    }
    
    let rows: Vec<ModelRow> = response.take(0)?;
    Ok(rows.into_iter().filter_map(|r| r.model_refno).collect())
}

/// 查询表单关联的所有审批意见
#[cfg(feature = "web_server")]
async fn query_workflow_opinions(form_id: &str) -> anyhow::Result<Vec<WorkflowOpinion>> {
    let sql = r#"
        SELECT model_refnos, node, seq_order, author, opinion, created_at 
        FROM review_opinion 
        WHERE form_id = $form_id
        ORDER BY seq_order ASC
    "#;
    
    let mut response = SUL_DB
        .query(sql)
        .bind(("form_id", form_id.to_string()))
        .await?;
    
    #[derive(Debug, serde::Deserialize, SurrealValue)]
    struct OpinionRow {
        model_refnos: Option<Vec<String>>,
        node: Option<String>,
        seq_order: Option<i32>,
        author: Option<String>,
        opinion: Option<String>,
        created_at: Option<String>,
    }
    
    let rows: Vec<OpinionRow> = response.take(0)?;
    Ok(rows.into_iter().map(|r| WorkflowOpinion {
        model: r.model_refnos.unwrap_or_default(),
        node: r.node.unwrap_or_default(),
        order: r.seq_order.unwrap_or(0),
        author: r.author.unwrap_or_default(),
        opinion: r.opinion.unwrap_or_default(),
        created_at: r.created_at.unwrap_or_default(),
    }).collect())
}

/// 查询表单关联的所有附件
#[cfg(feature = "web_server")]
async fn query_workflow_attachments(form_id: &str) -> anyhow::Result<Vec<WorkflowAttachment>> {
    let sql = r#"
        SELECT model_refnos, file_id, file_type, download_url, description, file_ext 
        FROM review_attachment 
        WHERE form_id = $form_id
    "#;
    
    let mut response = SUL_DB
        .query(sql)
        .bind(("form_id", form_id.to_string()))
        .await?;
    
    #[derive(Debug, serde::Deserialize, SurrealValue)]
    struct AttachmentRow {
        model_refnos: Option<Vec<String>>,
        file_id: Option<String>,
        file_type: Option<String>,
        download_url: Option<String>,
        description: Option<String>,
        file_ext: Option<String>,
    }
    
    let rows: Vec<AttachmentRow> = response.take(0)?;
    Ok(rows.into_iter().map(|r| WorkflowAttachment {
        model: r.model_refnos.unwrap_or_default(),
        id: r.file_id.unwrap_or_default(),
        r#type: r.file_type.unwrap_or_default(),
        download_url: r.download_url.unwrap_or_default(),
        description: r.description.unwrap_or_default(),
        file_ext: r.file_ext.unwrap_or_default(),
    }).collect())
}

/// 汇总查询表单的所有校审数据
#[cfg(feature = "web_server")]
async fn query_workflow_data(form_id: &str) -> anyhow::Result<SyncWorkflowData> {
    let models = query_workflow_models(form_id).await.unwrap_or_default();
    let opinions = query_workflow_opinions(form_id).await.unwrap_or_default();
    let attachments = query_workflow_attachments(form_id).await.unwrap_or_default();
    
    Ok(SyncWorkflowData {
        models,
        opinions,
        attachments,
    })
}
