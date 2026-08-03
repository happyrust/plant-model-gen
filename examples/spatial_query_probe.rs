//! 只挂载空间查询路由的探针服务，用于在没有完整站点配置时实跑验证这几个接口。
//!
//! 索引路径通过 `AIOS_SPATIAL_INDEX_SQLITE` 指定，监听地址通过
//! `SPATIAL_PROBE_ADDR` 指定（默认 127.0.0.1:3199）。
//!
//! ```text
//! $env:AIOS_SPATIAL_INDEX_SQLITE = "...\spatial_index.sqlite"
//! cargo run --example spatial_query_probe
//! ```

use aios_database::web_server::sqlite_spatial_api;
use axum::{
    Router,
    routing::{get, post},
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let app = Router::new()
        .route(
            "/api/sqlite-spatial/query",
            get(sqlite_spatial_api::api_sqlite_spatial_query),
        )
        .route(
            "/api/sqlite-spatial/nearby",
            get(sqlite_spatial_api::api_sqlite_spatial_nearby),
        )
        .route(
            "/api/sqlite-spatial/nearby/refnos",
            get(sqlite_spatial_api::api_sqlite_spatial_nearby_refnos),
        )
        .route(
            "/api/sqlite-spatial/nearest-clearance",
            get(sqlite_spatial_api::api_sqlite_spatial_nearest_clearance),
        )
        .route(
            "/api/sqlite-spatial/backfill-names",
            post(sqlite_spatial_api::api_sqlite_spatial_backfill_names),
        )
        .route(
            "/api/sqlite-spatial/stats",
            get(sqlite_spatial_api::api_sqlite_spatial_stats),
        )
        .route(
            "/api/space/nearest-points",
            post(sqlite_spatial_api::api_space_nearest_points),
        );

    let addr =
        std::env::var("SPATIAL_PROBE_ADDR").unwrap_or_else(|_| "127.0.0.1:3199".to_string());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("SPATIAL_PROBE_READY http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}
