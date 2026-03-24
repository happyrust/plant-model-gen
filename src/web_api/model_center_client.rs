//! Review Embed URL API
//!
//! Provides the embed URL for 3D review interface.
//! This is the Platform's API that external systems (like Model Center) call.

use axum::{Router, extract::Json, http::StatusCode, response::IntoResponse, routing::post};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

#[cfg(feature = "web_server")]
use aios_core::{init_surreal, project_primary_db};

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
    /// 前端基地址（用于拼接完整 URL），为空时不返回 url 字段
    pub frontend_base_url: String,
}

impl Default for PlatformConfig {
    fn default() -> Self {
        Self {
            // plant3d-web 前端的校审页面路径
            frontend_relative_path: "/review/3d-view".to_string(),
            frontend_base_url: String::new(),
        }
    }
}

impl PlatformConfig {
    /// 从配置文件加载
    pub fn from_config_file() -> Self {
        if let Some(config) = super::jwt_auth::load_config() {
            return Self {
                frontend_base_url: config
                    .get_string("model_center.frontend_base_url")
                    .unwrap_or_default(),
                ..Self::default()
            };
        }
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
    /// 拼接好的完整嵌入 URL（含 output_project），外部系统可直接使用
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct EmbedUrlData {
    /// 前端相对路径
    pub relative_path: String,
    /// 平台生成的访问 token
    pub token: String,
    /// 查询参数
    pub query: EmbedUrlQuery,
    /// 稳定的业务 lineage，供前端与 validator 直接读取
    pub lineage: EmbedLineage,
    /// 主单据状态
    pub form: ReviewFormSummary,
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

#[derive(Debug, Serialize)]
pub struct EmbedLineage {
    /// 外部业务主键，跨 open/save/submit/read 必须保持稳定
    pub form_id: String,
    /// 当前打开命中的任务 ID；无既有任务时为空
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// 当前任务节点；无既有任务时为空
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_node: Option<String>,
    /// 当前任务状态；无既有任务时为空
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReviewFormSummary {
    pub form_id: String,
    pub exists: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_created: Option<bool>,
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
    /// 主单据是否存在
    pub form_exists: bool,
    /// 主单据状态
    #[serde(skip_serializing_if = "Option::is_none")]
    pub form_status: Option<String>,
    /// 任务是否已在三维端创建 (false 表示 PMS 已分配 form_id 但设计人员尚未在三维端提交提资单)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_created: Option<bool>,
    /// 当前工作流节点 (sj/jd/sh/pz)，仅当 task 存在时有值
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_node: Option<String>,
    /// 当前任务状态 (draft/submitted/in_review/approved/returned)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_status: Option<String>,
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

#[cfg(feature = "web_server")]
#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
pub struct ReviewForm {
    pub form_id: String,
    pub project_id: String,
    pub requester_id: String,
    pub source: String,
    pub status: String,
    pub task_created: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub deleted_at: Option<i64>,
}

#[cfg(feature = "web_server")]
#[derive(Debug, Deserialize, SurrealValue)]
pub struct ReviewFormRow {
    pub id: surrealdb_types::RecordId,
    pub form_id: Option<String>,
    pub project_id: Option<String>,
    pub user_id: Option<String>,
    pub requester_id: Option<String>,
    pub source: Option<String>,
    pub status: Option<String>,
    pub task_created: Option<bool>,
    pub deleted: Option<bool>,
    pub created_at: Option<surrealdb_types::Datetime>,
    pub updated_at: Option<surrealdb_types::Datetime>,
    pub deleted_at: Option<surrealdb_types::Datetime>,
}

#[cfg(feature = "web_server")]
fn review_form_from_row(row: ReviewFormRow) -> ReviewForm {
    let form_id = row
        .form_id
        .or_else(|| match row.id.key {
            surrealdb_types::RecordIdKey::String(value) => Some(value),
            _ => None,
        })
        .unwrap_or_default();

    ReviewForm {
        form_id,
        project_id: row.project_id.unwrap_or_default(),
        requester_id: row.requester_id.or(row.user_id).unwrap_or_default(),
        source: row.source.unwrap_or_default(),
        status: row
            .status
            .or_else(|| {
                row.deleted
                    .filter(|value| *value)
                    .map(|_| "deleted".to_string())
            })
            .unwrap_or_else(|| "blank".to_string()),
        task_created: row.task_created.unwrap_or(false),
        created_at: row
            .created_at
            .map(|value| value.timestamp_millis())
            .unwrap_or_default(),
        updated_at: row
            .updated_at
            .map(|value| value.timestamp_millis())
            .unwrap_or_default(),
        deleted_at: row.deleted_at.map(|value| value.timestamp_millis()),
    }
}

#[cfg(feature = "web_server")]
fn normalize_review_form_status(status: &str) -> String {
    match status.trim() {
        "draft" => "draft".to_string(),
        "deleted" => "deleted".to_string(),
        "blank" => "blank".to_string(),
        _ => "active".to_string(),
    }
}

#[cfg(feature = "web_server")]
pub fn derive_review_form_status_from_task_status(task_status: &str) -> String {
    if task_status.trim().eq_ignore_ascii_case("draft") {
        "draft".to_string()
    } else {
        "active".to_string()
    }
}

#[cfg(feature = "web_server")]
async fn ensure_review_forms_schema() -> anyhow::Result<()> {
    project_primary_db()
        .query(
            r#"
            DEFINE TABLE IF NOT EXISTS review_forms SCHEMAFULL;
            DEFINE FIELD IF NOT EXISTS form_id ON TABLE review_forms TYPE string;
            DEFINE FIELD IF NOT EXISTS project_id ON TABLE review_forms TYPE string;
            DEFINE FIELD IF NOT EXISTS user_id ON TABLE review_forms TYPE string;
            DEFINE FIELD IF NOT EXISTS role ON TABLE review_forms TYPE none | string;
            DEFINE FIELD IF NOT EXISTS requester_id ON TABLE review_forms TYPE string;
            DEFINE FIELD IF NOT EXISTS source ON TABLE review_forms TYPE string;
            DEFINE FIELD IF NOT EXISTS status ON TABLE review_forms TYPE string;
            DEFINE FIELD IF NOT EXISTS task_created ON TABLE review_forms TYPE bool DEFAULT false;
            DEFINE FIELD IF NOT EXISTS deleted ON TABLE review_forms TYPE bool DEFAULT false;
            DEFINE FIELD IF NOT EXISTS created_at ON TABLE review_forms TYPE datetime;
            DEFINE FIELD IF NOT EXISTS updated_at ON TABLE review_forms TYPE datetime DEFAULT time::now();
            DEFINE FIELD IF NOT EXISTS deleted_at ON TABLE review_forms TYPE option<datetime>;
            DEFINE INDEX IF NOT EXISTS idx_form_id ON TABLE review_forms FIELDS form_id UNIQUE;
            "#,
        )
        .await?;
    Ok(())
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
async fn get_embed_url(Json(request): Json<EmbedUrlRequest>) -> impl IntoResponse {
    info!(
        "Embed URL request: project_id={}, user_id={}",
        request.project_id, request.user_id
    );

    let jwt_claim_form_id = if let Some(ref token) = request.token {
        let token = token.trim();
        if token.split('.').count() == 3 {
            match verify_token(token) {
                Ok(claims) => Some(claims.form_id),
                Err(e) => {
                    warn!("Embed URL JWT verification failed: {}", e);
                    return (
                        StatusCode::UNAUTHORIZED,
                        Json(EmbedUrlResponse {
                            code: 401,
                            message: "unauthorized".to_string(),
                            data: None,
                            url: None,
                        }),
                    );
                }
            }
        } else {
            None
        }
    } else {
        None
    };

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
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(EmbedUrlResponse {
                        code: 401,
                        message: "unauthorized".to_string(),
                        data: None,
                        url: None,
                    }),
                );
            }
        } else {
            form_id = Some(jwt_form_id);
        }
    }

    let form_id = form_id.unwrap_or_else(generate_form_id);
    let ensured_form = match ensure_review_form_stub(
        &form_id,
        request.project_id.as_str(),
        request.user_id.as_str(),
        "pms_embed",
    )
    .await
    {
        Ok(form) => form,
        Err(error) => {
            let message = error.to_string();
            let status = if message.contains("已删除") {
                StatusCode::CONFLICT
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            return (
                status,
                Json(EmbedUrlResponse {
                    code: status.as_u16() as i32,
                    message,
                    data: None,
                    url: None,
                }),
            );
        }
    };
    let existing_task = match find_task_by_form_id(&form_id).await {
        Ok(task) => task,
        Err(e) => {
            warn!("Failed to load task for form_id={}: {}", form_id, e);
            None
        }
    };

    // 2. 生成 JWT token
    match create_token(&request.project_id, &request.user_id, None, &form_id, None) {
        Ok((token, _expires_at)) => {
            // 3. 判断是否为审核人员 (可以根据 extra_parameters 或其他逻辑判断)
            let is_reviewer = request
                .extra_parameters
                .as_ref()
                .and_then(|p| p.get("is_reviewer"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            info!("Generated form_id={}, token_len={}", form_id, token.len());

            // 4. 拼接完整 URL（如果配置了 frontend_base_url）
            let full_url = {
                let base = PLATFORM_CONFIG
                    .frontend_base_url
                    .trim()
                    .trim_end_matches('/');
                if base.is_empty() {
                    None
                } else {
                    let path = &PLATFORM_CONFIG.frontend_relative_path;
                    let clean_path = if path.starts_with('/') {
                        path.clone()
                    } else {
                        format!("/{}", path)
                    };
                    Some(format!(
                        "{}{}?user_token={}&form_id={}&user_id={}&project_id={}&output_project={}",
                        base,
                        clean_path,
                        token,
                        form_id,
                        request.user_id,
                        request.project_id,
                        request.project_id
                    ))
                }
            };

            // 5. 返回响应
            (
                StatusCode::OK,
                Json(EmbedUrlResponse {
                    code: 200,
                    message: "ok".to_string(),
                    data: Some(EmbedUrlData {
                        relative_path: PLATFORM_CONFIG.frontend_relative_path.clone(),
                        token,
                        query: EmbedUrlQuery {
                            form_id: form_id.clone(),
                            is_reviewer,
                        },
                        lineage: EmbedLineage {
                            form_id: form_id.clone(),
                            task_id: existing_task.as_ref().map(|task| task.id.clone()),
                            current_node: existing_task
                                .as_ref()
                                .map(|task| task.current_node.clone()),
                            status: existing_task.as_ref().map(|task| task.status.clone()),
                        },
                        form: ReviewFormSummary {
                            form_id: form_id.clone(),
                            exists: true,
                            status: Some(ensured_form.status.clone()),
                            task_created: Some(ensured_form.task_created),
                        },
                        task: existing_task,
                    }),
                    url: full_url,
                }),
            )
        }
        Err(e) => {
            warn!("Token generation failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(EmbedUrlResponse {
                    code: -1,
                    message: format!("Token generation failed: {}", e),
                    data: None,
                    url: None,
                }),
            )
        }
    }
}

/// 触发缓存预加载 - 代理到模型中心
#[cfg(feature = "web_server")]
async fn preload_cache(Json(request): Json<CachePreloadRequest>) -> impl IntoResponse {
    info!(
        "Cache preload request: project_id={}, initiator={}",
        request.project_id, request.initiator
    );

    // 按文档：token 为 SHA256 哈希校验
    let expected_plain = format!("{}:{}", request.project_id, request.initiator);
    if !verify_sha256_token(&expected_plain, &request.token) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(CachePreloadResponse {
                code: 401,
                message: "unauthorized".to_string(),
                data: None,
            }),
        );
    }

    // 这里可以调用模型中心的缓存接口
    // 目前返回一个占位响应
    (
        StatusCode::OK,
        Json(CachePreloadResponse {
            code: 0,
            message: "accepted".to_string(),
            data: Some(serde_json::json!({
                "task_id": format!("cache_{}", request.project_id)
            })),
        }),
    )
}

/// 同步校审流程信息
///
/// action 取值：
/// - `query`  — 纯查询，PMS 打开单据时获取最新审批数据，不写入任何记录
/// - `active` — 发起流程
/// - `agree`  — 同意
/// - `return` — 驳回
/// - `stop`   — 终止
#[cfg(feature = "web_server")]
async fn sync_workflow_handler(Json(request): Json<SyncWorkflowRequest>) -> impl IntoResponse {
    let is_query = request.action.eq_ignore_ascii_case("query");
    let request_start_time = std::time::Instant::now();

    info!(
        "[WORKFLOW_SYNC] 请求开始 - form_id={}, action={}, actor_id={}, actor_name={}, actor_role={}{}",
        request.form_id,
        request.action,
        request.actor.id,
        request.actor.name,
        request.actor.roles,
        if is_query {
            " (只读查询)"
        } else {
            " (数据写入)"
        }
    );

    // 记录请求详细信息
    if let Some(ref metadata) = request.metadata {
        info!(
            "[WORKFLOW_SYNC] 请求元数据: form_id={}, metadata={}",
            request.form_id,
            serde_json::to_string_pretty(metadata)
                .unwrap_or_else(|_| "metadata序列化失败".to_string())
        );
    }

    if let Some(ref next) = request.next_step {
        info!(
            "[WORKFLOW_SYNC] 下一步信息: form_id={}, assignee_id={}, assignee_name={}, assignee_roles={}",
            request.form_id, next.assignee_id, next.name, next.roles
        );
    }

    if let Some(ref comments) = request.comments {
        let comment_preview = if comments.len() > 100 {
            format!("{}...", &comments[..100])
        } else {
            comments.clone()
        };
        info!(
            "[WORKFLOW_SYNC] 审批意见: form_id={}, comment_preview={}",
            request.form_id, comment_preview
        );
    }

    // 按文档：token 为 SHA256 哈希校验
    let expected_plain = format!("{}:{}", request.form_id, request.actor.id);
    let token_valid = verify_sha256_token(&expected_plain, &request.token);

    info!(
        "[WORKFLOW_SYNC] Token校验: form_id={}, actor_id={}, token_valid={}",
        request.form_id, request.actor.id, token_valid
    );

    if !token_valid {
        warn!(
            "[WORKFLOW_SYNC] Token校验失败 - form_id={}, actor_id={}, expected_plain={}",
            request.form_id, request.actor.id, expected_plain
        );
        return (
            StatusCode::UNAUTHORIZED,
            Json(SyncWorkflowResponse {
                code: 401,
                message: "unauthorized".to_string(),
                data: None,
            }),
        );
    }

    info!(
        "[WORKFLOW_SYNC] Token校验通过 - form_id={}, 开始处理业务逻辑",
        request.form_id
    );

    // action=query 时跳过意见写入，仅返回数据
    if !is_query {
        info!(
            "[WORKFLOW_SYNC] 非查询操作，开始写入审批意见 - form_id={}, action={}, node={}",
            request.form_id, request.action, request.actor.roles
        );

        let node = request.actor.roles.trim().to_string();
        let seq_order = match node.as_str() {
            "sj" => 1,
            "jd" => 2,
            "sh" => 3,
            "pz" => 4,
            _ => 0,
        };

        info!(
            "[WORKFLOW_SYNC] 节点顺序映射 - form_id={}, node={}, seq_order={}",
            request.form_id, node, seq_order
        );

        let model_refnos = query_workflow_models(&request.form_id)
            .await
            .unwrap_or_default();

        info!(
            "[WORKFLOW_SYNC] 查询到关联模型 - form_id={}, model_count={}, models={:?}",
            request.form_id,
            model_refnos.len(),
            if model_refnos.len() <= 10 {
                format!("{:?}", model_refnos)
            } else {
                format!("前10个: {:?}", &model_refnos[..10])
            }
        );

        if let Some(comment) = request
            .comments
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            info!(
                "[WORKFLOW_SYNC] 写入审批意见 - form_id={}, node={}, seq_order={}, author={}, comment_length={}",
                request.form_id,
                node,
                seq_order,
                request.actor.id,
                comment.len()
            );

            let db_result = project_primary_db()
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

            match db_result {
                Ok(_) => {
                    info!(
                        "[WORKFLOW_SYNC] 审批意见写入成功 - form_id={}, author={}",
                        request.form_id, request.actor.id
                    );
                }
                Err(e) => {
                    warn!(
                        "[WORKFLOW_SYNC] 审批意见写入失败 - form_id={}, author={}, error={}",
                        request.form_id, request.actor.id, e
                    );
                }
            }
        } else {
            info!(
                "[WORKFLOW_SYNC] 无审批意见，跳过写入 - form_id={}, action={}",
                request.form_id, request.action
            );
        }
    } else {
        info!(
            "[WORKFLOW_SYNC] 查询模式，跳过数据写入 - form_id={}",
            request.form_id
        );
    }

