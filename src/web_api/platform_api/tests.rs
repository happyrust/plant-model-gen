//! Tests for platform API handlers.

#![cfg(feature = "web_server")]

use super::*;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use serde::Deserialize;
use tower::ServiceExt;

use crate::web_api::jwt_auth::{create_token, verify_token};
use aios_core::{init_surreal, project_primary_db};

#[derive(Debug, Deserialize)]
struct EmbedUrlResponseBody {
    code: i32,
    message: String,
    data: Option<serde_json::Value>,
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EmbedLineageBody {
    form_id: String,
    task_id: Option<String>,
    current_node: Option<String>,
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
struct VerifyWorkflowResponseBody {
    code: i32,
    message: String,
    data: Option<VerifyWorkflowDataBody>,
    error_code: Option<String>,
    annotation_check: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct VerifyWorkflowDataBody {
    passed: bool,
    action: String,
    block_code: Option<String>,
    current_node: Option<String>,
    task_status: Option<String>,
    next_step: Option<String>,
    actor_id: Option<String>,
    owner_id: Option<String>,
    owner_source: Option<String>,
    expected_next_node: Option<String>,
    requested_next_step: Option<VerifyWorkflowNextStepBody>,
    reason: String,
    recommended_action: String,
}

#[derive(Debug, Deserialize)]
struct VerifyWorkflowNextStepBody {
    assignee_id: String,
    name: String,
    roles: String,
}

async fn cleanup_form(form_id: &str) {
    let _ = project_primary_db()
        .query(
            "LET $ids = SELECT VALUE id FROM review_tasks WHERE form_id = $form_id; DELETE $ids;",
        )
        .bind(("form_id", form_id.to_string()))
        .await;
    let _ = project_primary_db()
        .query("DELETE review_forms WHERE form_id = $form_id;")
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

async fn insert_task_seed(
    form_id: &str,
    requester_id: &str,
    checker_id: &str,
    approver_id: &str,
    current_node: &str,
    status: &str,
) {
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
                checker_id = $checker_id,
                checker_name = $checker_id,
                approver_id = $approver_id,
                approver_name = $approver_id,
                reviewer_id = $checker_id,
                reviewer_name = $checker_id,
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
        .bind(("description", "seeded verify task".to_string()))
        .bind(("model_name", "demo-model".to_string()))
        .bind(("status", status.to_string()))
        .bind(("requester_id", requester_id.to_string()))
        .bind(("checker_id", checker_id.to_string()))
        .bind(("approver_id", approver_id.to_string()))
        .bind(("current_node", current_node.to_string()))
        .await
        .expect("seed verify task");
}

async fn insert_pending_review_record(task_id: &str, form_id: &str) {
    insert_review_record_with_state(
        task_id,
        form_id,
        "ann-verify-pending-1",
        "待确认批注",
        Some("fixed"),
        Some("pending"),
        "verify-pending",
    )
    .await;
}

/// 通用批注种子辅助：按给定 (resolution_status, decision_status) 生成单条
/// text 批注，由 `classify_annotation_state` 决定最终 gate 状态：
/// - `decision_status = "agreed"` → Approved
/// - `decision_status = "rejected"` → Rejected
/// - `resolution_status = "fixed" | "wont_fix"` 且 decision_status 非 agreed/rejected → PendingReview
/// - 其他（含完全没传 reviewState）→ Open
#[allow(clippy::too_many_arguments)]
async fn insert_review_record_with_state(
    task_id: &str,
    form_id: &str,
    annotation_id: &str,
    annotation_text: &str,
    resolution_status: Option<&str>,
    decision_status: Option<&str>,
    record_label: &str,
) {
    let review_state_json = match (resolution_status, decision_status) {
        (None, None) => "NONE".to_string(),
        _ => format!(
            r#"{{
                resolutionStatus: {res},
                decisionStatus: {dec},
                updatedAt: 1710000000000,
                updatedByName: "SJ",
                updatedByRole: "sj"
            }}"#,
            res = resolution_status
                .map(|v| format!("\"{}\"", v))
                .unwrap_or_else(|| "NONE".to_string()),
            dec = decision_status
                .map(|v| format!("\"{}\"", v))
                .unwrap_or_else(|| "NONE".to_string()),
        ),
    };

    let sql = format!(
        r#"
        CREATE review_records CONTENT {{
            id: $id,
            task_id: $task_id,
            form_id: $form_id,
            type: "batch",
            annotations: [
                {{
                    id: $annotation_id,
                    annotationType: "text",
                    text: $annotation_text,
                    refnos: ["24381/145018"],
                    reviewState: {review_state}
                }}
            ],
            cloud_annotations: [],
            rect_annotations: [],
            obb_annotations: [],
            measurements: [],
            note: $record_label,
            current_node: "jd",
            operator_id: "SJ",
            operator_name: "SJ",
            slot_key: $slot_key,
            snapshot_hash: $snapshot_hash,
            confirmed_at: time::now()
        }}
        "#,
        review_state = review_state_json,
    );

    project_primary_db()
        .query(sql)
        .bind(("id", format!("record-{}-{}", form_id.to_lowercase(), record_label)))
        .bind(("task_id", task_id.to_string()))
        .bind(("form_id", form_id.to_string()))
        .bind(("annotation_id", annotation_id.to_string()))
        .bind(("annotation_text", annotation_text.to_string()))
        .bind(("record_label", format!("seed {}", record_label)))
        .bind(("slot_key", format!("slot-{}", record_label)))
        .bind(("snapshot_hash", format!("hash-{}", record_label)))
        .await
        .expect("seed review record with state");
}

#[tokio::test]
async fn test_embed_url_ignores_mismatched_form_id_from_jwt() {
    let app = create_platform_api_routes();
    let (token, _) = create_token("project-1", "user-1", None, Some("sj"), None).unwrap();

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

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let payload: EmbedUrlResponseBody = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload.code, 200);
    let data = payload.data.expect("embed data");
    let query = data
        .get("query")
        .and_then(|value| value.as_object())
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        query.get("form_id").and_then(|value| value.as_str()),
        Some("FORM-OTHER")
    );
}

