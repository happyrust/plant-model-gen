//! Embed URL handler — external systems request an embeddable review page URL.

use axum::{extract::Json, http::StatusCode, response::IntoResponse};
use tracing::{info, warn};

use crate::web_api::jwt_auth::{
    create_token, generate_form_id, normalize_workflow_mode, verify_token, Role,
};

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

    let mut verified_claim_role: Option<String> = None;
    let mut verified_claim_workflow_mode: Option<String> = None;
    let jwt_claim_form_id = if let Some(ref token) = request.token {
        let token = token.trim();
        if token.split('.').count() == 3 {
            match verify_token(token) {
                Ok(claims) => {
                    if claims.project_id != request.project_id || claims.user_id != request.user_id
                    {
                        warn!(
                            "Embed URL JWT identity mismatch: token project/user={}/{}, request={}/{}",
                            claims.project_id, claims.user_id, request.project_id, request.user_id
                        );
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
                    verified_claim_role = normalize_embed_role(claims.role.as_deref());
                    verified_claim_workflow_mode =
                        normalize_workflow_mode(claims.workflow_mode.as_deref());
                    Some(claims.form_id)
                }
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

    let requested_role = match resolve_embed_request_role(&request, verified_claim_role.as_deref())
    {
        Ok(role) => role,
        Err(message) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(EmbedUrlResponse {
                    code: 400,
                    message,
                    data: None,
                    url: None,
                }),
            );
        }
    };
    let requested_workflow_mode = match resolve_embed_request_workflow_mode(
        &request,
        verified_claim_workflow_mode.as_deref(),
    ) {
        Ok(mode) => mode,
        Err(message) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(EmbedUrlResponse {
                    code: 400,
                    message,
                    data: None,
                    url: None,
                }),
            );
        }
    };

    let form_id = form_id.unwrap_or_else(generate_form_id);
    let ensured_form = match ensure_review_form_stub(
        &form_id,
        request.project_id.as_str(),
        request.user_id.as_str(),
        requested_role.as_deref(),
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

    let resolved_role = requested_role.clone().or_else(|| ensured_form.role.clone());

    match create_token(
        &request.project_id,
        &request.user_id,
        None,
        &form_id,
        resolved_role.as_deref(),
        requested_workflow_mode.as_deref(),
    ) {
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
                    Some(format!("{}{}?user_token={}", base, clean_path, token))
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

fn resolve_embed_request_role(
    request: &EmbedUrlRequest,
    verified_claim_role: Option<&str>,
) -> Result<Option<String>, String> {
    let explicit_role = request
        .role
        .as_deref()
        .or_else(|| extract_role_from_extra_parameters(request, "user_role"))
        .or_else(|| extract_role_from_extra_parameters(request, "role"));

    if let Some(role) = verified_claim_role {
        if let Some(explicit_role) = explicit_role {
            let validated_explicit = validate_embed_role(explicit_role)?;
            if validated_explicit.as_deref() != Some(role) {
                return Err("JWT role mismatch".to_string());
            }
        }
        return validate_embed_role(role);
    }

    if let Some(role) = explicit_role {
        return validate_embed_role(role);
    }

    Ok(None)
}

fn resolve_embed_request_workflow_mode(
    request: &EmbedUrlRequest,
    verified_claim_workflow_mode: Option<&str>,
) -> Result<Option<String>, String> {
    let explicit = request
        .workflow_mode
        .as_deref()
        .or_else(|| extract_extra_parameter(request, "workflow_mode"))
        .or_else(|| extract_extra_parameter(request, "workflowMode"));

    if let Some(mode) = verified_claim_workflow_mode {
        if let Some(explicit_mode) = explicit {
            let validated_explicit = validate_embed_workflow_mode(explicit_mode)?;
            if validated_explicit.as_deref() != Some(mode) {
                return Err("JWT workflow_mode mismatch".to_string());
            }
        }
        return validate_embed_workflow_mode(mode);
    }

    if let Some(mode) = explicit {
        return validate_embed_workflow_mode(mode);
    }

    Ok(None)
}

fn extract_extra_parameter<'a>(request: &'a EmbedUrlRequest, key: &str) -> Option<&'a str> {
    request
        .extra_parameters
        .as_ref()
        .and_then(|value| value.get(key))
        .and_then(|value| value.as_str())
}

fn extract_role_from_extra_parameters<'a>(
    request: &'a EmbedUrlRequest,
    key: &str,
) -> Option<&'a str> {
    extract_extra_parameter(request, key)
}

fn validate_embed_role(raw_role: &str) -> Result<Option<String>, String> {
    let normalized = normalize_embed_role(Some(raw_role));
    if normalized.is_some() {
        return Ok(normalized);
    }

    Err(format!(
        "Invalid role: '{}'. Valid values are: {:?}",
        raw_role,
        Role::valid_values()
    ))
}

fn validate_embed_workflow_mode(raw_mode: &str) -> Result<Option<String>, String> {
    let normalized = normalize_workflow_mode(Some(raw_mode));
    if normalized.is_some() {
        return Ok(normalized);
    }

    Err("Invalid workflow_mode: valid values are external, manual, internal".to_string())
}

fn normalize_embed_role(role: Option<&str>) -> Option<String> {
    let trimmed = role.map(str::trim).filter(|value| !value.is_empty())?;
    Role::from_str(trimmed).map(|value| value.as_str().to_string())
}
