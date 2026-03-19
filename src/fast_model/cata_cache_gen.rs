//! model cache 专用的几何生成模块
//!
//! 将 CATA、BRAN/HANG、tubi 三个阶段分离：
//! 1. `gen_cata_geos_for_cache` - CATA 几何生成，不收集 tubi 相关信息
//! 2. `gen_bran_geos_for_cache` - BRAN/HANG 几何生成，不生成 tubi
//! 3. `gen_tubi_for_cache` - 在所有 BRAN 模型生成完成后单独遍历生成

use crate::fast_model::gen_model::cate_single::{CateCsgShapeMap, gen_cata_single_geoms};
use crate::fast_model::gen_model::utilities::is_valid_cata_hash;
use crate::fast_model::instance_cache::InstanceCacheManager;
use crate::fast_model::model_cache::cata_resolve_cache::{CataResolvedComp, PreparedInstGeo};
use crate::fast_model::refno_errors::{RefnoErrorKind, RefnoErrorStage, record_refno_error};
use crate::fast_model::{SEND_INST_SIZE, shared};
use crate::fast_model::{debug_model, debug_model_debug};
use crate::options::DbOptionExt;
use aios_core::RefnoEnum;
use aios_core::Transform;
use aios_core::geometry::{
    EleGeosInfo, EleInstGeo, EleInstGeosData, GeoBasicType, ShapeInstancesData,
};
use aios_core::parsed_data::CateAxisParam;
use aios_core::parsed_data::geo_params_data::PdmsGeoParam;
use aios_core::pdms_types::CataHashRefnoKV;
use aios_core::pe::SPdmsElement;
use aios_core::prim_geo::category::CateCsgShape;
use chrono::Utc;
use dashmap::DashMap;
use glam::Vec3;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Instant;
#[cfg(feature = "profile")]
use tokio::sync::Mutex;
use tokio::sync::Semaphore;

use crate::fast_model::cata_model::BranchTubiOutcome;

/// model cache 专用的简化 CATA 输出结构
#[derive(Debug, Default)]
pub struct SimpleCataOutcome {
    pub time_stats: HashMap<String, u64>,
    pub unique_cata_cnt: usize,
    pub elapsed_ms: u128,
}

/// model cache 专用的简化 BRAN 输出结构
#[derive(Debug, Default)]
pub struct SimpleBranOutcome {
    pub time_stats: HashMap<String, u64>,
    pub bran_count: usize,
    pub elapsed_ms: u128,
}