#[tokio::test]
async fn test_embed_url_accepts_matching_form_id_from_jwt() {
    let _ = init_surreal().await;
    cleanup_form("FORM-EXPECTED").await;
    let app = create_platform_api_routes();
    let (token, _) = create_token("project-1", "user-1", None, Some("sj"), None).unwrap();

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
async fn test_embed_url_includes_workflow_role_in_signed_token_when_extra_has_workflow_role() {
    let form_id = "FORM-ROLE-REQUEST";
    let _ = init_surreal().await;
    cleanup_form(form_id).await;
    let app = create_platform_api_routes();

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
                        "form_id": form_id,
                        "extra_parameters": {
                            "workflow_role": "jd"
                        }
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
    let data = payload.data.expect("embed data");
    let response_token = data
        .get("token")
        .and_then(|v| v.as_str())
        .expect("response token");
    let claims = verify_token(response_token).unwrap();
    assert_eq!(claims.role.as_deref(), Some("jd"));
}

#[tokio::test]
async fn test_embed_url_accepts_workflow_role_field_name() {
    let form_id = "FORM-WORKFLOW-ROLE-ALIAS";
    let _ = init_surreal().await;
    cleanup_form(form_id).await;
    let app = create_platform_api_routes();

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
                        "form_id": form_id,
                        "workflow_role": "sh"
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
    let data = payload.data.expect("embed data");
    let response_token = data
        .get("token")
        .and_then(|v| v.as_str())
        .expect("response token");
    let claims = verify_token(response_token).unwrap();
    assert_eq!(claims.role.as_deref(), Some("sh"));
}

#[tokio::test]
async fn test_embed_url_accepts_legacy_json_key_role() {
    let form_id = "FORM-LEGACY-ROLE-KEY";
    let _ = init_surreal().await;
    cleanup_form(form_id).await;
    let app = create_platform_api_routes();

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
                        "form_id": form_id,
                        "role": "pz"
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
    let data = payload.data.expect("embed data");
    let response_token = data
        .get("token")
        .and_then(|v| v.as_str())
        .expect("response token");
    let claims = verify_token(response_token).unwrap();
    assert_eq!(claims.role.as_deref(), Some("pz"));
}

#[tokio::test]
async fn test_embed_url_jwt_defaults_role_sj_when_workflow_role_omitted() {
    let form_id = "FORM-DEFAULT-JWT-ROLE";
    let _ = init_surreal().await;
    cleanup_form(form_id).await;
    let app = create_platform_api_routes();

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
                        "form_id": form_id
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
    let data = payload.data.expect("embed data");
    let response_token = data
        .get("token")
        .and_then(|v| v.as_str())
        .expect("response token");
    let claims = verify_token(response_token).unwrap();
    assert_eq!(claims.role.as_deref(), Some("sj"));
}

#[tokio::test]
#[ignore = "requires an initialized review_forms database backing store"]
async fn test_embed_url_reuses_persisted_form_role_when_followup_request_omits_role() {
    let form_id = "FORM-ROLE-PERSISTED";
    let _ = init_surreal().await;
    cleanup_form(form_id).await;

    let app = create_platform_api_routes();
    let first_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/review/embed-url")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "project_id": "project-1",
                        "user_id": "JH",
                        "form_id": form_id,
                        "extra_parameters": {
                            "workflow_role": "jd"
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(first_response.status(), StatusCode::OK);

    let stored_form = super::review_form::get_review_form_by_form_id(form_id)
        .await
        .expect("query review form")
        .expect("stored review form");
    assert_eq!(stored_form.role.as_deref(), Some("jd"));

    let second_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/review/embed-url")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "project_id": "project-1",
                        "user_id": "JH",
                        "form_id": form_id
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(second_response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(second_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let payload: EmbedUrlResponseBody = serde_json::from_slice(&body).unwrap();
    let data = payload.data.expect("embed data");
    let response_token = data
        .get("token")
        .and_then(|v| v.as_str())
        .expect("response token");
    let claims = verify_token(response_token).unwrap();
    assert_eq!(claims.role.as_deref(), Some("jd"));
}

#[tokio::test]
async fn test_embed_url_rejects_tampered_jwt_even_if_form_id_matches() {
    let app = create_platform_api_routes();
    let (token, _) = create_token("project-1", "user-1", None, Some("sj"), None).unwrap();
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

    let app = create_platform_api_routes();
    let (token, _) = create_token("project-1", "user-existing", None, Some("jd"), None).unwrap();

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
    let token_claims = verify_token(response_token).unwrap();
    assert_eq!(token_claims.legacy_form_id, None);

    cleanup_form(form_id).await;
}

#[tokio::test]
async fn test_workflow_verify_rejects_unauthorized_token() {
    let app = create_platform_api_routes();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/review/workflow/verify")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "form_id": "FORM-VERIFY-UNAUTH",
                        "token": "invalid-token",
                        "action": "active",
                        "actor": {
                            "id": "SJ",
                            "name": "SJ",
                            "roles": "sj"
                        },
                        "next_step": {
                            "assignee_id": "JH",
                            "name": "JH",
                            "roles": "jd"
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let payload: VerifyWorkflowResponseBody = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload.code, 401);
    assert_eq!(payload.message, "unauthorized");
    assert!(payload.data.is_none());
}

#[tokio::test]
async fn test_workflow_verify_rejects_unsupported_action() {
    let app = create_platform_api_routes();
    let (token, _) = create_token("project-1", "SJ", None, Some("sj"), None).unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/review/workflow/verify")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "form_id": "FORM-VERIFY-BAD-ACTION",
                        "token": token,
                        "action": "query",
                        "actor": {
                            "id": "SJ",
                            "name": "SJ",
                            "roles": "sj"
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let payload: VerifyWorkflowResponseBody = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload.code, 400);
    assert!(payload.message.contains("verify 不支持 action=query"));
}

#[tokio::test]
async fn test_workflow_verify_active_requires_next_step() {
    let form_id = "FORM-VERIFY-MISSING-NEXT";
    let _ = init_surreal().await;
    insert_task_seed(form_id, "SJ", "JH", "SH", "sj", "draft").await;
    let app = create_platform_api_routes();
    let (token, _) = create_token("project-1", "SJ", None, Some("sj"), None).unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/review/workflow/verify")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "form_id": form_id,
                        "token": token,
                        "action": "active",
                        "actor": {
                            "id": "SJ",
                            "name": "SJ",
                            "roles": "sj"
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let payload: VerifyWorkflowResponseBody = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload.code, 400);
    assert!(payload.message.contains("active 缺少 next_step"));

    cleanup_form(form_id).await;
}

#[tokio::test]
async fn test_workflow_verify_blank_form_without_task_returns_soft_block() {
    let form_id = "FORM-VERIFY-BLANK-NO-TASK";
    let _ = init_surreal().await;
    cleanup_form(form_id).await;
    super::review_form::ensure_review_form_stub(form_id, "project-1", "SJ", Some("sj"), "pms")
        .await
        .expect("seed blank review form");
    let app = create_platform_api_routes();
    let (token, _) = create_token("project-1", "SJ", None, Some("sj"), None).unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/review/workflow/verify")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "form_id": form_id,
                        "token": token,
                        "action": "active",
                        "actor": {
                            "id": "SJ",
                            "name": "SJ",
                            "roles": "sj"
                        },
                        "next_step": {
                            "assignee_id": "JH",
                            "name": "JH",
                            "roles": "jd"
                        }
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
    let payload: VerifyWorkflowResponseBody = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload.code, 200);
    let data = payload.data.expect("verify data");
    assert!(!data.passed);
    assert_eq!(data.action, "active");
    assert_eq!(data.current_node.as_deref(), Some("sj"));
    assert_eq!(data.task_status.as_deref(), Some("blank"));
    assert_eq!(data.recommended_action, "block");
    assert!(data.reason.contains("尚未创建活动 review task"));

    cleanup_form(form_id).await;
}

#[tokio::test]
async fn test_workflow_verify_active_on_non_sj_node_returns_soft_block() {
    let form_id = "FORM-VERIFY-ACTIVE-NON-SJ";
    let _ = init_surreal().await;
    insert_task_seed(form_id, "SJ", "JH", "SH", "jd", "submitted").await;
    let app = create_platform_api_routes();
    let (token, _) = create_token("project-1", "JH", None, Some("jd"), None).unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/review/workflow/verify")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "form_id": form_id,
                        "token": token,
                        "action": "active",
                        "actor": {
                            "id": "JH",
                            "name": "JH",
                            "roles": "jd"
                        },
                        "next_step": {
                            "assignee_id": "JH",
                            "name": "JH",
                            "roles": "jd"
                        }
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
    let payload: VerifyWorkflowResponseBody = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload.code, 200);
    let data = payload.data.expect("verify data");
    assert!(!data.passed);
    assert_eq!(data.action, "active");
    assert_eq!(data.current_node.as_deref(), Some("jd"));
    assert_eq!(data.task_status.as_deref(), Some("submitted"));
    assert_eq!(data.next_step.as_deref(), Some("jd"));
    assert_eq!(data.recommended_action, "block");
    assert!(data.reason.contains("active 仅允许从 sj 发起"));

    let task_after = super::review_form::find_task_by_form_id(form_id)
        .await
        .expect("query task after verify")
        .expect("task after verify");
    assert_eq!(task_after.current_node, "jd");
    assert_eq!(task_after.status, "submitted");

    cleanup_form(form_id).await;
}

#[tokio::test]
async fn test_workflow_verify_missing_form_returns_404() {
    let form_id = "FORM-VERIFY-FORM-NOT-FOUND";
    let _ = init_surreal().await;
    cleanup_form(form_id).await;
    let app = create_platform_api_routes();
    let (token, _) = create_token("project-1", "SJ", None, Some("sj"), None).unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/review/workflow/verify")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "form_id": form_id,
                        "token": token,
                        "action": "active",
                        "actor": {
                            "id": "SJ",
                            "name": "SJ",
                            "roles": "sj"
                        },
                        "next_step": {
                            "assignee_id": "JH",
                            "name": "JH",
                            "roles": "jd"
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let payload: VerifyWorkflowResponseBody = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload.code, 404);
    assert!(payload.data.is_none());
    assert!(payload.message.contains("未找到 review form"));
}

#[tokio::test]
async fn test_workflow_verify_agree_terminal_task_returns_soft_block() {
    let form_id = "FORM-VERIFY-AGREE-TERMINAL";
    let _ = init_surreal().await;
    insert_task_seed(form_id, "SJ", "JH", "SH", "pz", "approved").await;
    let app = create_platform_api_routes();
    let (token, _) = create_token("project-1", "PZ", None, Some("pz"), None).unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/review/workflow/verify")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "form_id": form_id,
                        "token": token,
                        "action": "agree",
                        "actor": {
                            "id": "PZ",
                            "name": "PZ",
                            "roles": "pz"
                        }
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
    let payload: VerifyWorkflowResponseBody = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload.code, 200);
    let data = payload.data.expect("verify data");
    assert!(!data.passed);
    assert_eq!(data.action, "agree");
    assert_eq!(data.current_node.as_deref(), Some("pz"));
    assert_eq!(data.task_status.as_deref(), Some("approved"));
    assert_eq!(data.recommended_action, "block");
    assert!(data.reason.contains("当前单据已处于终态 approved"));

    let task_after = super::review_form::find_task_by_form_id(form_id)
        .await
        .expect("query task after verify")
        .expect("task after verify");
    assert_eq!(task_after.current_node, "pz");
    assert_eq!(task_after.status, "approved");

    cleanup_form(form_id).await;
}

#[tokio::test]
async fn test_workflow_verify_agree_owner_mismatch_returns_structured_diagnostics() {
    let form_id = "FORM-VERIFY-OWNER-MISMATCH";
    let _ = init_surreal().await;
    insert_task_seed(form_id, "SJ", "JH", "SH", "jd", "submitted").await;
    let app = create_platform_api_routes();
    let (token, _) = create_token("project-1", "SH", None, Some("sh"), None).unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/review/workflow/verify")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "form_id": form_id,
                        "token": token,
                        "action": "agree",
                        "actor": {
                            "id": "SH",
                            "name": "SH",
                            "roles": "sh"
                        },
                        "next_step": {
                            "assignee_id": "SH",
                            "name": "SH",
                            "roles": "sh"
                        }
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
    let payload: VerifyWorkflowResponseBody = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload.code, 200);
    let data = payload.data.expect("verify data");
    assert!(!data.passed);
    assert_eq!(data.action, "agree");
    assert_eq!(data.block_code.as_deref(), Some("OWNER_MISMATCH"));
    assert_eq!(data.current_node.as_deref(), Some("jd"));
    assert_eq!(data.task_status.as_deref(), Some("submitted"));
    assert_eq!(data.next_step.as_deref(), Some("sh"));
    assert_eq!(data.actor_id.as_deref(), Some("SH"));
    assert_eq!(data.owner_id.as_deref(), Some("JH"));
    assert_eq!(data.owner_source.as_deref(), Some("checker"));
    assert_eq!(data.expected_next_node.as_deref(), Some("sh"));
    let requested_next_step = data
        .requested_next_step
        .expect("requested next step diagnostics");
    assert_eq!(requested_next_step.assignee_id, "SH");
    assert_eq!(requested_next_step.name, "SH");
    assert_eq!(requested_next_step.roles, "sh");
    assert_eq!(data.recommended_action, "block");

    cleanup_form(form_id).await;
}

