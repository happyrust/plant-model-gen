//! Request/Response types for PMS ↔ platform API.

use serde::{Deserialize, Serialize};

#[cfg(feature = "web_server")]
use surrealdb::types::{self as surrealdb_types, SurrealValue};

use crate::web_api::review_api::ReviewTask;

// ============================================================================
// Embed URL
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct EmbedUrlRequest {
    pub project_id: String,
    pub user_id: String,
    pub form_id: Option<String>,
    pub token: Option<String>,
    #[serde(default)]
    pub extra_parameters: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct EmbedUrlResponse {
    pub code: i32,
    pub message: String,
    pub data: Option<EmbedUrlData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct EmbedUrlData {
    pub relative_path: String,
    pub token: String,
    pub query: EmbedUrlQuery,
    pub lineage: EmbedLineage,
    pub form: ReviewFormSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<ReviewTask>,
}

#[derive(Debug, Serialize)]
pub struct EmbedUrlQuery {
    pub form_id: String,
    pub is_reviewer: bool,
}

#[derive(Debug, Serialize)]
pub struct EmbedLineage {
    pub form_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_node: Option<String>,
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

// ============================================================================
// Cache Preload
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct CachePreloadRequest {
    pub project_id: String,
    pub initiator: String,
    pub token: String,
}

#[derive(Debug, Serialize)]
pub struct CachePreloadResponse {
    pub code: i32,
    pub message: String,
    pub data: Option<serde_json::Value>,
}

// ============================================================================
// Workflow Sync
// ============================================================================

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

#[derive(Debug, Serialize)]
pub struct SyncWorkflowResponse {
    pub code: i32,
    pub message: String,
    pub data: Option<SyncWorkflowData>,
}

#[derive(Debug, Serialize, Default)]
pub struct SyncWorkflowData {
    pub models: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub records: Vec<WorkflowRecord>,
    pub annotation_comments: Vec<WorkflowAnnotationComment>,
    pub attachments: Vec<WorkflowAttachment>,
    pub form_exists: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub form_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_created: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_node: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_status: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct WorkflowRecord {
    pub id: String,
    pub task_id: String,
    pub r#type: String,
    pub annotations: Vec<serde_json::Value>,
    pub cloud_annotations: Vec<serde_json::Value>,
    pub rect_annotations: Vec<serde_json::Value>,
    pub obb_annotations: Vec<serde_json::Value>,
    pub measurements: Vec<serde_json::Value>,
    pub note: String,
    pub confirmed_at: String,
}

#[derive(Debug, Serialize)]
pub struct WorkflowAnnotationComment {
    pub id: String,
    pub annotation_id: String,
    pub annotation_type: String,
    pub author_id: String,
    pub author_name: String,
    pub author_role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to_id: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct WorkflowAttachment {
    pub model: Vec<String>,
    pub id: String,
    pub r#type: String,
    pub route_url: String,
    pub download_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_url: Option<String>,
    pub description: String,
    pub file_ext: String,
}

// ============================================================================
// Delete
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct DeleteReviewRequest {
    pub form_ids: Vec<String>,
    pub operator_id: String,
    pub token: String,
}

#[derive(Debug, Serialize)]
pub struct DeleteReviewResponse {
    pub code: i32,
    pub message: String,
}

// ============================================================================
// Review Form (DB entity)
// ============================================================================

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
pub fn review_form_from_row(row: ReviewFormRow) -> ReviewForm {
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
pub fn normalize_review_form_status(status: &str) -> String {
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