fn build_prepared_inst_geos_from_shapes(shapes: Vec<CateCsgShape>) -> (Vec<PreparedInstGeo>, bool) {
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

        // resolve_desi_comp 的缓存只保存“可生成”的条目；无效几何直接跳过。
        if !csg_shape.check_valid() {
            continue;
        }

        // 获取形状自身的变换（包含 scale）
        let shape_trans = csg_shape.get_trans();
        let geo_hash = csg_shape.hash_unit_mesh_params();
        let unit_flag = csg_shape.is_reuse_unit();

        // 合并变换：shape_transform 是元件变换，shape_trans 是形状自身变换
        let translation =
            shape_transform.translation + shape_transform.rotation * shape_trans.translation;
        let rotation = shape_transform.rotation;
        let scale = shape_trans.scale;

        let mut transform = Transform {
            translation,
            rotation,
            scale,
        };

        if transform.translation.is_nan() || transform.rotation.is_nan() || transform.scale.is_nan()
        {
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
        crate::fast_model::reuse_unit::normalize_transform_scale(
            &mut transform,
            unit_flag,
            geo_hash,
        );

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

/// model cache 专用：仅生成 CATA 几何体，不收集 tubi 相关信息
///
/// 与 `gen_cata_instances` 的区别：
/// - 不创建 `tubi_info_map`
/// - 不收集 `local_al_map`
/// - 不设置 `geos_info.tubi_info_id`
///
/// 内部对每个 cata_hash 并行处理（JoinSet + Semaphore），
/// 每个 task 独立通过 sender 发送 ShapeInstancesData。
pub async fn gen_cata_geos_for_cache(
    db_option: Arc<DbOptionExt>,
    target_cata_map: Arc<DashMap<String, CataHashRefnoKV>>,
    _sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32)>>,
    sender: flume::Sender<ShapeInstancesData>,
) -> anyhow::Result<SimpleCataOutcome> {
    let total_t = Instant::now();
    #[cfg(feature = "profile")]
    let total_time_stats: Arc<DashMap<String, u64>> = Arc::new(DashMap::new());

    let all_unique_keys: Vec<String> = target_cata_map
        .iter()
        .map(|x| x.cata_hash.clone())
        .collect();

    let unique_cata_cnt = all_unique_keys.len();
    debug_model_debug!(
        "[gen_cata_geos_for_cache] start: unique_cata_cnt={}",
        unique_cata_cnt
    );

    if all_unique_keys.is_empty() {
        return Ok(SimpleCataOutcome {
            time_stats: HashMap::new(),
            unique_cata_cnt: 0,
            elapsed_ms: total_t.elapsed().as_millis(),
        });
    }

    let replace_exist = false; // replace_exist 已废弃
    let respect_tufl = std::env::var_os("AIOS_RESPECT_TUFL").is_some();
    let concurrency = std::thread::available_parallelism()
        .map(|n| n.get().min(8))
        .unwrap_or(4);
    let sem = Arc::new(Semaphore::new(concurrency));

    println!(
        "[gen_cata_geos_for_cache] 并行处理 {} 个 cata_hash, concurrency={}",
        unique_cata_cnt, concurrency
    );

    let mut join_set = tokio::task::JoinSet::new();

    for cata_hash in all_unique_keys {
        if cata_hash == "0" {
            continue;
        }

        let Some(target_cata) = target_cata_map.get(&cata_hash) else {
            continue;
        };
        let target_exist_inst = target_cata.exist_inst;
        let target_group_refnos = target_cata.group_refnos.clone();
        let target_ptset = target_cata.ptset.clone();
        drop(target_cata);

        let sender = sender.clone();
        let sem = sem.clone();
        #[cfg(feature = "profile")]
        let time_stats = total_time_stats.clone();

        join_set.spawn(async move {
            let _permit = sem.acquire().await.expect("semaphore closed");

            // ── 复用路径：inst_info 已存在 ──
            if target_exist_inst {
                let reuse_ptset_map = target_ptset.clone().unwrap_or_default();
                let mut shape_insts_data = ShapeInstancesData::default();

                for &ele_refno in target_group_refnos.iter() {
                    let ele_att = match aios_core::get_named_attmap(ele_refno).await {
                        Ok(att) => att,
                        Err(_) => continue,
                    };

                    let (owner_refno, owner_type) =
                        shared::get_owner_info_from_attr(&ele_att).await;
                    let cata_hash_for_info = if is_valid_cata_hash(&cata_hash) {
                        Some(cata_hash.clone())
                    } else {
                        None
                    };

                    let geos_info = EleGeosInfo {
                        refno: ele_refno,
                        sesno: ele_att.sesno(),
                        owner_refno,
                        owner_type,
                        cata_hash: cata_hash_for_info,
                        visible: true,
                        ptset_map: reuse_ptset_map.clone(),
                        is_solid: true,
                        ..Default::default()
                    };
                    shape_insts_data.insert_info(ele_refno, geos_info);
                }

                if shape_insts_data.inst_cnt() > 0 {
                    let _ = sender.send(shape_insts_data);
                }
                return Ok::<_, anyhow::Error>(());
            }

            // ── 计算路径：resolve_desi_comp ──
            let ele_refno = match target_group_refnos.first().copied() {
                Some(r) => r,
                None => return Ok(()),
            };

            #[cfg(feature = "profile")]
            let t_get_cat_refno = Instant::now();
            let cata_refno = match aios_core::get_cat_refno(ele_refno).await {
                Ok(Some(refno)) => refno,
                _ => return Ok(()),
            };
            #[cfg(feature = "profile")]
            let dt_cat_refno = t_get_cat_refno.elapsed().as_millis() as u64;

            #[cfg(feature = "profile")]
            let t_query_single = Instant::now();
            let gmse_refno =
                aios_core::query_single_by_paths(cata_refno, &["->GMRE", "->GSTR"], &["REFNO"])
                    .await
                    .map(|x| x.get_refno_or_default());
            #[cfg(feature = "profile")]
            let dt_query_single = t_query_single.elapsed().as_millis() as u64;

            let valid_gmse = gmse_refno.as_ref().map(|r| r.is_valid()).unwrap_or(false);
            if !valid_gmse {
                let ngmr_refno =
                    aios_core::query_single_by_paths(cata_refno, &["->NGMR"], &["REFNO"])
                        .await
                        .map(|x| x.get_refno_or_default());
                let valid_ngmr = ngmr_refno.as_ref().map(|r| r.is_valid()).unwrap_or(false);
                if !valid_ngmr {
                    return Ok(());
                }
            }

            let csg_shapes_map = CateCsgShapeMap::new();
            let design_axis_map = DashMap::new();

            #[cfg(feature = "profile")]
            let t_get_named_attmap = Instant::now();
            let _desi_att = match aios_core::get_named_attmap(ele_refno).await {
                Ok(att) => att,
                Err(_) => return Ok(()),
            };
            #[cfg(feature = "profile")]
            let dt_attmap = t_get_named_attmap.elapsed().as_millis() as u64;

            #[cfg(feature = "profile")]
            let t_gen_single_geoms = Instant::now();
            let r = gen_cata_single_geoms(ele_refno, &csg_shapes_map, &design_axis_map).await;
            #[cfg(feature = "profile")]
            let dt_gen = t_gen_single_geoms.elapsed().as_millis() as u64;

            if r.is_err() {
                return Ok(());
            }

            // 汇总计时
            #[cfg(feature = "profile")]
            {
                *time_stats.entry("get_cat_refno".to_string()).or_insert(0) += dt_cat_refno;
                *time_stats.entry("query_single".to_string()).or_insert(0) += dt_query_single;
                *time_stats
                    .entry("get_named_attmap".to_string())
                    .or_insert(0) += dt_attmap;
                *time_stats
                    .entry("gen_single_geoms".to_string())
                    .or_insert(0) += dt_gen;
            }

            let ptset_map: BTreeMap<i32, CateAxisParam> = design_axis_map
                .get(&ele_refno)
                .map(|v| v.clone())
                .unwrap_or_default();

            let shapes: Vec<CateCsgShape> = csg_shapes_map
                .get(&ele_refno)
                .map(|v| v.clone())
                .unwrap_or_default();

            let (geos, has_solid) = build_prepared_inst_geos_from_shapes(shapes);
            let resolved_comp = CataResolvedComp {
                created_at: Utc::now().timestamp_millis(),
                ptset_items: ptset_map.into_iter().collect(),
                geos,
                has_solid,
            };

            // 构建 geo_insts
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

            let ptset_map_resolved = resolved_comp.ptset_map();

            // 处理 group_refnos 中的每个元件
            let mut shape_insts_data = ShapeInstancesData::default();
            for &group_refno in target_group_refnos.iter() {
                let ele_att = match aios_core::get_named_attmap(group_refno).await {
                    Ok(att) => att,
                    Err(_) => continue,
                };
                let type_name = ele_att.get_type_str().to_string();

                let (owner_refno, owner_type) = shared::get_owner_info_from_attr(&ele_att).await;
                let cata_hash_for_info = if is_valid_cata_hash(&cata_hash) {
                    Some(cata_hash.clone())
                } else {
                    None
                };

                let geos_info = EleGeosInfo {
                    refno: group_refno,
                    sesno: ele_att.sesno(),
                    owner_refno,
                    owner_type,
                    cata_hash: cata_hash_for_info.clone(),
                    visible: true,
                    ptset_map: ptset_map_resolved.clone(),
                    is_solid: has_solid,
                    ..Default::default()
                };

                shape_insts_data.insert_info(group_refno, geos_info.clone());

                if !geo_insts.is_empty() {
                    let inst_key = geos_info.get_inst_key();
                    let geos_data = EleInstGeosData {
                        inst_key: inst_key.clone(),
                        refno: group_refno,
                        insts: geo_insts.clone(),
                        aabb: None,
                        type_name,
                        ..Default::default()
                    };
                    shape_insts_data.insert_geos_data(inst_key, geos_data);
                }
            }

            if shape_insts_data.inst_cnt() > 0 {
                let _ = sender.send(shape_insts_data);
            }

            Ok::<_, anyhow::Error>(())
        });
    }

    // 等待所有并行任务完成
    while let Some(result) = join_set.join_next().await {
        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => eprintln!("[gen_cata_geos_for_cache] task error: {}", e),
            Err(e) => eprintln!("[gen_cata_geos_for_cache] task panic: {}", e),
        }
    }

    #[cfg(feature = "profile")]
    let time_stats: HashMap<String, u64> = total_time_stats
        .iter()
        .map(|e| (e.key().clone(), *e.value()))
        .collect();
    #[cfg(not(feature = "profile"))]
    let time_stats: HashMap<String, u64> = HashMap::new();

    Ok(SimpleCataOutcome {
        time_stats,
        unique_cata_cnt,
        elapsed_ms: total_t.elapsed().as_millis(),
    })
}