#[tokio::test]
async fn test_workflow_verify_agree_returns_annotation_gate_block_without_mutation() {
    let form_id = "FORM-VERIFY-ANNOTATION-BLOCK";
    let _ = init_surreal().await;
    insert_task_seed(form_id, "SJ", "JH", "SH", "jd", "submitted").await;
    let task = super::review_form::find_task_by_form_id(form_id)
        .await
        .expect("query task")
        .expect("seeded task");
    insert_pending_review_record(&task.id, form_id).await;

    let app = create_platform_api_routes();
    let (token, _) = create_token("project-1", "JH", None, Some("jd"), None).unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/review/workflow/verify")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "form_id": form_id,
                        "token": token,
                        "action": "agree",
                        "actor": {
                            "id": "JH",
                            "name": "JH",
                            "roles": "jd"
                        },
                        "next_step": {
                            "assignee_id": "SH",
                            "name": "SH",
                            "roles": "sh"
                        }
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
    let payload: VerifyWorkflowResponseBody = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload.code, 200);
    assert_eq!(
        payload.error_code.as_deref(),
        Some("ANNOTATION_CHECK_FAILED")
    );
    let data = payload.data.expect("verify data");
    assert!(!data.passed);
    assert_eq!(data.action, "agree");
    assert_eq!(data.block_code.as_deref(), Some("ANNOTATION_CHECK_FAILED"));
    assert_eq!(data.current_node.as_deref(), Some("jd"));
    assert_eq!(data.task_status.as_deref(), Some("submitted"));
    assert_eq!(data.next_step.as_deref(), Some("sh"));
    assert_eq!(data.actor_id.as_deref(), Some("JH"));
    assert_eq!(data.owner_id.as_deref(), Some("JH"));
    assert_eq!(data.owner_source.as_deref(), Some("checker"));
    assert_eq!(data.expected_next_node.as_deref(), Some("sh"));
    let requested_next_step = data
        .requested_next_step
        .expect("requested next step diagnostics");
    assert_eq!(requested_next_step.assignee_id, "SH");
    assert_eq!(requested_next_step.name, "SH");
    assert_eq!(requested_next_step.roles, "sh");
    assert_eq!(data.recommended_action, "block");
    assert!(data.reason.contains("待确认批注"));
    let annotation_check = payload.annotation_check.expect("annotation_check");
    assert_eq!(
        annotation_check
            .get("current_node")
            .and_then(|value| value.as_str()),
        Some("jd")
    );

    let task_after = super::review_form::find_task_by_form_id(form_id)
        .await
        .expect("query task after verify")
        .expect("task after verify");
    assert_eq!(task_after.current_node, "jd");
    assert_eq!(task_after.status, "submitted");

    cleanup_form(form_id).await;
}

