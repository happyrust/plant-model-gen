use axum::{Json, Router, routing::get};
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionInfo {
    version: String,
    commit: String,
    build_date: String,
}

async fn get_version() -> Json<VersionInfo> {
    Json(VersionInfo {
        version: env!("APP_VERSION").to_string(),
        commit: option_env!("GIT_COMMIT").unwrap_or("unknown").to_string(),
        build_date: option_env!("BUILD_DATE").unwrap_or("unknown").to_string(),
    })
}

pub fn create_version_routes() -> Router {
    Router::new().route("/version", get(get_version))
}