/// model cache 专用：仅生成 BRAN/HANG 几何体，不生成 tubi
///
/// 与 `gen_branch_tubi` 的区别：
/// - 只处理 BRAN/HANG 的基本几何信息
/// - 不生成 tubi 管道
/// - 不创建 tubi_relate
pub async fn gen_bran_geos_for_cache(
    db_option: Arc<DbOptionExt>,
    branch_map: Arc<DashMap<RefnoEnum, Vec<SPdmsElement>>>,
    _sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32)>>,
    sender: flume::Sender<ShapeInstancesData>,
) -> anyhow::Result<SimpleBranOutcome> {
    let total_t = Instant::now();
    #[cfg(feature = "profile")]
    let total_time_stats = Arc::new(Mutex::new(HashMap::new()));

    let bran_count = branch_map.len();
    debug_model_debug!("[gen_bran_geos_for_cache] start: bran_count={}", bran_count);

    if branch_map.is_empty() {
        return Ok(SimpleBranOutcome {
            time_stats: HashMap::new(),
            bran_count: 0,
            elapsed_ms: total_t.elapsed().as_millis(),
        });
    }

    let mut shape_insts_data = ShapeInstancesData::default();
    shape_insts_data.fill_basic_shapes();

    #[cfg(feature = "profile")]
    let mut db_time_get_branch_att: u128 = 0;
    #[cfg(feature = "profile")]
    let mut db_time_get_branch_transform: u128 = 0;
    #[cfg(feature = "profile")]
    let mut db_time_get_children_att: u128 = 0;

    for bran_data in branch_map.iter() {
        let branch_refno = *bran_data.key();
        let children = bran_data.value();

        debug_model_debug!(
            "[gen_bran_geos_for_cache] processing BRAN/HANG: refno={}, children_len={}",
            branch_refno.to_string(),
            children.len()
        );

        // 获取 BRAN/HANG 属性
        #[cfg(feature = "profile")]
        let t_get_branch_att = Instant::now();
        let branch_att = match aios_core::get_named_attmap(branch_refno).await {
            Ok(att) => att,
            Err(e) => {
                record_refno_error(
                    RefnoErrorKind::NotFound,
                    RefnoErrorStage::Query,
                    "fast_model/cata_cache_gen.rs",
                    "get_named_attmap",
                    format!("BRAN/HANG 获取属性失败: {}", e),
                    Some(&branch_refno),
                    None,
                    &[],
                    None,
                );
                continue;
            }
        };
        #[cfg(feature = "profile")]
        {
            db_time_get_branch_att += t_get_branch_att.elapsed().as_millis();
        }

        // 获取 world_transform
        #[cfg(feature = "profile")]
        let t_get_branch_transform = Instant::now();
        let _branch_transform =
            match crate::fast_model::transform_cache::get_world_transform_cache_first(
                Some(db_option.as_ref()),
                branch_refno,
            )
            .await
            {
                Ok(Some(t)) => t,
                Ok(None) => {
                    record_refno_error(
                        RefnoErrorKind::Missing,
                        RefnoErrorStage::Query,
                        "fast_model/cata_cache_gen.rs",
                        "get_world_transform_cache_first",
                        "BRAN/HANG world_transform 为空",
                        Some(&branch_refno),
                        None,
                        &[],
                        None,
                    );
                    continue;
                }
                Err(e) => {
                    record_refno_error(
                        RefnoErrorKind::NotFound,
                        RefnoErrorStage::Query,
                        "fast_model/cata_cache_gen.rs",
                        "get_world_transform_cache_first",
                        format!("BRAN/HANG 获取 world_transform 失败: {}", e),
                        Some(&branch_refno),
                        None,
                        &[],
                        None,
                    );
                    continue;
                }
            };
        #[cfg(feature = "profile")]
        {
            db_time_get_branch_transform += t_get_branch_transform.elapsed().as_millis();
        }

        // 处理 BRAN/HANG 本身的几何信息
        let (owner_refno, owner_type) = shared::get_owner_info_from_attr(&branch_att).await;

        let bran_geos_info = EleGeosInfo {
            refno: branch_refno,
            sesno: branch_att.sesno(),
            owner_refno,
            owner_type,
            cata_hash: None,
            visible: true,
            is_solid: true,
            ..Default::default()
        };

        shape_insts_data.insert_info(branch_refno, bran_geos_info);

        // 处理子元件（不生成 tubi）
        for child in children.iter() {
            let child_refno = child.refno;

            #[cfg(feature = "profile")]
            let t_get_child_att = Instant::now();
            let child_att = match aios_core::get_named_attmap(child_refno).await {
                Ok(att) => att,
                Err(_) => continue,
            };
            #[cfg(feature = "profile")]
            {
                db_time_get_children_att += t_get_child_att.elapsed().as_millis();
            }

            let (child_owner_refno, child_owner_type) =
                shared::get_owner_info_from_attr(&child_att).await;

            let child_geos_info = EleGeosInfo {
                refno: child_refno,
                sesno: child_att.sesno(),
                owner_refno: child_owner_refno,
                owner_type: child_owner_type,
                cata_hash: None,
                visible: true,
                is_solid: true,
                ..Default::default()
            };

            shape_insts_data.insert_info(child_refno, child_geos_info);
        }

        if shape_insts_data.inst_cnt() >= SEND_INST_SIZE {
            sender
                .send(std::mem::take(&mut shape_insts_data))
                .expect("send bran shape_insts_data error");
            shape_insts_data.fill_basic_shapes();
        }
    }

    // 更新时间统计
    #[cfg(feature = "profile")]
    {
        let mut stats = total_time_stats.lock().await;
        *stats.entry("get_branch_att".to_string()).or_insert(0) += db_time_get_branch_att as u64;
        *stats.entry("get_branch_transform".to_string()).or_insert(0) +=
            db_time_get_branch_transform as u64;
        *stats.entry("get_children_att".to_string()).or_insert(0) +=
            db_time_get_children_att as u64;
    }

    if shape_insts_data.inst_cnt() > 0 {
        sender
            .send(shape_insts_data)
            .expect("send bran shape_insts_data error");
    }

    #[cfg(feature = "profile")]
    let time_stats = total_time_stats.lock().await.clone();
    #[cfg(not(feature = "profile"))]
    let time_stats: HashMap<String, u64> = HashMap::new();
    Ok(SimpleBranOutcome {
        time_stats,
        bran_count,
        elapsed_ms: total_t.elapsed().as_millis(),
    })
}

