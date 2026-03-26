//! Embed URL handler — external systems request an embeddable review page URL.

use axum::{extract::Json, http::StatusCode, response::IntoResponse};
use tracing::{info, warn};

use crate::web_api::jwt_auth::{create_token, generate_form_id, verify_token};

use super::config::PLATFORM_CONFIG;
use super::review_form::{ensure_review_form_stub, find_task_by_form_id};
use super::types::{
    EmbedLineage, EmbedUrlData, EmbedUrlQuery, EmbedUrlRequest, EmbedUrlResponse, ReviewFormSummary,
};

pub async fn get_embed_url(Json(request): Json<EmbedUrlRequest>) -> impl IntoResponse {
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

    let mut form_id = request
        .form_id
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

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

    match create_token(&request.project_id, &request.user_id, None, &form_id, None) {
        Ok((token, _expires_at)) => {
            let is_reviewer = request
                .extra_parameters
                .as_ref()
                .and_then(|p| p.get("is_reviewer"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            info!("Generated form_id={}, token_len={}", form_id, token.len());

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
                            task_id: existing_task.as_ref().map(|t| t.id.clone()),
                            current_node: existing_task.as_ref().map(|t| t.current_node.clone()),
                            status: existing_task.as_ref().map(|t| t.status.clone()),
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
