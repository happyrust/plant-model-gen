//! 分批生成时的批次间缓存清理。
//!
//! 保留策略：
//! - ✅ 清理：geom_input_cache（LOOP/PRIM/CATE 输入）、transform_cache（world_transform）
//! - ❌ 保留：cata_resolve_cache（按 cata_hash 索引，跨批复用率高）
//! - ❌ 保留：TreeIndex / DbMeta（只读基础设施）
//! - ❌ 保留：cache_miss_report（跨批累积统计）

use crate::fast_model::foyer_cache::geom_input_cache;
use crate::fast_model::transform_cache;
use aios_core::RefnoEnum;

/// 分批生成时，在每个批次完成后调用，清理本批临时缓存。
pub fn cleanup_batch_caches() {
    let geom_cleared = geom_input_cache::clear_global_geom_input_cache();
    let transform_cleared = transform_cache::clear_global_transform_cache();
    println!(
        "[batch_cleanup] 批次缓存已清理: geom_input={}, transform={}",
        geom_cleared, transform_cleared
    );
}

/// 分批生成：为本批次 refnos 增加缓存租约，防止并发任务互相清理。
pub fn pin_batch_caches_for_refnos(refnos: &[RefnoEnum]) {
    let geom_pinned = geom_input_cache::pin_global_geom_input_cache_for_refnos(refnos);
    let transform_pinned = transform_cache::pin_global_transform_cache_for_refnos(refnos);
    println!(
        "[batch_cleanup] 批次缓存租约已建立: refnos={}, geom_input_pins={}, transform_pins={}",
        refnos.len(),
        geom_pinned,
        transform_pinned
    );
}

/// 分批生成：释放本批次 refnos 的缓存租约（不清理条目）。
pub fn release_batch_caches_for_refnos(refnos: &[RefnoEnum]) {
    let geom_released = geom_input_cache::release_global_geom_input_cache_for_refnos(refnos);
    let transform_released = transform_cache::release_global_transform_cache_for_refnos(refnos);
    println!(
        "[batch_cleanup] 批次缓存租约已释放: refnos={}, geom_input_released={}, transform_released={}",
        refnos.len(),
        geom_released,
        transform_released
    );
}

/// 分批生成：按本批次 refnos 定向清理缓存，避免并发任务互相清空全局缓存。
pub fn cleanup_batch_caches_for_refnos(refnos: &[RefnoEnum]) {
    let geom_cleared = geom_input_cache::clear_global_geom_input_cache_for_refnos(refnos);
    let transform_cleared = transform_cache::clear_global_transform_cache_for_refnos(refnos);
    println!(
        "[batch_cleanup] 批次缓存已定向清理: refnos={}, geom_input={}, transform={}",
        refnos.len(),
        geom_cleared,
        transform_cleared
    );
}
