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

async fn cleanup_form(form_id: &str) {
    let _ = project_primary_db()
        .query(
            "LET $ids = SELECT VALUE id FROM review_tasks WHERE form_id = $form_id; DELETE $ids;",
        )
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
    let app = create_platform_api_routes();
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
    let app = create_platform_api_routes();
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
    let app = create_platform_api_routes();
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

    let app = create_platform_api_routes();
    let (token, _) = create_token("project-1", "user-existing", None, form_id, Some("jd")).unwrap();

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
