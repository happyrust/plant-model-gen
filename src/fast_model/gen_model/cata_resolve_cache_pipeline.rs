//! [foyer-removal] 桩模块：cata_resolve_cache_pipeline 已移除。
//!
//! 原计划第 7/8 项（基于 foyer_cache 的缓存持久化）已作废。
//! 若未来需要缓存层，应基于新架构重新设计（不再基于已移除的 foyer）。

use std::sync::Arc;
use dashmap::DashMap;
use aios_core::pdms_types::CataHashRefnoKV;
use crate::options::DbOptionExt;

pub struct PrefetchOutcome {
    pub failed: usize,
    pub success: usize,
}

#[deprecated(note = "foyer_cache 已移除，此函数为空操作桩。若需缓存层，请基于新架构重新设计")]
pub async fn prefetch_cata_resolve_cache_for_target_map(
    _db_option: Arc<DbOptionExt>,
    _target_cata_map: Arc<DashMap<String, CataHashRefnoKV>>,
) -> anyhow::Result<PrefetchOutcome> {
    Ok(PrefetchOutcome { failed: 0, success: 0 })
}