/// model cache 专用：在所有 BRAN 模型生成完成后，单独遍历 BRAN 生成 tubi
///
/// 从 model cache 获取点数据来生成 tubi（不依赖 SurrealDB）：
/// 1. 使用 `InstanceCacheManager::get_ptset_maps_for_refnos` 获取 ARRIVE/LEAVE 点
/// 2. 遍历 BRAN/HANG 的子元件，基于 cache 中的点数据生成 tubi
pub async fn gen_tubi_for_cache(
    db_option: Arc<DbOptionExt>,
    branch_refnos: &[RefnoEnum],
    sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32)>>,
    sender: flume::Sender<ShapeInstancesData>,
) -> anyhow::Result<BranchTubiOutcome> {
    // 兼容入口：内部自建 InstanceCacheManager。
    // 若调用方（如 orchestrator）已持有缓存管理器，请优先使用 `gen_tubi_for_cache_with_cache_manager`，
    // 以避免重复打开/加载 index。
    let cache_dir = db_option.get_model_cache_dir();
    let cache_manager = InstanceCacheManager::new(&cache_dir).await?;
    gen_tubi_for_cache_with_cache_manager(
        db_option,
        branch_refnos,
        sjus_map_arc,
        sender,
        &cache_manager,
    )
    .await
}