#[tokio::test]
async fn test_workflow_verify_pass_does_not_mutate_task_state() {
    let form_id = "FORM-VERIFY-PASS-NO-MUTATION";
    let _ = init_surreal().await;
    insert_task_seed(form_id, "SJ", "JH", "SH", "sj", "draft").await;
    let app = create_platform_api_routes();
    let (token, _) = create_token("project-1", "SJ", None, Some("sj"), None).unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/review/workflow/verify")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "form_id": form_id,
                        "token": token,
                        "action": "active",
                        "actor": {
                            "id": "SJ",
                            "name": "SJ",
                            "roles": "sj"
                        },
                        "next_step": {
                            "assignee_id": "JH",
                            "name": "JH",
                            "roles": "jd"
                        },
                        "comments": "送审"
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
    let payload: VerifyWorkflowResponseBody = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload.code, 200);
    let data = payload.data.expect("verify data");
    assert!(data.passed);
    assert_eq!(data.action, "active");
    assert_eq!(data.current_node.as_deref(), Some("sj"));
    assert_eq!(data.task_status.as_deref(), Some("draft"));
    assert_eq!(data.next_step.as_deref(), Some("jd"));
    assert_eq!(data.recommended_action, "proceed");

    let task_after = super::review_form::find_task_by_form_id(form_id)
        .await
        .expect("query task after verify")
        .expect("task after verify");
    assert_eq!(task_after.current_node, "sj");
    assert_eq!(task_after.status, "draft");

    cleanup_form(form_id).await;
}