    info!(
        "[WORKFLOW_SYNC] 开始查询工作流数据 - form_id={}",
        request.form_id
    );

    // 查询该 form_id 关联的数据
    let data = match query_workflow_data(&request.form_id).await {
        Ok(d) => {
            info!(
                "[WORKFLOW_SYNC] 工作流数据查询成功 - form_id={}, models={}, opinions={}, attachments={}",
                request.form_id,
                d.models.len(),
                d.opinions.len(),
                d.attachments.len()
            );

            // 记录详细的意见信息
            if !d.opinions.is_empty() {
                for (i, opinion) in d.opinions.iter().enumerate() {
                    info!(
                        "[WORKFLOW_SYNC] 意见详情[{}] - form_id={}, node={}, author={}, created_at={}, opinion_preview={}",
                        i,
                        request.form_id,
                        opinion.node,
                        opinion.author,
                        opinion.created_at,
                        if opinion.opinion.len() > 50 {
                            format!("{}...", &opinion.opinion[..50])
                        } else {
                            opinion.opinion.clone()
                        }
                    );
                }
            }

            // 记录附件信息
            if !d.attachments.is_empty() {
                for (i, attachment) in d.attachments.iter().enumerate() {
                    info!(
                        "[WORKFLOW_SYNC] 附件详情[{}] - form_id={}, file_id={}, file_type={}, description={}",
                        i,
                        request.form_id,
                        attachment.id,
                        attachment.r#type,
                        if attachment.description.is_empty() {
                            "无描述"
                        } else {
                            attachment.description.as_str()
                        }
                    );
                }
            }

            d
        }
        Err(e) => {
            warn!(
                "[WORKFLOW_SYNC] 工作流数据查询失败 - form_id={}, error={}",
                request.form_id, e
            );
            SyncWorkflowData::default()
        }
    };

