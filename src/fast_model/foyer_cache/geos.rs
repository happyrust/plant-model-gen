//! foyer cache 专用的几何生成入口（CATA/BRAN/TUBI）
//!
//! 目前几何生成的 cache-only 实现位于 `src/fast_model/cata_cache_gen.rs`，
//! 该文件本身已经按阶段拆分并具备较完善的注释（作为标杆）。
//!
//! 此模块作为“专区门面”，统一对外暴露 cache-only 的几何生成 API，
//! 便于 orchestrator 与其它模块按 `fast_model::foyer_cache::*` 的方式组织调用。

pub use crate::fast_model::cata_cache_gen::{
    gen_bran_geos_for_cache, gen_cata_geos_for_cache, gen_tubi_for_cache,
};

use std::sync::Arc;

use aios_core::RefnoEnum;
use aios_core::geometry::ShapeInstancesData;
use dashmap::DashMap;
use glam::Vec3;

use crate::fast_model::cata_model::BranchTubiOutcome;
use crate::fast_model::foyer_cache::FoyerCacheContext;
use crate::options::DbOptionExt;

/// cache-only：复用 `FoyerCacheContext`（避免重复打开 instance_cache）生成 tubing。
pub async fn gen_tubi_for_cache_with_ctx(
    ctx: &FoyerCacheContext,
    db_option: Arc<DbOptionExt>,
    branch_refnos: &[RefnoEnum],
    sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32)>>,
    sender: flume::Sender<ShapeInstancesData>,
) -> anyhow::Result<BranchTubiOutcome> {
    crate::fast_model::cata_cache_gen::gen_tubi_for_cache_with_cache_manager(
        db_option,
        branch_refnos,
        sjus_map_arc,
        sender,
        ctx.cache(),
    )
    .await
}
