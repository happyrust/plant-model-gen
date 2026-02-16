//! CATE resolve 产物预热流水线（方案 A：先填充 foyer cache，再走正常生成）
//!
//! 目标：
//! - 在进入 `cata_model::gen_cata_instances` 之前，按 `cata_hash` 预先生成并写入
//!   `foyer_cache/cata_resolve_cache`（rkyv payload）。
//! - 生成阶段即可尽量做到 “cache hit -> 不再调用 resolve_desi_comp”。
//!
//! 说明：
//! - 本模块不写入 inst_info/inst_geo/geo_relate；只负责填充 `cata_resolve_cache`。
//! - 是否需要预热由环境变量控制（见 `is_cata_resolve_cache_prefetch_enabled`）。

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use aios_core::geometry::{GeoBasicType};
use aios_core::parsed_data::geo_params_data::PdmsGeoParam;
use aios_core::pdms_types::CataHashRefnoKV;
use aios_core::prim_geo::category::CateCsgShape;
use aios_core::RefnoEnum;
use aios_core::Transform;
use dashmap::DashMap;
use tokio::sync::Semaphore;

use crate::fast_model::foyer_cache::cata_resolve_cache::{
    CataResolveCacheManager, CataResolvedComp, PreparedInstGeo,
};
use crate::fast_model::gen_model::cate_single::{gen_cata_single_geoms, CateCsgShapeMap};
use crate::options::DbOptionExt;

static NEXT_JOB_ID: AtomicU64 = AtomicU64::new(1);

pub fn is_cata_resolve_cache_prefetch_enabled() -> bool {
    std::env::var("AIOS_CATA_RESOLVE_CACHE_PREFETCH")
        .ok()
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
}

pub fn cata_resolve_cache_prefetch_concurrency() -> usize {
    // 默认并发：min(8, 机器并行度)
    let default_c = std::thread::available_parallelism()
        .ok()
        .map(|n| n.get().min(8))
        .unwrap_or(8);
    std::env::var("AIOS_CATA_RESOLVE_CACHE_PREFETCH_CONCURRENCY")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&v| v > 0)
        .unwrap_or(default_c)
}

#[derive(Debug, Default, Clone)]
pub struct PrefetchOutcome {
    pub total_groups: usize,
    pub skipped_reuse: usize,
    pub cache_hit: usize,
    pub computed: usize,
    pub failed: usize,
    pub elapsed_ms: u128,
}

fn build_prepared_inst_geos_from_shapes_for_cache(
    shapes: Vec<CateCsgShape>,
) -> (Vec<PreparedInstGeo>, bool) {
    let mut out: Vec<PreparedInstGeo> = Vec::new();
    let mut has_solid = false;

    for shape in shapes.into_iter() {
        let CateCsgShape {
            refno: geom_refno,
            csg_shape,
            transform: shape_transform,
            visible: shape_visible,
            is_tubi,
            pts,
            is_ngmr,
            ..
        } = shape;

        // 只缓存“可生成”的条目；无效几何直接跳过。
        if !csg_shape.check_valid() {
            continue;
        }

        // 获取形状自身的变换（包含 scale）
        let shape_trans = csg_shape.get_trans();
        let geo_hash = csg_shape.hash_unit_mesh_params();
        let unit_flag = csg_shape.is_reuse_unit();

        // 合并变换：shape_transform 是元件变换，shape_trans 是形状自身变换
        let translation = shape_transform.translation + shape_transform.rotation * shape_trans.translation;
        let rotation = shape_transform.rotation;
        let scale = shape_trans.scale;

        let mut transform = Transform {
            translation,
            rotation,
            scale,
        };

        if transform.translation.is_nan() || transform.rotation.is_nan() || transform.scale.is_nan() {
            continue;
        }

        // 获取 geo_param
        let mut geo_param = csg_shape
            .convert_to_geo_param()
            .unwrap_or(PdmsGeoParam::Unknown);

        // unit_flag=true 时，写入"单位参数"，保留 transform.scale
        if unit_flag {
            geo_param = csg_shape
                .gen_unit_shape()
                .convert_to_geo_param()
                .unwrap_or(geo_param);
        }

        // 统一处理 transform.scale
        crate::fast_model::reuse_unit::normalize_transform_scale(&mut transform, unit_flag, geo_hash);

        let geo_type = if is_ngmr {
            GeoBasicType::CataCrossNeg
        } else {
            GeoBasicType::Pos
        };

        if geo_type == GeoBasicType::Pos {
            has_solid = true;
        }

        out.push(PreparedInstGeo {
            geo_hash,
            geom_refno,
            pts,
            geo_transform: transform,
            geo_param,
            shape_visible,
            is_tubi,
            geo_type,
            unit_flag,
        });
    }

    (out, has_solid)
}