// ============================================================================
// v3 §3.6: action-aware annotation gate + verify 路径瘦身覆盖
// ============================================================================

#[derive(Debug, Deserialize)]
struct SyncWorkflowResponseBody {
    code: i32,
    message: String,
    error_code: Option<String>,
    annotation_check: Option<serde_json::Value>,
}

async fn seed_task_with_record(
    form_id: &str,
    current_node: &str,
    status: &str,
    annotation_id: &str,
    annotation_text: &str,
    resolution_status: Option<&str>,
    decision_status: Option<&str>,
    record_label: &str,
) {
    let _ = init_surreal().await;
    insert_task_seed(form_id, "SJ", "JH", "SH", current_node, status).await;
    let task = super::review_form::find_task_by_form_id(form_id)
        .await
        .expect("query task")
        .expect("seeded task");
    insert_review_record_with_state(
        &task.id,
        form_id,
        annotation_id,
        annotation_text,
        resolution_status,
        decision_status,
        record_label,
    )
    .await;
}

async fn post_verify(form_id: &str, token: &str, body: serde_json::Value) -> (StatusCode, Vec<u8>) {
    let app = create_platform_api_routes();
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/review/workflow/verify")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let _ = (form_id, token); // 留给调用方装填 body 的占位参数。
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    (status, bytes)
}