    let processing_time = request_start_time.elapsed();

    info!(
        "[WORKFLOW_SYNC] 请求处理完成 - form_id={}, action={}, processing_time_ms={}, models={}, opinions={}, attachments={}",
        request.form_id,
        request.action,
        processing_time.as_millis(),
        data.models.len(),
        data.opinions.len(),
        data.attachments.len()
    );

    let response = SyncWorkflowResponse {
        code: 200,
        message: "success".to_string(),
        data: Some(data),
    };

    info!(
        "[WORKFLOW_SYNC] 返回响应 - form_id={}, response_code={}, response_message={}",
        request.form_id, response.code, response.message
    );

    (StatusCode::OK, Json(response))
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
async fn delete_review_data(Json(request): Json<DeleteReviewRequest>) -> impl IntoResponse {
    let joined = request.form_ids.join(",");
    let expected_plain = format!("{}:{}", joined, request.operator_id);
    if !verify_sha256_token(&expected_plain, &request.token) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(DeleteReviewResponse {
                code: 401,
                message: "unauthorized".to_string(),
            }),
        );
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
                "LET $task_ids = SELECT VALUE id FROM review_tasks WHERE form_id = $form_id;\nDELETE FROM review_records WHERE task_id IN $task_ids;\nDELETE FROM review_workflow_history WHERE task_id IN $task_ids;\nDELETE FROM review_history WHERE task_id IN $task_ids;",
            )
            .bind(("form_id", form_id.clone()))
            .await;
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
        let _ = mark_review_form_deleted(form_id).await;
    }

    (
        StatusCode::OK,
        Json(DeleteReviewResponse {
            code: 200,
            message: "ok".to_string(),
        }),
    )
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

