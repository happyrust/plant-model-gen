//! foyer cache 专用的几何生成模块
//!
//! 将 CATA、BRAN/HANG、tubi 三个阶段分离：
//! 1. `gen_cata_geos_for_cache` - CATA 几何生成，不收集 tubi 相关信息
//! 2. `gen_bran_geos_for_cache` - BRAN/HANG 几何生成，不生成 tubi
//! 3. `gen_tubi_for_cache` - 在所有 BRAN 模型生成完成后单独遍历生成

use crate::fast_model::gen_model::cate_single::{CateCsgShapeMap, gen_cata_single_geoms};
use crate::fast_model::gen_model::utilities::is_valid_cata_hash;
use crate::fast_model::instance_cache::InstanceCacheManager;
use crate::fast_model::refno_errors::{RefnoErrorKind, RefnoErrorStage, record_refno_error};
use crate::fast_model::{SEND_INST_SIZE, get_generic_type, shared};
use crate::fast_model::{debug_model, debug_model_debug};
use crate::options::DbOptionExt;
use aios_core::parsed_data::CateAxisParam;
use aios_core::parsed_data::geo_params_data::PdmsGeoParam;
use aios_core::pe::SPdmsElement;
use aios_core::pdms_types::CataHashRefnoKV;
use aios_core::geometry::{EleGeosInfo, EleInstGeo, EleInstGeosData, GeoBasicType, ShapeInstancesData};
use aios_core::RefnoEnum;
use aios_core::prim_geo::category::CateCsgShape;
use bevy_transform::components::Transform;
use dashmap::DashMap;
use glam::Vec3;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;

use crate::fast_model::cata_model::BranchTubiOutcome;

/// foyer cache 专用的简化 CATA 输出结构
#[derive(Debug, Default)]
pub struct SimpleCataOutcome {
    pub time_stats: HashMap<String, u64>,
    pub unique_cata_cnt: usize,
    pub elapsed_ms: u128,
}

/// foyer cache 专用的简化 BRAN 输出结构
#[derive(Debug, Default)]
pub struct SimpleBranOutcome {
    pub time_stats: HashMap<String, u64>,
    pub bran_count: usize,
    pub elapsed_ms: u128,
}

