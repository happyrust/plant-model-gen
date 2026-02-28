pub mod noun_hierarchy_api;
pub mod spatial_query_api;
pub mod e3d_tree_api;
pub mod room_tree_api;
pub mod pdms_attr_api;
pub mod ptset_api;
pub mod pdms_transform_api;
pub mod collision_api;
pub mod pipeline_annotation_api;
pub mod mbd_pipe_api;
pub mod scene_tree_api;
pub mod search_api;

pub use noun_hierarchy_api::{NounHierarchyApiState, create_noun_hierarchy_routes};
pub use spatial_query_api::{SpatialQueryApiState, create_spatial_query_routes};
pub use e3d_tree_api::{E3dTreeApiState, create_e3d_tree_routes};
pub use room_tree_api::create_room_tree_routes;
pub use pdms_attr_api::create_pdms_attr_routes;
pub use ptset_api::create_ptset_routes;
pub use pdms_transform_api::create_pdms_transform_routes;
pub use collision_api::{CollisionApiState, create_collision_routes};
pub use pipeline_annotation_api::create_pipeline_annotation_routes;
pub use mbd_pipe_api::{
    create_mbd_pipe_routes, export_mbd_json_batch, generate_mbd_data, get_mbd_output_dir,
    MbdExportScope, MbdExportStats,
};
pub use pdms_model_query_api::create_pdms_model_query_routes;
pub use scene_tree_api::create_scene_tree_routes;
pub use search_api::{SearchApiState, create_search_routes};
pub mod review_integration;
pub use review_integration::create_review_integration_routes;
pub mod model_center_client;
pub use model_center_client::create_model_center_routes;

pub mod pdms_model_query_api;

#[cfg(feature = "web_server")]
pub mod review_api;
#[cfg(feature = "web_server")]
pub use review_api::create_review_api_routes;

#[cfg(feature = "web_server")]
pub mod jwt_auth;
#[cfg(feature = "web_server")]
pub use jwt_auth::create_jwt_auth_routes;

#[cfg(test)]
mod tests;