/// 查询主单据
#[cfg(feature = "web_server")]
pub async fn get_review_form_by_form_id(form_id: &str) -> anyhow::Result<Option<ReviewForm>> {
    ensure_review_forms_schema().await?;

    let mut response = project_primary_db()
        .query(
            r#"
            SELECT * FROM review_forms
            WHERE form_id = $form_id
            ORDER BY updated_at DESC, created_at DESC
            LIMIT 1
            "#,
        )
        .bind(("form_id", form_id.to_string()))
        .await?;

    let rows: Vec<ReviewFormRow> = response.take(0)?;
    Ok(rows.into_iter().next().map(review_form_from_row))
}

/// 创建或更新主单据桩记录
#[cfg(feature = "web_server")]
pub async fn ensure_review_form_stub(
    form_id: &str,
    project_id: &str,
    requester_id: &str,
    source: &str,
) -> anyhow::Result<ReviewForm> {
    ensure_review_forms_schema().await?;

    if let Some(existing) = get_review_form_by_form_id(form_id).await? {
        if existing.status == "deleted" {
            anyhow::bail!("form_id={} 对应主单据已删除，禁止重新打开", form_id);
        }

        project_primary_db()
            .query(
                r#"
                UPDATE review_forms
                SET
                    project_id = IF string::len(string::trim($project_id)) > 0 THEN $project_id ELSE project_id END,
                    user_id = IF string::len(string::trim($requester_id)) > 0 THEN $requester_id ELSE user_id END,
                    requester_id = IF string::len(string::trim($requester_id)) > 0 THEN $requester_id ELSE requester_id END,
                    source = $source,
                    deleted = false,
                    updated_at = time::now()
                WHERE form_id = $form_id
                "#,
            )
            .bind(("form_id", form_id.to_string()))
            .bind(("project_id", project_id.trim().to_string()))
            .bind(("requester_id", requester_id.trim().to_string()))
            .bind(("source", source.trim().to_string()))
            .await?;

        return get_review_form_by_form_id(form_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("form_id={} 主单据更新后读取失败", form_id));
    }

    project_primary_db()
        .query(
            r#"
            CREATE review_forms CONTENT {
                form_id: $form_id,
                project_id: $project_id,
                user_id: $requester_id,
                requester_id: $requester_id,
                role: NONE,
                source: $source,
                status: 'blank',
                task_created: false,
                deleted: false,
                created_at: time::now(),
                updated_at: time::now(),
                deleted_at: NONE
            }
            "#,
        )
        .bind(("form_id", form_id.to_string()))
        .bind(("project_id", project_id.trim().to_string()))
        .bind(("requester_id", requester_id.trim().to_string()))
        .bind(("source", source.trim().to_string()))
        .await?;

    get_review_form_by_form_id(form_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("form_id={} 主单据创建后读取失败", form_id))
}

