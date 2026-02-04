//! foyer cache 专用的布尔运算 Worker（不访问 SurrealDB）
//!
//! 布尔运算的核心实现目前仍位于 `fast_model::manifold_bool`（包含 DB 路径与 cache-only 路径的共享能力）。
//! 本模块提供清晰的 cache-only 入口，并与 `FoyerCacheContext` 对接，便于 orchestrator 统一编排。

use std::collections::HashSet;
use std::path::Path;

use aios_core::RefnoEnum;

use crate::fast_model::foyer_cache::FoyerCacheContext;

/// 基于 foyer 缓存的布尔运算（不访问 SurrealDB）
/// 
/// # 参数
/// - `ctx`: FoyerCacheContext 缓存上下文
/// - `filter_refnos`: 可选的 refno 过滤集合，仅处理该集合内的 refno（用于 debug_model 模式）
pub async fn run_boolean_worker_with_filter(
    ctx: &FoyerCacheContext,
    filter_refnos: Option<&HashSet<RefnoEnum>>,
) -> anyhow::Result<usize> {
    crate::fast_model::foyer_cache::manifold_bool::run_boolean_worker_from_cache_manager(
        ctx.cache(),
        filter_refnos,
    )
    .await
}

/// 基于 foyer 缓存的布尔运算（不访问 SurrealDB，无过滤）
pub async fn run_boolean_worker(ctx: &FoyerCacheContext) -> anyhow::Result<usize> {
    run_boolean_worker_with_filter(ctx, None).await
}

/// 兼容入口：直接传入 cache_dir（内部自建 `InstanceCacheManager`）
pub async fn run_boolean_worker_from_cache_dir(cache_dir: &Path) -> anyhow::Result<usize> {
    let ctx = FoyerCacheContext::from_cache_dir(cache_dir).await?;
    run_boolean_worker(&ctx).await
}
