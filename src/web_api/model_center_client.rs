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
use aios_core::{project_primary_db, init_surreal};

#[cfg(feature = "web_server")]
use surrealdb::types::{self as surrealdb_types, SurrealValue};

#[cfg(feature = "web_server")]
use super::jwt_auth::{create_token, generate_form_id, verify_token};

#[cfg(feature = "web_server")]
use super::review_api::ReviewTask;

#[cfg(feature = "web_server")]
use sha2::{Digest, Sha256};

#[cfg(feature = "web_server")]
use super::jwt_auth::JwtConfig;


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
    /// 外部传入的 form_id（如果已创建单据，需要保持一致）
    pub form_id: Option<String>,
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
    /// 既有任务上下文（若该 form_id 已存在）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<ReviewTask>,
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
    pub token: String,
}

/// 缓存预加载响应
#[derive(Debug, Serialize)]
pub struct CachePreloadResponse {
    pub code: i32,
    pub message: String,
    pub data: Option<serde_json::Value>,
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
        .route("/api/review/delete", post(delete_review_data))
        // 代理接口 (我们调用模型中心) - 用于触发模型中心的缓存
        .route("/api/review/cache/preload", post(preload_cache))
}

#[cfg(feature = "web_server")]
fn token_secret() -> String {
    // 复用 DbOption.toml 的 [model_center].token_secret
    JwtConfig::from_config_file().secret
}

#[cfg(feature = "web_server")]
fn sha256_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(feature = "web_server")]
fn verify_sha256_token(expected_plain: &str, token: &str) -> bool {
    let secret = token_secret();
    let plain = format!("{}:{}", secret, expected_plain);
    sha256_hex(&plain) == token
}

