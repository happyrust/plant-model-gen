use super::cata_resolve_cache_pipeline;
use super::context::{GenStage, NounProcessContext};
use super::utilities::{build_cata_hash_map_from_tree, is_valid_cata_hash};
use crate::fast_model::cata_model;
use crate::fast_model::foyer_cache::cata_resolve_cache;
use crate::fast_model::foyer_cache::geom_input_cache;
use crate::fast_model::gen_model::cache_miss_report;
use crate::fast_model::SEND_INST_SIZE;
use aios_core::RefnoEnum;
use aios_core::geometry::{EleGeosInfo, EleInstGeo, EleInstGeosData, GeoBasicType, ShapeInstancesData};
use anyhow::Result;
use dashmap::DashMap;
use glam::Vec3;
use std::collections::BTreeMap;
use std::sync::Arc;

/// 处理 Cate (元件库) 类型的 refno 页面
///
/// # Arguments
/// * `ctx` - 处理上下文
/// * `loop_sjus_map_arc` - Loop SJUS 映射
/// * `sender` - 几何数据发送通道
/// * `refnos` - 要处理的 refno 列表
pub async fn process_cate_refno_page(
    ctx: &NounProcessContext,
    loop_sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32)>>,
    sender: flume::Sender<ShapeInstancesData>,
    refnos: &[RefnoEnum],
) -> Result<()> {
    if refnos.is_empty() {
        return Ok(());
    }

    // 查询 refnos 对应的 cata hash 分组
    let target_cata_map = match build_cata_hash_map_from_tree(refnos).await {
        Ok(map) => Arc::new(map),
        Err(e) => {
            // 离线生成不可回查 DB；此处失败即表示 prefetch/元数据准备不完整，必须直接失败。
            if ctx.is_offline_generate() {
                return Err(e);
            }

            // Direct/非离线路径：保守起见仅记录并跳过，避免影响历史行为。
            eprintln!(
                "[cate_processor] build_cata_hash_map_from_tree 失败（将跳过 CATE）: {}",
                e
            );
            cache_miss_report::with_global_report(|r| {
                r.record_simple_miss(
                    ctx.gen_stage.as_str(),
                    "cate:cata_hash_map_build_failed",
                    Some("build_cata_hash_map_from_tree failed (missing db_meta or tree files?)"),
                )
            });
            return Ok(());
        }
    };

    if target_cata_map.is_empty() {
        return Ok(());
    }

    // Generate 阶段（且非 Direct）：严格只读缓存，不回查 SurrealDB。
    if ctx.is_offline_generate() {
        return gen_cate_instances_from_cache_only(ctx, &target_cata_map, sender, refnos).await;
    }

    // 方案 A（Direct/非离线）：允许在 Prefetch 阶段预热 resolve_desi_comp 产物缓存（按 cata_hash）。
    // 注意：Generate 离线阶段禁止触发 prefetch。
    if matches!(ctx.gen_stage, GenStage::Prefetch)
        && cata_resolve_cache_pipeline::is_cata_resolve_cache_prefetch_enabled()
    {
        if let Err(e) = cata_resolve_cache_pipeline::prefetch_cata_resolve_cache_for_target_map(
            ctx.db_option.clone(),
            target_cata_map.clone(),
        )
        .await
        {
            eprintln!(
                "[cate_processor] cata_resolve_cache prefetch 失败（将继续走正常生成流程）: {}",
                e
            );
        }
    }

    // 生成 cata 几何体
    cata_model::gen_cata_instances(
        ctx.db_option.clone(),
        target_cata_map,
        loop_sjus_map_arc,
        sender,
    )
    .await?;

    Ok(())
}

