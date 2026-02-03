//! foyer cache 专用的 ptset（ARRIVE/LEAVE）读取门面
//!
//! ptset 数据主要用于 tubing 生成（ARRIVE/LEAVE 两个端点）以及部分诊断工具。
//! 为避免在各处散落 `InstanceCacheManager::get_ptset_*` 的调用点，本模块提供统一入口，
//! 并约定返回结构：
//! - `[0]` = ARRIVE（ptset[1]）
//! - `[1]` = LEAVE（ptset[2]）

use std::collections::HashMap;

use aios_core::parsed_data::CateAxisParam;
use aios_core::RefnoEnum;

use crate::fast_model::foyer_cache::FoyerCacheContext;

/// 批量获取指定 refno 列表的 ARRIVE/LEAVE 点（自动按 dbnum 分组）。
pub async fn get_ptset_maps_for_refnos_auto(
    ctx: &FoyerCacheContext,
    refnos: &[RefnoEnum],
) -> HashMap<RefnoEnum, [CateAxisParam; 2]> {
    ctx.cache().get_ptset_maps_for_refnos_auto(refnos).await
}

/// 批量获取指定 dbnum 下 refno 列表的 ARRIVE/LEAVE 点。
pub async fn get_ptset_maps_for_refnos(
    ctx: &FoyerCacheContext,
    dbnum: u32,
    refnos: &[RefnoEnum],
) -> HashMap<RefnoEnum, [CateAxisParam; 2]> {
    ctx.cache().get_ptset_maps_for_refnos(dbnum, refnos).await
}