/// foyer cache 专用：仅生成 CATA 几何体，不收集 tubi 相关信息
///
/// 与 `gen_cata_instances` 的区别：
/// - 不创建 `tubi_info_map`
/// - 不收集 `local_al_map`
/// - 不设置 `geos_info.tubi_info_id`
pub async fn gen_cata_geos_for_cache(
    db_option: Arc<DbOptionExt>,
    target_cata_map: Arc<DashMap<String, CataHashRefnoKV>>,
    sjus_map_arc: Arc<DashMap<RefnoEnum, (Vec3, f32)>>,
    sender: flume::Sender<ShapeInstancesData>,
) -> anyhow::Result<SimpleCataOutcome> {
    let total_t = Instant::now();
    let total_time_stats = Arc::new(Mutex::new(HashMap::new()));

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

    // 计算批次
    let mut batch_chunks_cnt = 4usize.min(unique_cata_cnt.max(1));
    let mut batch_size = (unique_cata_cnt + batch_chunks_cnt - 1) / batch_chunks_cnt;
    if batch_size == 0 {
        batch_size = 1;
    }
    if batch_size == 1 {
        batch_chunks_cnt = unique_cata_cnt;
    } else {
        batch_chunks_cnt = (unique_cata_cnt + batch_size - 1) / batch_size;
    }

    let replace_exist = db_option.inner.is_replace_mesh();

    for i in 0..batch_chunks_cnt {
        let start_idx = i * batch_size;
        if start_idx >= unique_cata_cnt {
            continue;
        }
        let end_idx = (start_idx + batch_size).min(unique_cata_cnt);

        let mut shape_insts_data = ShapeInstancesData::default();
        let mut db_time_get_named_attmap: u128 = 0;
        let mut db_time_get_cat_refno: u128 = 0;
        let mut db_time_query_single: u128 = 0;
        let mut db_time_gen_single_geoms: u128 = 0;
        let mut db_time_get_generic_type: u128 = 0;

        for j in start_idx..end_idx {
            let cata_hash = all_unique_keys[j].clone();
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

            let force_regen_cata = replace_exist;

            // 复用路径：inst_info 已存在
            if target_exist_inst && !force_regen_cata {
                debug_model_debug!(
                    "[gen_cata_geos_for_cache][cata_hash={}] reuse existing inst_info",
                    cata_hash
                );

                let reuse_ptset_map = target_ptset.clone().unwrap_or_default();

                for &ele_refno in target_group_refnos.iter() {
                    let ele_att = match aios_core::get_named_attmap(ele_refno).await {
                        Ok(att) => att,
                        Err(_) => continue,
                    };

                    let (owner_refno, owner_type) =
                        shared::get_owner_info_from_attr(&ele_att).await;
                    let generic_type = get_generic_type(ele_refno).await.unwrap_or_default();
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
                        generic_type,
                        ptset_map: reuse_ptset_map.clone(),
                        is_solid: true,
                        ..Default::default()
                    };

                    shape_insts_data.insert_info(ele_refno, geos_info);
                    if shape_insts_data.inst_cnt() >= SEND_INST_SIZE {
                        sender
                            .send(std::mem::take(&mut shape_insts_data))
                            .expect("send cate shape_insts_data error");
                    }
                }
                continue;
            }

            // 需要生成元件库几何
            let ele_refno = match target_group_refnos.first().copied() {
                Some(r) => r,
                None => continue,
            };

            let t_get_cat_refno = Instant::now();
            let result = aios_core::get_cat_refno(ele_refno).await;
            let cata_refno = match result {
                Ok(Some(refno)) => refno,
                _ => continue,
            };
            db_time_get_cat_refno += t_get_cat_refno.elapsed().as_millis();

            let t_query_single = Instant::now();
            let gmse_refno = aios_core::query_single_by_paths(
                cata_refno,
                &["->GMRE", "->GSTR"],
                &["REFNO"],
            )
            .await
            .map(|x| x.get_refno_or_default());
            db_time_query_single += t_query_single.elapsed().as_millis();

            let valid_gmse = gmse_refno.as_ref().map(|r| r.is_valid()).unwrap_or(false);
            if !valid_gmse {
                let ngmr_refno =
                    aios_core::query_single_by_paths(cata_refno, &["->NGMR"], &["REFNO"])
                        .await
                        .map(|x| x.get_refno_or_default());
                let valid_ngmr = ngmr_refno.as_ref().map(|r| r.is_valid()).unwrap_or(false);
                if !valid_ngmr {
                    continue;
                }
            }

            let csg_shapes_map = CateCsgShapeMap::new();
            let design_axis_map = DashMap::new();

            let t_get_named_attmap = Instant::now();
            let _desi_att = match aios_core::get_named_attmap(ele_refno).await {
                Ok(att) => att,
                Err(_) => continue,
            };
            db_time_get_named_attmap += t_get_named_attmap.elapsed().as_millis();

            let t_gen_single_geoms = Instant::now();
            let r = gen_cata_single_geoms(ele_refno, &csg_shapes_map, &design_axis_map).await;
            db_time_gen_single_geoms += t_gen_single_geoms.elapsed().as_millis();

            if r.is_err() {
                continue;
            }

            // 从 design_axis_map 获取 ptset_map
            let ptset_map: BTreeMap<i32, CateAxisParam> = design_axis_map
                .get(&ele_refno)
                .map(|v| v.clone())
                .unwrap_or_default();

            // ====== 关键修复：处理 csg_shapes_map 创建 EleInstGeo ======
            // 从 csg_shapes_map 获取当前元件的 shapes
            let shapes: Vec<CateCsgShape> = csg_shapes_map
                .get(&ele_refno)
                .map(|v| v.clone())
                .unwrap_or_default();

            let mut geo_insts: Vec<EleInstGeo> = Vec::new();
            let mut has_solid = false;

            for shape in shapes.into_iter() {
                let CateCsgShape {
                    refno: geom_refno,
                    csg_shape,
                    transform: shape_transform,
                    visible,
                    is_tubi,
                    pts,
                    is_ngmr,
                    ..
                } = shape;

                if !csg_shape.check_valid() || !visible {
                    continue;
                }

                // 获取形状自身的变换（包含 scale）
                let shape_trans = csg_shape.get_trans();
                let geo_hash = csg_shape.hash_unit_mesh_params();
                let unit_flag = csg_shape.is_reuse_unit();

                // 合并变换：shape_transform 是元件变换，shape_trans 是形状自身变换
                let translation = shape_transform.translation
                    + shape_transform.rotation * shape_trans.translation;
                let rotation = shape_transform.rotation;
                let scale = shape_trans.scale;

                let mut transform = Transform {
                    translation,
                    rotation,
                    scale,
                };

                if transform.translation.is_nan()
                    || transform.rotation.is_nan()
                    || transform.scale.is_nan()
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

                let geom_inst = EleInstGeo {
                    geo_hash,
                    refno: geom_refno,
                    pts,
                    aabb: None,
                    geo_transform: transform,
                    geo_param,
                    visible: geo_type == GeoBasicType::Pos,
                    is_tubi,
                    geo_type,
                    cata_neg_refnos: vec![],
                };

                geo_insts.push(geom_inst);
            }

            // 处理 group_refnos 中的每个元件
            for &group_refno in target_group_refnos.iter() {
                let t_get_generic_type = Instant::now();
                let generic_type = get_generic_type(group_refno).await.unwrap_or_default();
                db_time_get_generic_type += t_get_generic_type.elapsed().as_millis();

                let ele_att = match aios_core::get_named_attmap(group_refno).await {
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

                let mut geos_info = EleGeosInfo {
                    refno: group_refno,
                    sesno: ele_att.sesno(),
                    owner_refno,
                    owner_type,
                    cata_hash: cata_hash_for_info.clone(),
                    visible: true,
                    generic_type: generic_type.clone(),
                    ptset_map: ptset_map.clone(),
                    is_solid: has_solid,
                    ..Default::default()
                };

                // 插入 EleGeosInfo
                shape_insts_data.insert_info(group_refno, geos_info.clone());

                // ====== 关键：插入 EleInstGeosData（包含几何实例和变换） ======
                if !geo_insts.is_empty() {
                    let inst_key = geos_info.get_inst_key();
                    let geos_data = EleInstGeosData {
                        inst_key: inst_key.clone(),
                        refno: group_refno,
                        insts: geo_insts.clone(),
                        aabb: None,
                        type_name: generic_type.to_string(),
                        ..Default::default()
                    };
                    shape_insts_data.insert_geos_data(inst_key, geos_data);
                }
            }

            if shape_insts_data.inst_cnt() >= SEND_INST_SIZE {
                sender
                    .send(std::mem::take(&mut shape_insts_data))
                    .expect("send cate shape_insts_data error");
            }
        }

        // 更新时间统计
        {
            let mut stats = total_time_stats.lock().await;
            *stats.entry("get_named_attmap".to_string()).or_insert(0) +=
                db_time_get_named_attmap as u64;
            *stats.entry("get_cat_refno".to_string()).or_insert(0) +=
                db_time_get_cat_refno as u64;
            *stats.entry("query_single".to_string()).or_insert(0) +=
                db_time_query_single as u64;
            *stats.entry("gen_single_geoms".to_string()).or_insert(0) +=
                db_time_gen_single_geoms as u64;
            *stats.entry("get_generic_type".to_string()).or_insert(0) +=
                db_time_get_generic_type as u64;
        }

        if shape_insts_data.inst_cnt() > 0 {
            sender
                .send(shape_insts_data)
                .expect("send cate shape_insts_data error");
        }
    }

    let time_stats = total_time_stats.lock().await.clone();
    Ok(SimpleCataOutcome {
        time_stats,
        unique_cata_cnt,
        elapsed_ms: total_t.elapsed().as_millis(),
    })
}