/// 方案 A：在进入生成前，按 cata_hash 预热 resolve 产物缓存。
pub async fn prefetch_cata_resolve_cache_for_target_map(
    db_option: Arc<DbOptionExt>,
    target_cata_map: Arc<DashMap<String, CataHashRefnoKV>>,
) -> anyhow::Result<PrefetchOutcome> {
    let t0 = Instant::now();

    let cache_mgr = Arc::new(CataResolveCacheManager::new());

    let replace_exist = db_option.inner.is_replace_mesh();
    let concurrency = cata_resolve_cache_prefetch_concurrency();
    let sem = Arc::new(Semaphore::new(concurrency));

    // 先收集 key，避免持锁跨 await
    let keys: Vec<String> = target_cata_map.iter().map(|x| x.cata_hash.clone()).collect();
    let total_groups = keys.len();

    println!(
        "[cata_resolve_cache_pipeline] prefetch start: total_groups={}, concurrency={}, replace_exist={}",
        total_groups, concurrency, replace_exist
    );

    let mut join_set = tokio::task::JoinSet::new();
    let mut skipped_reuse = 0usize;
    for cata_hash in keys.into_iter() {
        if cata_hash == "0" || cata_hash.is_empty() {
            continue;
        }

        let Some(entry) = target_cata_map.get(&cata_hash) else {
            continue;
        };
        let exist_inst = entry.exist_inst;
        let group_refnos = entry.group_refnos.clone();
        drop(entry);

        // 非 replace_mesh 场景：inst_info 已存在时无需预热（生成阶段也不会走 resolve）。
        if exist_inst && !replace_exist {
            skipped_reuse += 1;
            continue;
        }

        let cache_mgr = cache_mgr.clone();
        let sem = sem.clone();
        join_set.spawn(async move {
            let job_id = NEXT_JOB_ID.fetch_add(1, Ordering::Relaxed);
            let _permit = sem.acquire_owned().await.expect("semaphore closed");

            if cache_mgr.get(&cata_hash).is_some() {
                return Ok::<_, anyhow::Error>((job_id, cata_hash, "hit".to_string()));
            }

            let ele_refno = match group_refnos.first().copied() {
                Some(r) => r,
                None => {
                    return Ok::<_, anyhow::Error>((job_id, cata_hash, "failed".to_string()));
                }
            };

            // 计算一次 resolve_desi_comp 产物（通过 gen_cata_single_geoms）
            let csg_shapes_map = CateCsgShapeMap::new();
            let design_axis_map: DashMap<RefnoEnum, crate::data_interface::structs::PlantAxisMap> =
                DashMap::new();

            // NOTE: gen_cata_single_geoms 内部会自行查询 attmap/resolve_desi_comp 等。
            let r = gen_cata_single_geoms(ele_refno, &csg_shapes_map, &design_axis_map).await;
            if r.is_err() {
                return Ok::<_, anyhow::Error>((job_id, cata_hash, "failed".to_string()));
            }

            let ptset_map: BTreeMap<i32, aios_core::parsed_data::CateAxisParam> = design_axis_map
                .get(&ele_refno)
                .map(|v| v.clone())
                .unwrap_or_default();

            let shapes: Vec<CateCsgShape> = csg_shapes_map
                .get(&ele_refno)
                .map(|v| v.clone())
                .unwrap_or_default();

            let (geos, has_solid) = build_prepared_inst_geos_from_shapes_for_cache(shapes);
            let resolved_comp = CataResolvedComp {
                created_at: chrono::Utc::now().timestamp_millis(),
                ptset_items: ptset_map.into_iter().collect(),
                geos,
                has_solid,
            };
            cache_mgr.insert(cata_hash.clone(), &resolved_comp);

            Ok::<_, anyhow::Error>((job_id, cata_hash, "computed".to_string()))
        });
    }

    let mut out = PrefetchOutcome {
        total_groups,
        skipped_reuse,
        ..Default::default()
    };
    while let Some(res) = join_set.join_next().await {
        match res {
            Ok(Ok((_job_id, _hash, status))) => match status.as_str() {
                "hit" => out.cache_hit += 1,
                "computed" => out.computed += 1,
                _ => out.failed += 1,
            },
            _ => out.failed += 1,
        }
    }

    out.elapsed_ms = t0.elapsed().as_millis();
    println!(
        "[cata_resolve_cache_pipeline] prefetch finish: total_groups={}, skipped_reuse={}, hit={}, computed={}, failed={}, elapsed_ms={}",
        out.total_groups, out.skipped_reuse, out.cache_hit, out.computed, out.failed, out.elapsed_ms
    );
    Ok(out)
}