/// 同 [`gen_tubi_for_cache`]，但复用外部提供的 `InstanceCacheManager`。
pub async fn gen_tubi_for_cache_with_cache_manager(
    db_option: Arc<DbOptionExt>,
    branch_refnos: &[RefnoEnum],
    sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32)>>,
    sender: flume::Sender<ShapeInstancesData>,
    cache_manager: &InstanceCacheManager,
) -> anyhow::Result<BranchTubiOutcome> {
    let start_time = Instant::now();

    if branch_refnos.is_empty() {
        return Ok(BranchTubiOutcome {
            tubi_relates: vec![],
            tubi_refnos: vec![],
            time_stats: HashMap::new(),
            tubi_count: 0,
            elapsed_ms: 0,
        });
    }

    debug_model!(
        "[gen_tubi_for_cache] 开始处理 {} 个 BRAN/HANG",
        branch_refnos.len()
    );

    // 1. 收集所有 BRAN/HANG 下的子元件 refno
    let mut all_child_refnos: Vec<RefnoEnum> = Vec::new();
    let branch_map: DashMap<RefnoEnum, Vec<SPdmsElement>> = DashMap::new();

    for &branch_refno in branch_refnos {
        match crate::fast_model::gen_model::tree_index_manager::TreeIndexManager
            ::collect_children_elements_from_tree(branch_refno).await
        {
            Ok(children) => {
                for child in &children {
                    all_child_refnos.push(child.refno);
                }
                if !children.is_empty() {
                    branch_map.insert(branch_refno, children);
                }
            }
            Err(e) => {
                debug_model!(
                    "[gen_tubi_for_cache] TreeIndex 查询子元件失败: {} - {}",
                    branch_refno,
                    e
                );
            }
        }
    }

    // 2. 从 model cache 获取 ARRIVE/LEAVE 点数据
    let local_al_map: Arc<DashMap<RefnoEnum, [CateAxisParam; 2]>> = Arc::new({
        let hm = cache_manager
            .get_ptset_maps_for_refnos_auto(&all_child_refnos)
            .await;
        let dm = DashMap::new();
        for (k, btree) in hm {
            let items: Vec<CateAxisParam> = btree.into_values().collect();
            if items.len() >= 2 {
                dm.insert(k, [items[0].clone(), items[1].clone()]);
            }
        }
        dm
    });

    debug_model!(
        "[gen_tubi_for_cache] 从 cache 获取到 {} 个元件的 arrive/leave 点",
        local_al_map.len()
    );

    // 3. 调用现有的 gen_branch_tubi 逻辑
    let outcome = crate::fast_model::cata_model::gen_branch_tubi(
        db_option,
        Arc::new(branch_map),
        sjus_map_arc,
        sender,
        local_al_map,
    )
    .await?;

    let elapsed = start_time.elapsed().as_millis();
    debug_model!(
        "[gen_tubi_for_cache] 完成，生成 {} 条 tubi，耗时 {} ms",
        outcome.tubi_count,
        elapsed
    );

    Ok(outcome)
}