/// foyer cache 专用：仅生成 BRAN/HANG 几何体，不生成 tubi
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
    let total_time_stats = Arc::new(Mutex::new(HashMap::new()));

    let bran_count = branch_map.len();
    debug_model_debug!(
        "[gen_bran_geos_for_cache] start: bran_count={}",
        bran_count
    );

    if branch_map.is_empty() {
        return Ok(SimpleBranOutcome {
            time_stats: HashMap::new(),
            bran_count: 0,
            elapsed_ms: total_t.elapsed().as_millis(),
        });
    }

    let mut shape_insts_data = ShapeInstancesData::default();
    shape_insts_data.fill_basic_shapes();

    let mut db_time_get_branch_att: u128 = 0;
    let mut db_time_get_branch_transform: u128 = 0;
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
        db_time_get_branch_att += t_get_branch_att.elapsed().as_millis();

        // 获取 world_transform
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
        db_time_get_branch_transform += t_get_branch_transform.elapsed().as_millis();

        // 处理 BRAN/HANG 本身的几何信息
        let (owner_refno, owner_type) = shared::get_owner_info_from_attr(&branch_att).await;
        let generic_type = get_generic_type(branch_refno).await.unwrap_or_default();

        let bran_geos_info = EleGeosInfo {
            refno: branch_refno,
            sesno: branch_att.sesno(),
            owner_refno,
            owner_type,
            cata_hash: None,
            visible: true,
            generic_type,
            is_solid: true,
            ..Default::default()
        };

        shape_insts_data.insert_info(branch_refno, bran_geos_info);

        // 处理子元件（不生成 tubi）
        for child in children.iter() {
            let child_refno = child.refno;

            let t_get_child_att = Instant::now();
            let child_att = match aios_core::get_named_attmap(child_refno).await {
                Ok(att) => att,
                Err(_) => continue,
            };
            db_time_get_children_att += t_get_child_att.elapsed().as_millis();

            let (child_owner_refno, child_owner_type) =
                shared::get_owner_info_from_attr(&child_att).await;
            let child_generic_type = get_generic_type(child_refno).await.unwrap_or_default();

            let child_geos_info = EleGeosInfo {
                refno: child_refno,
                sesno: child_att.sesno(),
                owner_refno: child_owner_refno,
                owner_type: child_owner_type,
                cata_hash: None,
                visible: true,
                generic_type: child_generic_type,
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
    {
        let mut stats = total_time_stats.lock().await;
        *stats.entry("get_branch_att".to_string()).or_insert(0) +=
            db_time_get_branch_att as u64;
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

    let time_stats = total_time_stats.lock().await.clone();
    Ok(SimpleBranOutcome {
        time_stats,
        bran_count,
        elapsed_ms: total_t.elapsed().as_millis(),
    })
}

/// foyer cache 专用：在所有 BRAN 模型生成完成后，单独遍历 BRAN 生成 tubi
///
/// 从 foyer cache 获取点数据来生成 tubi（不依赖 SurrealDB）：
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
    let cache_dir = db_option.get_foyer_cache_dir();
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

    // 2. 从 foyer cache 获取 ARRIVE/LEAVE 点数据
    let local_al_map: Arc<DashMap<RefnoEnum, [CateAxisParam; 2]>> = Arc::new(
        cache_manager
            .get_ptset_maps_for_refnos_auto(&all_child_refnos)
            .await
            .into_iter()
            .collect(),
    );

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