/// 按 task 状态回写主单据状态
#[cfg(feature = "web_server")]
pub async fn sync_review_form_with_task_status(
    form_id: &str,
    project_id: Option<&str>,
    requester_id: Option<&str>,
    source: &str,
    task_status: &str,
) -> anyhow::Result<()> {
    let _ = ensure_review_form_stub(
        form_id,
        project_id.unwrap_or_default(),
        requester_id.unwrap_or_default(),
        source,
    )
    .await?;

    let form_status = derive_review_form_status_from_task_status(task_status);
    project_primary_db()
        .query(
            r#"
            UPDATE review_forms
            SET
                project_id = IF string::len(string::trim($project_id)) > 0 THEN $project_id ELSE project_id END,
                user_id = IF string::len(string::trim($requester_id)) > 0 THEN $requester_id ELSE user_id END,
                requester_id = IF string::len(string::trim($requester_id)) > 0 THEN $requester_id ELSE requester_id END,
                source = $source,
                task_created = true,
                status = $status,
                deleted = false,
                deleted_at = NONE,
                updated_at = time::now()
            WHERE form_id = $form_id
            "#,
        )
        .bind(("form_id", form_id.to_string()))
        .bind(("project_id", project_id.unwrap_or_default().trim().to_string()))
        .bind(("requester_id", requester_id.unwrap_or_default().trim().to_string()))
        .bind(("source", source.trim().to_string()))
        .bind(("status", form_status))
        .await?;

    Ok(())
}

/// 标记主单据已删除
#[cfg(feature = "web_server")]
pub async fn mark_review_form_deleted(form_id: &str) -> anyhow::Result<()> {
    ensure_review_forms_schema().await?;

    if get_review_form_by_form_id(form_id).await?.is_none() {
        return Ok(());
    }

    project_primary_db()
        .query(
            r#"
            UPDATE review_forms
            SET
                status = 'deleted',
                task_created = false,
                deleted = true,
                deleted_at = time::now(),
                updated_at = time::now()
            WHERE form_id = $form_id
            "#,
        )
        .bind(("form_id", form_id.to_string()))
        .await?;

    Ok(())
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
    Ok(rows
        .into_iter()
        .map(|r| WorkflowOpinion {
            model: r.model_refnos.unwrap_or_default(),
            node: r.node.unwrap_or_default(),
            order: r.seq_order.unwrap_or(0),
            author: r.author.unwrap_or_default(),
            opinion: r.opinion.unwrap_or_default(),
            created_at: r.created_at.map(|dt| dt.to_string()).unwrap_or_default(),
        })
        .collect())
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
    Ok(rows
        .into_iter()
        .map(|r| WorkflowAttachment {
            model: r.model_refnos.unwrap_or_default(),
            id: r.file_id.unwrap_or_default(),
            r#type: r.file_type.unwrap_or_default(),
            download_url: r.download_url.unwrap_or_default(),
            description: r.description.unwrap_or_default(),
            file_ext: r.file_ext.unwrap_or_default(),
        })
        .collect())
}

