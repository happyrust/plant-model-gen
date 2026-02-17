//! 分批生成时的批次间缓存清理。
//!
//! 保留策略：
//! - ✅ 清理：geom_input_cache（LOOP/PRIM/CATE 输入）、transform_cache（world_transform）
//! - ❌ 保留：cata_resolve_cache（按 cata_hash 索引，跨批复用率高）
//! - ❌ 保留：TreeIndex / DbMeta（只读基础设施）
//! - ❌ 保留：cache_miss_report（跨批累积统计）

use crate::fast_model::foyer_cache::geom_input_cache;
use crate::fast_model::transform_cache;

/// 分批生成时，在每个批次完成后调用，清理本批临时缓存。
pub fn cleanup_batch_caches() {
    let geom_cleared = geom_input_cache::clear_global_geom_input_cache();
    let transform_cleared = transform_cache::clear_global_transform_cache();
    println!(
        "[batch_cleanup] 批次缓存已清理: geom_input={}, transform={}",
        geom_cleared, transform_cleared
    );
}