/// 获取嵌入地址 - 平台提供给外部的接口
#[cfg(feature = "web_server")]
async fn get_embed_url(
    Json(request): Json<EmbedUrlRequest>,
) -> impl IntoResponse {
    info!("Embed URL request: project_id={}, user_id={}", request.project_id, request.user_id);
    
    // 外部若传 token：
    // - 文档 2) embed-url：token 使用 JWT
    // - 兼容旧调用：若不是 JWT 结构，则回退到 SHA256 token 校验
    let mut jwt_claim_form_id: Option<String> = None;
    if let Some(ref token) = request.token {
        let token = token.trim();
        if token.split('.').count() == 3 {
            match verify_token(token) {
                Ok(claims) => {
                    if claims.project_id != request.project_id || claims.user_id != request.user_id {
                        return (StatusCode::UNAUTHORIZED, Json(EmbedUrlResponse {
                            code: 401,
                            message: "unauthorized".to_string(),
                            data: None,
                        }));
                    }
                    jwt_claim_form_id = Some(claims.form_id);
                }
                Err(_) => {
                    return (StatusCode::UNAUTHORIZED, Json(EmbedUrlResponse {
                        code: 401,
                        message: "unauthorized".to_string(),
                        data: None,
                    }));
                }
            }
        } else {
            let expected_plain = format!("{}:{}", request.project_id, request.user_id);
            if !verify_sha256_token(&expected_plain, token) {
                return (StatusCode::UNAUTHORIZED, Json(EmbedUrlResponse {
                    code: 401,
                    message: "unauthorized".to_string(),
                    data: None,
                }));
            }
        }
    }

    // 1. 使用外部传入的 form_id 或生成新的 form_id
    let mut form_id = request
        .form_id
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    // 若 token 是 JWT 且携带 form_id，则优先保证一致
    if let Some(jwt_form_id) = jwt_claim_form_id {
        if let Some(ref req_form_id) = form_id {
            if req_form_id != &jwt_form_id {
                return (StatusCode::UNAUTHORIZED, Json(EmbedUrlResponse {
                    code: 401,
                    message: "unauthorized".to_string(),
                    data: None,
                }));
            }
        } else {
            form_id = Some(jwt_form_id);
        }
    }

    let form_id = form_id.unwrap_or_else(generate_form_id);
    let existing_task = match find_task_by_form_id(&form_id).await {
        Ok(task) => task,
        Err(e) => {
            warn!("Failed to load task for form_id={}: {}", form_id, e);
            None
        }
    };
    
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
                    task: existing_task,
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

    // 按文档：token 为 SHA256 哈希校验
    let expected_plain = format!("{}:{}", request.project_id, request.initiator);
    if !verify_sha256_token(&expected_plain, &request.token) {
        return (StatusCode::UNAUTHORIZED, Json(CachePreloadResponse {
            code: 401,
            message: "unauthorized".to_string(),
            data: None,
        }));
    }
    
    // 这里可以调用模型中心的缓存接口
    // 目前返回一个占位响应
    (StatusCode::OK, Json(CachePreloadResponse {
        code: 0,
        message: "accepted".to_string(),
        data: Some(serde_json::json!({
            "task_id": format!("cache_{}", request.project_id)
        })),
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

    // 按文档：token 为 SHA256 哈希校验
    let expected_plain = format!("{}:{}", request.form_id, request.actor.id);
    if !verify_sha256_token(&expected_plain, &request.token) {
        return (StatusCode::UNAUTHORIZED, Json(SyncWorkflowResponse {
            code: 401,
            message: "unauthorized".to_string(),
            data: None,
        }));
    }

    // 记录当前节点的审批意见（comments）
    let node = request.actor.roles.trim().to_string();
    let seq_order = match node.as_str() {
        "sj" => 1,
        "jd" => 2,
        "sh" => 3,
        "pz" => 4,
        _ => 0,
    };

    // 模型清单来自 review_form_model
    let model_refnos = query_workflow_models(&request.form_id).await.unwrap_or_default();
    if let Some(comment) = request.comments.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        let _ = project_primary_db()
            .query(
                r#"
                CREATE review_opinion CONTENT {
                    form_id: $form_id,
                    model_refnos: $model_refnos,
                    node: $node,
                    seq_order: $seq_order,
                    author: $author,
                    opinion: $opinion,
                    created_at: time::now()
                }
                "#,
            )
            .bind(("form_id", request.form_id.clone()))
            .bind(("model_refnos", model_refnos.clone()))
            .bind(("node", node))
            .bind(("seq_order", seq_order))
            .bind(("author", request.actor.id.clone()))
            .bind(("opinion", comment.to_string()))
            .await;
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

/// 删除校审数据（模型中心侧）
#[cfg(feature = "web_server")]
#[derive(Debug, Deserialize)]
pub struct DeleteReviewRequest {
    pub form_ids: Vec<String>,
    pub operator_id: String,
    pub token: String,
}

#[cfg(feature = "web_server")]
#[derive(Debug, Serialize)]
pub struct DeleteReviewResponse {
    pub code: i32,
    pub message: String,
}

/// POST /api/review/delete
#[cfg(feature = "web_server")]
async fn delete_review_data(
    Json(request): Json<DeleteReviewRequest>,
) -> impl IntoResponse {
    let joined = request.form_ids.join(",");
    let expected_plain = format!("{}:{}", joined, request.operator_id);
    if !verify_sha256_token(&expected_plain, &request.token) {
        return (StatusCode::UNAUTHORIZED, Json(DeleteReviewResponse {
            code: 401,
            message: "unauthorized".to_string(),
        }));
    }

    for form_id in &request.form_ids {
        // 删除物理附件文件（如果有）
        if let Ok(mut resp) = project_primary_db()
            .query("SELECT file_id, file_ext FROM review_attachment WHERE form_id = $form_id")
            .bind(("form_id", form_id.clone()))
            .await
        {
            #[derive(Debug, serde::Deserialize, SurrealValue)]
            struct AttachmentFileRow {
                file_id: Option<String>,
                file_ext: Option<String>,
            }

            let rows: Vec<AttachmentFileRow> = resp.take(0).unwrap_or_default();
            for row in rows {
                let file_id = row.file_id.unwrap_or_default();
                if file_id.trim().is_empty() {
                    continue;
                }
                let ext = row.file_ext.unwrap_or_default();
                let ext = ext.trim();
                let file_name = if ext.is_empty() {
                    file_id.clone()
                } else if ext.starts_with('.') {
                    format!("{}{}", file_id, ext)
                } else {
                    format!("{}.{}", file_id, ext)
                };
                let path = format!("assets/review_attachments/{}", file_name);
                let _ = std::fs::remove_file(&path);
            }
        }

        let _ = project_primary_db()
            .query(
                "LET $ids = SELECT VALUE id FROM review_form_model WHERE form_id = $form_id;\nDELETE $ids;",
            )
            .bind(("form_id", form_id.clone()))
            .await;
        let _ = project_primary_db()
            .query(
                "LET $ids = SELECT VALUE id FROM review_opinion WHERE form_id = $form_id;\nDELETE $ids;",
            )
            .bind(("form_id", form_id.clone()))
            .await;
        let _ = project_primary_db()
            .query(
                "LET $ids = SELECT VALUE id FROM review_attachment WHERE form_id = $form_id;\nDELETE $ids;",
            )
            .bind(("form_id", form_id.clone()))
            .await;
        let _ = project_primary_db()
            .query(
                "LET $ids = SELECT VALUE id FROM review_tasks WHERE form_id = $form_id;\nDELETE $ids;",
            )
            .bind(("form_id", form_id.clone()))
            .await;
    }

    (StatusCode::OK, Json(DeleteReviewResponse {
        code: 200,
        message: "ok".to_string(),
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
    
    let mut response = project_primary_db()
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

/// 查询 form_id 对应的既有任务
#[cfg(feature = "web_server")]
async fn find_task_by_form_id(form_id: &str) -> anyhow::Result<Option<ReviewTask>> {
    #[derive(Debug, serde::Deserialize, SurrealValue)]
    struct TaskRow {
        id: surrealdb::types::RecordId,
        form_id: Option<String>,
        title: Option<String>,
        description: Option<String>,
        model_name: Option<String>,
        status: Option<String>,
        priority: Option<String>,
        requester_id: Option<String>,
        requester_name: Option<String>,
        checker_id: Option<String>,
        checker_name: Option<String>,
        approver_id: Option<String>,
        approver_name: Option<String>,
        reviewer_id: Option<String>,
        reviewer_name: Option<String>,
        components: Option<Vec<super::review_api::ReviewComponent>>,
        attachments: Option<Vec<super::review_api::ReviewAttachment>>,
        review_comment: Option<String>,
        created_at: Option<surrealdb::types::Datetime>,
        updated_at: Option<surrealdb::types::Datetime>,
        due_date: Option<surrealdb::types::Datetime>,
        current_node: Option<String>,
        workflow_history: Option<Vec<super::review_api::WorkflowStep>>,
        return_reason: Option<String>,
    }

    fn to_millis(value: Option<surrealdb::types::Datetime>) -> Option<i64> {
        value.map(|dt| dt.timestamp_millis())
    }

    let mut response = project_primary_db()
        .query(
            r#"
            SELECT * FROM review_tasks
            WHERE form_id = $form_id
            ORDER BY updated_at DESC, created_at DESC
            LIMIT 1
            "#,
        )
        .bind(("form_id", form_id.to_string()))
        .await?;

    let rows: Vec<TaskRow> = response.take(0)?;
    Ok(rows.into_iter().next().map(|row| {
        let id = match row.id.key {
            surrealdb::types::RecordIdKey::String(value) => value,
            other => format!("{:?}", other),
        };
        let checker_id = row
            .checker_id
            .clone()
            .filter(|value| !value.is_empty())
            .or_else(|| row.reviewer_id.clone())
            .unwrap_or_default();
        let checker_name = row
            .checker_name
            .clone()
            .filter(|value| !value.is_empty())
            .or_else(|| row.reviewer_name.clone())
            .unwrap_or_default();

        ReviewTask {
            id,
            form_id: row.form_id.unwrap_or_default(),
            title: row.title.unwrap_or_default(),
            description: row.description.unwrap_or_default(),
            model_name: row.model_name.unwrap_or_default(),
            status: row.status.unwrap_or_else(|| "draft".to_string()),
            priority: row.priority.unwrap_or_else(|| "medium".to_string()),
            requester_id: row.requester_id.unwrap_or_default(),
            requester_name: row.requester_name.unwrap_or_default(),
            checker_id: checker_id.clone(),
            checker_name: checker_name.clone(),
            approver_id: row.approver_id.unwrap_or_default(),
            approver_name: row.approver_name.unwrap_or_default(),
            reviewer_id: row.reviewer_id.unwrap_or_else(|| checker_id),
            reviewer_name: row.reviewer_name.unwrap_or_else(|| checker_name),
            components: row.components.unwrap_or_default(),
            attachments: row.attachments,
            review_comment: row.review_comment,
            created_at: to_millis(row.created_at).unwrap_or_default(),
            updated_at: to_millis(row.updated_at).unwrap_or_default(),
            due_date: to_millis(row.due_date),
            current_node: row.current_node.unwrap_or_else(|| "sj".to_string()),
            workflow_history: row.workflow_history.unwrap_or_default(),
            return_reason: row.return_reason,
        }
    }))
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
    
    let mut response = project_primary_db()
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
        created_at: Option<surrealdb::types::Datetime>,
    }
    
    let rows: Vec<OpinionRow> = response.take(0)?;
    Ok(rows.into_iter().map(|r| WorkflowOpinion {
        model: r.model_refnos.unwrap_or_default(),
        node: r.node.unwrap_or_default(),
        order: r.seq_order.unwrap_or(0),
        author: r.author.unwrap_or_default(),
        opinion: r.opinion.unwrap_or_default(),
        created_at: r.created_at.map(|dt| dt.to_string()).unwrap_or_default(),
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
    
    let mut response = project_primary_db()
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

// ============================================================================
// 外部校审系统出站调用
// ============================================================================

/// 外部校审系统配置
#[derive(Clone, Debug)]
pub struct ExternalReviewConfig {
    pub base_url: String,
    pub workflow_sync_path: String,
    pub workflow_delete_path: String,
    pub auth_secret: String,
    pub timeout_seconds: u64,
}

impl Default for ExternalReviewConfig {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            workflow_sync_path: "/api/workflow/sync".to_string(),
            workflow_delete_path: "/api/workflow/delete".to_string(),
            auth_secret: "shared-review-secret".to_string(),
            timeout_seconds: 15,
        }
    }
}

impl ExternalReviewConfig {
    /// 从 DbOption.toml 加载 [external_review] 配置
    pub fn from_config_file() -> Self {
        use config as cfg;

        let mut names = Vec::new();
        if let Ok(config_path) = std::env::var("DB_OPTION_FILE") {
            names.push(
                config_path
                    .strip_suffix(".toml")
                    .unwrap_or(&config_path)
                    .to_string(),
            );
        }
        names.extend([
            "db_options/DbOption".to_string(),
            "../db_options/DbOption".to_string(),
            "DbOption".to_string(),
        ]);

        for name in &names {
            let file_path = format!("{}.toml", name);
            if std::path::Path::new(&file_path).exists() {
                if let Ok(config) = cfg::Config::builder()
                    .add_source(cfg::File::with_name(name))
                    .build()
                {
                    return Self {
                        base_url: config
                            .get_string("external_review.base_url")
                            .unwrap_or_default(),
                        workflow_sync_path: config
                            .get_string("external_review.workflow_sync_path")
                            .unwrap_or_else(|_| "/api/workflow/sync".to_string()),
                        workflow_delete_path: config
                            .get_string("external_review.workflow_delete_path")
                            .unwrap_or_else(|_| "/api/workflow/delete".to_string()),
                        auth_secret: config
                            .get_string("external_review.auth_secret")
                            .unwrap_or_else(|_| "shared-review-secret".to_string()),
                        timeout_seconds: config
                            .get_int("external_review.timeout_seconds")
                            .unwrap_or(15) as u64,
                    };
                }
            }
        }
        Self::default()
    }

    /// base_url 为空时启用 mock 模式
    pub fn is_mock(&self) -> bool {
        self.base_url.trim().is_empty()
    }
}

#[cfg(feature = "web_server")]
lazy_static::lazy_static! {
    pub static ref EXTERNAL_REVIEW_CONFIG: ExternalReviewConfig = ExternalReviewConfig::from_config_file();
}

/// 异步通知外部系统工作流状态变更（提交/驳回）
/// fire-and-forget：不阻塞主流程
#[cfg(feature = "web_server")]
pub fn notify_workflow_sync_async(task_id: String, action: String, operator_id: String, comment: Option<String>) {
    if EXTERNAL_REVIEW_CONFIG.is_mock() {
        info!("[外部校审] mock 模式，跳过工作流同步: task={}, action={}", task_id, action);
        return;
    }
    tokio::spawn(async move {
        if let Err(e) = notify_workflow_sync(&task_id, &action, &operator_id, comment.as_deref()).await {
            warn!("[外部校审] 工作流同步失败: task={}, err={}", task_id, e);
        }
    });
}

#[cfg(feature = "web_server")]
async fn notify_workflow_sync(task_id: &str, action: &str, operator_id: &str, comment: Option<&str>) -> anyhow::Result<()> {
    let config = &*EXTERNAL_REVIEW_CONFIG;
    let url = format!("{}{}", config.base_url.trim_end_matches('/'), config.workflow_sync_path);
    
    let token_plain = format!("{}:{}", task_id, operator_id);
    let token = sha256_hex(&format!("{}:{}", config.auth_secret, token_plain));
    
    let body = serde_json::json!({
        "task_id": task_id,
        "action": action,
        "operator_id": operator_id,
        "comment": comment.unwrap_or(""),
        "token": token,
    });
    
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(config.timeout_seconds))
        .build()?;
    
    let resp = client.post(&url)
        .json(&body)
        .send()
        .await?;
    
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("外部系统返回错误 {}: {}", status, text);
    }
    
    info!("[外部校审] 工作流同步成功: task={}, action={}", task_id, action);
    Ok(())
}

/// 异步通知外部系统删除校审数据
#[cfg(feature = "web_server")]
pub fn notify_workflow_delete_async(task_id: String, operator_id: String) {
    if EXTERNAL_REVIEW_CONFIG.is_mock() {
        info!("[外部校审] mock 模式，跳过删除通知: task={}", task_id);
        return;
    }
    tokio::spawn(async move {
        if let Err(e) = notify_workflow_delete(&task_id, &operator_id).await {
            warn!("[外部校审] 删除通知失败: task={}, err={}", task_id, e);
        }
    });
}

#[cfg(feature = "web_server")]
async fn notify_workflow_delete(task_id: &str, operator_id: &str) -> anyhow::Result<()> {
    let config = &*EXTERNAL_REVIEW_CONFIG;
    let url = format!("{}{}", config.base_url.trim_end_matches('/'), config.workflow_delete_path);
    
    let token_plain = format!("{}:{}", task_id, operator_id);
    let token = sha256_hex(&format!("{}:{}", config.auth_secret, token_plain));
    
    let body = serde_json::json!({
        "task_id": task_id,
        "operator_id": operator_id,
        "token": token,
    });
    
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(config.timeout_seconds))
        .build()?;
    
    let resp = client.post(&url)
        .json(&body)
        .send()
        .await?;
    
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("外部系统返回错误 {}: {}", status, text);
    }
    
    info!("[外部校审] 删除通知成功: task={}", task_id);
    Ok(())
}

#[cfg(test)]
#[cfg(feature = "web_server")]
mod tests {
    use super::*;
    use axum::{body::Body, http::{Request, StatusCode}};
    use serde::Deserialize;
    use tower::ServiceExt;

    #[derive(Debug, Deserialize)]
    struct EmbedUrlResponseBody {
        code: i32,
        message: String,
        data: Option<serde_json::Value>,
    }

    #[derive(Debug, Deserialize)]
    struct EmbedUrlQueryBody {
        form_id: String,
    }

    #[derive(Debug, Deserialize)]
    struct EmbedTaskBody {
        id: String,
        form_id: String,
        requester_id: String,
        current_node: String,
        status: String,
    }

    async fn cleanup_form(form_id: &str) {
        let _ = project_primary_db()
            .query("LET $ids = SELECT VALUE id FROM review_tasks WHERE form_id = $form_id; DELETE $ids;")
            .bind(("form_id", form_id.to_string()))
            .await;
    }

    async fn insert_task_with_form_id(form_id: &str, user_id: &str) {
        let _ = init_surreal().await;
        let _ = cleanup_form(form_id).await;
        project_primary_db()
            .query(
                r#"
                CREATE ONLY review_tasks SET
                    id = $id,
                    form_id = $form_id,
                    title = $title,
                    description = $description,
                    model_name = $model_name,
                    status = $status,
                    priority = 'medium',
                    requester_id = $requester_id,
                    requester_name = $requester_id,
                    checker_id = 'checker-1',
                    checker_name = 'checker-1',
                    approver_id = 'approver-1',
                    approver_name = 'approver-1',
                    reviewer_id = 'checker-1',
                    reviewer_name = 'checker-1',
                    components = [],
                    attachments = NONE,
                    current_node = $current_node,
                    workflow_history = [],
                    created_at = time::now(),
                    updated_at = time::now()
                "#,
            )
            .bind(("id", format!("task-{}", form_id.to_lowercase())))
            .bind(("form_id", form_id.to_string()))
            .bind(("title", format!("Task for {form_id}")))
            .bind(("description", "existing seeded task".to_string()))
            .bind(("model_name", "demo-model".to_string()))
            .bind(("status", "in_review".to_string()))
            .bind(("requester_id", user_id.to_string()))
            .bind(("current_node", "jd".to_string()))
            .await
            .expect("seed review task");
    }

    #[tokio::test]
    async fn test_embed_url_rejects_mismatched_form_id_from_jwt() {
        let app = create_model_center_routes();
        let (token, _) = create_token("project-1", "user-1", "FORM-EXPECTED", Some("sj")).unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/review/embed-url")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "project_id": "project-1",
                            "user_id": "user-1",
                            "form_id": "FORM-OTHER",
                            "token": token
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_embed_url_accepts_matching_form_id_from_jwt() {
        let app = create_model_center_routes();
        let (token, _) = create_token("project-1", "user-1", "FORM-EXPECTED", Some("sj")).unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/review/embed-url")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "project_id": "project-1",
                            "user_id": "user-1",
                            "form_id": "FORM-EXPECTED",
                            "token": token
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let payload: EmbedUrlResponseBody = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload.code, 200);
        assert_eq!(payload.message, "ok");
        let data = payload.data.expect("embed data");
        assert_eq!(data.get("query").and_then(|q| q.get("form_id").or_else(|| q.get("formId"))).and_then(|v| v.as_str()), Some("FORM-EXPECTED"));
        let response_token = data.get("token").and_then(|v| v.as_str()).expect("response token");
        assert_eq!(verify_token(response_token).unwrap().user_id, "user-1");
        assert!(data.get("task").is_none() || data.get("task").is_some_and(|v| v.is_null()));
    }

    #[tokio::test]
    async fn test_embed_url_returns_existing_task_for_form_id() {
        let form_id = "FORM-DB-BACKED-EXISTING";
        insert_task_with_form_id(form_id, "user-existing").await;

        let app = create_model_center_routes();
        let (token, _) = create_token("project-1", "user-existing", form_id, Some("jd")).unwrap();

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/review/embed-url")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "project_id": "project-1",
                            "user_id": "user-existing",
                            "form_id": form_id,
                            "token": token
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let payload: EmbedUrlResponseBody = serde_json::from_slice(&body).unwrap();
        assert_eq!(status, StatusCode::OK);
        assert_eq!(payload.code, 200);
        let data = payload.data.expect("embed data");
        assert_eq!(data.get("query").and_then(|q| q.get("form_id").or_else(|| q.get("formId"))).and_then(|v| v.as_str()), Some(form_id));
        let task = data.get("task").and_then(|v| v.as_object()).expect("existing task restored");
        assert_eq!(task.get("form_id").or_else(|| task.get("formId")).and_then(|v| v.as_str()), Some(form_id));
        assert_eq!(task.get("requesterId").and_then(|v| v.as_str()), Some("user-existing"));
        assert_eq!(task.get("currentNode").and_then(|v| v.as_str()), Some("jd"));
        assert_eq!(task.get("status").and_then(|v| v.as_str()), Some("in_review"));
        assert!(task.get("id").and_then(|v| v.as_str()).is_some_and(|id| id.starts_with("task-form-db-backed-existing")));
        let response_token = data.get("token").and_then(|v| v.as_str()).expect("response token");
        assert_eq!(verify_token(response_token).unwrap().form_id, form_id);

        cleanup_form(form_id).await;
    }
}
