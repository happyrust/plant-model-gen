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

/// 方案 B：从 SurrealDB 的 `tubi_relate` 读取最小必要信息，并写入 foyer cache 的 `inst_tubi_map`。
///
/// 写入字段（至少）：
/// - `owner_refno`（BRAN/HANG）
/// - `refno`（leave_refno，用作 tubi 的稳定主键）
/// - `tubi_arrive_refno`（arrive_refno）
/// - `tubi_index`（index/order）
/// 以及导出/渲染常用的：`world_transform`、`aabb`、`tubi_start_pt`、`tubi_end_pt`。
pub async fn write_tubi_relate_into_cache_with_ctx(
    ctx: &FoyerCacheContext,
    dbnum: u32,
    owner_refnos: &[RefnoEnum],
) -> anyhow::Result<usize> {
    use aios_core::geometry::EleGeosInfo;
    use aios_core::rs_surreal::geometry_query::PlantTransform;
    use aios_core::shape::pdms_shape::RsVec3;
    use aios_core::types::PlantAabb;
    use aios_core::{SUL_DB, SurrealQueryExt};
    use serde::{Deserialize, Serialize};
    use surrealdb::types::SurrealValue;

    if owner_refnos.is_empty() {
        return Ok(0);
    }

    // tubi_relate 读取依赖 SurrealDB：此处统一确保连接已就绪。
    aios_core::init_surreal().await?;

    #[derive(Serialize, Deserialize, Debug, SurrealValue)]
    struct TubiRelateRow {
        pub owner_refno: RefnoEnum,
        pub leave_refno: RefnoEnum,
        pub arrive_refno: RefnoEnum,
        #[serde(default)]
        pub world_trans: Option<PlantTransform>,
        #[serde(default)]
        pub world_aabb: Option<PlantAabb>,
        #[serde(default)]
        pub start_pt: Option<RsVec3>,
        #[serde(default)]
        pub end_pt: Option<RsVec3>,
        #[serde(default)]
        pub index: Option<i64>,
    }

    let cache_manager = ctx.cache_arc();
    let mut total_written = 0usize;

    for &owner in owner_refnos {
        let owner_att = aios_core::get_named_attmap(owner).await.unwrap_or_default();
        let owner_type = owner_att.get_type_str().to_string();

        let pe_key = owner.to_pe_key();
        let sql = format!(
            r#"
            SELECT
                id[0] as owner_refno,
                in as leave_refno,
                out as arrive_refno,
                world_trans.d as world_trans,
                aabb.d as world_aabb,
                start_pt.d as start_pt,
                end_pt.d as end_pt,
                id[1] as index
            FROM tubi_relate:[{pe_key}, 0]..[{pe_key}, ..];
            "#
        );

        let rows: Vec<TubiRelateRow> = SUL_DB.query_take(&sql, 0).await?;
        if rows.is_empty() {
            continue;
        }

        let mut shape_insts = aios_core::geometry::ShapeInstancesData::default();
        for row in rows {
            let info = EleGeosInfo {
                refno: row.leave_refno,
                sesno: owner_att.sesno(),
                owner_refno: row.owner_refno,
                owner_type: owner_type.clone(),
                cata_hash: Some(aios_core::prim_geo::basic::TUBI_GEO_HASH.to_string()),
                visible: true,
                aabb: row.world_aabb.map(|a| a.0),
                world_transform: row.world_trans.unwrap_or_default().0,
                tubi_start_pt: row.start_pt.map(|p| p.0),
                tubi_end_pt: row.end_pt.map(|p| p.0),
                tubi_arrive_refno: Some(row.arrive_refno),
                tubi_index: row.index.and_then(|i| u32::try_from(i).ok()),
                is_solid: true,
                ..Default::default()
            };
            shape_insts.insert_tubi(row.leave_refno, info);
        }

        total_written += shape_insts.inst_tubi_map.len();
        cache_manager.insert_from_shape(dbnum, &shape_insts);
    }

    Ok(total_written)
}
