//! S2S authentication helpers shared across platform API handlers.

use axum::http::StatusCode;
use tracing::warn;

use crate::web_api::jwt_auth::{verify_token, REVIEW_AUTH_CONFIG};

/// Unified S2S token verification: validates JWT when auth is enabled, skips otherwise.
pub fn verify_s2s_token(
    token: &str,
    expected_form_id: Option<&str>,
) -> Result<(), (StatusCode, String)> {
    if !REVIEW_AUTH_CONFIG.enabled {
        return Ok(());
    }

    match verify_token(token) {
        Ok(claims) => {
            if let Some(form_id) = expected_form_id {
                if claims.form_id != form_id {
                    warn!(
                        "S2S JWT form_id mismatch: token={}, request={}",
                        claims.form_id, form_id
                    );
                    return Err((StatusCode::UNAUTHORIZED, "form_id mismatch".to_string()));
                }
            }
            Ok(())
        }
        Err(e) => {
            warn!("S2S JWT verification failed: {}", e);
            Err((StatusCode::UNAUTHORIZED, format!("invalid token: {}", e)))
        }
    }
}
