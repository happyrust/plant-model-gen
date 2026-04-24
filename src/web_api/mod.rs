pub mod collision_api;
pub mod e3d_tree_api;
pub mod mbd_pipe_api;
pub mod noun_hierarchy_api;
pub mod pdms_attr_api;
pub mod pdms_transform_api;
pub mod pipeline_annotation_api;
pub mod ptset_api;
pub mod room_tree_api;
pub mod scene_tree_api;
pub mod search_api;
pub mod spatial_query_api;
pub mod upload_api;
pub mod version_api;

pub use collision_api::{CollisionApiState, create_collision_routes};
pub use e3d_tree_api::{E3dTreeApiState, create_e3d_tree_routes};
pub use mbd_pipe_api::{
    MbdExportScope, MbdExportStats, create_mbd_pipe_routes, export_mbd_json_batch,
    generate_mbd_data, get_mbd_output_dir,
};
pub use noun_hierarchy_api::{NounHierarchyApiState, create_noun_hierarchy_routes};
pub use pdms_attr_api::create_pdms_attr_routes;
pub use pdms_model_query_api::create_pdms_model_query_routes;
pub use pdms_transform_api::create_pdms_transform_routes;
pub use pipeline_annotation_api::create_pipeline_annotation_routes;
pub use ptset_api::create_ptset_routes;
pub use room_tree_api::create_room_tree_routes;
pub use scene_tree_api::create_scene_tree_routes;
pub use search_api::{SearchApiState, create_search_routes};
pub use spatial_query_api::{SpatialQueryApiState, create_spatial_query_routes};
pub use upload_api::{UploadApiState, create_upload_routes};
pub use version_api::create_version_routes;
pub mod review_integration;
pub use review_integration::create_review_integration_routes;
pub mod platform_api;
pub use platform_api::create_platform_api_routes;

pub mod pdms_model_query_api;

#[cfg(feature = "web_server")]
pub mod review_api;
#[cfg(feature = "web_server")]
pub use review_api::create_review_api_routes;
#[cfg(feature = "web_server")]
pub mod review_annotation_state;
#[cfg(feature = "web_server")]
pub use review_annotation_state::create_annotation_state_routes;
#[cfg(feature = "web_server")]
pub mod review_db;

#[cfg(feature = "web_server")]
pub mod jwt_auth;
#[cfg(feature = "web_server")]
pub use jwt_auth::create_jwt_auth_routes;

/// 一次性装配所有"无状态" web_api 路由（含 nest 前缀）。
///
/// 设计目的：消除 `web_server::start_web_server_with_config` 里"新增路由必须手动
/// import + let + .merge()"的重复装配，避免类似 2026-04-23 `pdms_transform` 漏挂载
/// 导致前端 `q pos` / `q ori` 全报 404 的问题再次发生（详见
/// `docs/plans/2026-04-23-pdms-transform-route-missing-registration-fix.md`）。
///
/// 约束：仅收纳**无状态**（`Router<()>`）的 `create_*_routes()`；带 state 的路由
/// （collision / e3d_tree / noun_hierarchy / spatial_query / search / upload / room_api）
/// 仍由 `web_server/mod.rs` 在拿到对应 state 后单独挂载。
///
/// merge 顺序保持与历史 `web_server/mod.rs` 完全一致（含 nest 前缀），便于 diff 审阅。
#[cfg(feature = "web_server")]
pub fn assemble_stateless_web_api_routes() -> axum::Router {
    axum::Router::new()
        .merge(create_room_tree_routes())
        .merge(create_pdms_attr_routes())
        .merge(create_pdms_transform_routes())
        .merge(create_ptset_routes())
        .merge(create_pdms_model_query_routes())
        .merge(create_review_integration_routes())
        .merge(create_platform_api_routes())
        .merge(create_jwt_auth_routes())
        .merge(create_review_api_routes())
        .merge(create_annotation_state_routes())
        .merge(create_scene_tree_routes())
        .merge(create_mbd_pipe_routes())
        .nest("/api/pipeline", create_pipeline_annotation_routes())
        .nest("/api", create_version_routes())
}

#[cfg(test)]
mod tests;
