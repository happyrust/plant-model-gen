//! foyer cache 专用的布尔运算 Worker（不访问 SurrealDB）
//!
//! 布尔运算的核心实现目前仍位于 `fast_model::manifold_bool`（包含 DB 路径与 cache-only 路径的共享能力）。
//! 本模块提供清晰的 cache-only 入口，并与 `FoyerCacheContext` 对接，便于 orchestrator 统一编排。

use std::path::Path;

use crate::fast_model::foyer_cache::FoyerCacheContext;

/// 基于 foyer 缓存的布尔运算（不访问 SurrealDB）
pub async fn run_boolean_worker(ctx: &FoyerCacheContext) -> anyhow::Result<usize> {
    crate::fast_model::manifold_bool::run_boolean_worker_from_cache_manager(ctx.cache()).await
}

/// 兼容入口：直接传入 cache_dir（内部自建 `InstanceCacheManager`）
pub async fn run_boolean_worker_from_cache_dir(cache_dir: &Path) -> anyhow::Result<usize> {
    let ctx = FoyerCacheContext::from_cache_dir(cache_dir).await?;
    run_boolean_worker(&ctx).await
}