/// 汇总查询表单的所有校审数据
#[cfg(feature = "web_server")]
async fn query_workflow_data(form_id: &str) -> anyhow::Result<SyncWorkflowData> {
    let models = query_workflow_models(form_id).await.unwrap_or_default();
    let opinions = query_workflow_opinions(form_id).await.unwrap_or_default();
    let attachments = query_workflow_attachments(form_id)
        .await
        .unwrap_or_default();

    let review_form = get_review_form_by_form_id(form_id).await.unwrap_or(None);
    let task = find_task_by_form_id(form_id).await.unwrap_or(None);
    let task_created = Some(task.is_some());
    let current_node = task.as_ref().map(|t| t.current_node.clone());
    let task_status = task.as_ref().map(|t| t.status.clone());
    let form_exists = review_form.is_some();
    let form_status = review_form
        .as_ref()
        .map(|form| normalize_review_form_status(form.status.as_str()));

    Ok(SyncWorkflowData {
        models,
        opinions,
        attachments,
        form_exists,
        form_status,
        task_created,
        current_node,
        task_status,
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
pub fn notify_workflow_sync_async(
    task_id: String,
    action: String,
    operator_id: String,
    comment: Option<String>,
) {
    let notify_start_time = std::time::Instant::now();

    info!(
        "[WORKFLOW_NOTIFY] 异步通知开始 - task_id={}, action={}, operator_id={}, has_comment={}",
        task_id,
        action,
        operator_id,
        comment.is_some()
    );

    if let Some(ref comment_text) = comment {
        let comment_preview = if comment_text.len() > 100 {
            format!("{}...", &comment_text[..100])
        } else {
            comment_text.clone()
        };
        info!(
            "[WORKFLOW_NOTIFY] 通知备注内容 - task_id={}, comment_preview={}",
            task_id, comment_preview
        );
    }

    if EXTERNAL_REVIEW_CONFIG.is_mock() {
        info!(
            "[WORKFLOW_NOTIFY] Mock模式，跳过实际通知 - task_id={}, action={}, operator_id={}",
            task_id, action, operator_id
        );
        return;
    }

    info!(
        "[WORKFLOW_NOTIFY] 启动异步任务 - task_id={}, action={}, target_url={}",
        task_id,
        action,
        format!(
            "{}{}",
            EXTERNAL_REVIEW_CONFIG.base_url.trim_end_matches('/'),
            EXTERNAL_REVIEW_CONFIG.workflow_sync_path
        )
    );

    let spawn_task_id = task_id.clone();
    let spawn_action = action.clone();
    let spawn_operator_id = operator_id.clone();
    let spawn_comment = comment.clone();

    tokio::spawn(async move {
        let task_start_time = std::time::Instant::now();
        let result = notify_workflow_sync(
            &spawn_task_id,
            &spawn_action,
            &spawn_operator_id,
            spawn_comment.as_deref(),
        )
        .await;

        let task_duration = task_start_time.elapsed();

        match result {
            Ok(_) => {
                info!(
                    "[WORKFLOW_NOTIFY] 异步通知成功 - task_id={}, action={}, operator_id={}, duration_ms={}",
                    spawn_task_id,
                    spawn_action,
                    spawn_operator_id,
                    task_duration.as_millis()
                );
            }
            Err(e) => {
                warn!(
                    "[WORKFLOW_NOTIFY] 异步通知失败 - task_id={}, action={}, operator_id={}, duration_ms={}, error={}",
                    spawn_task_id,
                    spawn_action,
                    spawn_operator_id,
                    task_duration.as_millis(),
                    e
                );
            }
        }
    });

    info!(
        "[WORKFLOW_NOTIFY] 异步任务已启动 - task_id={}, action={}, setup_time_ms={}",
        task_id,
        action,
        notify_start_time.elapsed().as_millis()
    );
}

#[cfg(feature = "web_server")]
async fn notify_workflow_sync(
    task_id: &str,
    action: &str,
    operator_id: &str,
    comment: Option<&str>,
) -> anyhow::Result<()> {
    let request_start_time = std::time::Instant::now();
    let config = &*EXTERNAL_REVIEW_CONFIG;
    let url = format!(
        "{}{}",
        config.base_url.trim_end_matches('/'),
        config.workflow_sync_path
    );

    info!(
        "[WORKFLOW_NOTIFY] HTTP请求开始 - task_id={}, action={}, operator_id={}, url={}, timeout_sec={}",
        task_id, action, operator_id, url, config.timeout_seconds
    );

    let token_plain = format!("{}:{}", task_id, operator_id);
    let token = sha256_hex(&format!("{}:{}", config.auth_secret, token_plain));

    let body = serde_json::json!({
        "task_id": task_id,
        "action": action,
        "operator_id": operator_id,
        "comment": comment.unwrap_or(""),
        "token": token,
    });

    info!(
        "[WORKFLOW_NOTIFY] 请求体准备完成 - task_id={}, body_size={} bytes",
        task_id,
        body.to_string().len()
    );

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(config.timeout_seconds))
        .build()?;

    info!(
        "[WORKFLOW_NOTIFY] 发送HTTP POST请求 - task_id={}, url={}",
        task_id, url
    );

    let http_start_time = std::time::Instant::now();
    let resp = client.post(&url).json(&body).send().await?;
    let http_duration = http_start_time.elapsed();

    info!(
        "[WORKFLOW_NOTIFY] HTTP响应接收 - task_id={}, status_code={}, http_duration_ms={}",
        task_id,
        resp.status(),
        http_duration.as_millis()
    );

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();

        warn!(
            "[WORKFLOW_NOTIFY] 外部系统返回错误 - task_id={}, status_code={}, response_body={}",
            task_id, status, text
        );

        anyhow::bail!("外部系统返回错误 {}: {}", status, text);
    }

    let total_duration = request_start_time.elapsed();

    info!(
        "[WORKFLOW_NOTIFY] 工作流同步成功 - task_id={}, action={}, operator_id={}, total_duration_ms={}",
        task_id,
        action,
        operator_id,
        total_duration.as_millis()
    );

    Ok(())
}

/// 异步通知外部系统删除校审数据
#[cfg(feature = "web_server")]
pub fn notify_workflow_delete_async(task_id: String, operator_id: String) {
    let delete_start_time = std::time::Instant::now();

    info!(
        "[WORKFLOW_DELETE] 异步删除通知开始 - task_id={}, operator_id={}",
        task_id, operator_id
    );

    if EXTERNAL_REVIEW_CONFIG.is_mock() {
        info!(
            "[WORKFLOW_DELETE] Mock模式，跳过删除通知 - task_id={}, operator_id={}",
            task_id, operator_id
        );
        return;
    }

    info!(
        "[WORKFLOW_DELETE] 启动删除异步任务 - task_id={}, operator_id={}, target_url={}",
        task_id,
        operator_id,
        format!(
            "{}{}",
            EXTERNAL_REVIEW_CONFIG.base_url.trim_end_matches('/'),
            EXTERNAL_REVIEW_CONFIG.workflow_delete_path
        )
    );

    let spawn_task_id = task_id.clone();
    let spawn_operator_id = operator_id.clone();

    tokio::spawn(async move {
        let task_start_time = std::time::Instant::now();
        let result = notify_workflow_delete(&spawn_task_id, &spawn_operator_id).await;

        let task_duration = task_start_time.elapsed();

        match result {
            Ok(_) => {
                info!(
                    "[WORKFLOW_DELETE] 异步删除通知成功 - task_id={}, operator_id={}, duration_ms={}",
                    spawn_task_id,
                    spawn_operator_id,
                    task_duration.as_millis()
                );
            }
            Err(e) => {
                warn!(
                    "[WORKFLOW_DELETE] 异步删除通知失败 - task_id={}, operator_id={}, duration_ms={}, error={}",
                    spawn_task_id,
                    spawn_operator_id,
                    task_duration.as_millis(),
                    e
                );
            }
        }
    });

    info!(
        "[WORKFLOW_DELETE] 异步删除任务已启动 - task_id={}, operator_id={}, setup_time_ms={}",
        task_id,
        operator_id,
        delete_start_time.elapsed().as_millis()
    );
}

#[cfg(feature = "web_server")]
async fn notify_workflow_delete(task_id: &str, operator_id: &str) -> anyhow::Result<()> {
    let request_start_time = std::time::Instant::now();
    let config = &*EXTERNAL_REVIEW_CONFIG;
    let url = format!(
        "{}{}",
        config.base_url.trim_end_matches('/'),
        config.workflow_delete_path
    );

    info!(
        "[WORKFLOW_DELETE] HTTP删除请求开始 - task_id={}, operator_id={}, url={}, timeout_sec={}",
        task_id, operator_id, url, config.timeout_seconds
    );

    let token_plain = format!("{}:{}", task_id, operator_id);
    let token = sha256_hex(&format!("{}:{}", config.auth_secret, token_plain));

    let body = serde_json::json!({
        "task_id": task_id,
        "operator_id": operator_id,
        "token": token,
    });

    info!(
        "[WORKFLOW_DELETE] 删除请求体准备完成 - task_id={}, body_size={} bytes",
        task_id,
        body.to_string().len()
    );

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(config.timeout_seconds))
        .build()?;

    info!(
        "[WORKFLOW_DELETE] 发送HTTP删除请求 - task_id={}, url={}",
        task_id, url
    );

    let http_start_time = std::time::Instant::now();
    let resp = client.post(&url).json(&body).send().await?;
    let http_duration = http_start_time.elapsed();

    info!(
        "[WORKFLOW_DELETE] HTTP删除响应接收 - task_id={}, status_code={}, http_duration_ms={}",
        task_id,
        resp.status(),
        http_duration.as_millis()
    );

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();

        warn!(
            "[WORKFLOW_DELETE] 外部系统删除返回错误 - task_id={}, status_code={}, response_body={}",
            task_id, status, text
        );

        anyhow::bail!("外部系统返回错误 {}: {}", status, text);
    }

    let total_duration = request_start_time.elapsed();

    info!(
        "[WORKFLOW_DELETE] 删除通知成功 - task_id={}, operator_id={}, total_duration_ms={}",
        task_id,
        operator_id,
        total_duration.as_millis()
    );

    Ok(())
}

