//! [foyer-removal] 桩模块：cata_resolve_cache_pipeline 已移除。

use std::sync::Arc;
use dashmap::DashMap;
use aios_core::pdms_types::CataHashRefnoKV;
use crate::options::DbOptionExt;

pub struct PrefetchOutcome {
    pub failed: usize,
    pub success: usize,
}

pub async fn prefetch_cata_resolve_cache_for_target_map(
    _db_option: Arc<DbOptionExt>,
    _target_cata_map: Arc<DashMap<String, CataHashRefnoKV>>,
) -> anyhow::Result<PrefetchOutcome> {
    Ok(PrefetchOutcome { failed: 0, success: 0 })
}
