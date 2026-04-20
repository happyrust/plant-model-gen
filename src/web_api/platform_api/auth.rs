//! S2S authentication helpers shared across platform API handlers.

use axum::http::StatusCode;
use tracing::warn;

use crate::web_api::jwt_auth::{PLATFORM_AUTH_CONFIG, verify_token};

/// Unified S2S token verification: validates JWT when auth is enabled;
/// disabled mode still requires exact debug_token match.
pub fn verify_s2s_token(token: &str) -> Result<(), (StatusCode, String)> {
    if !PLATFORM_AUTH_CONFIG.enabled {
        let expected = PLATFORM_AUTH_CONFIG.debug_token.trim();
        if expected.is_empty() {
            warn!("platform_auth.enabled=false 但未配置 debug_token，拒绝 S2S 请求");
            return Err((
                StatusCode::UNAUTHORIZED,
                "platform auth debug_token 未配置".to_string(),
            ));
        }

        if token.trim() == expected {
            return Ok(());
        }

        warn!("S2S debug_token 不匹配");
        return Err((StatusCode::UNAUTHORIZED, "invalid debug_token".to_string()));
    }

    match verify_token(token) {
        Ok(claims) => {
            if let Some(legacy_form_id) = claims.legacy_form_id.as_deref() {
                warn!(
                    "S2S JWT decoded legacy form_id={} for project_id={}, user_id={}; explicit request form_id remains authoritative",
                    legacy_form_id, claims.project_id, claims.user_id
                );
            }
            Ok(())
        }
        Err(e) => {
            warn!("S2S JWT verification failed: {}", e);
            Err((StatusCode::UNAUTHORIZED, format!("invalid token: {}", e)))
        }
    }
}