#[cfg(test)]
#[cfg(feature = "web_server")]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use serde::Deserialize;
    use tower::ServiceExt;

    #[derive(Debug, Deserialize)]
    struct EmbedUrlResponseBody {
        code: i32,
        message: String,
        data: Option<serde_json::Value>,
        url: Option<String>,
    }

    #[derive(Debug, Deserialize)]
    struct EmbedUrlQueryBody {
        form_id: String,
    }

    #[derive(Debug, Deserialize)]
    struct EmbedLineageBody {
        form_id: String,
        task_id: Option<String>,
        current_node: Option<String>,
        status: Option<String>,
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
        let (token, _) =
            create_token("project-1", "user-1", None, "FORM-EXPECTED", Some("sj")).unwrap();

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
        let (token, _) =
            create_token("project-1", "user-1", None, "FORM-EXPECTED", Some("sj")).unwrap();

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
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let payload: EmbedUrlResponseBody = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload.code, 200);
        assert_eq!(payload.message, "ok");
        let data = payload.data.expect("embed data");
        assert_eq!(
            data.get("query")
                .and_then(|q| q.get("form_id").or_else(|| q.get("formId")))
                .and_then(|v| v.as_str()),
            Some("FORM-EXPECTED")
        );
        let lineage: EmbedLineageBody =
            serde_json::from_value(data.get("lineage").cloned().expect("lineage")).unwrap();
        assert_eq!(lineage.form_id, "FORM-EXPECTED");
        assert_eq!(lineage.task_id, None);
        assert_eq!(lineage.current_node, None);
        assert_eq!(lineage.status, None);
        let response_token = data
            .get("token")
            .and_then(|v| v.as_str())
            .expect("response token");
        assert_eq!(verify_token(response_token).unwrap().user_id, "user-1");
        assert!(data.get("task").is_none() || data.get("task").is_some_and(|v| v.is_null()));
    }

    #[tokio::test]
    async fn test_embed_url_rejects_tampered_jwt_even_if_form_id_matches() {
        let app = create_model_center_routes();
        let (token, _) =
            create_token("project-1", "user-1", None, "FORM-EXPECTED", Some("sj")).unwrap();
        let mut parts = token.split('.').collect::<Vec<_>>();
        parts[2] = "tampered-signature";
        let tampered_token = parts.join(".");

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
                            "token": tampered_token
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
    #[ignore = "requires an initialized review_tasks database backing store"]
    async fn test_embed_url_returns_existing_task_for_form_id() {
        let form_id = "FORM-DB-BACKED-EXISTING";
        insert_task_with_form_id(form_id, "user-existing").await;

        let app = create_model_center_routes();
        let (token, _) =
            create_token("project-1", "user-existing", None, form_id, Some("jd")).unwrap();

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
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let payload: EmbedUrlResponseBody = serde_json::from_slice(&body).unwrap();
        assert_eq!(status, StatusCode::OK);
        assert_eq!(payload.code, 200);
        let data = payload.data.expect("embed data");
        assert_eq!(
            data.get("query")
                .and_then(|q| q.get("form_id").or_else(|| q.get("formId")))
                .and_then(|v| v.as_str()),
            Some(form_id)
        );
        let lineage: EmbedLineageBody =
            serde_json::from_value(data.get("lineage").cloned().expect("lineage")).unwrap();
        assert_eq!(lineage.form_id, form_id);
        assert!(
            lineage
                .task_id
                .as_deref()
                .is_some_and(|task_id| task_id.starts_with("task-form-db-backed-existing"))
        );
        assert_eq!(lineage.current_node.as_deref(), Some("jd"));
        assert_eq!(lineage.status.as_deref(), Some("in_review"));
        let task = data
            .get("task")
            .and_then(|v| v.as_object())
            .expect("existing task restored");
        assert_eq!(
            task.get("form_id")
                .or_else(|| task.get("formId"))
                .and_then(|v| v.as_str()),
            Some(form_id)
        );
        assert_eq!(
            task.get("requesterId").and_then(|v| v.as_str()),
            Some("user-existing")
        );
        assert_eq!(task.get("currentNode").and_then(|v| v.as_str()), Some("jd"));
        assert_eq!(
            task.get("status").and_then(|v| v.as_str()),
            Some("in_review")
        );
        assert!(
            task.get("id")
                .and_then(|v| v.as_str())
                .is_some_and(|id| id.starts_with("task-form-db-backed-existing"))
        );
        let response_token = data
            .get("token")
            .and_then(|v| v.as_str())
            .expect("response token");
        assert_eq!(verify_token(response_token).unwrap().form_id, form_id);

        cleanup_form(form_id).await;
    }
}