// 3.6.1 active + sj 节点 + 1 条 open 批注 → soft block "未处理批注"
#[tokio::test]
async fn test_verify_active_blocks_when_open_exists() {
    let form_id = "FORM-V3-ACTIVE-OPEN";
    let _ = init_surreal().await;
    cleanup_form(form_id).await;
    seed_task_with_record(
        form_id,
        "sj",
        "draft",
        "ann-open-1",
        "未回复批注",
        None,
        None,
        "v3-active-open",
    )
    .await;
    let (token, _) = create_token("project-1", "SJ", None, Some("sj"), None).unwrap();

    let (status, bytes) = post_verify(
        form_id,
        &token,
        serde_json::json!({
            "form_id": form_id,
            "token": token,
            "action": "active"
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let payload: VerifyWorkflowResponseBody = serde_json::from_slice(&bytes).unwrap();
    let data = payload.data.expect("verify data");
    assert!(!data.passed);
    assert_eq!(data.action, "active");
    assert_eq!(data.recommended_action, "block");
    assert!(data.reason.contains("未处理批注"));

    cleanup_form(form_id).await;
}

// 3.6.2 active + sj + 仅有"被回复过"的批注（pending/approved/rejected） → pass
#[tokio::test]
async fn test_verify_active_passes_with_only_replied_annotations() {
    let form_id = "FORM-V3-ACTIVE-REPLIED";
    let _ = init_surreal().await;
    cleanup_form(form_id).await;
    insert_task_seed(form_id, "SJ", "JH", "SH", "sj", "draft").await;
    let task = super::review_form::find_task_by_form_id(form_id)
        .await
        .expect("query task")
        .expect("seeded task");
    insert_review_record_with_state(
        &task.id,
        form_id,
        "ann-pending",
        "已修复待确认",
        Some("fixed"),
        Some("pending"),
        "v3-active-replied-pending",
    )
    .await;
    insert_review_record_with_state(
        &task.id,
        form_id,
        "ann-approved",
        "已通过",
        Some("fixed"),
        Some("agreed"),
        "v3-active-replied-approved",
    )
    .await;
    insert_review_record_with_state(
        &task.id,
        form_id,
        "ann-rejected",
        "被驳回",
        Some("fixed"),
        Some("rejected"),
        "v3-active-replied-rejected",
    )
    .await;
    let (token, _) = create_token("project-1", "SJ", None, Some("sj"), None).unwrap();

    let (status, bytes) = post_verify(
        form_id,
        &token,
        serde_json::json!({
            "form_id": form_id,
            "token": token,
            "action": "active"
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let payload: VerifyWorkflowResponseBody = serde_json::from_slice(&bytes).unwrap();
    let data = payload.data.expect("verify data");
    assert!(data.passed);
    assert_eq!(data.action, "active");
    assert_eq!(data.recommended_action, "proceed");

    cleanup_form(form_id).await;
}

// 3.6.3 agree + jd + 仅 pending → block "待确认批注"
#[tokio::test]
async fn test_verify_agree_blocks_on_pending() {
    let form_id = "FORM-V3-AGREE-PENDING";
    let _ = init_surreal().await;
    cleanup_form(form_id).await;
    seed_task_with_record(
        form_id,
        "jd",
        "submitted",
        "ann-pending-only",
        "待确认",
        Some("fixed"),
        Some("pending"),
        "v3-agree-pending",
    )
    .await;
    let (token, _) = create_token("project-1", "JH", None, Some("jd"), None).unwrap();

    let (status, bytes) = post_verify(
        form_id,
        &token,
        serde_json::json!({
            "form_id": form_id,
            "token": token,
            "action": "agree"
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let payload: VerifyWorkflowResponseBody = serde_json::from_slice(&bytes).unwrap();
    let data = payload.data.expect("verify data");
    assert!(!data.passed);
    assert_eq!(data.action, "agree");
    assert_eq!(data.recommended_action, "block");
    assert!(data.reason.contains("待确认批注"));

    cleanup_form(form_id).await;
}

// 3.6.4 agree + jd + rejected → recommend "return"
#[tokio::test]
async fn test_verify_agree_recommends_return_when_rejected() {
    let form_id = "FORM-V3-AGREE-REJECTED";
    let _ = init_surreal().await;
    cleanup_form(form_id).await;
    seed_task_with_record(
        form_id,
        "jd",
        "submitted",
        "ann-rejected-1",
        "已驳回的批注",
        Some("fixed"),
        Some("rejected"),
        "v3-agree-rejected",
    )
    .await;
    let (token, _) = create_token("project-1", "JH", None, Some("jd"), None).unwrap();

    let (status, bytes) = post_verify(
        form_id,
        &token,
        serde_json::json!({
            "form_id": form_id,
            "token": token,
            "action": "agree"
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let payload: VerifyWorkflowResponseBody = serde_json::from_slice(&bytes).unwrap();
    let data = payload.data.expect("verify data");
    assert!(!data.passed);
    assert_eq!(data.action, "agree");
    assert_eq!(data.recommended_action, "return");

    cleanup_form(form_id).await;
}

// 3.6.5 return + jd + 仅 approved → block "无问题批注"
#[tokio::test]
async fn test_verify_return_blocks_when_no_problem() {
    let form_id = "FORM-V3-RETURN-NOPROB";
    let _ = init_surreal().await;
    cleanup_form(form_id).await;
    seed_task_with_record(
        form_id,
        "jd",
        "submitted",
        "ann-approved-only",
        "已通过批注",
        Some("fixed"),
        Some("agreed"),
        "v3-return-noprob",
    )
    .await;
    let (token, _) = create_token("project-1", "JH", None, Some("jd"), None).unwrap();

    let (status, bytes) = post_verify(
        form_id,
        &token,
        serde_json::json!({
            "form_id": form_id,
            "token": token,
            "action": "return"
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let payload: VerifyWorkflowResponseBody = serde_json::from_slice(&bytes).unwrap();
    let data = payload.data.expect("verify data");
    assert!(!data.passed);
    assert_eq!(data.action, "return");
    assert_eq!(data.recommended_action, "block");
    assert!(data.reason.contains("不允许驳回"));

    cleanup_form(form_id).await;
}

// 3.6.6 return + jd + 1 条 open 批注 → pass
#[tokio::test]
async fn test_verify_return_passes_with_open_or_rejected() {
    let form_id = "FORM-V3-RETURN-PASS";
    let _ = init_surreal().await;
    cleanup_form(form_id).await;
    seed_task_with_record(
        form_id,
        "jd",
        "submitted",
        "ann-open-prob",
        "未回复有问题",
        None,
        None,
        "v3-return-pass",
    )
    .await;
    let (token, _) = create_token("project-1", "JH", None, Some("jd"), None).unwrap();

    let (status, bytes) = post_verify(
        form_id,
        &token,
        serde_json::json!({
            "form_id": form_id,
            "token": token,
            "action": "return"
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let payload: VerifyWorkflowResponseBody = serde_json::from_slice(&bytes).unwrap();
    let data = payload.data.expect("verify data");
    assert!(data.passed);
    assert_eq!(data.action, "return");
    assert_eq!(data.recommended_action, "proceed");

    cleanup_form(form_id).await;
}

// 3.6.7 stop + jd + 即使有 pending 批注也通过（stop 不查 annotation）
#[tokio::test]
async fn test_verify_stop_passes_without_annotation_check() {
    let form_id = "FORM-V3-STOP-PASS";
    let _ = init_surreal().await;
    cleanup_form(form_id).await;
    seed_task_with_record(
        form_id,
        "jd",
        "submitted",
        "ann-still-pending",
        "强行终止",
        Some("fixed"),
        Some("pending"),
        "v3-stop-pass",
    )
    .await;
    let (token, _) = create_token("project-1", "JH", None, Some("jd"), None).unwrap();

    let (status, bytes) = post_verify(
        form_id,
        &token,
        serde_json::json!({
            "form_id": form_id,
            "token": token,
            "action": "stop"
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let payload: VerifyWorkflowResponseBody = serde_json::from_slice(&bytes).unwrap();
    let data = payload.data.expect("verify data");
    assert!(data.passed);
    assert_eq!(data.action, "stop");
    assert_eq!(data.recommended_action, "proceed");

    cleanup_form(form_id).await;
}

// 3.6.8 verify 路径完全忽略 next_step：传与不传结果一致
#[tokio::test]
async fn test_verify_ignores_next_step_field() {
    let form_id = "FORM-V3-IGNORE-NEXTSTEP";
    let _ = init_surreal().await;
    cleanup_form(form_id).await;
    seed_task_with_record(
        form_id,
        "jd",
        "submitted",
        "ann-open-ignore",
        "open 批注",
        None,
        None,
        "v3-ignore-nextstep",
    )
    .await;
    let (token, _) = create_token("project-1", "JH", None, Some("jd"), None).unwrap();

    // 故意传一个非法 next_step（roles=WRONG，不是 sj/jd/sh/pz），如果 verify
    // 真的读了 next_step，会因为 roles 解析失败或 rank 不识别报错。
    let (status, bytes) = post_verify(
        form_id,
        &token,
        serde_json::json!({
            "form_id": form_id,
            "token": token,
            "action": "return",
            "next_step": {
                "assignee_id": "BOGUS",
                "name": "WRONG",
                "roles": "WRONG"
            }
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    let payload: VerifyWorkflowResponseBody = serde_json::from_slice(&bytes).unwrap();
    let data = payload.data.expect("verify data");
    assert!(data.passed, "verify must ignore next_step");
    assert_eq!(data.action, "return");
    assert_eq!(data.recommended_action, "proceed");

    cleanup_form(form_id).await;
}

// 3.6.9 sync return + 仅 approved 批注 → 409 + ANNOTATION_CHECK_FAILED
#[tokio::test]
async fn test_sync_return_blocks_without_problem_annotation() {
    let form_id = "FORM-V3-SYNC-RETURN-NOPROB";
    let _ = init_surreal().await;
    cleanup_form(form_id).await;
    seed_task_with_record(
        form_id,
        "jd",
        "submitted",
        "ann-only-approved",
        "全部已通过",
        Some("fixed"),
        Some("agreed"),
        "v3-sync-return-noprob",
    )
    .await;
    let (token, _) = create_token("project-1", "JH", None, Some("jd"), None).unwrap();

    let app = create_platform_api_routes();
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/review/workflow/sync")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "form_id": form_id,
                        "token": token,
                        "action": "return",
                        "actor": { "id": "JH", "name": "JH", "roles": "jd" },
                        "next_step": { "assignee_id": "SJ", "name": "SJ", "roles": "sj" }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let payload: SyncWorkflowResponseBody = serde_json::from_slice(&body).unwrap();
    assert_eq!(payload.code, 409);
    assert_eq!(
        payload.error_code.as_deref(),
        Some("ANNOTATION_CHECK_FAILED")
    );
    assert!(payload.message.contains("不允许驳回") || payload.message.contains("无未处理"));
    let annotation_check = payload.annotation_check.expect("annotation_check");
    assert_eq!(
        annotation_check
            .get("recommended_action")
            .and_then(|v| v.as_str()),
        Some("block")
    );

    let task_after = super::review_form::find_task_by_form_id(form_id)
        .await
        .expect("query task after sync return")
        .expect("task after sync return");
    assert_eq!(task_after.current_node, "jd");
    assert_eq!(task_after.status, "submitted");

    cleanup_form(form_id).await;
}

// 3.6.10 sync 路径 actor.name 兜底：仅传 id 时 DB 落 operator_name == id
#[tokio::test]
async fn test_sync_actor_name_default_falls_back_to_id() {
    let form_id = "FORM-V3-ACTOR-NAME-FALLBACK";
    let _ = init_surreal().await;
    cleanup_form(form_id).await;
    insert_task_seed(form_id, "SJ", "JH", "SH", "sj", "draft").await;
    let task = super::review_form::find_task_by_form_id(form_id)
        .await
        .expect("seed task")
        .expect("seed task");
    let task_id = task.id.clone();

    let (token, _) = create_token("project-1", "SJ", None, Some("sj"), None).unwrap();

    let app = create_platform_api_routes();
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/review/workflow/sync")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "form_id": form_id,
                        "token": token,
                        "action": "active",
                        // actor 显式传，但 name 字段缺失（serde default → ""）
                        "actor": { "id": "SJ", "roles": "sj" },
                        "next_step": { "assignee_id": "JH", "name": "JH", "roles": "jd" }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    use surrealdb::types::SurrealValue;
    #[derive(Debug, Deserialize, SurrealValue)]
    struct HistoryRow {
        operator_id: Option<String>,
        operator_name: Option<String>,
    }

    let mut response = project_primary_db()
        .query(
            r#"
            SELECT operator_id, operator_name
            FROM review_workflow_history
            WHERE task_id = $task_id
            ORDER BY timestamp DESC
            LIMIT 1
            "#,
        )
        .bind(("task_id", task_id))
        .await
        .expect("query workflow history");
    let rows: Vec<HistoryRow> = response.take(0).expect("take history rows");
    let row = rows.into_iter().next().expect("at least one history row");
    assert_eq!(row.operator_id.as_deref(), Some("SJ"));
    assert_eq!(
        row.operator_name.as_deref(),
        Some("SJ"),
        "operator_name 应当从空串兜底为 actor.id"
    );

    cleanup_form(form_id).await;
}