async fn gen_cate_instances_from_cache_only(
    ctx: &NounProcessContext,
    target_cata_map: &Arc<DashMap<String, aios_core::pdms_types::CataHashRefnoKV>>,
    sender: flume::Sender<ShapeInstancesData>,
    refnos: &[RefnoEnum],
) -> Result<()> {
    // 1) cate inst 输入：仅从 geom_input_cache 读取
    geom_input_cache::init_global_geom_input_cache(ctx.db_option.as_ref()).await?;
    let cate_inputs = geom_input_cache::load_cate_inputs_for_refnos_from_global(refnos).await?;
    if cate_inputs.len() != refnos.len() {
        let mut missing: Vec<RefnoEnum> = refnos
            .iter()
            .copied()
            .filter(|r| !cate_inputs.contains_key(r))
            .collect();
        missing.sort_by_key(|r| r.refno());
        cache_miss_report::with_global_report(|r| {
            for &rno in &missing {
                r.record_refno_miss(
                    ctx.gen_stage.as_str(),
                    "cate:cate_input_miss",
                    rno,
                    Some("missing cate input in geom_input_cache (need prefetch)"),
                );
            }
        });
        anyhow::bail!(
            "离线生成禁止 CATE 输入 miss：request={}, hit={}, missing={}, sample=[{}]",
            refnos.len(),
            cate_inputs.len(),
            missing.len(),
            missing
                .iter()
                .take(32)
                .map(|r| r.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    // 2) prepared geos/ptset：仅从 cata_resolve_cache 读取
    let cache_dir = ctx.db_option.get_foyer_cache_dir().join("cata_resolve_cache");
    cata_resolve_cache::init_global_cata_resolve_cache(cache_dir).await?;
    let Some(resolve_cache) = cata_resolve_cache::global_cata_resolve_cache() else {
        cache_miss_report::with_global_report(|r| {
            r.record_simple_miss(
                ctx.gen_stage.as_str(),
                "cate:cata_resolve_cache_uninitialized",
                Some("global_cata_resolve_cache is None"),
            )
        });
        anyhow::bail!("global_cata_resolve_cache 未初始化");
    };

    let respect_tufl = std::env::var_os("AIOS_RESPECT_TUFL").is_some();
    let mut shape_insts_data = ShapeInstancesData::default();

    // 3) 按 cata_hash 分组处理（注意：dashmap entry 不能跨 await 持有）
    for kv in target_cata_map.iter() {
        let cata_hash = kv.key().clone();
        let group_refnos = kv.value().group_refnos.clone();
        drop(kv);

        let Some(resolved_comp) = resolve_cache.get(&cata_hash).await else {
            cache_miss_report::with_global_report(|r| {
                for &rno in &group_refnos {
                    r.record_refno_miss(
                        ctx.gen_stage.as_str(),
                        "cate:cata_resolve_cache_miss",
                        rno,
                        Some("missing prepared geos in cata_resolve_cache (need prefetch)"),
                    );
                }
            });
            anyhow::bail!(
                "离线生成禁止 cata_resolve_cache miss：cata_hash={}, group_refnos_sample={:?}",
                cata_hash,
                group_refnos.iter().take(8).collect::<Vec<_>>()
            );
        };

        // 3.1) 将 prepared geos 转为 inst_geo 列表（复用于组内每个实例）
        let mut geo_insts: Vec<EleInstGeo> = Vec::new();
        for g in resolved_comp.geos.iter() {
            if respect_tufl && !g.shape_visible {
                continue;
            }
            let visible = g.geo_type == GeoBasicType::Pos;
            geo_insts.push(EleInstGeo {
                geo_hash: g.geo_hash,
                refno: g.geom_refno,
                pts: g.pts.clone(),
                aabb: None,
                geo_transform: g.geo_transform,
                geo_param: g.geo_param.clone(),
                visible,
                is_tubi: g.is_tubi,
                geo_type: g.geo_type.clone(),
                cata_neg_refnos: vec![],
            });
        }

        let has_solid = resolved_comp.has_solid;
        let ptset_map: BTreeMap<i32, aios_core::parsed_data::CateAxisParam> = resolved_comp.ptset_map();

        for &group_refno in group_refnos.iter() {
            let Some(input) = cate_inputs.get(&group_refno) else {
                cache_miss_report::with_global_report(|r| {
                    r.record_refno_miss(
                        ctx.gen_stage.as_str(),
                        "cate:cate_input_miss",
                        group_refno,
                        Some("missing cate input in geom_input_cache (need prefetch)"),
                    )
                });
                anyhow::bail!(
                    "离线生成禁止 CATE 输入 miss：refno={}, cata_hash={}",
                    group_refno,
                    cata_hash
                );
            };

            let type_name = input.attmap.get_type_str().to_string();
            let cata_hash_for_info = if is_valid_cata_hash(&cata_hash) {
                Some(cata_hash.clone())
            } else {
                None
            };

            let geos_info = EleGeosInfo {
                refno: group_refno,
                sesno: input.attmap.sesno(),
                owner_refno: input.owner_refno,
                owner_type: input.owner_type.clone(),
                cata_hash: cata_hash_for_info,
                visible: input.visible,
                ptset_map: ptset_map.clone(),
                is_solid: has_solid,
                world_transform: input.world_transform,
                ..Default::default()
            };

            shape_insts_data.insert_info(group_refno, geos_info.clone());

            if !geo_insts.is_empty() {
                let inst_key = geos_info.get_inst_key();
                shape_insts_data.insert_geos_data(
                    inst_key.clone(),
                    EleInstGeosData {
                        inst_key,
                        refno: group_refno,
                        insts: geo_insts.clone(),
                        aabb: None,
                        type_name,
                        ..Default::default()
                    },
                );
            }

            if shape_insts_data.inst_cnt() >= SEND_INST_SIZE {
                sender
                    .send(std::mem::take(&mut shape_insts_data))
                    .expect("send cate shape_insts_data error");
            }
        }
    }

    if shape_insts_data.inst_cnt() > 0 {
        sender
            .send(shape_insts_data)
            .expect("send cate shape_insts_data error");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::DbOptionExt;
    use aios_core::options::DbOption;

    #[tokio::test]
    async fn test_empty_refnos() {
        let ctx = NounProcessContext::new(
            Arc::new(DbOptionExt::from(DbOption::default())),
            100,
            4,
        );
        let loop_sjus_map = Arc::new(DashMap::new());
        let (sender, _receiver) = flume::unbounded();

        let result = process_cate_refno_page(&ctx, loop_sjus_map, sender, &[]).await;
        assert!(result.is_ok());
    }
}
